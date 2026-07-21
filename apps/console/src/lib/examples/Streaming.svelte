<script>
  import init, { RuleEngine } from 'vrules-wasm/vrules_wasm.js';
  import wasmUrl from 'vrules-wasm/vrules_wasm_bg.wasm?url';

  let events = $state(5000);
  let highPercent = $state(35);
  let rows = $state([]);
  let status = $state('');
  let error = $state('');
  let busy = $state(false);

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

  function nextPercent(seed) {
    const next = (seed * 6364136223846793005n + 1n) & ((1n << 64n) - 1n);
    return [next, Number((next >> 32n) % 100n)];
  }

  function workload(count, pct) {
    let seed = 0x9E3779B97F4A7C15n;
    const out = [];
    for (let i = 0; i < count; i++) {
      let p;
      [seed, p] = nextPercent(seed);
      const high = p < pct;
      out.push({ fact: { cpu_pct: high ? 95 : 20, timestamp_ms: i }, expect: high });
    }
    return out;
  }

  function summarize(name, total, correct, started) {
    const elapsedMs = performance.now() - started;
    return {
      name,
      total,
      correct,
      accuracy: correct / total,
      eps: total / (elapsedMs / 1000),
      elapsedMs
    };
  }

  async function run() {
    busy = true; error = ''; rows = []; status = 'loading wasm…';
    try {
      await ensureWasm();
      const samples = workload(Number(events), Number(highPercent));
      const rule = `rule "HighCpu" no-loop {
    when
        Metric.cpu_pct > 80
    then
        Decision.high_cpu = true;
}`;

      status = 'evaluating sequential inputs…';
      const engine = new RuleEngine();
      engine.register_rule(rule);
      let correct = 0;
      const started = performance.now();
      for (const sample of samples) {
        const out = deep(engine.evaluate('Metric', JSON.stringify(sample.fact), false));
        if ((out.fired || []).includes('HighCpu') === sample.expect) correct++;
      }
      rows = [summarize('sequential_rule_engine', samples.length, correct, started)];
      status = 'done — each input produced one isolated output';
    } catch (e) {
      status = '';
      error = e.message;
    } finally {
      busy = false;
    }
  }
</script>

<section>
  <h3>Sequential records — browser WASM</h3>
  <p class="muted">
    Feeds records sequentially through <code>RuleEngine</code>. Each input receives
    one isolated output; this example does not provide windows or persistent state.
  </p>

  <div class="controls">
    <label>events<input type="number" min="100" max="50000" step="100" bind:value={events} /></label>
    <label>high %<input type="number" min="0" max="100" bind:value={highPercent} /></label>
    <button class="primary" onclick={run} disabled={busy}>{busy ? 'running…' : 'Run'}</button>
    <span class="muted status">{status}</span>
  </div>

  {#if error}<div class="error">Error: {error}</div>{/if}

  {#if rows.length}
    <table>
      <thead><tr><th>mode</th><th>accuracy</th><th>records/s</th><th>elapsed ms</th></tr></thead>
      <tbody>
        {#each rows as r}
          <tr>
            <td>{r.name}</td>
            <td>{(r.accuracy * 100).toFixed(2)}%</td>
            <td>{r.eps.toFixed(0)}</td>
            <td>{r.elapsedMs.toFixed(1)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  section { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 14px; max-width: 760px; }
  h3 { margin: 0 0 4px; font-size: 14px; }
  .muted { font-size: 12px; }
  .controls { display: flex; align-items: flex-end; gap: 10px; margin: 12px 0; flex-wrap: wrap; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--fg-muted); }
  input { width: 110px; }
  table { width: 100%; border-collapse: collapse; margin-top: 10px; }
  th, td { text-align: left; border-bottom: 1px solid var(--border); padding: 6px 4px; font-size: 12px; }
  .error { color: var(--red); margin: 8px 0; }
</style>
