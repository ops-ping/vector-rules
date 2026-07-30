<script>
  import init, { RuleEngine, WasmStreamProcessor } from 'vrules-wasm/vrules_wasm.js';
  import wasmUrl from 'vrules-wasm/vrules_wasm_bg.wasm?url';

  let events = $state(10000);
  let highPercent = $state(35);
  let windowType = $state('sliding'); // sliding vs tumbling
  let windowDurationMs = $state(5000);
  let rows = $state([]);
  let status = $state('');
  let error = $state('');
  let busy = $state(false);
  let streamSummary = $state(null);

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
      out.push({
        event_type: "SystemMetric",
        source: "telemetry",
        data: { cpu_pct: high ? 95 : 20, timestamp_ms: i * 10 },
        expect: high
      });
    }
    return out;
  }

  async function run() {
    busy = true; error = ''; rows = []; streamSummary = null; status = 'loading wasm…';
    try {
      await ensureWasm();
      const samples = workload(Number(events), Number(highPercent));

      const streamGrl = `rule "HighCpuAlert" no-loop {
    when
        SystemMetric.cpu_pct > 80
    then
        Alert.level = "HIGH";
        Alert.message = "CPU spike in stream window";
}`;

      status = `evaluating ${samples.length} stream events with ${windowType} window (${windowDurationMs}ms)…`;
      
      const processor = new WasmStreamProcessor(streamGrl, windowType, BigInt(windowDurationMs));
      let correct = 0;
      let firedCount = 0;
      const started = performance.now();

      for (const sample of samples) {
        const res = deep(processor.process_event(JSON.stringify(sample)));
        const fired = res.fired_rules || [];
        if (fired.includes('HighCpuAlert')) {
          firedCount++;
          if (sample.expect) correct++;
        } else {
          if (!sample.expect) correct++;
        }
      }

      const elapsedMs = performance.now() - started;
      const eps = samples.length / (elapsedMs / 1000);

      rows = [{
        mode: `stream_processor (${windowType})`,
        events: samples.length,
        fired: firedCount,
        accuracy: (correct / samples.length * 100).toFixed(2) + '%',
        throughput: eps.toFixed(0) + ' rec/s',
        elapsed: elapsedMs.toFixed(1) + ' ms'
      }];

      streamSummary = {
        rule: streamGrl,
        windowType,
        windowMs: windowDurationMs,
        totalEvents: samples.length,
        alertsTriggered: firedCount
      };

      status = `done — ${samples.length} events processed in ${elapsedMs.toFixed(1)}ms (${eps.toFixed(0)} rec/s)`;
    } catch (e) {
      status = '';
      error = e.message;
    } finally {
      busy = false;
    }
  }
</script>

<section>
  <div class="head-row">
    <h3>Stateful Event-Stream Engine — WASM & Windowing</h3>
    <button class="primary" onclick={run} disabled={busy}>{busy ? 'processing…' : '▶ Run Stream'}</button>
  </div>
  <p class="muted">
    Processes real-time event streams through <code>WasmStreamProcessor</code> using 
    <code>rust-rule-engine</code>'s native windowing (Sliding & Tumbling windows).
    {#if status}<span class="status">— {status}</span>{/if}
  </p>

  <div class="controls">
    <label>Events <input type="number" min="1000" max="50000" step="1000" bind:value={events} /></label>
    <label>High % <input type="number" min="0" max="100" bind:value={highPercent} /></label>
    <label>Window Type
      <select bind:value={windowType}>
        <option value="sliding">Sliding Window</option>
        <option value="tumbling">Tumbling Window</option>
        <option value="session">Session Window</option>
      </select>
    </label>
    <label>Window Duration (ms) <input type="number" min="1000" max="60000" step="1000" bind:value={windowDurationMs} /></label>
  </div>

  {#if error}<div class="error">Error: {error}</div>{/if}

  {#if rows.length}
    <table>
      <thead>
        <tr>
          <th>Stream Mode</th>
          <th>Total Events</th>
          <th>Alerts Fired</th>
          <th>Accuracy</th>
          <th>Throughput</th>
          <th>Elapsed Time</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as r}
          <tr>
            <td><code>{r.mode}</code></td>
            <td>{r.events.toLocaleString()}</td>
            <td><code>{r.fired}</code></td>
            <td>{r.accuracy}</td>
            <td><strong>{r.throughput}</strong></td>
            <td>{r.elapsed}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}

  {#if streamSummary}
    <div class="summary-box">
      <div class="summary-title">Stream Engine Execution Summary</div>
      <div class="summary-detail">
        Windowing Strategy: <code>{streamSummary.windowType}</code> ({streamSummary.windowMs}ms window) | Evaluated over {streamSummary.totalEvents.toLocaleString()} events
      </div>
      <pre class="grl-box">{streamSummary.rule}</pre>
    </div>
  {/if}
</section>

<style>
  section { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 14px; max-width: 760px; }
  h3 { margin: 0 0 4px; font-size: 14px; }
  .muted { font-size: 12px; }
  .head-row { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
  .status { color: var(--fg-muted); }
  .controls { display: flex; align-items: flex-end; gap: 12px; margin: 12px 0; flex-wrap: wrap; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--fg-muted); }
  input, select { width: 120px; padding: 4px 6px; border: 1px solid var(--border); border-radius: 4px; background: var(--bg); color: var(--fg); font-size: 12px; }
  .primary { font-size: 12px; padding: 6px 14px; border: 1px solid var(--border); border-radius: 6px; background: var(--green, #22c55e); color: #fff; font-weight: 600; cursor: pointer; }
  .primary:hover:not(:disabled) { opacity: 0.9; }
  .primary:disabled { opacity: 0.6; cursor: default; }
  table { width: 100%; border-collapse: collapse; margin-top: 12px; }
  th, td { text-align: left; border-bottom: 1px solid var(--border); padding: 8px 6px; font-size: 12px; }
  .summary-box { margin-top: 14px; padding: 12px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg); }
  .summary-title { font-weight: 600; font-size: 13px; margin-bottom: 4px; }
  .summary-detail { font-size: 11.5px; color: var(--fg-muted); margin-bottom: 8px; }
  .grl-box { font-size: 11px; margin: 0; padding: 8px; background: var(--bg-elev2, #1e1e1e); border-radius: 4px; overflow-x: auto; white-space: pre-wrap; }
  .error { color: var(--red); margin: 8px 0; }
</style>
