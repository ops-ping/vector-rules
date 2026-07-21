<script>
  let { record } = $props();
  const r = $derived(record);
</script>

<div class="rec">
  <div class="grid">
    <div><span class="muted">session</span><div class="mono">{r.session_id}</div></div>
    {#if r.child_session}
      <div><span class="muted">child</span><div class="mono">{r.child_session}</div></div>
    {/if}
    <div><span class="muted">process</span><div class="mono">{r.process_id}</div></div>
    <div><span class="muted">event id</span><div class="mono">{r.id}</div></div>
    <div><span class="muted">backend</span><div class="mono">{r.backend}</div></div>
    <div>
      <span class="muted">cache</span>
      <div><span class="pill" class:hit={r.cache === 'hit'} class:miss={r.cache === 'miss'}>{r.cache}</span></div>
    </div>
  </div>

  <h4>request</h4>
  <pre>{JSON.stringify(r.request, null, 2)}</pre>

  <h4>answer</h4>
  <pre class="answer">{r.answer}</pre>

  {#if r.fired?.length}
    <h4>fired routing rules</h4>
    <div class="chips">
      {#each r.fired as f}<span class="pill">{f}</span>{/each}
    </div>
  {/if}

  {#if r.trace}
    <h4>engine trace</h4>
    <pre>{JSON.stringify(r.trace, null, 2)}</pre>
  {/if}
</div>

<style>
  .rec { padding: 4px 2px 10px; }
  .grid {
    display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 10px 18px; margin-bottom: 12px;
  }
  .grid .muted { font-size: 11px; text-transform: uppercase; letter-spacing: 0.03em; }
  h4 { margin: 12px 0 5px; font-size: 12px; text-transform: uppercase; color: var(--fg-muted); letter-spacing: 0.03em; }
  .answer { white-space: pre-wrap; }
  .chips { display: flex; gap: 6px; flex-wrap: wrap; }
</style>
