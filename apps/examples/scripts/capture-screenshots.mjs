import { existsSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import { spawn } from 'child_process';
import puppeteer from 'puppeteer';
import { textHash } from '../src/lib/cache-key.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '../../..');
const appRoot = resolve(__dirname, '..');

// Errors that are expected and not a symptom of a broken page.
const IGNORED_CONSOLE = [
  /Download the React DevTools/i,
  /\[vite\] connect/i
];

const pause = (ms) => new Promise((r) => setTimeout(r, ms));

/** Replace the contents of a single-line input, firing the events a binding needs. */
async function retypeParam(page, name, text) {
  return retype(page, `.param input[data-param="${name}"]`, text);
}

async function retype(page, selector, text) {
  await page.click(selector, { clickCount: 3 });
  await page.keyboard.press('Backspace');
  await page.type(selector, text);
}

/** Replace the contents of the editor inside an expanded input section. */
const editorIn = (panel) => `[data-section="${panel}"] textarea`;
async function retypeEditor(page, panel, text) {
  const sel = editorIn(panel);
  await page.click(sel);
  await page.keyboard.down('Control');
  await page.keyboard.press('KeyA');
  await page.keyboard.up('Control');
  await page.keyboard.press('Backspace');
  await page.type(sel, text);
}

/** Expand or collapse an input section, waiting for the state it should reach. */
async function setSection(page, panel, open) {
  const head = `[data-section="${panel}"] .head`;
  const state = await page.$eval(head, (el) => el.getAttribute('aria-expanded'));
  if ((state === 'true') !== open) await page.click(head);
  await page.waitForFunction(
    (sel, want) => document.querySelector(sel)?.getAttribute('aria-expanded') === String(want),
    { timeout: 5000 },
    head,
    open
  );
}

const openSection = (page, panel) => setSection(page, panel, true);
const closeSection = (page, panel) => setSection(page, panel, false);

/** Which output tab is selected, and whether an input column is being shown. */
const isSplit = (page) => page.$eval('.panes', (el) => el.classList.contains('split'));

async function openTab(page, panel) {
  const tab = `.pane-tabs button[data-panel="${panel}"]`;
  await page.click(tab);
  await page.waitForFunction(
    (sel) => document.querySelector(sel)?.getAttribute('aria-selected') === 'true',
    { timeout: 5000 },
    tab
  );
}

const firedCount = (page) => page.$$eval('.step.fired', (els) => els.length);

async function runAndWait(page, timeout = 30000) {
  await page.click('button.primary');
  await page.waitForFunction(
    () => document.querySelector('.trace')?.dataset.state === 'populated',
    { timeout, polling: 500 }
  );
}

// How long the page may show no observable change before a run counts as hung.
const STALL_MS = Number(process.env.EXAMPLES_STALL_MS || 60000);

/**
 * Press Run and wait for a result, bounded by *progress* rather than by a
 * wall-clock guess. Fetching 236 MB and running inference in wasm takes as long
 * as the machine takes, so a fixed timeout is either too short on a slow box or
 * too slow to report a genuine hang. This fails only when the model transfer
 * and the trace have both stopped changing.
 */
async function runAndWaitForModel(page) {
  await page.click('button.primary');
  const marker = () =>
    page.evaluate(() => {
      const model = document.querySelector('[data-phase]');
      const trace = document.querySelector('.trace');
      const status = document.querySelector('.status-row');
      return [
        model?.dataset.phase ?? '',
        model?.dataset.loaded ?? '',
        trace?.dataset.state ?? '',
        status?.textContent?.trim() ?? ''
      ].join('|');
    });

  let last = await marker();
  let changedAt = Date.now();
  for (;;) {
    const done = await page.evaluate(
      () => document.querySelector('.trace')?.dataset.state === 'populated'
    );
    if (done) return;
    const now = await marker();
    if (now !== last) {
      last = now;
      changedAt = Date.now();
      const [phase, loaded, , status] = now.split('|');
      if (phase === 'downloading' && loaded) {
        process.stdout.write(`\r  MODEL: ${(Number(loaded) / 1e6).toFixed(0)} MB transferred   `);
      } else if (status) {
        process.stdout.write(`\r  ${status.slice(0, 70).padEnd(72)}`);
      }
    } else if (Date.now() - changedAt > STALL_MS) {
      throw new Error(
        `no progress for ${STALL_MS / 1000}s while computing a free-form vector (state: ${last})`
      );
    }
    await pause(500);
  }
}

function expectEqual(actual, wanted, what) {
  if (actual !== wanted) throw new Error(`${what}: expected ${wanted}, got ${actual}`);
}

/** Every `⇒ path = value` the trace reports, keyed by fact path. */
const derivedValues = (page) =>
  page.$$eval('.clause.derived code', (els) =>
    Object.fromEntries(
      els.map((el) => {
        const [path, ...rest] = el.textContent.split(' = ');
        return [path.trim(), rest.join(' = ').trim()];
      })
    )
  );

/**
 * Assert the numbers a run produced, not merely that it produced some.
 *
 * Counting fired rules is not enough: an example can render a confident-looking
 * result that is wrong, which is exactly how the Proof example shipped saying
 * NOT PROVABLE against an empty proof tree while its check — that an output
 * element existed — passed on every run. These values are independently
 * corroborated against llama-server measurements of the same model.
 */
async function expectDerived(page, wanted, what) {
  const actual = await derivedValues(page);
  for (const [path, value] of Object.entries(wanted)) {
    if (actual[path] !== value) {
      throw new Error(`${what}: ${path} expected ${value}, got ${actual[path] ?? '(absent)'}`);
    }
  }
}

// Address, Fraud triage, Streaming and Proof are not driven here: they are
// hidden from the app until their runs assert what they compute (issues #1, #2).
const targets = [
  {
    id: 'semantic',
    // The editable bench, driven entirely from seeded vectors. Asserts the 2x2
    // the example exists to show — sovereign voice against punitive intent —
    // by the numbers each run derives, not by counting elements.
    expect: ['.trace[data-state="populated"]', '.step.fired', '.disclosure'],
    route: '#/semantic',
    output: resolve(repoRoot, 'docs/examples-semantic.png'),
    viewport: { width: 1500, height: 1000 },
    prepare: async (page) => {
      await page.waitForSelector('[data-section="rules"] .head', { timeout: 30000 });
      await page.waitForSelector('button.primary', { timeout: 30000 });
      await page.waitForFunction(() => !document.querySelector('button.primary').disabled, {
        timeout: 30000
      });

      // Loading the page must not execute anything, and must not open an editor.
      expectEqual(
        await page.$eval('.trace', (el) => el.dataset.state),
        'empty',
        'trace state before pressing Run'
      );
      expectEqual(await firedCount(page), 0, 'rules fired before pressing Run');
      expectEqual(
        await page.$$eval('.disclosure', (els) => els.filter((e) => e.classList.contains('open')).length),
        0,
        'input sections open on load'
      );
      expectEqual(await isSplit(page), false, 'split layout with every section closed');

      // Every fact field the rules feed into vector math is a parameter.
      expectEqual(
        (await page.$$eval('.param input', (els) => els.map((e) => e.dataset.param))).join(','),
        'speaker,text',
        'parameters derived from the rules'
      );

      // A sovereign speaking in anger: both axes clear their thresholds.
      await runAndWait(page);
      expectEqual(await firedCount(page), 4, 'rules fired for a displeased proclamation');
      await expectDerived(
        page,
        {
          'Message.queenlike': '0.8199',
          'Message.voice_pct': '93.7500',
          'Message.displeasure_pct': '93.7500',
          'Decision.response': '"fall on your sword"'
        },
        'a displeased proclamation'
      );

      // Same anger, no throne behind it: the voice axis alone changes the
      // decision. Two independent measurements, one symbolic outcome.
      await retypeParam(page, 'text', 'this is completely unacceptable, someone needs to answer for it');
      await runAndWait(page);
      await expectDerived(
        page,
        {
          'Message.voice_pct': '50',
          'Message.displeasure_pct': '87.5000',
          'Decision.response': '"apologise"'
        },
        'displeasure without the throne'
      );

      // Ceremonial but content: high voice, low displeasure, no apology owed.
      await retypeParam(page, 'text', 'We hereby decree that the market shall be held on the first day of each month');
      await runAndWait(page);
      await expectDerived(
        page,
        {
          'Message.voice_pct': '75',
          'Message.displeasure_pct': '68.7500',
          'Decision.response': '"acknowledge"'
        },
        'a calm decree'
      );

      // The analogy generalises to a regnal name it was never shown.
      await retypeParam(page, 'speaker', 'Elizabeth I of England');
      await runAndWait(page);
      await expectDerived(page, { 'Message.queenlike': '0.7228' }, 'Elizabeth I as speaker');

      // A parameter and the facts JSON are one value, read from the facts.
      await openSection(page, 'facts');
      expectEqual(await isSplit(page), true, 'split layout with a section open');
      const facts = await page.$eval(editorIn('facts'), (el) => el.value);
      if (!facts.includes('Elizabeth I of England')) {
        throw new Error(`input facts did not follow the parameter: ${facts.slice(0, 160)}`);
      }
      await retypeEditor(page, 'facts', '{"Message":{"speaker":"queen","text":"thanks for sorting that out, much appreciated"}}');
      await page.waitForFunction(
        () => document.querySelector('.param input[data-param="speaker"]')?.value === 'queen',
        { timeout: 5000 }
      );
      await runAndWait(page);
      await expectDerived(
        page,
        { 'Message.displeasure_pct': '12.5000', 'Decision.response': '"acknowledge"' },
        'plain thanks'
      );

      // Sections are independent, and the fitted geometry is editable input.
      await openSection(page, 'axes');
      expectEqual(await page.$$eval('.disclosure.open', (els) => els.length), 2, 'open sections');
      const axes = await page.$eval(editorIn('axes'), (el) => el.value);
      if (!axes.includes('sovereign_voice') || !axes.includes('displeasure')) {
        throw new Error(`axis section is missing an axis: ${axes.slice(0, 160)}`);
      }

      // Closing every section gives the whole width back to the output.
      await closeSection(page, 'facts');
      await closeSection(page, 'axes');
      expectEqual(await isSplit(page), false, 'split layout after closing every section');

      // Capture the headline case with the rules open beside its trace.
      await retypeParam(page, 'text', 'We are gravely displeased by this betrayal, and Our judgement shall be swift');
      await runAndWait(page);
      await openSection(page, 'rules');
      await pause(600);
    }
  },
  {
    id: 'semantic-dynamic',
    // Free-form input is the claim a seeded corpus cannot fake: an unseeded
    // message must pull the real model and be scored by vectors computed here.
    expect: ['.chip[data-source="computed"]', '.trace[data-state="populated"]'],
    route: '#/semantic',
    output: resolve(repoRoot, 'docs/examples-semantic-dynamic.png'),
    viewport: { width: 1500, height: 1000 },
    allowModelDownload: true,
    allowSeedMiss: ['We are most gravely displeased, and Our wrath shall be known to every corner of the realm'],
    prepare: async (page) => {
      await page.waitForSelector('button.primary', { timeout: 30000 });
      await page.waitForFunction(() => !document.querySelector('button.primary').disabled, {
        timeout: 30000
      });

      const message =
        'We are most gravely displeased, and Our wrath shall be known to every corner of the realm';
      await retypeParam(page, 'text', message);
      await page.waitForFunction(
        () => [...document.querySelectorAll('.src')].some((el) => el.dataset.source === 'compute'),
        { timeout: 15000 }
      );

      // Bounded by progress, not by a wall-clock guess.
      await runAndWaitForModel(page);
      process.stdout.write('\n');

      const computed = await page.$$eval('.chip[data-source="computed"]', (els) =>
        els.map((el) => el.textContent.trim())
      );
      if (!computed.some((text) => text.startsWith(message.slice(0, 30)))) {
        throw new Error(`the message was not computed in-browser; chips: ${JSON.stringify(computed)}`);
      }

      // The decision must follow from a vector this tab computed. Percentiles
      // for unseeded text are not pinned here; the thresholds they cross are.
      const derived = await derivedValues(page);
      const voice = Number(derived['Message.voice_pct']);
      const displeasure = Number(derived['Message.displeasure_pct']);
      if (!(voice > 60 && displeasure > 75)) {
        throw new Error(`computed vector did not clear both thresholds: voice ${voice}, displeasure ${displeasure}`);
      }
      await expectDerived(page, { 'Decision.response': '"fall on your sword"' }, 'a computed proclamation');

      await openTab(page, 'output');
      await pause(600);
    }
  }
];

function startPreview(port) {
  return new Promise((resolvePort, reject) => {
    const proc = spawn('npx', ['vite', 'preview', '--port', String(port), '--strictPort'], {
      cwd: appRoot,
      stdio: ['ignore', 'pipe', 'pipe']
    });
    const fail = setTimeout(() => reject(new Error('vite preview did not start in 30s')), 30000);
    const onData = (buf) => {
      if (/Local:\s+http/.test(buf.toString())) {
        clearTimeout(fail);
        resolvePort(proc);
      }
    };
    proc.stdout.on('data', onData);
    proc.stderr.on('data', onData);
    proc.on('exit', (code) => {
      clearTimeout(fail);
      reject(new Error(`vite preview exited early with code ${code}`));
    });
  });
}

async function main() {
  const port = Number(process.env.EXAMPLES_PORT || 5173);
  const base = process.env.EXAMPLES_URL || `http://localhost:${port}/`;
  const only = process.env.EXAMPLES_ONLY?.split(',').map((s) => s.trim()).filter(Boolean);
  const selected = only ? targets.filter((t) => only.includes(t.id)) : targets;
  if (only && selected.length !== only.length) {
    throw new Error(`EXAMPLES_ONLY named an unknown target: ${only.join(',')}`);
  }
  let preview = null;

  // Reuse an already-running dev server if there is one; otherwise serve the
  // built `dist/`, so a single command verifies exactly what ships.
  const reachable = await fetch(base).then((r) => r.ok).catch(() => false);
  if (!reachable) {
    if (!existsSync(resolve(appRoot, 'dist/index.html'))) {
      throw new Error('No server on ' + base + ' and no dist/ to serve. Run `npm run build` first.');
    }
    console.log(`No server on ${base} — starting vite preview...`);
    preview = await startPreview(port);
  }

  console.log('Launching headless browser to capture all example screenshots...');

  const chromePath = '/usr/bin/google-chrome';
  const launchOptions = {
    headless: 'new',
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage']
  };
  if (existsSync(chromePath)) {
    launchOptions.executablePath = chromePath;
  }

  const failures = [];
  const browser = await puppeteer.launch(launchOptions);
  try {
    for (const t of selected) {
      console.log(`\n--- Capturing ${t.id} (${t.route}) ---`);
      // Each target gets its own storage: vectors one target caches must not
      // change what the next one reports as the source of the same vector, and
      // a target's result must not depend on which ones ran before it.
      const context = await browser.createBrowserContext();
      const page = await context.newPage();
      const problems = [];
      // Vectors a target is expected to miss on, by their content-addressed
      // path segment: an unseeded text is the point of the dynamic target, but
      // any *other* miss still means the seed corpus has drifted.
      const allowedMisses = new Set((t.allowSeedMiss ?? []).map(textHash));
      page.on('console', (msg) => {
        const text = msg.text();
        console.log('  PAGE LOG:', text);
        if (msg.type() === 'error' && !IGNORED_CONSOLE.some((re) => re.test(text))) {
          problems.push(`console error: ${text}`);
        }
      });
      page.on('pageerror', (err) => {
        console.error('  PAGE ERROR:', err);
        problems.push(`uncaught: ${err.message}`);
      });
      page.on('requestfailed', (req) => {
        if (/\.gguf(\?|$)/.test(req.url())) return; // reported by the guard below
        problems.push(`request failed: ${req.method()} ${req.url()} (${req.failure()?.errorText})`);
      });
      page.on('request', (req) => {
        // Most scripted runs only touch prepared inputs, so every vector must
        // come from the seeded cache. A model fetch means the seed corpus has
        // drifted and the live demo would pull ~236 MB on load. The dynamic
        // target opts in: downloading the model IS what it verifies.
        if (/\.gguf(\?|$)/.test(req.url())) {
          if (t.allowModelDownload) console.log('  MODEL: fetching', req.url());
          else problems.push(`model download triggered by prepared inputs: ${req.url()}`);
        }
      });
      page.on('response', (res) => {
        // A miss on the seeded vector bucket means the demo would silently fall
        // back to a 236 MB model download, so treat it as a failure.
        if (res.status() >= 400 && res.url().includes('/vrules-rest/')) {
          if (allowedMisses.has(res.url().split('/').pop())) return;
          problems.push(`seeded vector missing: ${res.url()} -> ${res.status()}`);
        }
      });

      await page.setViewport({ width: t.viewport.width, height: t.viewport.height, deviceScaleFactor: 2 });

      const url = new URL(t.route, base).href;
      console.log(`Navigating to ${url}...`);
      await page.goto(url, { waitUntil: 'networkidle0' });

      console.log(`Preparing UI state for ${t.id}...`);
      try {
        await t.prepare(page);
        for (const sel of t.expect ?? []) {
          if (!(await page.$(sel))) problems.push(`expected selector missing: ${sel}`);
        }
      } catch (err) {
        problems.push(`prepare failed: ${err.message}`);
      }

      // Frame the example itself rather than the stage around it: each card
      // caps its own width, so shooting the stage would pad the image with
      // however much room the shell happened to give it.
      const section =
        (await page.$('main section.stage > *')) ?? (await page.$('main section.stage'));
      if (!section) {
        problems.push('could not find the example card in main section.stage');
      } else {
        console.log(`Saving screenshot to ${t.output}...`);
        await section.screenshot({ path: t.output, type: 'png' });
      }

      if (problems.length) {
        failures.push({ id: t.id, problems });
        console.error(`  FAILED ${t.id}:\n    - ${problems.join('\n    - ')}`);
      } else {
        console.log(`  OK ${t.id}`);
      }

      await page.close();
      await context.close();
    }
  } finally {
    await browser.close();
    if (preview) preview.kill();
  }

  if (failures.length) {
    console.error(`\n${failures.length} of ${selected.length} examples failed verification.`);
    process.exit(1);
  }
  console.log(`\nAll ${selected.length} examples verified and captured.`);
}

main().catch((err) => {
  console.error('Error capturing screenshots:', err);
  process.exit(1);
});
