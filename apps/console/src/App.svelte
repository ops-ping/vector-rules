<script>
  import { onMount } from 'svelte';
  import Activity from './lib/panels/Activity.svelte';
  import Audit from './lib/panels/Audit.svelte';
  import Memory from './lib/panels/Memory.svelte';
  import Rules from './lib/panels/Rules.svelte';
  import Storage from './lib/panels/Storage.svelte';
  import Test from './lib/panels/Test.svelte';
  import WhatIf from './lib/panels/WhatIf.svelte';
  import Examples from './lib/pages/Examples.svelte';
  import { health } from './lib/api.js';

  const tabs = [
    { id: 'rules', label: 'Rules', hint: 'see & compare the rules' },
    { id: 'activity', label: 'Activity', hint: 'persisted MCP tool calls' },
    { id: 'storage', label: 'Storage', hint: 'append-only component event store' },
    { id: 'memory', label: 'Memory', hint: 'governed memory events' },
    { id: 'audit', label: 'Audit', hint: 'what happened' },
    { id: 'test', label: 'Test', hint: 'run an input' },
    { id: 'whatif', label: 'What-if', hint: 'A/B + prove' },
    { id: 'examples', label: 'Examples', hint: 'standalone capability demonstrations' }
  ];
  const routes = new Set(tabs.map((tab) => tab.id));
  let route = $state('rules');
  let online = $state(null); // null = unknown, true/false = checked

  async function ping() {
    try {
      online = await health();
    } catch {
      online = false;
    }
  }

  function routeFromHash() {
    const next = window.location.hash.replace(/^#\/?/, '') || 'rules';
    if (next === 'examples') return 'examples/address';
    return routes.has(next.split('/')[0]) ? next : 'rules';
  }

  function navigate(next) {
    route = next;
    window.location.hash = `/${next}`;
  }

  onMount(() => {
    const syncRoute = () => (route = routeFromHash());
    syncRoute();
    window.addEventListener('hashchange', syncRoute);
    ping();
    const timer = window.setInterval(ping, 10_000);
    return () => {
      window.removeEventListener('hashchange', syncRoute);
      window.clearInterval(timer);
    };
  });
</script>

<header>
  <div class="brand">
    <span class="logo">⟩</span>
    <strong>vrules</strong>
    <span class="muted">console</span>
  </div>
  <nav>
    {#each tabs as t}
      <button
        class:active={route.split('/')[0] === t.id}
        onclick={() => navigate(t.id === 'examples' ? 'examples/address' : t.id)}
        title={t.hint}
      >{t.label}</button>
    {/each}
  </nav>
  <div class="status" title="daemon /health">
    <span class="dot" class:up={online === true} class:down={online === false}></span>
    <span class="muted">{online === null ? 'checking' : online ? 'daemon up' : 'daemon down'}</span>
  </div>
</header>

<main>
  {#if route === 'rules'}
    <Rules />
  {:else if route === 'activity'}
    <Activity />
  {:else if route === 'storage'}
    <Storage />
  {:else if route === 'memory'}
    <Memory />
  {:else if route === 'audit'}
    <Audit />
  {:else if route === 'test'}
    <Test />
  {:else if route === 'whatif'}
    <WhatIf />
  {:else if route.startsWith('examples/')}
    <Examples
      example={route.split('/')[1] || 'address'}
      onnavigate={(example) => navigate(`examples/${example}`)}
    />
  {/if}
</main>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 18px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-elev);
    position: sticky;
    top: 0;
    z-index: 10;
  }
  .brand { display: flex; align-items: center; gap: 8px; }
  .logo { color: var(--accent); font-size: 20px; font-weight: 700; }
  nav { display: flex; gap: 6px; flex-wrap: wrap; }
  nav button { background: transparent; border-color: transparent; }
  nav button.active { background: var(--bg-elev2); border-color: var(--border); }
  .status { margin-left: auto; display: flex; align-items: center; gap: 6px; }
  .dot {
    width: 9px; height: 9px; border-radius: 50%;
    background: var(--fg-muted);
  }
  .dot.up { background: var(--green); }
  .dot.down { background: var(--red); }
  main { padding: 18px; max-width: 1100px; margin: 0 auto; }
</style>
