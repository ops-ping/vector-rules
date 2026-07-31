// Rewrite the built model manifest for a static public deploy.
//
// Locally the examples resolve the model from `public/models/`, where a symlink
// points at the developer's model dir. A public deploy ships no model at all:
// the site is ~9 MB, seeded vectors come from the bucket cache, and the model is
// streamed from the CDN only to compute a genuine cache miss.
//
// `embed.js` resolves the manifest's `file` with `new URL(file, MODEL_DIR)`, so
// an absolute URL here overrides the local path with no code change. The digest
// is carried through as `sha256`, keeping the deployed site pinned to the exact
// quantization the bucket cache was seeded from.

import { readFile, writeFile, rm, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const dist = fileURLToPath(new URL('../dist/', import.meta.url));
const manifestPath = `${dist}models/model.json`;

const url = process.env.EXAMPLES_MODEL_URL;
const sha = process.env.EXAMPLES_MODEL_SHA256;

if (!url) {
  console.error(
    'prepare-deploy: EXAMPLES_MODEL_URL is unset. Source release/pins.env first —\n' +
      '  set -a && . release/pins.env && set +a\n' +
      'Refusing to publish a site whose model URL would 404 on the first cache miss.'
  );
  process.exit(1);
}

const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));

if (sha && manifest.sha256 && sha !== manifest.sha256) {
  console.error(
    `prepare-deploy: digest mismatch. pins.env has ${sha}, but the manifest was\n` +
      `seeded from ${manifest.sha256}. Publishing these together would mix vector\n` +
      'spaces: re-seed the bucket cache before repinning.'
  );
  process.exit(1);
}

manifest.file = url;
await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

// A local build dereferences the symlink and copies the model into dist. Drop it:
// the deploy streams from the CDN, and 226 MB has no business in a Pages artifact.
const models = await readdir(`${dist}models`);
for (const name of models.filter((n) => n.endsWith('.gguf'))) {
  await rm(`${dist}models/${name}`);
  console.log(`prepare-deploy: removed bundled ${name}`);
}

console.log(`prepare-deploy: model resolves from ${url}`);
