import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Two in-repo, source-imported dependencies, aliased to their locations:
//
//   vrules-wasm  — the browser rule engine. `build:wasm` runs `wasm-pack build`
//                  on `crates/vrules-wasm` (three levels up) and emits its `pkg/`.
//   wllama       — the in-browser GGUF runtime for EmbeddingGemma. It is
//                  dependency-free and ships its worker as an inlined string, so
//                  Vite consumes its TypeScript source directly; the CPU wasm is
//                  a committed asset pulled in via `?url`.
//
// This app has NO daemon dependency: embeddings are computed in the browser and
// the whole `dist/` is static, deployable to a plain object bucket.
const wasmPkg = fileURLToPath(new URL('../../crates/vrules-wasm/pkg', import.meta.url));
const wllamaSrc = fileURLToPath(new URL('../../vendor/wllama/src/index.ts', import.meta.url));
const wllamaWasm = fileURLToPath(new URL('../../vendor/wllama/src/wasm/wllama.wasm', import.meta.url));
const repoRoot = fileURLToPath(new URL('../..', import.meta.url));

export default defineConfig({
  // Relative base so the bundle works served from any bucket path.
  base: './',
  resolve: {
    // Array form with RegExp finds: `wllama-wasm?url` carries a query, which an
    // exact object-key alias would not match — a prefix RegExp does.
    alias: [
      { find: /^vrules-wasm/, replacement: wasmPkg },
      { find: /^wllama-wasm/, replacement: wllamaWasm },
      { find: /^wllama$/, replacement: wllamaSrc }
    ]
  },
  assetsInclude: ['**/*.wasm'],
  plugins: [svelte()],
  server: {
    // The wasm pkg and wllama source live outside the project root.
    fs: { allow: ['..', repoRoot] }
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // The model file is large; don't inline anything as base64.
    assetsInlineLimit: 0
  }
});
