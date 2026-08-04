<script>
  // A collapsed-by-default input section. Editors are tall, and an example that
  // opens with three of them stacked buries the thing a reader came to see, so
  // each one stays a single header row until it is asked for. Sections are
  // independent rather than mutually exclusive: opening one does not close
  // another, and closing the last one leaves the caller free to reclaim the
  // space entirely.
  let {
    label,
    badge = '',
    bad = false,
    open = $bindable(false),
    panel = '',
    children
  } = $props();
</script>

<section class="disclosure" class:open data-section={panel}>
  <button
    class="head"
    type="button"
    aria-expanded={open}
    data-panel={panel}
    onclick={() => (open = !open)}
  >
    <span class="caret" aria-hidden="true">{open ? '▾' : '▸'}</span>
    <span class="label">{label}</span>
    {#if badge}<span class="badge" class:bad>{badge}</span>{/if}
  </button>
  {#if open}
    <div class="body">{@render children()}</div>
  {/if}
</section>

<style>
  .disclosure {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    min-width: 0;
  }
  .disclosure.open { border-color: var(--accent); }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    text-align: left;
    font-size: 12px;
  }
  .head:hover { background: var(--bg-elev2); }
  .disclosure.open .head { color: var(--accent); border-radius: 6px 6px 0 0; }
  .caret { color: var(--fg-muted); font-size: 10px; width: 10px; }
  .disclosure.open .caret { color: var(--accent); }
  .label { font-weight: 600; }
  .badge {
    margin-left: auto;
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--fg-muted);
  }
  .badge.bad { color: var(--red); }

  .body {
    display: grid;
    gap: 6px;
    padding: 0 10px 10px;
    min-width: 0;
  }
</style>
