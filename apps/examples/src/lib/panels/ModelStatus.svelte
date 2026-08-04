<script>
  // Makes the embedding path observable. Seeded vectors are a file read; a
  // computed vector runs EmbeddingGemma in this tab. Showing which one produced
  // a result — and the model digest that produced it — is what makes the claim
  // checkable rather than asserted.
  import { onMount } from 'svelte';
  import { subscribeModel, subscribeResolution } from '../embed.js';

  let model = $state({ phase: 'absent', loaded: 0, total: 0, fromCache: false });
  let counts = $state({ seeded: 0, computed: 0, memory: 0, last: null });

  onMount(() => {
    const off = [subscribeModel((s) => (model = s)), subscribeResolution((r) => (counts = r))];
    return () => off.forEach((fn) => fn());
  });

  const mb = (n) => `${(n / 1e6).toFixed(0)} MB`;
  let pct = $derived(model.total > 0 ? Math.min(100, (model.loaded / model.total) * 100) : 0);
  let short = $derived(model.revision ? `${model.revision.slice(0, 12)}…` : '');
  let sizeLabel = $derived(model.bytes ? mb(model.bytes) : '236 MB');
  let active = $derived(model.phase === 'downloading' || model.phase === 'preparing');
</script>

<!-- Transfer state is exposed as data attributes so automation can wait on
     observable progress instead of guessing a wall-clock timeout. -->
<div
  class="model"
  class:active
  class:ready={model.phase === 'ready'}
  class:failed={model.phase === 'error'}
  data-phase={model.phase}
  data-loaded={model.loaded}
  data-total={model.total}
>
  {#if model.phase === 'absent'}
    <span class="dot" aria-hidden="true"></span>
    <span>
      Vectors for the prepared inputs are served from a seeded cache. Editing any
      text computes a new embedding with the real model, downloaded once
      ({sizeLabel}) and kept in this browser.
    </span>
  {:else if model.phase === 'starting'}
    <span class="dot busy" aria-hidden="true"></span>
    <span>Requesting EmbeddingGemma ({sizeLabel})…</span>
  {:else if model.phase === 'downloading'}
    <span class="dot busy" aria-hidden="true"></span>
    <span class="grow">
      <span class="line">
        Downloading EmbeddingGemma — {mb(model.loaded)} of {mb(model.total)}
        <strong>{pct.toFixed(0)}%</strong>
      </span>
      <progress max="100" value={pct}></progress>
      <span class="note">One time per browser. Cached afterwards.</span>
    </span>
  {:else if model.phase === 'preparing'}
    <span class="dot busy" aria-hidden="true"></span>
    <span>
      {model.fromCache ? 'Model already in this browser — loading' : 'Download complete — loading'}
      into WebAssembly…
    </span>
  {:else if model.phase === 'ready'}
    <span class="dot ok" aria-hidden="true"></span>
    <span>
      <strong>{model.model}</strong> running in this tab
      {#if short}<code title={model.revision}>{short}</code>{/if}
      {#if counts.computed > 0}
        — {counts.computed} embedding{counts.computed === 1 ? '' : 's'} computed here
      {/if}
    </span>
  {:else if model.phase === 'error'}
    <span class="dot bad" aria-hidden="true"></span>
    <span>Model unavailable: {model.error}</span>
  {/if}
</div>

<style>
  .model {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    font-size: 12px;
    color: var(--fg-muted, #8b949e);
    border: 1px solid var(--border, #21262d);
    border-radius: 8px;
    padding: 8px 12px;
    max-width: 78ch;
  }
  .model.active { border-color: var(--accent, #58a6ff); color: var(--fg, #c9d1d9); }
  .model.ready { border-color: #2ea043; }
  .model.failed { border-color: #f85149; color: #f85149; }
  .grow { display: grid; gap: 5px; width: 100%; }
  .line { display: flex; gap: 6px; align-items: baseline; }
  .note { opacity: 0.75; }
  progress { width: 100%; height: 6px; }
  code { font-size: 11px; opacity: 0.85; }
  .dot {
    width: 8px; height: 8px; border-radius: 50%; margin-top: 4px; flex: none;
    background: var(--fg-muted, #8b949e);
  }
  .dot.ok { background: #2ea043; }
  .dot.bad { background: #f85149; }
  .dot.busy { background: var(--accent, #58a6ff); animation: pulse 1.1s ease-in-out infinite; }
  @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
</style>
