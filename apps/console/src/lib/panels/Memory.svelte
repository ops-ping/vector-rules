<script>
  import { memorySearch, memoryHistory, memoryStats } from '../api.js';
  import { fmtTime, truncate } from '../format.js';

  let query = $state('');
  let mode = $state('recent');
  let includeSuperseded = $state(false);
  let includeDeleted = $state(false);
  let records = $state([]);
  let loading = $state(false);
  let error = $state('');
  let expanded = $state(null); // memory id of the open row
  let lineage = $state([]);

  let stats = $state(null);

  async function refresh() {
    loading = true;
    error = '';
    try {
      records = await memorySearch({
        query: query.trim() || undefined,
        k: 50,
        mode,
        include_superseded: includeSuperseded,
        include_deleted: includeDeleted
      });
    } catch (e) {
      error = e.message;
      records = [];
    } finally {
      loading = false;
    }
    loadStats();
  }

  async function loadStats() {
    try {
      stats = await memoryStats();
    } catch {
      stats = null;
    }
  }

  async function toggleRow(id) {
    if (expanded === id) {
      expanded = null;
      lineage = [];
      return;
    }
    expanded = id;
    lineage = [];
    try {
      lineage = await memoryHistory(id);
    } catch (e) {
      error = e.message;
    }
  }

  function runRecent() { mode = 'recent'; query = ''; refresh(); }
  function runSearch() { mode = query.trim() ? 'semantic' : 'recent'; refresh(); }

  // Initial view: most recent memories + usage.
  refresh();
</script>

<div class="toolbar">
  <div class="search">
    <input
      placeholder="recall by meaning…"
      bind:value={query}
      onkeydown={(e) => e.key === 'Enter' && (mode = query.trim() ? 'semantic' : 'recent', refresh())}
    />
    <select bind:value={mode} onchange={refresh} title="retrieval axis">
      <option value="semantic">semantic</option>
      <option value="recent">recent</option>
    </select>
    <button onclick={runSearch} disabled={loading}>Search</button>
    <button onclick={runRecent} disabled={loading} title="newest first">↻ Recent</button>
  </div>
  <label class="chk"><input type="checkbox" bind:checked={includeSuperseded} onchange={refresh} /> superseded</label>
  <label class="chk"><input type="checkbox" bind:checked={includeDeleted} onchange={refresh} /> deleted</label>
  <span class="muted">{records.length} memor{records.length === 1 ? 'y' : 'ies'}</span>
</div>

{#if error}<div class="error">{error}</div>{/if}

{#if stats}
  <div class="stats">
    <div class="stat"><span class="n">{stats.streams?.memory ?? 0}</span><span class="l">memory events</span></div>
    <div class="stat"><span class="n">{stats.segments ?? 0}</span><span class="l">segments</span></div>
  </div>
{/if}

<table>
  <thead>
    <tr><th>time</th><th>status</th><th>tags</th><th>fact</th></tr>
  </thead>
  <tbody>
    {#each records as r (r.id)}
      <tr class="row" onclick={() => toggleRow(r.id)}>
        <td class="mono muted nowrap">{fmtTime(r.created_at)}</td>
        <td>
          <span class="pill" class:deleted={r.status === 'deleted'}>{r.status}</span>
        </td>
        <td class="mono muted">{(r.tags ?? []).join(', ')}</td>
        <td>{truncate(r.fact, 110)}</td>
      </tr>
      {#if expanded === r.id}
        <tr class="detail-row"><td colspan="4">
          <div class="lineage">
            <span class="muted">lineage ({lineage.length})</span>
            {#each lineage as h}
              <div class="lin-row">
                <span class="pill"
                  class:live={h.status === 'live'}
                  class:superseded={h.status === 'superseded'}
                  class:deleted={h.status === 'deleted'}>{h.status}</span>
                <span class="mono muted nowrap">{fmtTime(h.created_at)}</span>
                <span>{h.fact}</span>
                {#if h.reason}<span class="muted">— {h.reason}</span>{/if}
              </div>
            {/each}
            <span class="mono muted id">id {r.id}</span>
          </div>
        </td></tr>
      {/if}
    {/each}
    {#if !records.length && !loading}
      <tr><td colspan="4" class="muted empty">no memories — agents persist facts with memory_write, then they appear here</td></tr>
    {/if}
  </tbody>
</table>

<style>
  .toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-bottom: 14px; }
  .search { display: flex; gap: 6px; flex: 1; min-width: 300px; }
  .search input { flex: 1; }
  .chk { display: flex; align-items: center; gap: 4px; font-size: 13px; color: var(--fg-muted); }
  .stats {
    display: flex; align-items: center; gap: 18px; flex-wrap: wrap;
    padding: 10px 14px; margin-bottom: 14px;
    border: 1px solid var(--border); border-radius: 8px; background: var(--bg-elev);
  }
  .stat { display: flex; flex-direction: column; align-items: center; }
  .stat .n { font-size: 20px; font-weight: 700; }
  .stat .l { font-size: 11px; text-transform: uppercase; letter-spacing: 0.03em; color: var(--fg-muted); }
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
  .empty { text-align: center; padding: 30px; }
  .lineage { display: flex; flex-direction: column; gap: 6px; padding: 4px 2px; }
  .lin-row { display: flex; align-items: baseline; gap: 8px; }
  .lineage .id { margin-top: 4px; }
  .pill {
    display: inline-block; padding: 1px 8px; border-radius: 999px;
    font-size: 12px; border: 1px solid var(--border); background: var(--bg-elev2);
  }
  .pill.live { color: var(--green); border-color: var(--green); }
  .pill.superseded { color: var(--fg-muted); }
  .pill.deleted { color: var(--red); border-color: var(--red); }
</style>
