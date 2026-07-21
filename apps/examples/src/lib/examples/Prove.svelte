<script>
  // Client-side backward chaining: the vrules-wasm `prove` runs the engine's
  // goal-directed proof here in the browser — the same `vrules_core::prove` the native
  // daemon runs, no server round-trip. Pose a goal against GRL knowledge rules and
  // the engine works backward, reporting provability, missing facts, and the proof
  // tree.
  import init, { prove } from 'vrules-wasm/vrules_wasm.js';
  import wasmUrl from 'vrules-wasm/vrules_wasm_bg.wasm?url';

  // A two-rule CHAIN: UpgradeRule depends on IsVIP, which VIPRule derives from
  // LoyaltyPoints. Proving EligibleForUpgrade walks back through both rules —
  // the proof tree shows the chain (EligibleForUpgrade ← IsVIP ← LoyaltyPoints).
  let grl = $state('rule "VIPRule" {\n    when\n        User.LoyaltyPoints >= 1000\n    then\n        User.IsVIP = true;\n}\n\nrule "UpgradeRule" {\n    when\n        User.IsVIP == true\n    then\n        User.EligibleForUpgrade = true;\n}');
  let query = $state('query "CheckUpgrade" {\n    goal: User.EligibleForUpgrade == true\n    strategy: depth-first\n    max-depth: 5\n}');
  let factsText = $state('{ "User.LoyaltyPoints": 1200 }');

  let result = $state(null);
  let error = $state('');
  let busy = $state(false);
  let status = $state('');

  let initPromise;
  function ensureWasm() {
    if (!initPromise) initPromise = init(wasmUrl);
    return initPromise;
  }

  // serde-wasm-bindgen returns nested JS Maps; convert to plain objects for display.
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
</script>

<section>
  <h3>Backward-chaining <code>prove</code> — in the browser</h3>
  <p class="muted">
    The wasm engine runs the same <code>vrules::prove</code> the daemon runs, here in your
    browser. Pose a goal against GRL knowledge rules; the engine works backward and reports
    provability, missing facts, and the proof tree. The example <b>chains two rules</b>:
    <code>EligibleForUpgrade ← IsVIP ← LoyaltyPoints ≥ 1000</code> — the proof tree shows each
    link.
  </p>

  <div class="cols">
    <label class="col">GRL rules<textarea rows="7" bind:value={grl}></textarea></label>
    <label class="col">query (goal)<textarea rows="7" bind:value={query}></textarea></label>
  </div>
  <label>facts (JSON)<input bind:value={factsText} /></label>

  <div class="controls">
    <button class="primary" onclick={run} disabled={busy}>{busy ? 'proving…' : 'Prove'}</button>
    <span class="muted status">{status}</span>
  </div>

  {#if error}<div class="error">Error: {error}</div>{/if}

  {#if result}
    <div class="out">
      <div class="row">
        <span class="pill {result.provable ? 'hit' : 'miss'}">{result.provable ? 'PROVABLE ✓' : 'not provable'}</span>
        {#if result.missing_facts?.length}<span class="muted">missing: <code>{result.missing_facts.join(', ')}</code></span>{/if}
      </div>
      {#if result.bindings && Object.keys(result.bindings).length}
        <div class="row"><span class="muted">bindings</span><span class="mono">{JSON.stringify(result.bindings)}</span></div>
      {/if}
      <div class="label">proof tree</div>
      <pre class="detail">{JSON.stringify(result.proof, null, 2)}</pre>
    </div>
  {/if}
</section>

<style>
  section { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 14px; max-width: 860px; }
  h3 { margin: 0 0 4px; font-size: 14px; }
  .muted { font-size: 12px; }
  .cols { display: flex; gap: 12px; margin: 10px 0; flex-wrap: wrap; }
  .col { flex: 1; min-width: 240px; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--fg-muted); margin: 6px 0; }
  textarea, input { font-family: var(--mono, monospace); font-size: 12px; }
  .controls { display: flex; align-items: center; gap: 10px; margin: 10px 0; flex-wrap: wrap; }
  .status { margin-left: 4px; }
  .out { margin-top: 12px; border-top: 1px solid var(--border); padding-top: 10px; }
  .row { display: flex; align-items: center; gap: 10px; margin: 6px 0; flex-wrap: wrap; }
  .label { font-size: 11px; text-transform: uppercase; color: var(--fg-muted); margin: 10px 0 4px; }
  .pill.hit { color: var(--green); }
  .pill.miss { color: var(--fg-muted); }
  .mono { font-family: var(--mono, monospace); font-size: 12px; }
  .detail {
    background: var(--bg); border: 1px solid var(--border); border-radius: 6px;
    padding: 10px 12px; font-size: 11.5px; white-space: pre-wrap; overflow-x: auto;
  }
  .error { color: var(--red); margin: 8px 0; }
</style>
