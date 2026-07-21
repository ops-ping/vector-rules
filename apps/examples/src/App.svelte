<script>
  import { onMount } from 'svelte';
  import AddressExample from './lib/examples/AddressExample.svelte';
  import FraudTriage from './lib/examples/FraudTriage.svelte';
  import Prove from './lib/examples/Prove.svelte';
  import Semantic from './lib/examples/Semantic.svelte';
  import Streaming from './lib/examples/Streaming.svelte';
  import { getBackendInfo } from './lib/embed.js';

  // WebGPU capability + the backend the embedder actually used. Probed on mount
  // (adapter availability) and refined as examples run (ggml's chosen backend).
  let gpu = $state({ webgpuApi: false, adapter: null, backend: 'unknown' });
  async function refreshBackend() {
    gpu = await getBackendInfo();
  }

  const examples = [
    { id: 'address', label: 'Address verification', hint: 'canonicalization, reference matching, and organizational policy' },
    { id: 'semantic', label: 'Semantic rules', hint: 'vector predicates and forward chaining' },
    { id: 'fraud', label: 'Fraud triage', hint: 'fitted geometry artifacts + calibrated features + symbolic decisions' },
    { id: 'streaming', label: 'Streaming', hint: 'incremental rules in browser WebAssembly' },
    { id: 'prove', label: 'Proof', hint: 'goal-directed backward chaining' }
  ];
  const ids = new Set(examples.map((item) => item.id));

  let route = $state('address');
  let selected = $derived(ids.has(route) ? route : 'address');

  function navigate(next) {
    window.location.hash = `/${next}`;
  }

  onMount(() => {
    const sync = () => {
      const next = window.location.hash.replace(/^#\/?/, '') || 'address';
      route = ids.has(next) ? next : 'address';
    };
    sync();
    window.addEventListener('hashchange', sync);
    refreshBackend();
    // The chosen backend is only known once a model has loaded, so keep polling.
    const timer = window.setInterval(refreshBackend, 3000);
    return () => {
      window.removeEventListener('hashchange', sync);
      window.clearInterval(timer);
    };
  });

  let computeLabel = $derived(
    gpu.backend === 'webgpu'
      ? `WebGPU${gpu.adapter && gpu.adapter !== 'available' ? ` · ${gpu.adapter}` : ''}`
      : gpu.backend === 'cpu'
        ? 'CPU'
        : gpu.adapter
          ? 'WebGPU available'
          : gpu.webgpuApi
            ? 'CPU (no GPU adapter)'
            : 'CPU (no WebGPU)'
  );
  let computeOn = $derived(gpu.backend === 'webgpu' || (gpu.backend === 'unknown' && !!gpu.adapter));
</script>

<header>
  <div class="brand">
    <strong>vector-rules</strong>
    <span class="muted">browser examples</span>
    <span class="compute" class:on={computeOn} title="Embedding compute backend (WebGPU when the browser exposes a GPU adapter, else CPU)">
      {computeLabel}
    </span>
  </div>
  <p class="muted tagline">
    Every demonstration below runs entirely in your browser — the rule engine and
    EmbeddingGemma both execute as WebAssembly, with no server.
  </p>
</header>

<main>
  <nav aria-label="Examples">
    {#each examples as item}
      <button
        class:active={selected === item.id}
        title={item.hint}
        onclick={() => navigate(item.id)}
      >{item.label}</button>
    {/each}
  </nav>

  <section class="stage">
    {#if selected === 'address'}
      <AddressExample />
    {:else if selected === 'semantic'}
      <Semantic />
    {:else if selected === 'fraud'}
      <FraudTriage />
    {:else if selected === 'streaming'}
      <Streaming />
    {:else if selected === 'prove'}
      <Prove />
    {/if}
  </section>
</main>

<style>
  header {
    display: grid;
    gap: 8px;
    padding: 20px 24px;
    border-bottom: 1px solid var(--border, #21262d);
  }
  .brand { display: flex; align-items: baseline; gap: 10px; }
  .brand strong { font-size: 18px; }
  .compute {
    margin-left: auto;
    font-size: 12px;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border, #21262d);
    color: var(--muted, #8b949e);
    align-self: center;
  }
  .compute.on {
    border-color: var(--accent, #58a6ff);
    color: var(--accent, #58a6ff);
  }
  .tagline { margin: 0; max-width: 60ch; }
  main { display: grid; gap: 18px; padding: 20px 24px; max-width: 980px; }
  nav { display: flex; gap: 6px; flex-wrap: wrap; }
  nav button.active {
    border-color: var(--accent, #58a6ff);
    color: var(--accent, #58a6ff);
    background: var(--bg-elev2, #161b22);
  }
  .stage { display: grid; gap: 18px; }
</style>
