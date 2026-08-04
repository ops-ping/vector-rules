<script>
  // An editable semantic rule bench, laid out as input beside output: the left
  // pane holds everything that goes in — the rules, the asserted facts, and the
  // fitted geometry the rules decide on — and the right pane holds what the
  // engine produced. Nothing here is a transcript of a canned run: the trace,
  // the value each rule derived and the output facts are read back from the
  // engine's own result, so editing a rule changes what the trace reports.
  import { onMount } from 'svelte';
  import init, { RuleEngine, validate_rule } from 'vrules-wasm/vrules_wasm.js';
  import wasmUrl from 'vrules-wasm/vrules_wasm_bg.wasm?url';
  import { embedText, probeVectorSource, subscribeModel } from '../embed.js';
  import { splitRules, pathsRead, pathsAssigned, vectorInputs } from '../syntax.js';
  import CodeEditor from '../panels/CodeEditor.svelte';
  import Disclosure from '../panels/Disclosure.svelte';

  // Royal proclamations. A message is scored on two independent calibrated
  // axes — is this a sovereign speaking, and is the intent punitive — and the
  // response falls out of the 2x2. Both axes separate cleanly on this model and
  // are near-orthogonal (cosine 0.136 between their directions), so they are
  // genuinely two questions rather than one signal under two names.
  //
  // `s_cosine` stays a measurement and gates nothing: it identifies the speaker
  // via inline vector algebra, and generalises to regnal names it was never
  // shown (Elizabeth I of England 0.72, Queen Victoria 0.75, against the mayor
  // 0.65 and tractor 0.62). The decisions are made on `c_project`, whose
  // percentile means the same thing on any model.
  const DEFAULT_RULES = `rule "IdentifySpeaker" no-loop {
    when
        Message.speaker != ""
    then
        Message.queenlike = s_cosine(
            ["v:add", ["v:sub", "king", "man"], "woman"],
            Message.speaker);
}

rule "MeasureVoice" no-loop {
    when
        Message.text != ""
    then
        Message.voice_pct =
            c_project(Message.text, "sovereign_voice");
}

rule "MeasureIntent" no-loop {
    when
        Message.text != ""
    then
        Message.displeasure_pct =
            c_project(Message.text, "displeasure");
}

// c_project is calibrated, so it may be thresholded here in \`when\`.
// s_cosine may not: the lint rejects thresholding a raw scalar.
rule "FallOnSword" no-loop {
    when
        Message.voice_pct > 60 && Message.displeasure_pct > 75
    then
        Message.verdict = "sovereign displeasure";
        Decision.response = "fall on your sword";
}

rule "Apologise" no-loop {
    when
        Message.voice_pct <= 60 && Message.displeasure_pct > 75
    then
        Message.verdict = "displeasure, but not from the throne";
        Decision.response = "apologise";
}

rule "Acknowledge" no-loop {
    when
        Message.displeasure_pct <= 75
    then
        Message.verdict = "no displeasure";
        Decision.response = "acknowledge";
}`;

  const DEFAULT_FACTS = `{
  "Message": {
    "speaker": "queen",
    "text": "We are gravely displeased by this betrayal, and Our judgement shall be swift"
  }
}`;

  const DEFAULT_AXES = `{
  "sovereign_voice": {
    "positive": [
      "We hereby decree, by Our sovereign authority, that it shall be so",
      "By royal command, let it be proclaimed throughout the realm",
      "It is Our will that this matter be settled before the coming feast",
      "We, by the grace of God, Queen of these lands, do declare",
      "Let the herald announce Our judgement to every province"
    ],
    "negative": [
      "hey can you send me that file when you get a sec",
      "please find attached the monthly invoice for your records",
      "just following up on my last email about the meeting",
      "thanks for the update, looks good to me",
      "can we move the standup to 10 tomorrow"
    ],
    "calibration": [
      "We hereby decree that the harvest tax is reduced this season",
      "By Our command the gates shall open at dawn",
      "please could you review the attached draft",
      "the meeting notes are in the shared folder",
      "we are gravely displeased with this dereliction",
      "this is completely unacceptable and must be answered for",
      "we are delighted with the outcome, thank you",
      "great work everyone, really pleased with this",
      "the quarterly numbers are attached for your review",
      "let it be known that Our judgement is final",
      "sorry for the delay, been swamped this week",
      "We commend the loyal service of Our subjects",
      "can you confirm receipt of the shipment",
      "your continued failure has exhausted Our patience",
      "thanks, that clears it up nicely",
      "the maintenance window is scheduled for Sunday"
    ]
  },
  "displeasure": {
    "positive": [
      "We are gravely displeased and this failure shall not go unanswered",
      "this is unacceptable and there will be consequences for it",
      "you have failed us, and we shall remember who was responsible",
      "our patience is exhausted; answer for this at once",
      "we are bitterly disappointed by what has been allowed to happen"
    ],
    "negative": [
      "We are delighted, and offer Our warmest thanks for your service",
      "this is excellent work and we are grateful for the effort",
      "thank you kindly, we are pleased with how this was handled",
      "wonderful news, well done to everyone involved",
      "we are content, and commend all who took part"
    ],
    "calibration": [
      "We hereby decree that the harvest tax is reduced this season",
      "By Our command the gates shall open at dawn",
      "please could you review the attached draft",
      "the meeting notes are in the shared folder",
      "we are gravely displeased with this dereliction",
      "this is completely unacceptable and must be answered for",
      "we are delighted with the outcome, thank you",
      "great work everyone, really pleased with this",
      "the quarterly numbers are attached for your review",
      "let it be known that Our judgement is final",
      "sorry for the delay, been swamped this week",
      "We commend the loyal service of Our subjects",
      "can you confirm receipt of the shipment",
      "your continued failure has exhausted Our patience",
      "thanks, that clears it up nicely",
      "the maintenance window is scheduled for Sunday"
    ]
  }
}`;

  // The engine reports an unresolvable vector by name; that message is the
  // authority on what a ruleset needs, so a miss by the pre-resolver below is
  // recovered from rather than guessed around.
  const MISSING_EMBEDDING = /no prefetched embedding for "((?:[^"\\]|\\.)*)"/;
  const RESOLVE_ATTEMPTS = 8;

  let rulesText = $state(DEFAULT_RULES);
  let factsText = $state(DEFAULT_FACTS);
  let axesText = $state(DEFAULT_AXES);
  let result = $state(null);
  let busy = $state(false);
  let status = $state('');
  let error = $state('');
  let ready = $state(false);
  let modelPhase = $state('absent');
  let outputTab = $state('trace'); // 'trace' | 'facts'
  // Where each parameter's vector would come from, keyed by its text.
  let sources = $state({});

  // Input sections are independent and all start closed, so the bench opens on
  // its result rather than on three stacked editors.
  let showRules = $state(false);
  let showFacts = $state(false);
  let showAxes = $state(false);

  let initPromise;
  function ensureWasm() {
    if (!initPromise) initPromise = init(wasmUrl);
    return initPromise;
  }

  // serde-wasm-bindgen returns nested JS Maps; convert to plain objects.
  function deep(value) {
    if (value instanceof Map) {
      const out = {};
      for (const [key, inner] of value) out[key] = deep(inner);
      return out;
    }
    if (Array.isArray(value)) return value.map(deep);
    if (value && typeof value === 'object') {
      return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, deep(v)]));
    }
    return value;
  }

  // --- live validation of the three editable sources ------------------------

  function checkFacts(text) {
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch (e) {
      return { ok: false, message: e.message };
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return { ok: false, message: 'facts must be a JSON object keyed by fact type' };
    }
    // `evaluate` asserts one fact type and adds an empty `Decision` itself.
    const types = Object.keys(parsed).filter((key) => key !== 'Decision');
    if (types.length !== 1) {
      return {
        ok: false,
        message: `assert exactly one fact type (Decision is added by the engine) — found ${types.length}`
      };
    }
    const body = parsed[types[0]];
    if (!body || typeof body !== 'object' || Array.isArray(body)) {
      return { ok: false, message: `${types[0]} must be a JSON object` };
    }
    return { ok: true, message: '', type: types[0], body };
  }

  function checkAxes(text) {
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch (e) {
      return { ok: false, message: e.message };
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return { ok: false, message: 'axes must be a JSON object keyed by axis name' };
    }
    const axes = [];
    for (const [name, spec] of Object.entries(parsed)) {
      for (const set of ['positive', 'negative', 'calibration']) {
        if (!Array.isArray(spec?.[set]) || !spec[set].every((t) => typeof t === 'string')) {
          return { ok: false, message: `axis "${name}" needs a "${set}" array of strings` };
        }
      }
      if (spec.calibration.length < 2) {
        return { ok: false, message: `axis "${name}": a calibration window needs at least two texts` };
      }
      axes.push({ name, ...spec });
    }
    return { ok: true, message: '', axes };
  }

  function checkRules(text) {
    if (!ready) return { ok: true, message: '' };
    const report = deep(validate_rule(text));
    if (report?.ok) return { ok: true, message: '' };
    const message = (report?.errors ?? []).map((e) => e.message).join('; ');
    return { ok: false, message: message || 'the rule parser rejected this source' };
  }

  let factsCheck = $derived(checkFacts(factsText));
  let axesCheck = $derived(checkAxes(axesText));
  let rulesCheck = $derived(checkRules(rulesText));
  let rules = $derived(splitRules(rulesText));
  let runnable = $derived(ready && factsCheck.ok && axesCheck.ok && rulesCheck.ok);
  let anyInputOpen = $derived(showRules || showFacts || showAxes);

  function factValue(facts, path) {
    const [type, field] = path.split('.');
    const value = facts?.[type]?.[field];
    return value === undefined ? null : value;
  }

  // Every fact field the rules feed into vector math becomes an editable
  // parameter. The facts JSON stays the single source of truth — each input
  // reads its value straight out of it, so the two cannot drift apart.
  let params = $derived(
    factsCheck.ok
      ? vectorInputs(rulesText)
          .paths.filter((path) => path.startsWith(`${factsCheck.type}.`))
          .map((path) => ({ path, field: path.split('.')[1] }))
          .filter(({ field }) => typeof factsCheck.body[field] === 'string')
          .map((p) => ({ ...p, value: factsCheck.body[p.field] }))
      : []
  );

  function onParamInput(field, value) {
    if (!factsCheck.ok) return;
    const next = JSON.parse(factsText);
    next[factsCheck.type][field] = value;
    factsText = JSON.stringify(next, null, 2);
  }

  // Whether running will read a file or download the model, answered per
  // parameter before the run rather than discovered during it.
  $effect(() => {
    const texts = params.map((p) => p.value).filter(Boolean);
    let live = true;
    const timer = setTimeout(async () => {
      const found = {};
      for (const text of texts) {
        found[text] = await probeVectorSource(text).catch(() => '');
      }
      if (live) sources = found;
    }, 250);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  });

  onMount(() => {
    // Loading the engine is not running the example: it makes the rule parser
    // available so edits are validated as they are typed.
    ensureWasm().then(
      () => (ready = true),
      (e) => (error = `engine failed to load: ${e.message}`)
    );
    return subscribeModel((state) => (modelPhase = state.phase));
  });

  // --- execution -------------------------------------------------------------

  /** Resolve one text's vector and hand it to the engine, recording its source. */
  async function resolve(engine, provenance, text) {
    if (provenance.has(text)) return;
    let payload;
    try {
      payload = await embedText(text);
    } catch (e) {
      throw new Error(`embed "${text}": ${e.message}`);
    }
    const { info, vector, source } = payload;
    if (!info?.model || !info?.revision || !info?.dimensions) {
      throw new Error(`embed "${text}": host omitted model metadata`);
    }
    engine.set_embedding(text, new Float32Array(vector), info.model, info.revision, info.dimensions);
    provenance.set(text, { source, dimensions: info.dimensions });
  }

  /** Every text a run needs a vector for, before the engine is asked. */
  function plannedTexts(facts, axes) {
    const { literals, paths } = vectorInputs(rulesText);
    const texts = new Set(literals);
    for (const path of paths) {
      const value = factValue({ [facts.type]: facts.body }, path);
      if (typeof value === 'string' && value) texts.add(value);
    }
    // An axis is fitted from its exemplars' vectors, so they are inputs too.
    for (const axis of axes.axes ?? []) {
      for (const text of [...axis.positive, ...axis.negative, ...axis.calibration]) {
        texts.add(text);
      }
    }
    return [...texts];
  }

  function buildTrace(outcome) {
    const byName = new Map(rules.map((rule) => [rule.name, rule]));
    const fired = outcome.fired ?? [];
    const firedNames = new Set(fired);
    const alreadyDerived = new Set();
    const steps = [];

    // Firing order first — this is what ran — then everything that stayed idle.
    for (const name of fired) {
      const rule = byName.get(name);
      const reads = rule ? pathsRead(rule) : [];
      const assigns = rule ? pathsAssigned(rule) : [];
      steps.push({
        name,
        rule,
        fired: true,
        chained: reads.some((path) => alreadyDerived.has(path)),
        derived: assigns.map((path) => ({ path, value: factValue(outcome.facts, path) }))
      });
      for (const path of assigns) alreadyDerived.add(path);
    }
    for (const rule of rules) {
      if (firedNames.has(rule.name)) continue;
      steps.push({ name: rule.name, rule, fired: false, chained: false, derived: [] });
    }
    return { steps, derivedPaths: [...alreadyDerived] };
  }

  async function run() {
    if (busy) return;
    busy = true;
    error = '';
    result = null;
    status = 'loading engine…';
    try {
      await ensureWasm();
      ready = true;

      const facts = checkFacts(factsText);
      if (!facts.ok) throw new Error(`input facts — ${facts.message}`);
      const axes = checkAxes(axesText);
      if (!axes.ok) throw new Error(`axes — ${axes.message}`);
      const grl = checkRules(rulesText);
      if (!grl.ok) throw new Error(`rules — ${grl.message}`);

      const engine = new RuleEngine();
      engine.register_rule(rulesText);

      const provenance = new Map();
      const planned = plannedTexts(facts, axes);
      if (planned.length) {
        status = `resolving ${planned.length} vectors…`;
        await Promise.all(planned.map((text) => resolve(engine, provenance, text)));
      }

      // Fit after the vectors are in: an axis is the normalized difference of
      // its exemplar centroids, calibrated over the reference window.
      for (const axis of axes.axes) {
        status = `fitting axis "${axis.name}"…`;
        engine.fit_axis(
          axis.name,
          JSON.stringify(axis.positive),
          JSON.stringify(axis.negative),
          JSON.stringify(axis.calibration)
        );
      }

      status = 'evaluating…';
      let outcome = null;
      const started = performance.now();
      for (let attempt = 0; attempt < RESOLVE_ATTEMPTS && !outcome; attempt++) {
        try {
          outcome = deep(engine.evaluate(facts.type, JSON.stringify(facts.body), true));
        } catch (e) {
          const missing = MISSING_EMBEDDING.exec(String(e?.message ?? e));
          if (!missing) throw new Error(String(e?.message ?? e));
          status = `resolving vector for "${missing[1]}"…`;
          await resolve(engine, provenance, missing[1]);
        }
      }
      if (!outcome) {
        throw new Error(`rules still requested unresolved vectors after ${RESOLVE_ATTEMPTS} attempts`);
      }
      const wallMs = performance.now() - started;

      const vectors = [...provenance].map(([text, meta]) => ({ text, ...meta }));
      const counts = vectors.reduce((acc, v) => ({ ...acc, [v.source]: (acc[v.source] ?? 0) + 1 }), {});
      const { steps, derivedPaths } = buildTrace(outcome);
      result = {
        outcome,
        steps,
        derivedPaths,
        wallMs,
        factType: facts.type,
        assertText: JSON.stringify(facts.body),
        output: JSON.stringify(outcome.facts ?? {}, null, 2),
        axisNames: axes.axes.map((a) => a.name),
        vectors,
        counts,
        // Only the interesting vectors are itemized: whatever was computed
        // here, plus the match string. A calibration window is 24 chips of
        // noise otherwise.
        chips: vectors.filter(
          (v) => v.source === 'computed' || params.some((p) => p.value === v.text)
        ),
        dimensions: vectors[0]?.dimensions ?? null
      };
      outputTab = 'trace';

      const fired = outcome.fired?.length ?? 0;
      status = `done — ${fired} of ${rules.length} rules fired in ${wallMs.toFixed(1)} ms`;
    } catch (e) {
      status = '';
      error = e.message ?? String(e);
    } finally {
      busy = false;
    }
  }

  // --- presentation ----------------------------------------------------------

  // The engine's own time is routinely sub-millisecond, where three decimals of
  // a millisecond reads as a broken zero rather than a fast run.
  function duration(nanos) {
    const ns = nanos ?? 0;
    return ns < 1e6 ? `${(ns / 1000).toFixed(1)} µs` : `${(ns / 1e6).toFixed(2)} ms`;
  }

  function show(value) {
    if (typeof value === 'number') {
      return Number.isInteger(value) ? String(value) : value.toFixed(4);
    }
    return JSON.stringify(value);
  }

  const SOURCE_LABEL = {
    memory: 'cached in this browser',
    seeded: 'served from the seeded cache',
    computed: 'computed by EmbeddingGemma in this tab'
  };

  function sourceNote(text) {
    const source = sources[text];
    if (source === 'compute') {
      return modelPhase === 'ready'
        ? 'not cached — computed in this tab'
        : 'not cached — running downloads EmbeddingGemma (236 MB), once';
    }
    return source ? `${SOURCE_LABEL[source]} — no model download` : '';
  }
</script>

<section class="bench">
  <div class="head-row">
    <h3>Royal proclamations — editable, in the browser</h3>
    <button class="primary" onclick={run} disabled={busy || !runnable}>
      {busy ? 'running…' : '▶ Run'}
    </button>
  </div>
  <p class="muted lede">
    A message is scored on two independent calibrated axes — is a sovereign speaking, and is
    the intent punitive — and the response falls out of the 2×2. Rules, facts and fitted
    geometry on the left are all yours to edit. Vector functions carry their return kind:
    <code>s_</code> raw scalar identifies the speaker through inline vector algebra and gates
    nothing, because a cosine means something different on every model; <code>c_</code>
    calibrated makes the decisions, because a percentile against a calibration window does not.
  </p>

  <div class="controls">
    {#each params as param (param.path)}
      <label class="param">
        <span>{param.field} <code>{param.path}</code></span>
        <input
          value={param.value}
          oninput={(e) => onParamInput(param.field, e.currentTarget.value)}
          spellcheck="false"
          autocomplete="off"
          data-param={param.field}
        />
        <span
          class="src"
          data-source={sources[param.value] ?? ''}
          class:warn={sources[param.value] === 'compute'}
        >{sourceNote(param.value)}</span>
      </label>
    {:else}
      <span class="src">the rules pass no fact field to a vector function — edit the input facts directly</span>
    {/each}
  </div>

  <!-- Height is reserved so a run's progress never shifts the panes below it. -->
  <div class="status-row" class:bad={!!error}>{error || status}</div>

  <div class="panes" class:split={anyInputOpen}>
    <div class="pane inputs">
      <Disclosure
        label="Rules"
        badge="{rules.length} GRL"
        bad={!rulesCheck.ok}
        panel="rules"
        bind:open={showRules}
      >
        <CodeEditor bind:value={rulesText} lang="grl" rows={18} label="Rules, GRL source" />
        <div class="foot" class:bad={!rulesCheck.ok}>
          {rulesCheck.ok
            ? `parses — ${rules.length} rule${rules.length === 1 ? '' : 's'}, checked by the engine's own parser as you type`
            : rulesCheck.message}
        </div>
      </Disclosure>

      <Disclosure
        label="Input facts"
        badge={factsCheck.ok ? factsCheck.type : 'invalid'}
        bad={!factsCheck.ok}
        panel="facts"
        bind:open={showFacts}
      >
        <CodeEditor bind:value={factsText} lang="json" rows={8} label="Input facts, JSON" />
        <div class="foot" class:bad={!factsCheck.ok}>
          {factsCheck.ok
            ? `asserts one ${factsCheck.type} fact; the engine adds an empty Decision fact alongside it`
            : factsCheck.message}
        </div>
      </Disclosure>

      <Disclosure
        label="Axes"
        badge={axesCheck.ok ? `${axesCheck.axes.length} fitted` : 'invalid'}
        bad={!axesCheck.ok}
        panel="axes"
        bind:open={showAxes}
      >
        <CodeEditor bind:value={axesText} lang="json" rows={14} label="Axis exemplars, JSON" />
        <div class="foot" class:bad={!axesCheck.ok}>
          {axesCheck.ok
            ? `fitted from these exemplars each run; c_project scores a percentile against the calibration window`
            : axesCheck.message}
        </div>
      </Disclosure>
    </div>

    <div class="pane">
      <div class="pane-tabs" role="tablist" aria-label="Engine output">
        <button role="tab" aria-selected={outputTab === 'trace'} class:on={outputTab === 'trace'}
          onclick={() => (outputTab = 'trace')} data-panel="trace">
          Execution trace
          <span class="count">{result ? `${result.outcome.fired?.length ?? 0} fired` : 'not run'}</span>
        </button>
        <button role="tab" aria-selected={outputTab === 'facts'} class:on={outputTab === 'facts'}
          onclick={() => (outputTab = 'facts')} data-panel="output" disabled={!result}>
          Output facts
          <span class="count">{result ? `${result.derivedPaths.length} derived` : 'not run'}</span>
        </button>
      </div>

      <div class="pane-body">
        {#if !result}
          <div class="trace empty-state" data-state="empty">
            <p class="empty">
              Nothing has run yet. Edit anything on the left and press <strong>Run</strong> — the
              rules that fired, what each one derived, and the resulting facts are all read back
              from the engine after it executes.
            </p>
          </div>
        {:else if outputTab === 'trace'}
          <div class="trace" data-state="populated">
            <div class="muted metrics">
              {result.outcome.trace?.cycles ?? 0} cycles ·
              {result.outcome.trace?.rules_evaluated ?? 0} evaluated ·
              {result.outcome.trace?.rules_fired ?? 0} fired ·
              {duration(result.outcome.trace?.execution_time_ns)} in the engine
            </div>
            <div class="assert">
              <span class="kw">assert</span>
              <code>{result.factType} {result.assertText}</code>
            </div>
            {#each result.steps as step, index}
              <div class="step" class:fired={step.fired} class:chained={step.chained}>
                <div class="step-head">
                  <span class="ord">{step.fired ? `${index + 1}.` : '—'}</span>
                  <span class="rule">{step.name}</span>
                  <span class="pill {step.fired ? 'hit' : 'miss'}">
                    {step.fired ? 'FIRED ✓' : 'did not fire'}
                  </span>
                  {#if step.chained}<span class="link">chained</span>{/if}
                </div>
                {#if step.rule}
                  <div class="clause"><span class="kw">when</span><code>{step.rule.when}</code></div>
                  <div class="clause"><span class="kw">then</span><code>{step.rule.then}</code></div>
                {/if}
                {#each step.derived as entry}
                  <div class="clause derived">
                    <span class="kw">⇒</span><code>{entry.path} = {show(entry.value)}</code>
                  </div>
                {/each}
              </div>
            {/each}
          </div>
        {:else}
          <div class="trace" data-state="populated">
            <div class="derived-list">
              {#each result.derivedPaths as path}<code class="badge">{path}</code>{:else}
                <span class="muted">no fact was derived by this run</span>
              {/each}
            </div>
            <CodeEditor value={result.output} lang="json" rows={16} readonly label="Output facts, JSON" />
          </div>
        {/if}
      </div>

      <div class="foot">
        {#if result}
          <span class="vecs">
            {result.vectors.length} vectors{result.dimensions ? ` · dim ${result.dimensions}` : ''}
            {#each Object.entries(result.counts) as [source, n]}<span class="tally">{n} {source}</span>{/each}
          </span>
          {#each result.chips as chip}
            <span class="chip" data-source={chip.source} title={SOURCE_LABEL[chip.source] ?? chip.source}>
              {chip.text}<span class="chip-src">{chip.source}</span>
            </span>
          {/each}
        {:else}
          the engine resolves a vector for every text the rules and axes reference
        {/if}
      </div>
    </div>
  </div>
</section>

<style>
  /* The bench sizes to the space it is given, not to an assumed viewport. */
  section {
    container-type: inline-size;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 14px;
  }
  h3 { margin: 0 0 4px; font-size: 14px; }
  .muted { font-size: 12px; }
  .head-row { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
  .lede { max-width: 82ch; }

  .controls { display: grid; gap: 10px; margin: 12px 0 0; }
  @container (min-width: 46rem) {
    .controls { grid-template-columns: repeat(auto-fit, minmax(min(22rem, 100%), 1fr)); }
  }
  .param { display: grid; gap: 4px; font-size: 12px; color: var(--fg-muted); min-width: 0; }
  .param code { color: var(--fg-muted); }
  .param input { width: 100%; font-family: var(--mono); }
  .src { font-size: 11.5px; color: var(--fg-muted); }
  .src.warn { color: var(--amber); }

  .status-row { min-height: 1.5em; font-size: 11.5px; color: var(--fg-muted); margin: 6px 0 8px; }
  .status-row.bad { color: var(--red); }

  .panes { display: grid; gap: 12px; grid-template-columns: minmax(0, 1fr); }
  /* Side by side only when an input section is open AND the bench itself is
     wide enough for two columns of code — a container query, so it holds
     inside any shell. */
  @container (min-width: 60rem) {
    .panes.split { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
  }

  /* The input column is a stack of self-contained sections, so it carries no
     frame of its own. */
  .pane.inputs { display: grid; gap: 8px; align-content: start; border: 0; padding: 0; background: none; }

  .pane {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    gap: 8px;
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 10px;
    background: var(--bg);
  }
  .pane-body { min-height: 24rem; min-width: 0; overflow: auto; }
  .pane-tabs { display: flex; gap: 6px; flex-wrap: wrap; }
  .pane-tabs button {
    font-size: 12px; padding: 4px 9px; display: flex; align-items: center; gap: 7px;
    background: transparent;
  }
  .pane-tabs button.on { border-color: var(--accent); color: var(--accent); background: var(--bg-elev2); }
  .count {
    font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.04em;
    color: var(--fg-muted); border-left: 1px solid var(--border); padding-left: 7px;
  }

  .foot { font-size: 11px; color: var(--fg-muted); display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .foot.bad { color: var(--red); font-family: var(--mono); }
  .vecs { display: inline-flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .tally { border-left: 1px solid var(--border); padding-left: 6px; }

  .trace { font-size: 12px; }
  .metrics { margin-bottom: 6px; }
  .empty { font-size: 12px; color: var(--fg-muted); margin: 4px 0; max-width: 62ch; }
  .assert { font-size: 12px; margin: 6px 0 4px; }

  .step { padding: 7px 0; border-top: 1px solid var(--border); }
  .step:first-of-type { border-top: 0; }
  .step.chained { padding-left: 12px; border-left: 2px solid var(--green); }
  .step:not(.fired) { opacity: 0.62; }
  .step-head { display: flex; align-items: center; gap: 9px; flex-wrap: wrap; margin-bottom: 3px; }
  .step-head .ord { color: var(--fg-muted); font-size: 11px; width: 16px; }
  .step-head .rule { font-weight: 600; font-size: 12.5px; }
  .step-head .link { color: var(--green); font-size: 11px; font-weight: 600; }
  .clause { display: flex; gap: 6px; font-size: 11.5px; margin: 2px 0 2px 16px; }
  .clause code { white-space: pre-wrap; }
  .clause .kw { color: var(--fg-muted); font-size: 11px; min-width: 34px; }
  .derived code { color: var(--green); }

  .derived-list { display: flex; gap: 5px; flex-wrap: wrap; margin-bottom: 8px; }
  .badge { font-size: 11px; color: var(--green); border: 1px solid var(--border); border-radius: 4px; padding: 1px 6px; }

  .chip {
    display: inline-flex; align-items: center; gap: 6px; font-family: var(--mono);
    font-size: 11px; border: 1px solid var(--border); border-radius: 999px; padding: 1px 4px 1px 9px;
  }
  .chip-src { font-size: 9.5px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--fg-muted); border-left: 1px solid var(--border); padding: 0 7px 0 6px; }
  .chip[data-source='computed'] { border-color: var(--amber); }
  .chip[data-source='computed'] .chip-src { color: var(--amber); }
</style>
