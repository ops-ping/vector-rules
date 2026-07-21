<script>
  import { abRun, whatifProve, rulesBranches, rulesDiff, rulesPromote } from '../api.js';

  // A/B by git branch (the management interface's core op) + backward-chaining
  // prove + the human sign-off (promote a branch to main).
  let branches = $state([]);
  let a = $state('main');
  let b = $state('main');
  let factsText = $state('{ "tool": "web_ground", "query_len": 39, "query": "who won the 2024 nobel prize in physics" }');
  let ab = $state(null);
  let fileDiff = $state(null);
  let error = $state('');
  let busy = $state(false);
  let promoteMsg = $state('');

  async function loadBranches() {
    try {
      branches = await rulesBranches();
      if (branches.length > 1) b = branches.find((x) => x !== a) ?? branches[0];
    } catch { branches = []; }
  }
  loadBranches();

  async function runAb() {
    busy = true; error = ''; ab = null; fileDiff = null;
    try {
      const facts = JSON.parse(factsText);
      ab = await abRun(a, b, facts);
      fileDiff = await rulesDiff(a, b);
    } catch (e) { error = e.message; } finally { busy = false; }
  }

  async function promote() {
    promoteMsg = ''; error = '';
    if (!confirm(`Sign off: promote "${b}" → main? This changes what the runtime runs.`)) return;
    try {
      const r = await rulesPromote(b, 'main');
      promoteMsg = `✓ promoted ${b} → main @ ${r.sha.slice(0, 12)}`;
      await loadBranches();
    } catch (e) { error = e.message; }
  }

  // backward
  // A two-rule CHAIN (see Prove panel): EligibleForUpgrade ← IsVIP ← LoyaltyPoints.
  let grl = $state('rule "VIPRule" {\n    when\n        User.LoyaltyPoints >= 1000\n    then\n        User.IsVIP = true;\n}\n\nrule "UpgradeRule" {\n    when\n        User.IsVIP == true\n    then\n        User.EligibleForUpgrade = true;\n}');
  let query = $state('query "CheckUpgrade" {\n    goal: User.EligibleForUpgrade == true\n    strategy: depth-first\n    max-depth: 5\n}');
  let proveFacts = $state('{ "User.LoyaltyPoints": 1200 }');
  let proof = $state(null);

  async function runProve() {
    error = ''; proof = null;
    try {
      proof = await whatifProve(grl, query, JSON.parse(proveFacts));
    } catch (e) { error = e.message; }
  }
</script>

<section>
  <h3>A/B by branch</h3>
  <p class="muted">Run the same facts through two rule revisions and diff what fired — evaluated from each commit, no checkout.</p>
  <div class="controls">
    <label>baseline
      <select bind:value={a}>{#each branches as x}<option value={x}>{x}</option>{/each}</select>
    </label>
    <label>variant
      <select bind:value={b}>{#each branches as x}<option value={x}>{x}</option>{/each}</select>
    </label>
    <label class="grow">facts (JSON)<input bind:value={factsText} /></label>
    <button class="primary" onclick={runAb} disabled={busy}>{busy ? '…' : 'Compare'}</button>
  </div>

  {#if ab}
    <div class="cols">
      <div class="col"><h4>{ab.a.ref}</h4>{#each ab.a.result.fired as f}<span class="pill">{f}</span> {/each}</div>
      <div class="col"><h4>{ab.b.ref}</h4>{#each ab.b.result.fired as f}<span class="pill">{f}</span> {/each}</div>
    </div>
    <div class="diff">
      {#if ab.diff.fired_only_a.length}<div>only in <b>{ab.a.ref}</b>: {#each ab.diff.fired_only_a as f}<span class="pill miss">{f}</span> {/each}</div>{/if}
      {#if ab.diff.fired_only_b.length}<div>only in <b>{ab.b.ref}</b>: {#each ab.diff.fired_only_b as f}<span class="pill hit">{f}</span> {/each}</div>{/if}
      {#if !ab.diff.fired_only_a.length && !ab.diff.fired_only_b.length}<span class="muted">same fired set</span>{/if}
      <div class="muted">decision changed: {ab.diff.decision_changed}</div>
      {#if fileDiff?.changes?.length}<div class="muted">files: {#each fileDiff.changes as c}<span class="mono">{c.path} ({c.status})</span> {/each}</div>{/if}
    </div>
    {#if b !== 'main'}
      <div class="signoff">
        <button onclick={promote}>✓ Sign off &amp; promote “{b}” → main</button>
        <span class="muted">Human approval — the LLM proposes, you promote.</span>
      </div>
    {/if}
    {#if promoteMsg}<div class="ok">{promoteMsg}</div>{/if}
  {/if}
</section>

<section>
  <h3>Backward-chaining (prove)</h3>
  <p class="muted">Pose a goal against GRL knowledge rules; the engine works backward and reports provability + missing facts.</p>
  <div class="cols">
    <label class="col">GRL rules<textarea rows="6" bind:value={grl}></textarea></label>
    <label class="col">query (goal)<textarea rows="6" bind:value={query}></textarea></label>
  </div>
  <label>facts (JSON)<input bind:value={proveFacts} /></label>
  <button class="primary" onclick={runProve}>Prove</button>
  {#if proof}
    <div class="out">
      <span class="pill" class:hit={proof.provable} class:miss={!proof.provable}>{proof.provable ? 'PROVABLE' : 'not provable'}</span>
      {#if proof.missing_facts?.length}<span class="muted">missing: {proof.missing_facts.join(', ')}</span>{/if}
      <details><summary>proof</summary><pre>{JSON.stringify(proof.proof, null, 2)}</pre></details>
    </div>
  {/if}
</section>

{#if error}<div class="error">{error}</div>{/if}

<style>
  section { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 14px; margin-bottom: 16px; }
  h3 { margin: 0 0 4px; font-size: 14px; }
  h4 { margin: 0 0 6px; font-size: 12px; text-transform: uppercase; color: var(--fg-muted); }
  .muted { font-size: 12px; }
  .controls { display: flex; gap: 10px; align-items: flex-end; flex-wrap: wrap; margin: 10px 0; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--fg-muted); }
  label.grow { flex: 1; min-width: 200px; }
  .cols { display: flex; gap: 14px; flex-wrap: wrap; margin: 10px 0; }
  .col { flex: 1; min-width: 200px; }
  .diff { border-top: 1px solid var(--border); padding-top: 10px; display: flex; flex-direction: column; gap: 4px; }
  .signoff { margin-top: 12px; display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .out { margin-top: 10px; display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  details summary { cursor: pointer; color: var(--accent); font-size: 12px; }
  .ok { color: var(--green); margin-top: 8px; }
</style>
