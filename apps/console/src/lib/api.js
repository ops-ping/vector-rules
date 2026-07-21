// Thin client over the daemon's REST surface (`/vrules-rest/v1`).
//
// Routes answer with the result JSON directly; errors are `{ "error": "..." }`
// with a 4xx/5xx status. All engine evaluation / validation is server-side;
// this module just shuttles JSON.

const BASE = '/vrules-rest/v1';

async function request(method, path, body) {
  const options = { method };
  if (body !== undefined) {
    options.headers = { 'content-type': 'application/json' };
    options.body = JSON.stringify(body);
  }
  let resp;
  try {
    resp = await fetch(`${BASE}${path}`, options);
  } catch (e) {
    throw new Error(`daemon unreachable: ${e.message}`);
  }
  const payload = await resp.json().catch(() => null);
  if (!resp.ok) throw new Error(payload?.error ?? `HTTP ${resp.status} from ${path}`);
  return payload;
}

const getJSON = (path) => request('GET', path);
const sendJSON = (method, path, body = {}) => request(method, path, body);

/** Build a query string, omitting null/undefined values. */
function qs(params) {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== null && value !== undefined) query.set(key, value);
  }
  const encoded = query.toString();
  return encoded ? `?${encoded}` : '';
}

function auditRecord(event, distance = null) {
  const payload = event?.payload ?? {};
  return {
    id: event?.id,
    ts_nanos: event?.timestamp_ns,
    kind: event?.kind,
    distance,
    ...payload,
    request: payload.args,
    answer: typeof payload.result === 'string'
      ? payload.result
      : JSON.stringify(payload.result ?? null, null, 2)
  };
}

function memoryRecord(event, distance = null, status = null) {
  const payload = event?.payload ?? {};
  return {
    id: event?.id,
    created_at: event?.timestamp_ns,
    kind: event?.kind,
    status: status ?? (event?.tombstone ? 'deleted' : event?.kind),
    supersedes: event?.supersedes,
    distance,
    fact: payload.fact ?? '',
    tags: payload.tags ?? [],
    source: payload.source,
    reason: payload.reason
  };
}

// --- typed-ish wrappers for the routes the daemon exposes ---

export const logScan = (limit = 50, session = null) =>
  getJSON(`/log${qs({ limit, session })}`)
    .then((r) => (r.events ?? []).map((event) => auditRecord(event)));

export const logSearch = (query, k = 10) =>
  getJSON(`/log/search${qs({ query, k })}`)
    .then((r) => (r.hits ?? []).map((hit) => auditRecord(hit.event, hit.distance)));

export const sessionsList = () => getJSON('/sessions');

export const testRun = (tool, args) => sendJSON('POST', '/test/run', { tool, args });

// --- rules engine: validate / what-if (fwd + bwd) / A-B / git / sign-off ---

export const rulesValidate = (grl) => sendJSON('POST', '/rules/validate', { grl });

/** Forward what-if: assert facts → fired/decision/trace. Optional `ruleset` branch. */
export const whatifAssert = (facts, type = 'Request', ruleset = null) =>
  sendJSON('POST', '/whatif/assert', ruleset ? { type, facts, ruleset } : { type, facts });

/** A/B two revisions against the same facts → per-branch results + diff. */
export const abRun = (a, b, facts, type = 'Request') =>
  sendJSON('POST', '/ab', { a, b, type, facts });

/** Backward-chaining: prove a GRL goal against GRL rules. */
export const whatifProve = (grl, query, facts = {}) =>
  sendJSON('POST', '/whatif/prove', { grl, query, facts });

export const rulesBranches = () => getJSON('/rules/branches');
export const rulesDiff = (a, b) => getJSON(`/rules/diff${qs({ a, b })}`);

/** The actual rule definitions for a revision (default: loaded branch). */
export const rulesList = (ruleset = null) => getJSON(`/rules${qs({ ruleset })}`);

/** Rule-level diff (added / removed / changed rules with their definitions). */
export const rulesCompare = (a, b) => getJSON(`/rules/compare${qs({ a, b })}`);

/** Human sign-off: promote `from` onto `to` (requires sign_off). */
export const rulesPromote = (from, to = 'main') =>
  sendJSON('POST', '/rules/promote', { from, to, sign_off: true });

// --- embeddings: cache-through embed of the active model ---

/** Embed `text` via the daemon's cache tier. Returns `{ info, vector }`. */
export const embedText = (text) => sendJSON('POST', '/embedding', { text });

// --- agent memory: read-only explorer over the queryable store ---

/** Search the memory store. `params`: { query?, k?, tags?, since?, until?, mode?,
 *  include_superseded?, include_deleted? }. Returns the result records. */
export const memorySearch = (params = {}) =>
  sendJSON('POST', '/memory/search', params).then((r) => {
    const entries = r.hits ?? (r.events ?? []).map((event) => ({ event }));
    return entries.map((entry) => memoryRecord(entry.event, entry.distance));
  });

/** Full supersede/delete lineage for a memory (oldest first). */
export const memoryHistory = (id) =>
  getJSON(`/memory/${encodeURIComponent(id)}/history`).then((r) => {
    const events = r.events ?? [];
    return events.map((event, index) => memoryRecord(
      event,
      null,
      event.tombstone ? 'deleted' : index < events.length - 1 ? 'superseded' : 'live'
    ));
  });

/** Usage metering — totals, per-source attribution, most-queried. Optional focus. */
export const memoryStats = (memoryId = null) =>
  getJSON(`/memory/stats${qs({ memory_id: memoryId })}`);

/** Persisted MCP tool-call counts from the activity event stream. */
export const toolStats = () => getJSON('/tools/stats');

export const health = () => fetch('/health').then((r) => r.ok);

/** Current append-only component-store totals and per-stream counts. */
export const storageStats = () => getJSON('/storage/stats');
