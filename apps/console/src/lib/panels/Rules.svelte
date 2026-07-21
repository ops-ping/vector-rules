<script>
  import { rulesBranches, rulesList, rulesCompare } from '../api.js';

  // Rule VISIBILITY: see the actual rule definitions in a branch, and compare
  // RULES (not fired outcomes) across two branches.
  let branches = $state([]);
  let mode = $state('browse'); // 'browse' | 'compare'
  let branch = $state('main');
  let a = $state('main');
  let b = $state('main');
  let grl = $state('');
  let ruleCount = $state(0);
  let cmp = $state(null);
  let error = $state('');

  async function init() {
    try {
      branches = await rulesBranches();
      if (branches.length > 1) b = branches.find((x) => x !== a) ?? branches[0];
      await load();
    } catch (e) { error = e.message; }
  }
  init();

  async function load() {
    error = ''; grl = ''; ruleCount = 0; cmp = null;
    try {
      if (mode === 'browse') {
        const r = await rulesList(branch);
        grl = r.grl;
        ruleCount = r.count;
      } else {
        cmp = await rulesCompare(a, b);
      }
    } catch (e) { error = e.message; }
  }
</script>

<div class="bar">
  <div class="seg">
    <button class:active={mode === 'browse'} onclick={() => { mode = 'browse'; load(); }}>Browse</button>
    <button class:active={mode === 'compare'} onclick={() => { mode = 'compare'; load(); }}>Compare</button>
  </div>
  {#if mode === 'browse'}
    <label>branch <select bind:value={branch} onchange={load}>{#each branches as x}<option>{x}</option>{/each}</select></label>
  {:else}
    <label>A <select bind:value={a} onchange={load}>{#each branches as x}<option>{x}</option>{/each}</select></label>
    <label>B <select bind:value={b} onchange={load}>{#each branches as x}<option>{x}</option>{/each}</select></label>
  {/if}
</div>

{#if error}<div class="error">{error}</div>{/if}

{#if mode === 'browse'}
  <p class="muted">{ruleCount} rule{ruleCount === 1 ? '' : 's'} in <span class="mono">{branch}</span></p>
  <pre class="source">{grl}</pre>
{:else if cmp}
  {#if cmp.only_b.length}
    <h4 class="added">+ Added in {cmp.b} ({cmp.only_b.length})</h4>
    {#each cmp.only_b as name}<div class="card added"><div class="name">{name}</div></div>{/each}
  {/if}
  {#if cmp.only_a.length}
    <h4 class="removed">− Removed from {cmp.b} ({cmp.only_a.length})</h4>
    {#each cmp.only_a as name}<div class="card removed"><div class="name">{name}</div></div>{/each}
  {/if}
  {#if cmp.changed.length}
    <h4 class="changed">~ Changed ({cmp.changed.length})</h4>
    {#each cmp.changed as name}
      <div class="card changed">
        <div class="name">{name}</div>
      </div>
    {/each}
  {/if}
  {#if !cmp.only_a.length && !cmp.only_b.length && !cmp.changed.length}
    <p class="muted">No rule differences — {cmp.unchanged} identical rule{cmp.unchanged === 1 ? '' : 's'}.</p>
  {:else}
    <p class="muted">{cmp.unchanged} unchanged.</p>
  {/if}
{/if}

<style>
  .bar { display: flex; gap: 14px; align-items: center; margin-bottom: 12px; flex-wrap: wrap; }
  .seg { display: flex; gap: 4px; }
  .seg button { background: transparent; }
  .seg button.active { background: var(--bg-elev2); border-color: var(--border); }
  label { font-size: 12px; color: var(--fg-muted); display: flex; gap: 6px; align-items: center; }
  .card { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 12px 14px; margin-bottom: 10px; }
  .card.added { border-left: 3px solid var(--green); }
  .card.removed { border-left: 3px solid var(--red); opacity: 0.85; }
  .card.changed { border-left: 3px solid var(--amber); }
  .name { font-weight: 600; font-family: var(--mono); margin-bottom: 6px; }
  .source { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 14px; white-space: pre-wrap; }
  h4 { margin: 16px 0 8px; font-size: 13px; }
  h4.added { color: var(--green); } h4.removed { color: var(--red); } h4.changed { color: var(--amber); }
</style>
