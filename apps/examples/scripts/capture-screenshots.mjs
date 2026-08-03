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
async function retype(page, selector, text) {
  await page.click(selector, { clickCount: 3 });
  await page.keyboard.press('Backspace');
  await page.type(selector, text);
}

/** Replace the contents of the editor in the input pane. */
const INPUT_EDITOR = '.panes > .pane:first-child textarea';
async function retypeEditor(page, text) {
  await page.click(INPUT_EDITOR);
  await page.keyboard.down('Control');
  await page.keyboard.press('KeyA');
  await page.keyboard.up('Control');
  await page.keyboard.press('Backspace');
  await page.type(INPUT_EDITOR, text);
}

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

function expectEqual(actual, wanted, what) {
  if (actual !== wanted) throw new Error(`${what}: expected ${wanted}, got ${actual}`);
}

const targets = [
  {
    id: 'address',
    route: '#/address',
    output: resolve(repoRoot, 'docs/examples-address.png'),
    viewport: { width: 1200, height: 950 },
    // Text that must be present for the capture to count as a passing run.
    expect: ['.pill'],
    prepare: async (page) => {
      await page.waitForSelector('.pill', { timeout: 30000 });
      await pause(1500);
    }
  },
  {
    id: 'semantic',
    // The editable bench, driven entirely from seeded vectors: proves the page
    // stays inert until Run, that the trace is the engine's and not a canned
    // script, and that the parameter and the input-facts JSON are one value.
    expect: ['.trace[data-state="populated"]', '.step.fired', '.pane-tabs'],
    route: '#/semantic',
    output: resolve(repoRoot, 'docs/examples-semantic.png'),
    viewport: { width: 1500, height: 1000 },
    prepare: async (page) => {
      await page.waitForSelector('.pane-tabs button[data-panel="rules"]', { timeout: 30000 });
      await page.waitForFunction(() => !document.querySelector('button.primary')?.disabled, {
        timeout: 30000
      });

      // Loading the page must not execute anything.
      const initial = await page.$eval('.trace', (el) => el.dataset.state);
      expectEqual(initial, 'empty', 'trace state before pressing Run');
      expectEqual(await firedCount(page), 0, 'rules fired before pressing Run');

      // The default parameter is seeded, and the whole chain fires on it: both
      // measurements, the calibrated classification, and the decision.
      await runAndWait(page);
      expectEqual(await firedCount(page), 4, 'rules fired for "queen"');

      // A different match string must genuinely change what fires: "tractor"
      // sits at the bottom of the calibration window, so both measurement rules
      // still fire but the classification does not. A canned trace could not
      // do this.
      await retype(page, '.param input', 'tractor');
      await runAndWait(page);
      expectEqual(await firedCount(page), 2, 'rules fired for "tractor"');

      // The parameter wrote through to the facts...
      await openTab(page, 'facts');
      const facts = await page.$eval(INPUT_EDITOR, (el) => el.value);
      if (!facts.includes('"tractor"')) {
        throw new Error(`input facts did not follow the parameter: ${facts}`);
      }

      // ...and editing the facts directly pulls the parameter back.
      await retypeEditor(page, '{"Concept":{"target":"queen"}}');
      await page.waitForFunction(
        () => document.querySelector('.param input')?.value === 'queen',
        { timeout: 5000 }
      );
      await runAndWait(page);
      expectEqual(await firedCount(page), 4, 'rules fired after editing the facts JSON');

      // The fitted geometry is an editable input too, not hidden machinery.
      await openTab(page, 'axes');
      const axes = await page.$eval(INPUT_EDITOR, (el) => el.value);
      if (!axes.includes('calibration')) {
        throw new Error(`axis pane did not show the calibration window: ${axes.slice(0, 120)}`);
      }

      // Capture in its resting state: the GRL that ran, beside the trace of it.
      await openTab(page, 'rules');
      await pause(600);
    }
  },
  {
    id: 'semantic-dynamic',
    // Free-form input is the claim that cannot be faked with a seeded corpus:
    // an unseeded match string must pull the real model and compute a vector.
    expect: ['.chip[data-source="computed"]', '.trace[data-state="populated"]'],
    route: '#/semantic',
    output: resolve(repoRoot, 'docs/examples-semantic-dynamic.png'),
    viewport: { width: 1500, height: 1000 },
    allowModelDownload: true,
    // A free-form phrase, deliberately absent from the seed corpus, so the
    // bucket misses for exactly this text and nothing else. It scores 0.86
    // against the analogy vector, so the whole chain still fires.
    allowSeedMiss: ['royal woman'],
    prepare: async (page) => {
      await page.waitForFunction(() => !document.querySelector('button.primary')?.disabled, {
        timeout: 30000
      });

      await retype(page, '.param input', 'royal woman');
      await page.waitForFunction(
        () => document.querySelector('.src')?.dataset.source === 'compute',
        { timeout: 15000 }
      );

      // Downloading 236 MB and running inference in wasm is the slow path by
      // design; this is the only target allowed to take it.
      await runAndWait(page, 300000);

      const computed = await page.$$eval('.chip[data-source="computed"]', (els) =>
        els.map((el) => el.textContent.trim())
      );
      if (!computed.some((text) => text.startsWith('royal woman'))) {
        throw new Error(`"royal woman" was not computed in-browser; chips: ${JSON.stringify(computed)}`);
      }
      expectEqual(await firedCount(page), 4, 'rules fired for the computed vector');

      // Capture with the derived facts open — they exist only after a run.
      await openTab(page, 'output');
      await pause(600);
    }
  },
  {
    id: 'fraud',
    expect: ['.decision', '.prov'],
    route: '#/fraud',
    output: resolve(repoRoot, 'docs/examples-fraud-triage.png'),
    viewport: { width: 1200, height: 950 },
    prepare: async (page) => {
      await page.waitForSelector('.decision', { timeout: 30000 });
      await page.waitForSelector('.prov', { timeout: 30000 });
      await pause(1500);
    }
  },
  {
    id: 'streaming',
    expect: ['button.primary'],
    route: '#/streaming',
    output: resolve(repoRoot, 'docs/examples-streaming.png'),
    viewport: { width: 1200, height: 950 },
    prepare: async (page) => {
      await page.waitForSelector('button.primary', { timeout: 30000 });
      await page.click('button.primary');
      await pause(3000);
    }
  },
  {
    id: 'prove',
    expect: ['.out'],
    route: '#/prove',
    output: resolve(repoRoot, 'docs/examples-proof.png'),
    viewport: { width: 1200, height: 950 },
    prepare: async (page) => {
      await page.waitForSelector('.out', { timeout: 30000 });
      await pause(1000);
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
