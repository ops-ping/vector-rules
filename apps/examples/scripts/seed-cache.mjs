// Seed the static embedding cache for the example corpora.
//
// Runs the native llama.cpp server on the SAME Q4_K_M GGUF the browser ships,
// with mean pooling + L2 normalization (matching `src/lib/embed.js`), embeds
// every fixed string the examples use, and writes little-endian f32 objects at
// the content-addressed REST path the client reads:
//
//   public/vrules-rest/v1/embeddings/<model>/<canon>/<hash>
//
// Vite copies `public/` into `dist/`, so these vectors ship with the static
// site and are committed for that reason: the Pages build has no model or
// inference engine and cannot regenerate them. A cache hit lets an example run
// WITHOUT downloading the 226 MB model; a miss (any text not seeded, e.g.
// free-form input) falls back to in-browser compute.
//
// Usage: node scripts/seed-cache.mjs
//   env MODEL_GGUF   path to the Q4_K_M gguf (default: the vrules models dir)
//   env ENGINE_DIR   dir containing llama-server (default: vrules engine dir)

import { spawn } from 'node:child_process';
import { mkdir, writeFile, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { textHash } from '../src/lib/cache-key.js';

const APP = fileURLToPath(new URL('..', import.meta.url));
const manifest = JSON.parse(readFileSync(path.join(APP, 'public/models/model.json'), 'utf8'));
const canon = manifest.canon || 'default-v1';

const ENGINE_DIR = process.env.ENGINE_DIR || path.join(homedir(), '.local/share/vrules/engine');
const MODEL_GGUF =
  process.env.MODEL_GGUF || path.join(homedir(), '.local/share/vrules/models', manifest.file);
const PORT = 8799;
const OUT_DIR = path.join(APP, 'public/vrules-rest/v1/embeddings', manifest.name, canon);

// The fixed strings the examples embed. Mirrors the constants in
// src/lib/examples/{Semantic,FraudTriage}.svelte; drift only causes a cache
// miss (graceful fallback to in-browser compute), never wrong output.
const CORPUS = [
  // Semantic.svelte — royal proclamations. Mirrors DEFAULT_RULES/AXES there.
  // Analogy operands, and speakers worth trying as the parameter.
  'king', 'man', 'woman', 'queen', 'Elizabeth I of England', 'Queen Victoria',
  'Henry VIII', 'a duke', 'the mayor', 'tractor',
  // sovereign_voice axis — positive exemplars
  'We hereby decree, by Our sovereign authority, that it shall be so',
  'By royal command, let it be proclaimed throughout the realm',
  'It is Our will that this matter be settled before the coming feast',
  'We, by the grace of God, Queen of these lands, do declare',
  'Let the herald announce Our judgement to every province',
  // sovereign_voice axis — negative exemplars
  'hey can you send me that file when you get a sec',
  'please find attached the monthly invoice for your records',
  'just following up on my last email about the meeting',
  'thanks for the update, looks good to me',
  'can we move the standup to 10 tomorrow',
  // displeasure axis — positive exemplars
  'We are gravely displeased and this failure shall not go unanswered',
  'this is unacceptable and there will be consequences for it',
  'you have failed us, and we shall remember who was responsible',
  'our patience is exhausted; answer for this at once',
  'we are bitterly disappointed by what has been allowed to happen',
  // displeasure axis — negative exemplars
  'We are delighted, and offer Our warmest thanks for your service',
  'this is excellent work and we are grateful for the effort',
  'thank you kindly, we are pleased with how this was handled',
  'wonderful news, well done to everyone involved',
  'we are content, and commend all who took part',
  // Shared calibration window: both registers and both intents, because a
  // percentile is only as portable as the population it is measured against.
  'We hereby decree that the harvest tax is reduced this season',
  'By Our command the gates shall open at dawn',
  'please could you review the attached draft',
  'the meeting notes are in the shared folder',
  'we are gravely displeased with this dereliction',
  'this is completely unacceptable and must be answered for',
  'we are delighted with the outcome, thank you',
  'great work everyone, really pleased with this',
  'the quarterly numbers are attached for your review',
  'let it be known that Our judgement is final',
  'sorry for the delay, been swamped this week',
  'We commend the loyal service of Our subjects',
  'can you confirm receipt of the shipment',
  'your continued failure has exhausted Our patience',
  'thanks, that clears it up nicely',
  'the maintenance window is scheduled for Sunday',
  // Messages the example and its verification run assert on.
  'We are gravely displeased by this betrayal, and Our judgement shall be swift',
  'We hereby decree that the market shall be held on the first day of each month',
  'this is completely unacceptable, someone needs to answer for it',
  'thanks for sorting that out, much appreciated',
  // FraudTriage URGENT_EXEMPLARS
  'urgent wire transfer needed immediately or we face penalty',
  'the ceo needs this payment today, keep it confidential',
  'act now, deadline is today, wire the account immediately',
  'executive request: transfer funds asap and be discreet',
  'immediately process this urgent payment, consequences otherwise',
  'boss says wire now, strictly confidential, deadline today',
  // FraudTriage CALM_EXEMPLARS
  'attached is the usual monthly invoice, thanks',
  'regular payment schedule attached, regards',
  'hello, the monthly account statement is attached',
  'hi, invoice attached per the usual schedule, thanks',
  'monthly transfer per the regular schedule, regards',
  'thanks, attached the invoice as usual',
  // FraudTriage NEUTRAL_CORPUS
  'hello, following up on the quarterly report',
  'the meeting is scheduled for next week, regards',
  'attached the notes from the review, thanks',
  'hi, can you confirm the delivery address',
  'regular maintenance window this weekend',
  'monthly newsletter draft attached',
  'thanks for the update, looks good',
  'invoice received, processing per the usual schedule',
  // FraudTriage PRESETS
  'urgent: the ceo needs a confidential wire transfer immediately, deadline today',
  'hi, attached is the usual monthly invoice, thanks and regards',
  'please process the transfer today, thanks'
];

function leBytes(vector) {
  const buf = Buffer.alloc(vector.length * 4);
  for (let i = 0; i < vector.length; i++) buf.writeFloatLE(vector[i], i * 4);
  return buf;
}

async function waitForHealth(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const r = await fetch(`http://127.0.0.1:${PORT}/health`);
      if (r.ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((res) => setTimeout(res, 500));
  }
  throw new Error('llama-server did not become healthy in time');
}

async function embed(text) {
  const r = await fetch(`http://127.0.0.1:${PORT}/v1/embeddings`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ input: text, model: manifest.name })
  });
  if (!r.ok) throw new Error(`embed failed (${r.status}) for ${JSON.stringify(text)}`);
  const json = await r.json();
  const vector = json?.data?.[0]?.embedding;
  if (!Array.isArray(vector) || vector.length !== manifest.dimensions) {
    throw new Error(`unexpected embedding shape for ${JSON.stringify(text)}`);
  }
  return vector;
}

async function main() {
  await rm(OUT_DIR, { recursive: true, force: true });
  await mkdir(OUT_DIR, { recursive: true });

  const server = spawn(
    path.join(ENGINE_DIR, 'llama-server'),
    ['-m', MODEL_GGUF, '--embeddings', '--pooling', 'mean', '--embd-normalize', '2',
     '-c', '2048', '--host', '127.0.0.1', '--port', String(PORT)],
    { stdio: ['ignore', 'ignore', 'inherit'], env: { ...process.env, LD_LIBRARY_PATH: ENGINE_DIR } }
  );

  try {
    await waitForHealth(120_000);
    let n = 0;
    for (const text of CORPUS) {
      const vector = await embed(text);
      await writeFile(path.join(OUT_DIR, textHash(text)), leBytes(vector));
      n++;
    }
    console.log(`seeded ${n} embeddings → ${path.relative(APP, OUT_DIR)}`);
  } finally {
    server.kill('SIGTERM');
  }
}

main().catch((e) => {
  console.error(e.message);
  process.exit(1);
});
