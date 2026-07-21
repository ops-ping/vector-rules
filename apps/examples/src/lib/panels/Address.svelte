<script>
  import init, { verify_address } from 'vrules-wasm/vrules_wasm.js';
  import wasmUrl from 'vrules-wasm/vrules_wasm_bg.wasm?url';

  let { initial } = $props();

  const pretty = (value) => JSON.stringify(value, null, 2);
  const defaults = (() => ({
    text: initial.text,
    structured: pretty(initial.structured),
    rules: initial.rules,
    addressIndex: pretty(initial.addressIndex),
    referenceIndex: pretty(initial.referenceIndex)
  }))();

  let mode = $state('unstructured');
  let textInput = $state(defaults.text);
  let structuredInput = $state(defaults.structured);
  let rulesInput = $state(defaults.rules);
  let indexInput = $state(defaults.addressIndex);
  let referenceInput = $state(defaults.referenceIndex);
  let result = $state(null);
  let error = $state('');
  let status = $state('');
  let busy = $state(false);

  let initPromise;
  function ensureWasm() {
    if (!initPromise) initPromise = init(wasmUrl);
    return initPromise;
  }

  function deep(value) {
    if (value instanceof Map) {
      return Object.fromEntries([...value].map(([key, item]) => [key, deep(item)]));
    }
    if (Array.isArray(value)) return value.map(deep);
    if (value && typeof value === 'object') {
      return Object.fromEntries(
        Object.entries(value).map(([key, item]) => [key, deep(item)])
      );
    }
    return value;
  }

  async function run() {
    busy = true;
    error = '';
    status = 'running address rules…';
    result = null;
    try {
      await ensureWasm();
      const input = mode === 'structured' ? JSON.parse(structuredInput) : textInput;
      result = deep(verify_address(
        mode,
        JSON.stringify(input),
        rulesInput,
        indexInput,
        referenceInput
      ));
      status = 'done';
    } catch (cause) {
      status = '';
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  function resetInput() {
    textInput = defaults.text;
    structuredInput = defaults.structured;
  }
</script>

<section>
  <div class="head-row">
    <div>
      <h3>Address verification + organizational policy</h3>
      <p class="muted">
        One Rust/WASM path handles chat-like text and arbitrary structured JSON. Editable rules
        apply organizational policy after policy-neutral standardization and reference matching.
      </p>
    </div>
    <button class="primary" onclick={run} disabled={busy}>{busy ? 'running…' : 'Run'}</button>
  </div>

  <div class="mode-row">
    <button class:active={mode === 'unstructured'} onclick={() => (mode = 'unstructured')}>Unstructured</button>
    <button class:active={mode === 'structured'} onclick={() => (mode = 'structured')}>Structured JSON</button>
    <button onclick={resetInput}>Reset input</button>
    <span class="muted">{status}</span>
  </div>

  <div class="grid">
    <div class="card">
      <h4>{mode === 'structured' ? 'Structured input' : 'Chat / plain text input'}</h4>
      {#if mode === 'structured'}
        <textarea rows="14" bind:value={structuredInput}></textarea>
      {:else}
        <textarea rows="8" bind:value={textInput}></textarea>
      {/if}
    </div>

    <div class="card">
      <div class="card-head">
        <h4>Editable policy rules</h4>
        <button onclick={() => (rulesInput = defaults.rules)}>Reset rules</button>
      </div>
      <textarea rows="18" bind:value={rulesInput}></textarea>
    </div>
  </div>

  <details class="card">
    <summary>Native Rust address index records</summary>
    <p class="muted">
      Records use the canonical Rust index shape accepted by OpenAddresses-compatible ingests.
      Rules call <code>addr_index_score()</code> and <code>addr_index_match()</code>.
    </p>
    <div class="card-head">
      <h4>Index JSON</h4>
      <button onclick={() => (indexInput = defaults.addressIndex)}>Reset index</button>
    </div>
    <textarea rows="9" bind:value={indexInput}></textarea>
  </details>

  <details class="card" open>
    <summary>Reference data index records</summary>
    <p class="muted">
      Definitive JSON reference data for customers, products, or other entities. Matches expose
      exact and deterministic lexical evidence separately.
    </p>
    <div class="card-head">
      <h4>Reference JSON</h4>
      <button onclick={() => (referenceInput = defaults.referenceIndex)}>Reset reference</button>
    </div>
    <textarea rows="9" bind:value={referenceInput}></textarea>
  </details>

  {#if error}<div class="error">Error: {error}</div>{/if}

  {#if result}
    {@const policy = result.policy_fact}
    {@const native = result.native}
    <div class="results">
      <div class="score-card">
        <div class="big">{Math.round(native.validity_score * 100)}%</div>
        <div class="muted">native validity score</div>
        <span class="pill {native.valid ? 'hit' : 'miss'}">{native.valid ? 'valid profile' : 'weak profile'}</span>
        <span class="pill {policy.policy_status === 'invalid' ? 'miss' : 'hit'}">{policy.policy_status}</span>
      </div>
      <div class="card grow">
        <h4>Standardized form</h4>
        <p class="standard">{native.display || '—'}</p>
        <p><strong>Canonical:</strong> <code>{native.canonical || '—'}</code></p>
        {#if native.matches.length}
          <p><strong>Native index match:</strong> {native.matches[0].id} ({(native.matches[0].score * 100).toFixed(0)}%)</p>
        {/if}
        <p>
          <strong>Rule function evidence:</strong>
          addr_index_score = {result.function_evidence.addr_index_score};
          addr_index_match_id = <code>{result.function_evidence.addr_index_match_id || '—'}</code>;
          reference lexical hits = {result.function_evidence.ref_lexical_hits}
        </p>
        <p>
          <strong>Customer reference:</strong>
          <span class="pill {policy.reference_status === 'matched' ? 'hit' : 'miss'}">{policy.reference_status}</span>
          {#each result.reference_matches as match}
            <span class="pill hit">{match.kind}: {match.name} ({(match.score * 100).toFixed(0)}%)</span>
          {/each}
        </p>
        <p><strong>Policy:</strong> {policy.policy_reason}</p>
        <p class="muted">Fired rules: {(result.engine.fired || []).join(', ') || 'none'}</p>
      </div>
    </div>

    <div class="grid">
      <div class="card">
        <h4>Components</h4>
        <pre>{JSON.stringify(native.components, null, 2)}</pre>
      </div>
      <div class="card">
        <h4>Native field evidence</h4>
        {#if native.field_evidence.length}
          <table>
            <thead><tr><th>path</th><th>role</th><th>name</th><th>content</th><th>final</th></tr></thead>
            <tbody>
              {#each native.field_evidence as field}
                <tr>
                  <td><code>{field.path}</code></td>
                  <td>{field.role}</td>
                  <td>{(field.name_score * 100).toFixed(0)}%</td>
                  <td>{(field.content_score * 100).toFixed(0)}%</td>
                  <td>{(field.score * 100).toFixed(0)}%</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="muted">Structured-field inference appears in Structured JSON mode.</p>
        {/if}
      </div>
    </div>

    {#if native.embedding_hints.length || native.matches.length}
      <div class="grid">
        <div class="card">
          <h4>Embedding cache hints</h4>
          <table>
            <thead><tr><th>field</th><th>embedding text</th><th>namespace</th></tr></thead>
            <tbody>
              {#each native.embedding_hints as hint}
                <tr>
                  <td><code>{hint.field}</code></td>
                  <td>{hint.embedding_text}</td>
                  <td><code>{hint.cache_namespace}</code></td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        <div class="card">
          <h4>Native index matches</h4>
          {#if native.matches.length}
            <table>
              <thead><tr><th>id</th><th>score</th><th>display</th></tr></thead>
              <tbody>
                {#each native.matches as match}
                  <tr><td><code>{match.id}</code></td><td>{(match.score * 100).toFixed(0)}%</td><td>{match.display}</td></tr>
                {/each}
              </tbody>
            </table>
          {:else}
            <p class="muted">No index match over the supplied records.</p>
          {/if}
        </div>
      </div>
    {/if}

    <div class="card">
      <h4>Reference data matches</h4>
      {#if result.reference_matches.length}
        <table>
          <thead><tr><th>id</th><th>kind</th><th>name</th><th>score</th><th>exact</th><th>lexical</th><th>matched text</th></tr></thead>
          <tbody>
            {#each result.reference_matches as match}
              <tr>
                <td><code>{match.id}</code></td>
                <td>{match.kind}</td>
                <td>{match.name}</td>
                <td>{(match.score * 100).toFixed(0)}%</td>
                <td>{(match.exact_score * 100).toFixed(0)}%</td>
                <td>{(match.lexical_score * 100).toFixed(0)}%</td>
                <td>{match.matched_text}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <p class="muted">No customer, product, or other reference data matched the source text.</p>
      {/if}
    </div>

    <details class="card">
      <summary>Engine trace + raw result</summary>
      <pre>{JSON.stringify(result, null, 2)}</pre>
    </details>
  {/if}
</section>

<style>
  section { display: grid; gap: 14px; }
  h3, h4 { margin: 0 0 6px; }
  h3 { font-size: 15px; }
  h4 { font-size: 13px; }
  .head-row, .card-head, .mode-row, .results { display: flex; gap: 10px; align-items: center; }
  .head-row { justify-content: space-between; }
  .mode-row { flex-wrap: wrap; }
  .mode-row button.active { border-color: var(--accent); color: var(--accent); }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  .card, .score-card {
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 12px;
  }
  .grow { flex: 1; }
  .score-card { width: 190px; display: grid; gap: 6px; }
  .big { font-size: 34px; font-weight: 700; color: var(--accent); line-height: 1; }
  .standard { font-size: 18px; margin: 4px 0 8px; }
  textarea { min-height: 120px; }
  table { width: 100%; border-collapse: collapse; }
  th, td { text-align: left; border-bottom: 1px solid var(--border); padding: 5px 4px; font-size: 12px; }
  details summary { cursor: pointer; color: var(--accent); }
  @media (max-width: 850px) {
    .grid, .results { grid-template-columns: 1fr; display: grid; }
    .score-card { width: auto; }
  }
</style>
