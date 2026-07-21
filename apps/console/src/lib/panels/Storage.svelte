<script>
  import { onDestroy } from 'svelte';
  import { storageStats } from '../api.js';
  import { fmtBytes } from '../format.js';

  let stats = $state(null);
  let error = $state('');

  async function refresh() {
    try {
      stats = await storageStats();
      error = '';
    } catch (e) {
      error = e.message;
    }
  }

  refresh();
  const timer = setInterval(refresh, 5000);
  onDestroy(() => clearInterval(timer));

  let streams = $derived(Object.entries(stats?.streams ?? {}).sort(([a], [b]) => a.localeCompare(b)));
</script>

{#if error}<div class="error">{error}</div>{/if}

<div class="toolbar">
  <button onclick={refresh}>↻ Refresh</button>
</div>

<div class="grid">
  <div class="card"><div class="k">Events</div><div class="v">{stats?.events ?? '—'}</div></div>
  <div class="card"><div class="k">Segments</div><div class="v">{stats?.segments ?? '—'}</div></div>
  <div class="card"><div class="k">Stored bytes</div><div class="v">{stats ? fmtBytes(stats.bytes) : '—'}</div></div>
  <div class="card"><div class="k">Streams</div><div class="v">{streams.length}</div></div>
</div>

<section>
  <h3>Append-only streams</h3>
  <table>
    <thead><tr><th>stream</th><th>events</th></tr></thead>
    <tbody>
      {#each streams as [name, count] (name)}
        <tr><td class="mono">{name}</td><td class="mono num">{count}</td></tr>
      {/each}
      {#if !streams.length}
        <tr><td colspan="2" class="muted empty">no events stored</td></tr>
      {/if}
    </tbody>
  </table>
</section>

<style>
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 14px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 12px; }
  .card {
    border: 1px solid var(--border); border-radius: 10px; padding: 12px 14px;
    background: var(--bg-elev);
  }
  .card .k { font-size: 12px; text-transform: uppercase; letter-spacing: 0.03em; color: var(--fg-muted); }
  .card .v { font-size: 24px; font-weight: 700; margin-top: 4px; }
  section { margin-top: 22px; }
  h3 { font-size: 14px; margin: 0 0 8px; }
  table { width: 100%; border-collapse: collapse; }
  th { text-align: left; font-weight: 600; color: var(--fg-muted); border-bottom: 1px solid var(--border);
       padding: 6px 10px; font-size: 12px; text-transform: uppercase; letter-spacing: 0.03em; }
  td { padding: 7px 10px; border-bottom: 1px solid var(--border); }
  .num { text-align: right; }
  .empty { text-align: center; padding: 24px; }
</style>
