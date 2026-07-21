<script>
  // Client-side vector reasoning uses vectors and model metadata from the host.
  import { onMount } from 'svelte';
  import init, { RuleEngine } from 'vrules-wasm/vrules_wasm.js';
  import wasmUrl from 'vrules-wasm/vrules_wasm_bg.wasm?url';
  import { embedText } from '../embed.js';

  let rows = $state([]);
  let chain = $state(null);
  let status = $state('');
  let error = $state('');
  let busy = $state(false);
  let dim = $state(null);

  // Init the wasm module exactly once, passing the hashed asset URL (the bundled
  // glue can't fetch its .wasm by a relative path).
  let initPromise;
  function ensureWasm() {
    if (!initPromise) initPromise = init(wasmUrl);
    return initPromise;
  }

  async function embed(text) {
    let payload;
    try {
      payload = await embedText(text);
    } catch (e) {
      throw new Error(`embed "${text}": ${e.message}`);
    }
    const { info, vector } = payload;
    if (!info?.model || !info?.revision || !info?.dimensions) {
      throw new Error(`embed "${text}": host omitted model metadata`);
    }
    return { info, vector: new Float32Array(vector) };
  }

  function setEmbedding(engine, text, embedding) {
    const { model, revision, dimensions } = embedding.info;
    engine.set_embedding(text, embedding.vector, model, revision, dimensions);
  }

  function cosine(a, b) {
    let dot = 0, na = 0, nb = 0;
    for (let i = 0; i < a.length; i++) { dot += a[i] * b[i]; na += a[i] * a[i]; nb += b[i] * b[i]; }
    return dot / (Math.sqrt(na) * Math.sqrt(nb));
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

  function detailText(r) {
    return 'GRL rules:\n' + r.rule +
      '\n\nEvaluated:\n' + r.exprText +
      '\n\nEngine result + trace:\n' + JSON.stringify(r.result, null, 2);
  }

  function toggle(r) { r.open = !r.open; }
  function toggleChain() { if (chain) chain.open = !chain.open; }

  function chainDetail(c) {
    return 'GRL rules:\n' + c.rules.join('\n\n') +
      '\n\nEngine result + trace:\n' + JSON.stringify(c.result, null, 2);
  }

  async function run() {
    busy = true; error = ''; rows = []; chain = null; status = 'loading wasm…';
    try {
      await ensureWasm();
      status = 'fetching embeddings and model metadata from the host…';
      const next = [];

      // 1) Semantic similarity, in the layered idiom: `s_cosine` is a raw
      //    geometry score, so a measurement rule assigns it to a fact and a
      //    decision rule thresholds the fact. Thresholding s_cosine directly
      //    in `when` is rejected at rule load ("raw scalar needs calibration").
      const royalty = await embed('royalty');
      const simRules = `rule "MeasureRoyalty" salience 100 no-loop {
    when
        Concept.word != ""
    then
        Concept.royal_sim = s_cosine(Concept.word, "royalty");
}

rule "AboutRoyalty" no-loop {
    when
        Concept.royal_sim > 0.80
    then
        Decision.about_royalty = true;
}`;
      for (const word of ['king', 'queen', 'banana', 'tractor']) {
        const wv = await embed(word);
        const eng = new RuleEngine();
        eng.register_rule(simRules);
        setEmbedding(eng, 'royalty', royalty);
        setEmbedding(eng, word, wv);
        const res = deep(eng.evaluate('Concept', JSON.stringify({ word }), true));
        const cos = cosine(wv.vector, royalty.vector).toFixed(3);
        next.push({
          headline: word,
          cos,
          fired: (res.fired || []).includes('AboutRoyalty'),
          rule: simRules,
          exprText: `Concept { word: "${word}" }\nConcept.royal_sim = s_cosine("${word}", "royalty") = ${cos}\nthreshold: Concept.royal_sim > 0.80`,
          result: res,
          open: false
        });
      }

      // 2) Contrast: which side of the king↔man axis does the word sit on?
      //    s_contrast(x, pos, neg) = cos(x, pos) − cos(x, neg); the shared
      //    topic component cancels, isolating the polarity direction.
      const [king, man, queen] = await Promise.all(
        ['king', 'man', 'queen'].map((w) => embed(w))
      );
      const contrastRules = `rule "MeasurePolarity" salience 100 no-loop {
    when
        Concept.word != ""
    then
        Concept.polarity = s_contrast(Concept.word, "king", "man");
}

rule "RoyalSide" no-loop {
    when
        Concept.polarity > 0.05
    then
        Decision.royal_side = true;
}`;
      const eng = new RuleEngine();
      eng.register_rule(contrastRules);
      setEmbedding(eng, 'king', king);
      setEmbedding(eng, 'man', man);
      setEmbedding(eng, 'queen', queen);
      const ares = deep(eng.evaluate('Concept', JSON.stringify({ word: 'queen' }), true));
      const contrast = (cosine(queen.vector, king.vector) - cosine(queen.vector, man.vector)).toFixed(3);
      next.push({
        headline: 'queen on the king↔man contrast axis',
        cos: contrast,
        fired: (ares.fired || []).includes('RoyalSide'),
        rule: contrastRules,
        exprText: `Concept { word: "queen" }\nConcept.polarity = s_contrast("queen", "king", "man") = ${contrast}\nthreshold: Concept.polarity > 0.05`,
        result: ares,
        open: false
      });

      // 3) Forward chaining — the measurement writes a fact, the decision rule
      //    derives category, and a third rule fires on the derived category.
      //    All in one canonical evaluation.
      const chainRules = [`rule "MeasurePolarity" salience 100 no-loop {
    when
        Concept.word != ""
    then
        Concept.polarity = s_contrast(Concept.word, "king", "man");
}`, `rule "RoyalCategory" salience 50 no-loop {
    when
        Concept.polarity > 0.05
    then
        Concept.category = "royalty";
}`, `rule "GrantRoyalAccess" no-loop {
    when
        Concept.category == "royalty"
    then
        Decision.access_granted = true;
}`];
      const ceng = new RuleEngine();
      for (const rule of chainRules) ceng.register_rule(rule);
      setEmbedding(ceng, 'king', king);
      setEmbedding(ceng, 'man', man);
      setEmbedding(ceng, 'queen', queen);
      const cres = deep(ceng.evaluate('Concept', JSON.stringify({ word: 'queen' }), true));
      const firedC = cres.fired || [];
      chain = {
        fact: 'Concept { word: "queen" }',
        fired: firedC,
        rules: chainRules,
        result: cres,
        open: false,
        steps: [
          {
            rule: 'MeasurePolarity',
            cond: 'Concept.word != ""',
            derives: `Concept.polarity = s_contrast(word, king, man) = ${contrast}`,
            fired: firedC.includes('MeasurePolarity'),
            chained: false
          },
          {
            rule: 'RoyalCategory',
            cond: `Concept.polarity = ${contrast} > 0.05`,
            derives: 'Concept.category = "royalty"',
            fired: firedC.includes('RoyalCategory'),
            chained: true
          },
          {
            rule: 'GrantRoyalAccess',
            cond: 'Concept.category == "royalty"',
            derives: 'Decision.access_granted = true',
            fired: firedC.includes('GrantRoyalAccess'),
            chained: true
          }
        ]
      };

      rows = next;
      dim = royalty.vector.length;
      status = `done — dim ${dim}, vectors and model identity supplied by the host.`;
    } catch (e) {
      status = '';
      error = e.message;
    } finally {
      busy = false;
    }
  }

  // Run as soon as the panel mounts — the demo populates itself, no click needed.
  onMount(run);
</script>

<section>
  <div class="head-row">
    <h3>Semantic vector rules — in the browser</h3>
    <button class="rerun" onclick={run} disabled={busy}>{busy ? 'running…' : '↻ Re-run'}</button>
  </div>
  <p class="muted">
    The wasm rule engine evaluates GRL vector functions here in your browser against
    real vectors and model identity from the host's configured embedding model. Function
    names carry their return kind: <code>s_</code> raw scalar (measure, never threshold),
    <code>c_</code> calibrated (thresholdable), <code>b_</code> boolean, <code>m_</code> metadata.
    Click any row to drill into the rules + the engine trace.
    {#if status}<span class="status">— {status}</span>{/if}
  </p>

  {#if error}<div class="error">Error: {error}</div>{/if}

  {#if rows.length}
    <h3 class="section-title">Layered similarity — is each word about <code>"royalty"</code>?</h3>
    <p class="muted">
      A measurement rule assigns <code>s_cosine(word, "royalty")</code> to a fact in
      <code>then</code>; the decision rule thresholds the fact. The engine's load-time lint
      rejects thresholding the raw score directly in <code>when</code>.
    </p>
  {/if}

  <div class="rows">
    {#each rows as r}
      <div
        class="vrow"
        class:fired={r.fired}
        role="button"
        tabindex="0"
        onclick={() => toggle(r)}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggle(r); }
        }}
      >
        <div class="head">
          <span class="word">{r.headline}</span>
          <span class="muted">score = <code>{r.cos}</code></span>
          <span class="pill {r.fired ? 'hit' : 'miss'}">{r.fired ? 'FIRED ✓' : 'did not fire'}</span>
          <span class="hint">▸ {r.open ? 'hide' : 'rule'}</span>
        </div>
        {#if r.open}
          <pre class="detail">{detailText(r)}</pre>
        {/if}
      </div>
    {/each}
  </div>

  {#if chain}
    <h3 class="chain-title">Forward chaining — measurement derives facts that drive decisions</h3>
    <p class="muted">
      One canonical evaluation in this wasm engine. <code>MeasurePolarity</code> writes the
      contrast score, <code>RoyalCategory</code> derives a category from it, and
      <code>GrantRoyalAccess</code> fires on the derived category.
    </p>
    <div
      class="chain"
      role="button"
      tabindex="0"
      onclick={toggleChain}
      onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleChain(); } }}
    >
      <div class="chain-fact"><span class="muted">assert</span> <code>{chain.fact}</code></div>
      {#each chain.steps as s}
        <div class="step" class:chained={s.chained}>
          <div class="step-head">
            {#if s.chained}<span class="link">└─▶ chains to</span>{/if}
            <span class="rule">{s.rule}</span>
            <span class="pill {s.fired ? 'hit' : 'miss'}">{s.fired ? 'FIRED ✓' : 'did not fire'}</span>
          </div>
          <div class="step-when"><span class="kw">when</span> <code>{s.cond}</code></div>
          <div class="step-then"><span class="kw">then</span> <code>{s.derives}</code></div>
        </div>
      {/each}
      <div class="hint">▸ {chain.open ? 'hide rules + trace' : 'show rules + trace'}</div>
      {#if chain.open}
        <pre class="detail">{chainDetail(chain)}</pre>
      {/if}
    </div>
  {/if}
</section>

<style>
  section { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 14px; max-width: 720px; }
  h3 { margin: 0 0 4px; font-size: 14px; }
  .muted { font-size: 12px; }
  .head-row { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
  .rerun {
    flex: none; font-size: 11px; padding: 4px 10px; border: 1px solid var(--border);
    border-radius: 6px; background: var(--bg-elev2); color: var(--fg-muted); cursor: pointer;
  }
  .rerun:hover:not(:disabled) { color: var(--fg); }
  .rerun:disabled { opacity: 0.6; cursor: default; }
  .status { color: var(--fg-muted); }
  .rows { margin-top: 8px; }
  .vrow { border: 1px solid var(--border); border-radius: 6px; padding: 8px 12px; margin: 6px 0; cursor: pointer; }
  .vrow:hover { background: var(--bg-elev2); }
  .head { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .word { font-weight: 600; }
  .hint { color: var(--fg-muted); font-size: 11px; margin-left: auto; }
  .pill.hit { color: var(--green); }
  .pill.miss { color: var(--fg-muted); }
  .section-title { margin: 16px 0 4px; font-size: 14px; }
  .chain-title { margin: 22px 0 4px; font-size: 14px; }
  .chain {
    margin-top: 10px; border: 1px solid var(--border); border-left: 3px solid var(--green);
    border-radius: 6px; padding: 12px 14px; cursor: pointer;
  }
  .chain:hover { background: var(--bg-elev2); }
  .chain-fact { font-size: 12px; margin-bottom: 10px; }
  .chain-fact .muted { text-transform: uppercase; letter-spacing: 0.04em; margin-right: 6px; }
  .step { padding: 8px 0; }
  .step.chained { padding-left: 14px; }
  .step-head { display: flex; align-items: center; gap: 10px; margin-bottom: 4px; flex-wrap: wrap; }
  .step-head .rule { font-weight: 600; }
  .step-head .link { color: var(--green); font-size: 11px; font-weight: 600; }
  .step-when, .step-then { font-size: 11.5px; margin: 2px 0; }
  .step .kw { display: inline-block; width: 38px; color: var(--fg-muted); font-size: 11px; }
  .chain .hint { color: var(--fg-muted); font-size: 11px; margin-top: 8px; }
  .detail {
    margin-top: 8px; background: var(--bg); border: 1px solid var(--border); border-radius: 6px;
    padding: 10px 12px; font-size: 11.5px; white-space: pre-wrap; overflow-x: auto;
  }
  .error { color: var(--red); margin: 8px 0; }
</style>
