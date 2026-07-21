<script>
  // Payment-request (BEC) triage: geometry artifacts fitted in the browser from
  // exemplar phrases, calibrated features consumed by symbolic GRL rules.
  // Mirrors shared-rules/fraud/triage.grl and the fraud_triage.rs test.
  import { onMount } from 'svelte';
  import init, { RuleEngine } from 'vrules-wasm/vrules_wasm.js';
  import wasmUrl from 'vrules-wasm/vrules_wasm_bg.wasm?url';
  import { embedText } from '../api.js';

  const URGENT_EXEMPLARS = [
    'urgent wire transfer needed immediately or we face penalty',
    'the ceo needs this payment today, keep it confidential',
    'act now, deadline is today, wire the account immediately',
    'executive request: transfer funds asap and be discreet',
    'immediately process this urgent payment, consequences otherwise',
    'boss says wire now, strictly confidential, deadline today'
  ];
  const CALM_EXEMPLARS = [
    'attached is the usual monthly invoice, thanks',
    'regular payment schedule attached, regards',
    'hello, the monthly account statement is attached',
    'hi, invoice attached per the usual schedule, thanks',
    'monthly transfer per the regular schedule, regards',
    'thanks, attached the invoice as usual'
  ];
  const NEUTRAL_CORPUS = [
    'hello, following up on the quarterly report',
    'the meeting is scheduled for next week, regards',
    'attached the notes from the review, thanks',
    'hi, can you confirm the delivery address',
    'regular maintenance window this weekend',
    'monthly newsletter draft attached',
    'thanks for the update, looks good',
    'invoice received, processing per the usual schedule'
  ];

  // Same pack as shared-rules/fraud/triage.grl.
  const TRIAGE_RULES = `rule "HoldUrgentPressureNewPayee" salience 100 no-loop {
    when
        c_project(Payment.text, "urgency_pressure_v1") >= 90.0 &&
        Payment.new_payee == true &&
        Payment.amount >= 10000.0
    then
        Decision.action = "hold";
        Decision.reason = "urgency-pressure language at or above the 90th percentile with a new payee and a large amount";
}

rule "HoldKnownBecPhrasing" salience 90 no-loop {
    when
        Decision.action != "hold" &&
        b_member(Payment.text, "bec_phrasing_v1") == true &&
        Payment.new_payee == true
    then
        Decision.action = "hold";
        Decision.reason = "message falls inside the known BEC phrasing region and the payee is new";
}

rule "RecordEvidence" salience 10 no-loop {
    when
        Payment.text != ""
    then
        Payment.urgency_pct = c_project(Payment.text, "urgency_pressure_v1");
        Payment.bec_depth = s_depth(Payment.text, "bec_phrasing_v1");
}`;

  const PRESETS = [
    {
      label: 'BEC-style request',
      text: 'urgent: the ceo needs a confidential wire transfer immediately, deadline today',
      newPayee: true,
      amount: 25000
    },
    {
      label: 'Routine invoice',
      text: 'hi, attached is the usual monthly invoice, thanks and regards',
      newPayee: false,
      amount: 25000
    },
    {
      label: 'Pushy but known payee',
      text: 'please process the transfer today, thanks',
      newPayee: false,
      amount: 3000
    }
  ];

  let engine = null;
  let status = $state('');
  let error = $state('');
  let busy = $state(false);
  let ready = $state(false);
  let text = $state(PRESETS[0].text);
  let newPayee = $state(PRESETS[0].newPayee);
  let amount = $state(PRESETS[0].amount);
  let outcome = $state(null);
  let provenance = $state(null);

  let initPromise;
  function ensureWasm() {
    if (!initPromise) initPromise = init(wasmUrl);
    return initPromise;
  }

  async function embed(t) {
    let payload;
    try {
      payload = await embedText(t);
    } catch (e) {
      throw new Error(`embed "${t}": ${e.message}`);
    }
    const { info, vector } = payload;
    if (!info?.model || !info?.revision || !info?.dimensions) {
      throw new Error(`embed "${t}": host omitted model metadata`);
    }
    return { info, vector: new Float32Array(vector) };
  }

  async function inject(eng, t) {
    const e = await embed(t);
    eng.set_embedding(t, e.vector, e.info.model, e.info.revision, e.info.dimensions);
  }

  function deep(v) {
    if (v instanceof Map) { const o = {}; for (const [k, val] of v) o[k] = deep(val); return o; }
    if (Array.isArray(v)) return v.map(deep);
    if (v && typeof v === 'object') {
      return Object.fromEntries(Object.entries(v).map(([k, val]) => [k, deep(val)]));
    }
    return v;
  }

  async function setup() {
    busy = true; error = ''; status = 'loading wasm…';
    try {
      await ensureWasm();
      const eng = new RuleEngine();
      status = 'embedding exemplar phrases via the host model…';
      for (const t of [...URGENT_EXEMPLARS, ...CALM_EXEMPLARS, ...NEUTRAL_CORPUS]) {
        await inject(eng, t);
      }
      status = 'fitting axis, calibration window, and region…';
      // Constructor tier, in the browser: axis = normalized difference of
      // exemplar centroids; percentile window from routine/neutral texts;
      // low-rank ellipsoid region over the urgent cloud.
      eng.fit_axis(
        'urgency_pressure_v1',
        JSON.stringify(URGENT_EXEMPLARS),
        JSON.stringify(CALM_EXEMPLARS),
        JSON.stringify([...NEUTRAL_CORPUS, ...CALM_EXEMPLARS])
      );
      eng.fit_region('bec_phrasing_v1', JSON.stringify(URGENT_EXEMPLARS), 3, 0.95);
      eng.register_rule(TRIAGE_RULES);
      const artifacts = JSON.parse(eng.artifacts_json());
      provenance = {
        axis: artifacts.axes?.[0]?.provenance,
        region: artifacts.regions?.[0]?.provenance,
        names: [
          ...(artifacts.axes || []).map((a) => a.name),
          ...(artifacts.regions || []).map((r) => r.name)
        ]
      };
      engine = eng;
      ready = true;
      status = 'artifacts fitted — triage a payment request below.';
      await triage();
    } catch (e) {
      status = '';
      error = e.message;
    } finally {
      busy = false;
    }
  }

  async function triage() {
    if (!engine) return;
    busy = true; error = '';
    try {
      status = 'embedding payment text…';
      await inject(engine, text);
      status = 'evaluating rules…';
      const res = deep(
        engine.evaluate(
          'Payment',
          JSON.stringify({ text, new_payee: newPayee, amount: Number(amount) || 0 }),
          true
        )
      );
      const decision = res.decision || {};
      const payment = res.facts?.Payment || {};
      outcome = {
        action: decision.action || 'approve',
        reason: decision.reason || 'routine — no hold rule fired',
        urgency: typeof payment.urgency_pct === 'number' ? payment.urgency_pct : null,
        depth: typeof payment.bec_depth === 'number' ? payment.bec_depth : null,
        fired: res.fired || [],
        raw: res
      };
      status = '';
    } catch (e) {
      status = '';
      error = e.message;
      outcome = null;
    } finally {
      busy = false;
    }
  }

  function applyPreset(p) {
    text = p.text;
    newPayee = p.newPayee;
    amount = p.amount;
    triage();
  }

  onMount(setup);
</script>

<section>
  <div class="head-row">
    <h3>Fraud triage — geometry + rules, in the browser</h3>
    <button class="rerun" onclick={triage} disabled={busy || !ready}>
      {busy ? 'working…' : '↻ Triage'}
    </button>
  </div>
  <p class="muted">
    A payment-request screen in the layered-fusion shape production fraud stacks use: the
    embedding layer supplies <em>named, versioned geometry</em> (an urgency-pressure axis with a
    percentile calibration window, and a BEC-phrasing region fitted from exemplars), and the
    symbolic rules make the decision — vector scores are evidence beside hard checks
    (new payee, amount), never a standalone gate. Every artifact carries model provenance and is
    validated against the active embedder at load.
    {#if status}<span class="status">— {status}</span>{/if}
  </p>

  {#if error}<div class="error">Error: {error}</div>{/if}

  <div class="presets">
    {#each PRESETS as p}
      <button class="preset" onclick={() => applyPreset(p)} disabled={busy || !ready}>{p.label}</button>
    {/each}
  </div>

  <div class="form">
    <label class="field">
      <span>Payment request text</span>
      <textarea rows="3" bind:value={text}></textarea>
    </label>
    <div class="row">
      <label class="check"><input type="checkbox" bind:checked={newPayee} /> new payee</label>
      <label class="field amount">
        <span>amount</span>
        <input type="number" bind:value={amount} min="0" step="100" />
      </label>
    </div>
  </div>

  {#if outcome}
    <div class="decision" class:hold={outcome.action === 'hold'}>
      <div class="action">{outcome.action === 'hold' ? '⛔ HOLD' : '✓ APPROVE'}</div>
      <div class="reason">{outcome.reason}</div>
      <div class="evidence">
        {#if outcome.urgency !== null}
          <span>urgency-pressure percentile: <code>{outcome.urgency.toFixed(1)}</code> <span class="muted">(hold ≥ 90 with new payee + amount)</span></span>
        {/if}
        {#if outcome.depth !== null}
          <span>BEC-region depth: <code>{outcome.depth.toFixed(2)}</code> <span class="muted">(≤ 1.0 is inside the fitted region)</span></span>
        {/if}
      </div>
      <div class="fired muted">fired: {outcome.fired.length ? outcome.fired.join(', ') : '(no rules)'}</div>
    </div>
  {/if}

  {#if provenance}
    <div class="prov">
      <span class="muted">artifacts:</span>
      {#each provenance.names as n}<code class="tag">{n}</code>{/each}
      {#if provenance.axis}
        <span class="muted">
          fitted against <code>{provenance.axis.model}</code> dim <code>{provenance.axis.dim}</code> —
          a different model or dimension is rejected at load.
        </span>
      {/if}
    </div>
  {/if}

  <details class="rules">
    <summary>GRL pack (shared-rules/fraud/triage.grl)</summary>
    <pre>{TRIAGE_RULES}</pre>
  </details>
</section>

<style>
  section { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 14px; max-width: 720px; }
  h3 { margin: 0 0 4px; font-size: 14px; }
  .muted { font-size: 12px; color: var(--fg-muted); }
  .head-row { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
  .rerun, .preset {
    flex: none; font-size: 11px; padding: 4px 10px; border: 1px solid var(--border);
    border-radius: 6px; background: var(--bg-elev2); color: var(--fg-muted); cursor: pointer;
  }
  .rerun:hover:not(:disabled), .preset:hover:not(:disabled) { color: var(--fg); }
  .rerun:disabled, .preset:disabled { opacity: 0.6; cursor: default; }
  .status { color: var(--fg-muted); }
  .presets { display: flex; gap: 6px; flex-wrap: wrap; margin: 10px 0 8px; }
  .form { display: grid; gap: 8px; }
  .field { display: grid; gap: 4px; font-size: 12px; color: var(--fg-muted); }
  .field textarea, .field input {
    background: var(--bg); color: var(--fg); border: 1px solid var(--border);
    border-radius: 6px; padding: 8px 10px; font-size: 12.5px; font-family: inherit;
  }
  .row { display: flex; gap: 16px; align-items: center; flex-wrap: wrap; }
  .check { font-size: 12.5px; display: flex; gap: 6px; align-items: center; }
  .amount input { width: 120px; }
  .decision {
    margin-top: 12px; border: 1px solid var(--border); border-left: 3px solid var(--green);
    border-radius: 6px; padding: 12px 14px;
  }
  .decision.hold { border-left-color: var(--red); }
  .action { font-weight: 700; font-size: 14px; }
  .decision.hold .action { color: var(--red); }
  .decision:not(.hold) .action { color: var(--green); }
  .reason { font-size: 12.5px; margin: 4px 0 8px; }
  .evidence { display: grid; gap: 3px; font-size: 12px; }
  .fired { margin-top: 8px; font-size: 11px; }
  .prov { margin-top: 12px; display: flex; gap: 6px; flex-wrap: wrap; align-items: baseline; font-size: 12px; }
  .prov .tag { border: 1px solid var(--border); border-radius: 4px; padding: 1px 6px; }
  .rules { margin-top: 12px; font-size: 12px; }
  .rules pre {
    margin-top: 8px; background: var(--bg); border: 1px solid var(--border); border-radius: 6px;
    padding: 10px 12px; font-size: 11.5px; white-space: pre-wrap; overflow-x: auto;
  }
  .error { color: var(--red); margin: 8px 0; }
</style>
