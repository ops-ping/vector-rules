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

  // `s_` scores are measurements and carry no portable meaning — a cosine of
  // 0.80 means something different on every model — so the decision is made on
  // `c_project`, a percentile against a calibration window. That is the whole
  // reason the vocabulary distinguishes the two kinds.
  const DEFAULT_RULES = `rule "MeasureAnalogy" salience 100 no-loop {
    when
        Concept.target != ""
    then
        Concept.analogy = s_cosine(
            ["v:add", ["v:sub", "king", "man"], "woman"],
            Concept.target);
}

rule "MeasureRoyalty" salience 90 no-loop {
    when
        Concept.target != ""
    then
        Concept.royal_pct = c_project(Concept.target, "royalty");
}

// c_project is calibrated, so it may be thresholded here in \`when\`.
// s_cosine may not: the lint rejects thresholding a raw scalar.
rule "ClassifyRoyal" salience 50 no-loop {
    when
        Concept.royal_pct > 75
    then
        Concept.category = "royalty";
}

rule "GrantRoyalAccess" no-loop {
    when
        Concept.category == "royalty"
    then
        Decision.access_granted = true;
}`;

  const DEFAULT_FACTS = `{
  "Concept": {
    "target": "queen"
  }
}`;

  const DEFAULT_AXES = `{
  "royalty": {
    "positive": ["king", "queen", "monarch", "emperor", "the royal court", "a reigning sovereign"],
    "negative": ["tractor", "diesel engine", "factory machine", "wrench", "conveyor belt", "warehouse pallet"],
    "calibration": [
      "bread", "river", "accountant", "sneakers", "thunderstorm", "library",
      "bicycle", "coffee", "hospital", "guitar", "harvest", "passport",
      "castle", "parliament", "a noble family", "the president", "a mayor", "a knight",
      "an ancient throne", "the crown jewels", "a coronation", "royal decree",
      "the palace guard", "an imperial dynasty"
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
  let matchText = $state('queen');
  let inputTab = $state('rules'); // 'rules' | 'facts' | 'axes'
  let outputTab = $state('trace'); // 'trace' | 'facts'
  let result = $state(null);
  let busy = $state(false);
  let status = $state('');
  let error = $state('');
  let ready = $state(false);
  let vectorSource = $state('');
  let modelPhase = $state('absent');

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
      const sets = ['positive', 'negative', 'calibration'];
      for (const set of sets) {
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
  // The match string is whichever fact field the rules feed into vector math.
  let matchPath = $derived(vectorInputs(rulesText).paths[0] ?? '');
  let runnable = $derived(ready && factsCheck.ok && axesCheck.ok && rulesCheck.ok);

  function factValue(facts, path) {
    const [type, field] = path.split('.');
    const value = facts?.[type]?.[field];
    return value === undefined ? null : value;
  }

  function boundMatch(check, path) {
    if (!check.ok || !path) return null;
    const [type, field] = path.split('.');
    if (type !== check.type) return null;
    const value = check.body[field];
    return typeof value === 'string' ? value : null;
  }

  // The parameter and the input facts are two views of one value. Editing the
  // facts pulls the parameter along; typing the parameter patches the facts.
  $effect(() => {
    const bound = boundMatch(factsCheck, matchPath);
    if (bound !== null && bound !== matchText) matchText = bound;
  });

  function onMatchInput(event) {
    matchText = event.currentTarget.value;
    if (!factsCheck.ok || !matchPath) return;
    const [type, field] = matchPath.split('.');
    if (type !== factsCheck.type) return;
    const next = JSON.parse(factsText);
    next[type][field] = matchText;
    factsText = JSON.stringify(next, null, 2);
  }

  // Whether running will read a file or download the model, answered before the
  // run rather than discovered during it.
  $effect(() => {
    const text = matchText;
    if (!text) {
      vectorSource = '';
      return;
    }
    let live = true;
    const timer = setTimeout(async () => {
      const source = await probeVectorSource(text).catch(() => '');
      if (live) vectorSource = source;
    }, 200);
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
      const value = boundMatch(facts, path);
      if (value) texts.add(value);
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
        chips: vectors.filter((v) => v.source === 'computed' || v.text === matchText),
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

  let matchNote = $derived(
    vectorSource === 'compute'
      ? modelPhase === 'ready'
        ? 'not cached — computed in this tab by the loaded model'
        : 'not cached — running downloads EmbeddingGemma (236 MB), once'
      : vectorSource
        ? `${SOURCE_LABEL[vectorSource]} — no model download`
        : ''
  );

  let inputFoot = $derived(
    inputTab === 'rules'
      ? rulesCheck.ok
        ? `parses — ${rules.length} rule${rules.length === 1 ? '' : 's'}, checked by the engine's own parser as you type`
        : rulesCheck.message
      : inputTab === 'facts'
        ? factsCheck.ok
          ? `asserts one ${factsCheck.type} fact; the engine adds an empty Decision fact alongside it`
          : factsCheck.message
        : axesCheck.ok
          ? `${axesCheck.axes.length} axis fitted from its exemplars each run; c_project scores a percentile against the calibration window`
          : axesCheck.message
  );
  let inputBad = $derived(
    inputTab === 'rules' ? !rulesCheck.ok : inputTab === 'facts' ? !factsCheck.ok : !axesCheck.ok
  );
</script>

<section class="bench">
  <div class="head-row">
    <h3>Semantic vector rules — editable, in the browser</h3>
    <button class="primary" onclick={run} disabled={busy || !runnable}>
      {busy ? 'running…' : '▶ Run'}
    </button>
  </div>
  <p class="muted lede">
    Rules, facts and fitted geometry on the left; what the engine did with them on the right.
    All three are yours to edit. Vector functions carry their return kind — <code>s_</code> raw
    scalar (a measurement, never thresholded), <code>c_</code> calibrated (thresholdable),
    <code>b_</code> boolean, <code>m_</code> metadata — so the analogy score is reported and
    the decision is made on a calibration percentile.
  </p>

  <div class="controls">
    <label class="param">
      <span>match string {#if matchPath}<code>{matchPath}</code>{/if}</span>
      <input
        value={matchText}
        oninput={onMatchInput}
        disabled={!matchPath}
        spellcheck="false"
        autocomplete="off"
        placeholder={matchPath ? 'queen' : 'no fact field is passed to a vector function'}
      />
    </label>
    {#if matchPath}
      <span class="src" data-source={vectorSource} class:warn={vectorSource === 'compute'}>
        {matchNote}
      </span>
    {:else}
      <span class="src">the rules pass no fact field to a vector function — edit the input facts directly</span>
    {/if}
  </div>

  <!-- Height is reserved so a run's progress never shifts the panes below it. -->
  <div class="status-row" class:bad={!!error}>{error || status}</div>

  <div class="panes">
    <div class="pane">
      <div class="pane-tabs" role="tablist" aria-label="Engine input">
        <button role="tab" aria-selected={inputTab === 'rules'} class:on={inputTab === 'rules'}
          onclick={() => (inputTab = 'rules')} data-panel="rules">
          Rules <span class="count" class:bad={!rulesCheck.ok}>{rules.length} GRL</span>
        </button>
        <button role="tab" aria-selected={inputTab === 'facts'} class:on={inputTab === 'facts'}
          onclick={() => (inputTab = 'facts')} data-panel="facts">
          Input facts <span class="count" class:bad={!factsCheck.ok}>{factsCheck.ok ? factsCheck.type : 'invalid'}</span>
        </button>
        <button role="tab" aria-selected={inputTab === 'axes'} class:on={inputTab === 'axes'}
          onclick={() => (inputTab = 'axes')} data-panel="axes">
          Axes <span class="count" class:bad={!axesCheck.ok}>{axesCheck.ok ? `${axesCheck.axes.length} fitted` : 'invalid'}</span>
        </button>
      </div>
      <div class="pane-body">
        {#if inputTab === 'rules'}
          <CodeEditor bind:value={rulesText} lang="grl" fill label="Rules, GRL source" />
        {:else if inputTab === 'facts'}
          <CodeEditor bind:value={factsText} lang="json" fill label="Input facts, JSON" />
        {:else}
          <CodeEditor bind:value={axesText} lang="json" fill label="Axis exemplars, JSON" />
        {/if}
      </div>
      <div class="foot" class:bad={inputBad}>{inputFoot}</div>
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

  .controls { display: flex; align-items: flex-end; gap: 12px; margin: 12px 0 0; flex-wrap: wrap; }
  .param { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--fg-muted); }
  .param code { color: var(--fg-muted); }
  .param input { width: min(18rem, 100%); font-family: var(--mono); }
  .src { font-size: 11.5px; color: var(--fg-muted); padding-bottom: 7px; }
  .src.warn { color: var(--amber); }

  .status-row { min-height: 1.5em; font-size: 11.5px; color: var(--fg-muted); margin: 6px 0 8px; }
  .status-row.bad { color: var(--red); }

  .panes { display: grid; gap: 12px; grid-template-columns: minmax(0, 1fr); }
  /* Side by side only when the bench itself is wide enough for two columns of
     code — a container query, so it holds inside any shell. */
  @container (min-width: 60rem) {
    .panes { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
  }

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
  .count.bad { color: var(--red); }

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
