<script>
  // Client-side backward chaining: vrules-wasm `prove` runs goal-directed
  // backward chaining in the browser with no server round-trip.
  import { onMount } from 'svelte';
  import init, { prove } from 'vrules-wasm/vrules_wasm.js';
  import wasmUrl from 'vrules-wasm/vrules_wasm_bg.wasm?url';
  import CodeEditor from '../panels/CodeEditor.svelte';
  import Disclosure from '../panels/Disclosure.svelte';

  // Multi-tiered e-commerce approval flow: ApproveOrder depends on FundsAvailable & RiskIsLow
  let grl = $state(`rule "CheckFunds" {
    when
        Order.Amount <= Account.Balance
    then
        Status.FundsAvailable = true;
}

rule "CheckRisk" {
    when
        Account.AgeDays > 30
    then
        Status.RiskIsLow = true;
}

rule "ApproveOrder" {
    when
        Status.FundsAvailable == true && Status.RiskIsLow == true
    then
        Order.Approved = true;
}`);

  let query = $state(`query "ProveOrderApproval" {
    goal: Order.Approved == true
    strategy: depth-first
    max-depth: 5
}`);

  let factsText = $state('{ "Order.Amount": 50, "Account.Balance": 100, "Account.AgeDays": 45 }');

  // Inputs stay collapsed until asked for, so the proof tree is what the
  // example opens on.
  let showGrl = $state(false);
  let showQuery = $state(false);
  let showFacts = $state(false);

  let factsValid = $derived.by(() => {
    try {
      JSON.parse(factsText);
      return true;
    } catch {
      return false;
    }
  });

  let result = $state(null);
  let error = $state('');
  let busy = $state(false);
  let status = $state('');

  let initPromise;
  function ensureWasm() {
    if (!initPromise) initPromise = init(wasmUrl);
    return initPromise;
  }

  function deep(v) {
    if (v instanceof Map) { const o = {}; for (const [k, val] of v) o[k] = deep(val); return o; }
    if (Array.isArray(v)) return v.map(deep);
    if (v && typeof v === 'object') {
      return Object.fromEntries(Object.entries(v).map(([k, val]) => [k, deep(val)]));
    }
    return v;
  }

  async function run() {
    busy = true; error = ''; result = null; status = 'loading wasm…';
    try {
      await ensureWasm();
      status = 'proving in the browser…';
      result = deep(prove(grl, query, factsText));
      status = 'proved in the browser — no server round-trip.';
    } catch (e) {
      status = '';
      error = e.message ?? String(e);
    } finally {
      busy = false;
    }
  }

  onMount(run);
</script>

<section>
  <div class="head-row">
    <h3>Backward-Chaining Proof Engine — Mathematical Verification</h3>
    <button class="primary" onclick={run} disabled={busy}>{busy ? 'proving…' : '▶ Prove Goal'}</button>
  </div>
  <p class="muted">
    Goal-directed backward chaining (`vrules::prove`). Proves whether a goal like <code>Order.Approved == true</code> 
    is provable from a GRL knowledge base, producing an auditable proof tree or listing missing facts.
    {#if status}<span class="status">— {status}</span>{/if}
  </p>

  <div class="inputs">
    <Disclosure label="Knowledge base" badge="GRL" panel="grl" bind:open={showGrl}>
      <CodeEditor bind:value={grl} lang="grl" rows={12} label="GRL knowledge base" />
    </Disclosure>
    <Disclosure label="Query goal" badge="QUERY" panel="query" bind:open={showQuery}>
      <CodeEditor bind:value={query} lang="grl" rows={8} label="Query goal" />
    </Disclosure>
    <Disclosure
      label="Working facts"
      badge={factsValid ? 'JSON' : 'invalid'}
      bad={!factsValid}
      panel="facts"
      bind:open={showFacts}
    >
      <CodeEditor bind:value={factsText} lang="json" rows={5} label="Working facts, JSON" />
    </Disclosure>
  </div>

  {#if error}<div class="error">Error: {error}</div>{/if}

  {#if result}
    <div class="out">
      <div class="row">
        <span class="pill {result.provable ? 'hit' : 'miss'}">{result.provable ? 'PROVABLE ✓' : 'NOT PROVABLE ✗'}</span>
        {#if result.missing_facts?.length}
          <span class="muted">Missing Facts: <code>{result.missing_facts.join(', ')}</code></span>
        {/if}
      </div>
      {#if result.bindings && Object.keys(result.bindings).length}
        <div class="row"><span class="muted">Bindings:</span> <span class="mono">{JSON.stringify(result.bindings)}</span></div>
      {/if}
      <div class="label">Mathematical Proof Tree</div>
      <pre class="detail">{JSON.stringify(result.proof, null, 2)}</pre>
    </div>
  {/if}
</section>

<style>
  section { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 14px; max-width: 860px; }
  h3 { margin: 0 0 4px; font-size: 14px; }
  .muted { font-size: 12px; color: var(--fg-muted); }
  .head-row { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
  .inputs { display: grid; gap: 8px; margin: 10px 0; }
  .status { color: var(--fg-muted); }
  .primary { font-size: 12px; padding: 6px 14px; border: 1px solid var(--border); border-radius: 6px; background: var(--green, #22c55e); color: #fff; font-weight: 600; cursor: pointer; }
  .primary:hover:not(:disabled) { opacity: 0.9; }
  .primary:disabled { opacity: 0.6; cursor: default; }
  .out { margin-top: 12px; border-top: 1px solid var(--border); padding-top: 10px; }
  .row { display: flex; align-items: center; gap: 10px; margin: 6px 0; flex-wrap: wrap; }
  .label { font-size: 11px; font-weight: 600; text-transform: uppercase; color: var(--fg-muted); margin: 10px 0 4px; }
  .pill.hit { color: var(--green, #22c55e); font-weight: 700; font-size: 13px; }
  .pill.miss { color: var(--red, #ef4444); font-weight: 700; font-size: 13px; }
  .mono { font-family: var(--mono, monospace); font-size: 12px; }
  .detail {
    background: var(--bg); border: 1px solid var(--border); border-radius: 6px;
    padding: 10px 12px; font-size: 11px; white-space: pre-wrap; overflow-x: auto; max-height: 380px;
  }
  .error { color: var(--red); margin: 8px 0; }
</style>
