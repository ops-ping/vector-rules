<script>
  import { onMount } from 'svelte';
  import AddressExample from './lib/examples/AddressExample.svelte';
  import FraudTriage from './lib/examples/FraudTriage.svelte';
  import Prove from './lib/examples/Prove.svelte';
  import Semantic from './lib/examples/Semantic.svelte';
  import Streaming from './lib/examples/Streaming.svelte';
  import ModelStatus from './lib/panels/ModelStatus.svelte';
  import { getBackendInfo } from './lib/embed.js';

  // WebGPU capability + the backend the embedder actually used. Probed on mount
  // (adapter availability) and refined as examples run (ggml's chosen backend).
  let gpu = $state({ webgpuApi: false, adapter: null, backend: 'unknown' });
  async function refreshBackend() {
    gpu = await getBackendInfo();
  }

  const examples = [
    { id: 'semantic', label: 'Semantic rules', hint: 'an editable bench: vector algebra, calibrated decisions, forward chaining' },
    { id: 'address', label: 'Address verification', hint: 'canonicalization, reference matching, and organizational policy' },
    { id: 'fraud', label: 'Fraud triage', hint: 'fitted geometry artifacts + calibrated features + symbolic decisions' },
    { id: 'streaming', label: 'Streaming', hint: 'incremental rules in browser WebAssembly' },
    { id: 'prove', label: 'Proof', hint: 'goal-directed backward chaining' }
  ];
  const ids = new Set(examples.map((item) => item.id));
  const DEFAULT_EXAMPLE = examples[0].id;

  let route = $state(DEFAULT_EXAMPLE);
  let selected = $derived(ids.has(route) ? route : DEFAULT_EXAMPLE);

  function navigate(next) {
    window.location.hash = `/${next}`;
  }

  onMount(() => {
    const sync = () => {
      const next = window.location.hash.replace(/^#\/?/, '') || DEFAULT_EXAMPLE;
      route = ids.has(next) ? next : DEFAULT_EXAMPLE;
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
  <ModelStatus />
</header>

<main>
  <nav aria-label="Examples">
    {#each examples as item}
      <button
        class:active={selected === item.id}
        aria-current={selected === item.id ? 'page' : undefined}
        onclick={() => navigate(item.id)}
      >
        <span class="nav-label">{item.label}</span>
        <span class="nav-hint">{item.hint}</span>
      </button>
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
  .brand { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap; }
  .brand strong { font-size: 18px; white-space: nowrap; }
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

  /* Fluid: the shell fills the space it is given and only stops growing where
     a line of prose would become unreadable. The cap is in rem so it tracks the
     reader's font size rather than a fixed pixel guess about their screen. */
  header, main {
    width: 100%;
    max-width: 110rem;
    margin-inline: auto;
    padding-inline: clamp(12px, 2.5vw, 28px);
  }
  main {
    display: grid;
    gap: clamp(12px, 2vw, 22px);
    padding-block: 20px;
    grid-template-columns: minmax(0, 1fr);
    align-items: start;
  }

  /* Narrow: a scrollable row of tabs. Wide: a vertical rail beside the stage. */
  nav {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: thin;
    padding-bottom: 4px;
  }
  nav button {
    flex: 0 0 auto;
    display: grid;
    gap: 2px;
    text-align: left;
    line-height: 1.35;
  }
  .nav-label { font-size: 13px; }
  .nav-hint { display: none; font-size: 11px; color: var(--muted, #8b949e); }
  nav button.active {
    border-color: var(--accent, #58a6ff);
    color: var(--accent, #58a6ff);
    background: var(--bg-elev2, #161b22);
  }
  nav button.active .nav-hint { color: var(--accent, #58a6ff); opacity: 0.75; }

  @media (min-width: 60rem) {
    main { grid-template-columns: minmax(10rem, 15rem) minmax(0, 1fr); }
    nav {
      position: sticky;
      top: 20px;
      flex-direction: column;
      overflow-x: visible;
      padding-bottom: 0;
    }
    nav button { width: 100%; padding: 8px 10px; }
    .nav-hint { display: block; }
  }

  .stage { display: grid; gap: 18px; min-width: 0; }
</style>
