<script>
  import { onDestroy } from 'svelte';
  import { toolStats } from '../api.js';
  import { fmtTime } from '../format.js';

  let data = $state(null);
  let loading = $state(false);
  let error = $state('');

  async function refresh() {
    loading = true;
    error = '';
    try {
      data = await toolStats();
    } catch (e) {
      error = e.message;
      data = null;
    } finally {
      loading = false;
    }
  }

  // Initial view + light polling so call counts update live.
  refresh();
  const timer = setInterval(refresh, 5000);
  onDestroy(() => clearInterval(timer));

  let tools = $derived(data?.tools ?? []);
  let totalCalls = $derived(tools.reduce((sum, tool) => sum + tool.calls, 0));
</script>

<div class="toolbar">
  <button onclick={refresh} disabled={loading}>↻ Refresh</button>
  <span class="muted">{totalCalls} persisted calls across {tools.length} tools</span>
</div>

{#if error}<div class="error">{error}</div>{/if}

<table>
  <thead>
    <tr><th>tool</th><th>calls</th><th>errors</th><th>last call</th></tr>
  </thead>
  <tbody>
    {#each tools as t (t.name)}
      <tr>
        <td class="mono">{t.name}</td>
        <td class="mono num">{t.calls}</td>
        <td class="mono num" class:err={t.errors > 0}>{t.errors}</td>
        <td class="mono muted nowrap">{fmtTime(t.last_called_ns)}</td>
      </tr>
    {/each}
    {#if !tools.length && !loading}
      <tr><td colspan="4" class="muted empty">no MCP tool calls recorded</td></tr>
    {/if}
  </tbody>
</table>

<style>
  .toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-bottom: 14px; }
  table { width: 100%; border-collapse: collapse; }
  th {
    text-align: left; font-weight: 600; color: var(--fg-muted);
    border-bottom: 1px solid var(--border); padding: 6px 10px; font-size: 12px;
    text-transform: uppercase; letter-spacing: 0.03em;
  }
  td { padding: 7px 10px; border-bottom: 1px solid var(--border); vertical-align: top; }
  .num { text-align: right; }
  .nowrap { white-space: nowrap; }
  .err { color: var(--red); }
  .empty { text-align: center; padding: 30px; }
</style>
