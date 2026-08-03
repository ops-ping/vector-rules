<script>
  // An editable, syntax-highlighted code surface with no editor dependency: a
  // transparent <textarea> over a highlighted <pre> that carries the same text
  // in the same metrics. The textarea keeps native editing, selection, undo and
  // accessibility; the layer beneath supplies colour.
  import { highlightGrl, highlightJson } from '../syntax.js';

  let {
    value = $bindable(''),
    lang = 'grl',
    rows = 14,
    // Fill the height of the parent instead of sizing to `rows`, so the editor
    // can be dropped into a pane whose height the layout decides.
    fill = false,
    readonly = false,
    label = '',
    describedBy = undefined,
    spellcheck = false
  } = $props();

  let view = $state(null);

  // A trailing newline collapses in a <pre>, which would leave the highlighted
  // layer one line short of the textarea; a sentinel keeps the two in step.
  let painted = $derived((lang === 'json' ? highlightJson(value) : highlightGrl(value)) + '\n');

  function syncScroll(event) {
    if (!view) return;
    view.scrollTop = event.currentTarget.scrollTop;
    view.scrollLeft = event.currentTarget.scrollLeft;
  }

  // Tab indents instead of leaving the field: this is a code surface, and both
  // GRL and JSON are read at a glance far more often than they are tabbed past.
  function onKeydown(event) {
    if (event.key !== 'Tab' || event.shiftKey) return;
    event.preventDefault();
    const field = event.currentTarget;
    const { selectionStart: from, selectionEnd: to } = field;
    value = `${value.slice(0, from)}    ${value.slice(to)}`;
    queueMicrotask(() => field.setSelectionRange(from + 4, from + 4));
  }
</script>

<div class="code" class:fill style:--rows={rows}>
  <pre class="paint" bind:this={view} aria-hidden="true"><code>{@html painted}</code></pre>
  <textarea
    bind:value
    onscroll={syncScroll}
    onkeydown={onKeydown}
    {readonly}
    aria-label={label}
    aria-describedby={describedBy}
    {spellcheck}
    wrap="off"
    autocapitalize="off"
    autocomplete="off"
    autocorrect="off"
  ></textarea>
</div>

<style>
  .code {
    position: relative;
    height: calc(var(--rows) * 1.55em + 20px);
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    overflow: hidden;
  }
  .code.fill { height: 100%; }

  /* Both layers must agree on every metric that moves a glyph. */
  .paint,
  .code textarea {
    margin: 0;
    padding: 10px 12px;
    border: 0;
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 1.55;
    tab-size: 4;
    white-space: pre;
    overflow: auto;
    width: 100%;
    height: 100%;
  }

  .paint {
    background: transparent;
    border-radius: 0;
    color: var(--fg);
    pointer-events: none;
  }

  .code textarea {
    position: absolute;
    inset: 0;
    resize: none;
    background: transparent;
    color: transparent;
    caret-color: var(--accent);
    outline: none;
  }
  .code:focus-within { border-color: var(--accent); }
  .code textarea::selection { background: rgba(88, 166, 255, 0.3); }

  .paint :global(.t-comment) { color: var(--fg-muted); font-style: italic; }
  .paint :global(.t-string) { color: #a5d6ff; }
  .paint :global(.t-vop) { color: var(--green); }
  .paint :global(.t-key) { color: #7ee787; }
  .paint :global(.t-number) { color: #79c0ff; }
  .paint :global(.t-keyword) { color: #ff7b72; }
  .paint :global(.t-fn) { color: #d2a8ff; }
  .paint :global(.t-path) { color: #ffa657; }
  .paint :global(.t-op) { color: #ff7b72; }
  .paint :global(.t-punct) { color: var(--fg-muted); }
</style>
