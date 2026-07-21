<script>
  import { whatifAssert, rulesValidate, rulesBranches } from '../api.js';

  // Two real-engine tools: forward what-if (assert facts → fired/decision/trace)
  // and validate (compile a rule the same way the engine does). Both hit the
  // shared service the LLM uses in-session.
  let factsText = $state('{ "tool": "web_ground", "query_len": 222, "query": "what are the current best-practice mitigations for the latest nginx ingress controller CVEs affecting a multi-tenant production GKE cluster, and how should we prioritize patching across teams in a phased maintenance window" }');
  let ruleset = $state('');
  let branches = $state([]);
  let result = $state(null);
  let error = $state('');
  let busy = $state(false);

  let ruleText = $state(`rule "EscalateLongQuery" no-loop {
    when Request.query_len > 200
    then Decision.effort = "high";
}`);
  let validation = $state(null);

  async function loadBranches() {
    try { branches = await rulesBranches(); } catch { branches = []; }
  }
  loadBranches();

  async function runWhatif() {
    busy = true; error = ''; result = null;
    try {
      const facts = JSON.parse(factsText);
      result = await whatifAssert(facts, 'Request', ruleset || null);
    } catch (e) { error = e.message; } finally { busy = false; }
  }

  async function validate() {
    error = ''; validation = null;
    try {
      validation = await rulesValidate(ruleText);
    } catch (e) { error = e.message; }
  }
</script>

<div class="grid">
  <section>
    <h3>Forward what-if</h3>
    <p class="muted">Assert facts into a ruleset, fire the engine, see what fired + the decision + the real trace.</p>
    <label>ruleset
      <select bind:value={ruleset}>
        <option value="">main (loaded)</option>
        {#each branches as b}<option value={b}>{b}</option>{/each}
      </select>
    </label>
    <label>facts (JSON)<textarea rows="4" bind:value={factsText}></textarea></label>
    <button class="primary" onclick={runWhatif} disabled={busy}>{busy ? 'firing…' : 'Assert & fire'}</button>

    {#if result}
      <div class="out">
        <div class="row"><span class="muted">fired</span>
          <span>{#each result.fired as f}<span class="pill">{f}</span> {/each}{#if !result.fired.length}<span class="muted">none</span>{/if}</span>
        </div>
        <div class="row"><span class="muted">decision</span><span class="mono">{JSON.stringify(result.decision)}</span></div>
        <div class="row"><span class="muted">cycles</span><span class="mono">{result.trace?.cycles}</span></div>
        <details><summary>full trace</summary><pre>{JSON.stringify(result.trace, null, 2)}</pre></details>
      </div>
    {/if}
  </section>

  <section>
    <h3>Validate a rule</h3>
    <p class="muted">Parse-check GRL with the engine's canonical parser.</p>
    <label>rule (GRL)<textarea rows="7" bind:value={ruleText}></textarea></label>
    <button onclick={validate}>Validate</button>
    {#if validation}
      <div class="out">
        {#if validation.ok}
          <span class="pill hit">✓ valid</span>
        {:else}
          <span class="pill miss">✗ invalid</span>
          <ul>{#each validation.errors as e}<li class="mono">{e.message}</li>{/each}</ul>
        {/if}
      </div>
    {/if}
  </section>
</div>

{#if error}<div class="error">{error}</div>{/if}

<style>
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
  @media (max-width: 760px) { .grid { grid-template-columns: 1fr; } }
  section { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 14px; }
  h3 { margin: 0 0 4px; font-size: 14px; }
  .muted { font-size: 12px; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--fg-muted); margin: 8px 0; }
  .out { margin-top: 12px; border-top: 1px solid var(--border); padding-top: 10px; }
  .row { display: flex; gap: 10px; padding: 3px 0; }
  .row .muted { min-width: 90px; }
  details summary { cursor: pointer; color: var(--accent); font-size: 12px; margin-top: 6px; }
  ul { margin: 6px 0 0; padding-left: 18px; }
  li { color: var(--red); }
</style>
