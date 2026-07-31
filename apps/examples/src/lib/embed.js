// In-browser EmbeddingGemma, a drop-in for the daemon's `embedText`, with a
// static cache-through tier.
//
// The admin console fetches vectors from the vrules-shim daemon over REST. This
// static app has no daemon: it resolves a vector in three stages, cheapest
// first, and only ever downloads the 226 MB model if it actually has to compute.
//
//   1. Cache API  — vectors this browser previously fetched or computed.
//   2. Bucket     — a static, content-addressed cache seeded offline for known
//                   inputs (the example corpora), served under the same REST
//                   path a daemon would use: `vrules-rest/v1/embeddings/...`.
//   3. wllama     — compute in the browser (WebGPU when an adapter exists, else
//                   CPU), matching the daemon component's mean pooling + L2 norm.
//
// The returned `{ info, vector }` shape is identical to the console's `embedText`,
// so the example components are unchanged apart from importing this module.

import { Wllama } from 'wllama';
import wasmSingleThread from 'wllama-wasm?url';
import { textHash, vectorFromLeBytes } from './cache-key.js';

const MODEL_DIR = new URL('models/', document.baseURI);
const MANIFEST_URL = new URL('model.json', MODEL_DIR);
const CACHE_NAME = 'vrules-embed-v1';

// --- backend reporting -----------------------------------------------------

let backend = 'unknown'; // 'webgpu' | 'cpu' | 'unknown' (no compute yet)

/** Report the embedder's compute backend and the browser's WebGPU capability. */
export async function getBackendInfo() {
  let adapter = null;
  if (navigator.gpu) {
    try {
      const a = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
      adapter = a ? (a.info?.description || a.info?.vendor || 'available') : null;
    } catch {
      adapter = null;
    }
  }
  return { webgpuApi: !!navigator.gpu, adapter, backend };
}

// --- observable model state -------------------------------------------------
//
// A seeded vector is a file read; a computed one runs the real model. That
// difference is the whole claim, so it is reported rather than hidden: the model
// state drives an explicit notice before the download, byte progress during it,
// and a persistent model identity afterwards.

const modelState = {
  // 'absent'      — no compute attempted; every vector so far came from cache
  // 'starting'    — model requested, transfer not yet characterized
  // 'downloading' — bytes moving over the network
  // 'preparing'   — bytes resident, ggml loading them
  // 'ready'       — inference available in this tab
  // 'error'
  phase: 'absent',
  loaded: 0,
  total: 0,
  fromCache: false,
  model: null,
  revision: null,
  bytes: null,
  error: null
};

const modelWatchers = new Set();

function emitModel() {
  const snapshot = { ...modelState };
  for (const fn of modelWatchers) fn(snapshot);
}

/** Subscribe to model download/load state. Fires immediately with the current value. */
export function subscribeModel(fn) {
  modelWatchers.add(fn);
  fn({ ...modelState });
  return () => modelWatchers.delete(fn);
}

// --- observable resolution source -------------------------------------------

const resolution = { seeded: 0, computed: 0, memory: 0, last: null };
const resolutionWatchers = new Set();

function emitResolution(source, text) {
  resolution[source] += 1;
  resolution.last = { source, text, at: Date.now() };
  const snapshot = { ...resolution };
  for (const fn of resolutionWatchers) fn(snapshot);
}

/** Subscribe to per-embedding provenance: served from cache, or computed here. */
export function subscribeResolution(fn) {
  resolutionWatchers.add(fn);
  fn({ ...resolution });
  return () => resolutionWatchers.delete(fn);
}


// Drops llama.cpp's per-eval DEBUG flood but keeps warnings, errors, and the
// `ggml_webgpu:` line that reveals whether the GPU backend engaged.
const filteringLogger = {
  debug: () => {},
  log: (message, ...rest) => {
    const text = String(message ?? '');
    if (/ggml_webgpu/i.test(text)) {
      console.log('[vrules-embed]', text, ...rest);
      backend = /failed|not available|no adapter/i.test(text) ? 'cpu' : 'webgpu';
    }
  },
  warn: (message, ...rest) => console.warn('[vrules-embed]', message, ...rest),
  error: (message, ...rest) => console.error('[vrules-embed]', message, ...rest)
};

// --- manifest (identity; small, always needed) -----------------------------

let manifestPromise = null;

function ensureManifest() {
  if (!manifestPromise) {
    manifestPromise = (async () => {
      const manifest = await fetch(MANIFEST_URL).then((r) => {
        if (!r.ok) throw new Error(`model manifest unavailable: HTTP ${r.status}`);
        return r.json();
      });
      const canon = manifest.canon || 'default-v1';
      // Content-addressed cache root, mirroring the daemon's REST tier path.
      const cacheBase = new URL(
        `vrules-rest/v1/embeddings/${manifest.name}/${canon}/`,
        document.baseURI
      );
      return {
        info: { model: manifest.name, revision: manifest.sha256, dimensions: manifest.dimensions },
        file: manifest.file,
        bytes: manifest.bytes ?? null,
        cacheBase
      };
    })().catch((e) => {
      manifestPromise = null;
      throw e;
    });
  }
  return manifestPromise;
}

// --- model (heavy; loaded lazily, only to compute a cache miss) -------------

let modelPromise = null;

function ensureModel() {
  if (!modelPromise) {
    modelPromise = (async () => {
      const { file, bytes, info } = await ensureManifest();
      modelState.phase = 'starting';
      modelState.model = info.model;
      modelState.revision = info.revision;
      modelState.bytes = bytes;
      modelState.loaded = 0;
      modelState.total = bytes ?? 0;
      modelState.error = null;
      emitModel();

      const wllama = new Wllama({ default: wasmSingleThread }, { logger: filteringLogger });

      // An already-cached model reports complete on its first callback, so the
      // first event distinguishes a network transfer from an OPFS cache hit.
      let firstProgress = true;
      const progressCallback = ({ loaded, total }) => {
        if (firstProgress) {
          firstProgress = false;
          modelState.fromCache = total > 0 && loaded >= total;
          modelState.phase = modelState.fromCache ? 'preparing' : 'downloading';
        } else if (modelState.phase === 'starting') {
          modelState.phase = 'downloading';
        }
        modelState.loaded = loaded;
        modelState.total = total || modelState.total;
        if (!modelState.fromCache && total > 0 && loaded >= total) {
          modelState.phase = 'preparing';
        }
        emitModel();
      };

      // Leave pooling_type unset: the GGUF declares its own
      // (`gemma-embedding.pooling_type`). n_gpu_layers offloads every layer to
      // WebGPU when an adapter is available (wllama's default is already high;
      // set explicitly to document the intent).
      await wllama.loadModelFromUrl(new URL(file, MODEL_DIR).href, {
        embeddings: true,
        n_gpu_layers: 999,
        progressCallback
      });
      modelState.phase = 'ready';
      emitModel();
      return wllama;
    })().catch((e) => {
      modelPromise = null;
      modelState.phase = 'error';
      modelState.error = e?.message || String(e);
      emitModel();
      throw e;
    });
  }
  return modelPromise;
}

// The single wllama worker cannot service concurrent createEmbedding calls (they
// race on its result stream and yield null), so serialize compute through one
// promise chain. Cache and bucket hits are not serialized.
let queue = Promise.resolve();

function computeSerialized(text) {
  const run = queue.then(async () => {
    const wllama = await ensureModel();
    const response = await wllama.createEmbedding({ input: text });
    const raw = response?.data?.[0]?.embedding;
    if (!raw || raw.length === 0) throw new Error('model returned no embedding vector');
    return l2normalize(Array.from(raw, Number));
  });
  queue = run.then(() => undefined, () => undefined);
  return run;
}

// --- cache tier (Cache API) ------------------------------------------------

function cacheApi() {
  return typeof caches !== 'undefined' ? caches.open(CACHE_NAME) : null;
}

async function cacheMatch(url) {
  const store = cacheApi();
  if (!store) return null;
  try {
    const hit = await (await store).match(url);
    if (!hit) return null;
    return vectorFromLeBytes(await hit.arrayBuffer());
  } catch {
    return null;
  }
}

async function cachePut(url, vector) {
  const store = cacheApi();
  if (!store) return;
  try {
    // Little-endian f32 body, matching the daemon wire format and the seed script.
    await (await store).put(url, new Response(new Float32Array(vector).buffer));
  } catch {
    /* cache is best-effort */
  }
}

async function fetchSeeded(url) {
  try {
    const resp = await fetch(url);
    if (!resp.ok) return null;
    return vectorFromLeBytes(await resp.arrayBuffer());
  } catch {
    return null;
  }
}

// --- helpers ---------------------------------------------------------------

function l2normalize(vector) {
  let sum = 0;
  for (const value of vector) sum += value * value;
  const norm = Math.sqrt(sum);
  if (!Number.isFinite(norm) || norm === 0) {
    throw new Error('model returned an invalid embedding norm');
  }
  for (let i = 0; i < vector.length; i++) vector[i] /= norm;
  return vector;
}

// --- public API ------------------------------------------------------------

/**
 * Embed `text` in the browser via the cache-through tier. Returns
 * `{ info: { model, revision, dimensions }, vector }` — the contract the
 * console's `embedText` returned from the daemon.
 */
export async function embedText(text) {
  const { info, cacheBase } = await ensureManifest();
  const url = new URL(textHash(text), cacheBase).href;

  const cached = await cacheMatch(url);
  if (cached) {
    emitResolution('memory', text);
    return { info, vector: cached };
  }

  const seeded = await fetchSeeded(url);
  if (seeded) {
    await cachePut(url, seeded);
    emitResolution('seeded', text);
    return { info, vector: seeded };
  }

  const vector = await computeSerialized(text);
  await cachePut(url, vector);
  emitResolution('computed', text);
  return { info, vector };
}
