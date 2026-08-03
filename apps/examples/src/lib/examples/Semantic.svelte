<script>
  // An editable semantic rule bench. Nothing on this page is a transcript of a
  // canned run: the rules, the asserted facts and the match string are all
  // input, and everything shown afterwards — the output facts, which rules
  // fired, in what order, and what each derived — is read back from the engine's
  // own result and trace. Editing the rules changes what the trace describes,
  // because the trace is built from the same source the engine parsed.
  import { onMount } from 'svelte';
  import init, { RuleEngine, validate_rule } from 'vrules-wasm/vrules_wasm.js';
  import wasmUrl from 'vrules-wasm/vrules_wasm_bg.wasm?url';
  import { embedText, probeVectorSource, subscribeModel } from '../embed.js';
  import { splitRules, pathsRead, pathsAssigned, vectorInputs } from '../syntax.js';
  import CodeEditor from '../panels/CodeEditor.svelte';

  const DEFAULT_RULES = `rule "MeasureAnalogy" salience 100 no-loop {
    when
        Concept.target != ""
    then
        Concept.similarity = s_cosine(["v:add", ["v:sub", "king", "man"], "woman"], Concept.target);
}

rule "AnalogyCategory" salience 50 no-loop {
    when
        Concept.similarity > 0.80
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

  // The engine reports an unresolvable vector by name; that message is the
  // authority on what a ruleset needs, so a miss by the pre-resolver below is
  // recovered from rather than guessed around.
  const MISSING_EMBEDDING = /no prefetched embedding for "((?:[^"\\]|\\.)*)"/;
  const RESOLVE_ATTEMPTS = 8;

  let rulesText = $state(DEFAULT_RULES);
  let factsText = $state(DEFAULT_FACTS);
  let matchText = $state('queen');
  let popout = $state(null); // 'rules' | 'facts' | 'output' | null
  let result = $state(null);
  let busy = $state(false);
  let status = $state('');
  let error = $state('');
  let ready = $state(false);
  let vectorSource = $state(''); // where the match string's vector comes from
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

  // --- live validation of the two editable sources --------------------------

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

  function checkRules(text) {
    if (!ready) return { ok: true, message: '' };
    const report = deep(validate_rule(text));
    if (report?.ok) return { ok: true, message: '' };
    const message = (report?.errors ?? []).map((e) => e.message).join('; ');
    return { ok: false, message: message || 'the rule parser rejected this source' };
  }

  let factsCheck = $derived(checkFacts(factsText));
  let rulesCheck = $derived(checkRules(rulesText));
  let rules = $derived(splitRules(rulesText));
  // The match string is whichever fact field the rules feed into vector math.
  let matchPath = $derived(vectorInputs(rulesText).paths[0] ?? '');
  let runnable = $derived(ready && factsCheck.ok && rulesCheck.ok);

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
    provenance.set(text, { source, dimensions: info.dimensions, model: info.model });
  }

  /** Texts the rules visibly need vectors for, before the engine is asked. */
  function plannedTexts(check) {
    const { literals, paths } = vectorInputs(rulesText);
    const texts = new Set(literals);
    for (const path of paths) {
      const value = boundMatch(check, path);
      if (value) texts.add(value);
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
      const grl = checkRules(rulesText);
      if (!grl.ok) throw new Error(`rules — ${grl.message}`);

      const engine = new RuleEngine();
      engine.register_rule(rulesText);

      const provenance = new Map();
      const planned = plannedTexts(facts);
      if (planned.length) {
        status = `resolving ${planned.length} vector${planned.length === 1 ? '' : 's'}…`;
        await Promise.all(planned.map((text) => resolve(engine, provenance, text)));
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
          const text = missing[1];
          status = `resolving vector for "${text}"…`;
          await resolve(engine, provenance, text);
        }
      }
      if (!outcome) {
        throw new Error(`rules still requested unresolved vectors after ${RESOLVE_ATTEMPTS} attempts`);
      }
      const wallMs = performance.now() - started;

      const { steps, derivedPaths } = buildTrace(outcome);
      result = {
        outcome,
        steps,
        derivedPaths,
        wallMs,
        factType: facts.type,
        assertText: JSON.stringify(facts.body),
        output: JSON.stringify(outcome.facts ?? {}, null, 2),
        vectors: [...provenance].map(([text, meta]) => ({ text, ...meta })),
        ruleCount: rules.length
      };

      const fired = outcome.fired?.length ?? 0;
      const dims = result.vectors[0]?.dimensions;
      status =
        `done — ${fired} of ${rules.length} rules fired in ${wallMs.toFixed(1)} ms` +
        (dims ? `, over ${result.vectors.length} vectors of dim ${dims}` : '');
    } catch (e) {
      status = '';
      error = e.message ?? String(e);
    } finally {
      busy = false;
    }
  }

  // --- presentation ----------------------------------------------------------

  function toggle(panel) {
    popout = popout === panel ? null : panel;
  }

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

  let popoutTitle = $derived(
    popout === 'rules'
      ? 'Rules — GRL'
      : popout === 'facts'
        ? 'Input facts — JSON'
        : popout === 'output'
          ? 'Output facts — after forward chaining'
          : ''
  );
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape' && popout) popout = null;
  }}
/>

<section>
  <div class="head-row">
    <h3>Semantic vector rules — editable, in the browser</h3>
    <button class="primary" onclick={run} disabled={busy || !runnable}>
      {busy ? 'running…' : '▶ Run'}
    </button>
  </div>
  <p class="muted lede">
    The rules, the asserted facts and the match string below are all yours to edit. The wasm
    rule engine parses what you type and evaluates GRL vector functions against real vectors
    from EmbeddingGemma. Function names carry their return kind: <code>s_</code> raw scalar
    (measure, never threshold), <code>c_</code> calibrated (thresholdable), <code>b_</code>
    boolean, <code>m_</code> metadata.
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

  <div class="tabs" role="group" aria-label="Rules, facts and results">
    <button class:open={popout === 'rules'} onclick={() => toggle('rules')} data-panel="rules">
      Rules <span class="count" class:bad={!rulesCheck.ok}>{rules.length} GRL</span>
    </button>
    <button class:open={popout === 'facts'} onclick={() => toggle('facts')} data-panel="facts">
      Input facts <span class="count" class:bad={!factsCheck.ok}>{factsCheck.ok ? factsCheck.type : 'invalid'}</span>
    </button>
    <button
      class:open={popout === 'output'}
      onclick={() => toggle('output')}
      data-panel="output"
      disabled={!result}
    >
      Output facts
      <span class="count">{result ? `${result.derivedPaths.length} derived` : 'not run'}</span>
    </button>
  </div>

  <!-- Height is reserved so a run's progress never shifts the panels below it. -->
  <div class="status-row" class:bad={!!error}>{error || status}</div>

  <div class="area" class:open={!!popout}>
    <div class="trace" data-state={result ? 'populated' : 'empty'}>
      <div class="trace-head">
        <strong>Execution trace</strong>
        {#if result}
          <div class="muted metrics">
            {result.outcome.trace?.cycles ?? 0} cycles ·
            {result.outcome.trace?.rules_evaluated ?? 0} evaluated ·
            {result.outcome.trace?.rules_fired ?? 0} fired ·
            {duration(result.outcome.trace?.execution_time_ns)} in the engine
          </div>
        {/if}
      </div>

      {#if !result}
        <p class="empty">
          Nothing has run yet. Edit anything above and press <strong>Run</strong> — the fired
          rules, what each one derived, and the resulting facts are all read back from the
          engine after it executes.
        </p>
      {:else}
        <div class="assert">
          <span class="kw">assert</span>
          <code>{result.factType} {result.assertText}</code>
        </div>
        {#each result.steps as step, index}
          <div class="step" class:fired={step.fired} class:chained={step.chained}>
            <div class="step-head">
              <span class="ord">{step.fired ? `${index + 1}.` : '—'}</span>
              <span class="rule">{step.name}</span>
              {#if step.rule?.salience !== null && step.rule?.salience !== undefined}
                <span class="muted tag">salience {step.rule.salience}</span>
              {/if}
              <span class="pill {step.fired ? 'hit' : 'miss'}">
                {step.fired ? 'FIRED ✓' : 'did not fire'}
              </span>
              {#if step.chained}<span class="link">chained from a derived fact</span>{/if}
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

        <div class="vectors">
          <span class="muted"
            >{result.vectors[0]?.dimensions ? `vectors · dim ${result.vectors[0].dimensions}` : 'vectors'}</span
          >
          {#each result.vectors as vector}
            <span class="chip" data-source={vector.source} title={SOURCE_LABEL[vector.source] ?? vector.source}>
              {vector.text}<span class="chip-src">{vector.source}</span>
            </span>
          {/each}
        </div>
      {/if}
    </div>

    {#if popout}
      <div class="popout" role="dialog" aria-label={popoutTitle}>
        <div class="popout-head">
          <strong>{popoutTitle}</strong>
          <button class="close" onclick={() => (popout = null)} aria-label="Close panel">✕</button>
        </div>

        {#if popout === 'rules'}
          <CodeEditor bind:value={rulesText} lang="grl" rows={18} label="Rules, GRL source" />
          <div class="foot" class:bad={!rulesCheck.ok}>
            {rulesCheck.ok
              ? `parses — ${rules.length} rule${rules.length === 1 ? '' : 's'}, checked by the engine's own parser as you type`
              : rulesCheck.message}
          </div>
        {:else if popout === 'facts'}
          <CodeEditor bind:value={factsText} lang="json" rows={10} label="Input facts, JSON" />
          <div class="foot" class:bad={!factsCheck.ok}>
            {factsCheck.ok
              ? `asserts one ${factsCheck.type} fact; the engine adds an empty Decision fact alongside it`
              : factsCheck.message}
          </div>
        {:else if result}
          <div class="derived-list">
            {#each result.derivedPaths as path}<code class="badge">{path}</code>{:else}
              <span class="muted">no fact was derived by this run</span>
            {/each}
          </div>
          <CodeEditor value={result.output} lang="json" rows={16} label="Output facts, JSON" />
          <div class="foot">
            every fact after forward chaining, read back from the engine — the asserted fact
            plus whatever the rules derived
          </div>
        {/if}
      </div>
    {/if}
  </div>
</section>

<style>
  section {
    position: relative;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 14px;
    max-width: 980px;
  }
  h3 { margin: 0 0 4px; font-size: 14px; }
  .muted { font-size: 12px; }
  .head-row { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
  .lede { max-width: 78ch; }

  .controls { display: flex; align-items: flex-end; gap: 12px; margin: 12px 0 10px; flex-wrap: wrap; }
  .param { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--fg-muted); }
  .param code { color: var(--fg-muted); }
  .param input { width: 260px; font-family: var(--mono); }
  .src { font-size: 11.5px; color: var(--fg-muted); padding-bottom: 7px; }
  .src.warn { color: var(--amber); }

  .tabs { display: flex; gap: 6px; flex-wrap: wrap; }
  .tabs button { font-size: 12px; padding: 5px 10px; display: flex; align-items: center; gap: 7px; }
  .tabs button.open { border-color: var(--accent); color: var(--accent); background: var(--bg-elev2); }
  .count {
    font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.04em;
    color: var(--fg-muted); border-left: 1px solid var(--border); padding-left: 7px;
  }
  .count.bad { color: var(--red); }

  .status-row {
    min-height: 1.5em;
    font-size: 11.5px;
    color: var(--fg-muted);
    margin-top: 8px;
  }
  .status-row.bad { color: var(--red); }

  /* The pop-out floats over this area, so the trace gives up exactly the width
     it covers, plus a gutter, rather than being hidden behind it. */
  .area { position: relative; margin-top: 4px; min-height: 360px; --popout-width: min(560px, 100%); }
  .area.open .trace { padding-right: calc(var(--popout-width) + 16px); }

  .trace { border: 1px solid var(--border); border-radius: 6px; padding: 12px 14px; background: var(--bg); }
  .trace-head strong { font-size: 13px; }
  .metrics { margin-top: 2px; }
  .empty { font-size: 12px; color: var(--fg-muted); margin: 10px 0 4px; max-width: 62ch; }
  .assert { font-size: 12px; margin: 10px 0 4px; }

  .step { padding: 8px 0; border-top: 1px solid var(--border); }
  .step:first-of-type { border-top: 0; }
  .step.chained { padding-left: 14px; border-left: 2px solid var(--green); }
  .step:not(.fired) { opacity: 0.62; }
  .step-head { display: flex; align-items: center; gap: 9px; flex-wrap: wrap; margin-bottom: 3px; }
  .step-head .ord { color: var(--fg-muted); font-size: 11px; width: 16px; }
  .step-head .rule { font-weight: 600; font-size: 12.5px; }
  .step-head .tag { font-size: 11px; }
  .step-head .link { color: var(--green); font-size: 11px; font-weight: 600; }
  .clause { display: flex; gap: 6px; font-size: 11.5px; margin: 2px 0 2px 16px; }
  .clause code { white-space: pre-wrap; }
  .clause .kw { color: var(--fg-muted); font-size: 11px; min-width: 34px; }
  .derived code { color: var(--green); }

  .vectors { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; margin-top: 12px; padding-top: 10px; border-top: 1px solid var(--border); }
  .chip {
    display: inline-flex; align-items: center; gap: 6px; font-family: var(--mono);
    font-size: 11px; border: 1px solid var(--border); border-radius: 999px; padding: 1px 4px 1px 9px;
  }
  .chip-src { font-size: 9.5px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--fg-muted); border-left: 1px solid var(--border); padding: 0 7px 0 6px; }
  .chip[data-source='computed'] { border-color: var(--amber); }
  .chip[data-source='computed'] .chip-src { color: var(--amber); }

  .popout {
    position: absolute;
    top: 0;
    right: 0;
    width: var(--popout-width);
    max-height: 100%;
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--bg-elev2);
    border: 1px solid var(--accent);
    border-radius: 8px;
    padding: 10px 12px 12px;
    box-shadow: 0 12px 30px rgba(0, 0, 0, 0.55);
  }
  .popout-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .popout-head strong { font-size: 12.5px; }
  .close { padding: 1px 8px; font-size: 12px; line-height: 1.5; color: var(--fg-muted); }
  .foot { font-size: 11px; color: var(--fg-muted); }
  .foot.bad { color: var(--red); font-family: var(--mono); }
  .derived-list { display: flex; gap: 5px; flex-wrap: wrap; }
  .badge { font-size: 11px; color: var(--green); border: 1px solid var(--border); border-radius: 4px; padding: 1px 6px; }

  @media (max-width: 780px) {
    .popout { position: static; width: 100%; max-height: none; margin-top: 12px; }
  }
</style>
