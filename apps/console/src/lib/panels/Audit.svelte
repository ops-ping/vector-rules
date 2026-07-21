<script>
  import { logScan, logSearch, sessionsList } from '../api.js';
  import { fmtTime, reqSummary, truncate } from '../format.js';
  import Record from '../Record.svelte';

  let records = $state([]);
  let sessions = $state([]);
  let sessionFilter = $state('');
  let query = $state('');
  let mode = $state('scan'); // 'scan' | 'search'
  let limit = $state(50);
  let loading = $state(false);
  let error = $state('');
  let expanded = $state(null);

  async function refresh() {
    loading = true;
    error = '';
    try {
      if (mode === 'search') {
        records = await logSearch(query, 20);
      } else {
        records = await logScan(limit, sessionFilter || null);
      }
    } catch (e) {
      error = e.message;
      records = [];
    } finally {
      loading = false;
    }
  }

  async function loadSessions() {
    try {
      sessions = await sessionsList();
    } catch {
      sessions = [];
    }
  }

  function runScan() { mode = 'scan'; refresh(); }
  function runSearch() {
    if (!query.trim()) return;
    mode = 'search';
    refresh();
  }
  function rowKey(r, i) { return `${r.ts_nanos}-${r.id}-${i}`; }

  loadSessions();
  refresh();
</script>

<div class="toolbar">
  <select bind:value={sessionFilter} onchange={runScan} title="filter by session">
    <option value="">all sessions</option>
    {#each sessions as s}
      <option value={s}>{s}</option>
    {/each}
  </select>
  <button onclick={runScan} disabled={loading}>↻ Scan</button>

  <div class="search">
    <input
      placeholder="semantic search the log…"
      bind:value={query}
      onkeydown={(e) => e.key === 'Enter' && runSearch()}
    />
    <button onclick={runSearch} disabled={loading || !query.trim()}>Search</button>
  </div>
  <span class="muted">{records.length} record{records.length === 1 ? '' : 's'}{mode === 'search' ? ' (by relevance)' : ''}</span>
</div>

{#if error}<div class="error">{error}</div>{/if}

<table>
  <thead>
    <tr>
      <th>time</th><th>session</th><th>tool</th><th>backend</th>
      <th>cache</th><th>request → answer</th>
    </tr>
  </thead>
  <tbody>
    {#each records as r, i (rowKey(r, i))}
      {@const key = rowKey(r, i)}
      <tr class="row" onclick={() => (expanded = expanded === key ? null : key)}>
        <td class="mono muted nowrap">{fmtTime(r.ts_nanos)}</td>
        <td class="mono nowrap">{truncate(r.session_id, 18)}</td>
        <td class="mono">{r.tool}</td>
        <td class="mono muted">{r.backend}</td>
        <td>
          <span class="pill" class:hit={r.cache === 'hit'} class:miss={r.cache === 'miss'}>
            {r.cache}
          </span>
        </td>
        <td>
          <span class="q">{truncate(reqSummary(r), 60)}</span>
          <span class="muted"> → </span>
          <span class="muted">{truncate(r.answer, 80)}</span>
        </td>
      </tr>
      {#if expanded === key}
        <tr class="detail-row"><td colspan="6"><Record record={r} /></td></tr>
      {/if}
    {/each}
    {#if !records.length && !loading}
      <tr><td colspan="6" class="muted empty">no records — run an MCP capability call, then scan</td></tr>
    {/if}
  </tbody>
</table>

<style>
  .toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-bottom: 14px; }
  .search { display: flex; gap: 6px; flex: 1; min-width: 260px; }
  .search input { flex: 1; }
  table { width: 100%; border-collapse: collapse; }
  th {
    text-align: left; font-weight: 600; color: var(--fg-muted);
    border-bottom: 1px solid var(--border); padding: 6px 10px; font-size: 12px;
    text-transform: uppercase; letter-spacing: 0.03em;
  }
  td { padding: 7px 10px; border-bottom: 1px solid var(--border); vertical-align: top; }
  .row { cursor: pointer; }
  .row:hover td { background: var(--bg-elev); }
  .detail-row td { background: var(--bg-elev); }
  .nowrap { white-space: nowrap; }
  .q { color: var(--fg); }
  .empty { text-align: center; padding: 30px; }
</style>
