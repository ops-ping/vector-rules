import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { VitePWA } from 'vite-plugin-pwa';

// The client-side `v:` engine: the `build:wasm` prebuild runs `wasm-pack build`
// on the in-repo vrules-wasm crate (`crates/vrules-wasm`, two levels up from this
// `apps/console` project), emitting its `pkg/`. Alias the crate name to that pkg
// so the console imports `RuleEngine` from it; the `.wasm` is pulled in via `?url`
// and Vite hashes it into `dist/assets`.
const wasmPkg = fileURLToPath(new URL('../../crates/vrules-wasm/pkg', import.meta.url));
const repoRoot = fileURLToPath(new URL('../..', import.meta.url));

// This standalone project builds the admin console; vrules-shim embeds
// the built `dist/` via `include_dir!` and serves it same-origin at `/`. In
// `npm run dev` we forward the shim's API (`/vrules-rest`, `/health`) to the
// running process so the dev server and production behave identically.
export default defineConfig({
  // Relative base so the embedded bundle works regardless of mount path.
  base: './',
  resolve: {
    alias: { 'vrules-wasm': wasmPkg },
  },
  // The wasm-pack pkg lives elsewhere in the repo (crates/vrules-wasm); treat .wasm as an asset.
  assetsInclude: ['**/*.wasm'],
  plugins: [
    svelte(),
    VitePWA({
      registerType: 'autoUpdate',
      // A real service worker is what the Android AVF host-access spike exercises
      // (secure-context: localhost-forward vs LAN-IP http). Precache the shell.
      manifest: {
        name: 'vector-rules admin console',
        short_name: 'vrules',
        description: 'Git-governed MCP policy, AI agent memory, embeddings, and browser WASM rules.',
        theme_color: '#0d1117',
        background_color: '#0d1117',
        display: 'standalone',
        icons: [
          { src: 'icon.svg', sizes: 'any', type: 'image/svg+xml', purpose: 'any maskable' }
        ]
      },
      workbox: {
        // Don't try to cache the management API; it must always hit the daemon.
        navigateFallbackDenylist: [/^\/vrules-rest/, /^\/mcp/, /^\/health/]
      }
    })
  ],
  server: {
    // The wasm pkg is outside the project root (crates/vrules-wasm) — let dev read the repo root.
    fs: { allow: ['..', repoRoot] },
    proxy: {
      '/vrules-rest': 'http://127.0.0.1:8765',
      '/health': 'http://127.0.0.1:8765'
    }
  },
  build: {
    // Built bundle is committed and embedded; keep it deterministic and lean.
    outDir: 'dist',
    emptyOutDir: true
  }
});
