import { existsSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import { spawn } from 'child_process';
import puppeteer from 'puppeteer';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '../../..');
const appRoot = resolve(__dirname, '..');

// Errors that are expected and not a symptom of a broken page.
const IGNORED_CONSOLE = [
  /Download the React DevTools/i,
  /\[vite\] connect/i
];

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
      await new Promise((r) => setTimeout(r, 1500));
    }
  },
  {
    id: 'semantic',
    expect: ['.chain', '.status'],
    route: '#/semantic',
    output: resolve(repoRoot, 'docs/examples-semantic.png'),
    viewport: { width: 1200, height: 1300 },
    prepare: async (page) => {
      await page.waitForSelector('.chain', { timeout: 30000 });
      await page.waitForSelector('.status', { timeout: 30000 });

      // Expand the first row ('queen') to show its rule & evaluated vector algebra expression
      const vrow = await page.$('.vrow');
      if (vrow) await vrow.click();

      // Expand the forward chain element to show the 3-step rules and trace
      const chain = await page.$('.chain');
      if (chain) await chain.click();

      await new Promise((r) => setTimeout(r, 1000));
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
      await new Promise((r) => setTimeout(r, 1500));
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
      await new Promise((r) => setTimeout(r, 3000));
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
      await new Promise((r) => setTimeout(r, 1000));
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
    for (const t of targets) {
      console.log(`\n--- Capturing ${t.id} (${t.route}) ---`);
      const page = await browser.newPage();
      const problems = [];
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
        problems.push(`request failed: ${req.url()} (${req.failure()?.errorText})`);
      });
      page.on('request', (req) => {
        // The scripted runs only touch prepared inputs, so every vector must
        // come from the seeded cache. A model fetch means the seed corpus has
        // drifted and the live demo would pull ~236 MB on load.
        if (/\.gguf(\?|$)/.test(req.url())) {
          problems.push(`model download triggered by prepared inputs: ${req.url()}`);
        }
      });
      page.on('response', (res) => {
        // A miss on the seeded vector bucket means the demo would silently fall
        // back to a 236 MB model download, so treat it as a failure.
        if (res.status() >= 400 && res.url().includes('/vrules-rest/')) {
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

      const section = await page.$('main section.stage');
      if (!section) {
        problems.push('could not find main section.stage');
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
    }
  } finally {
    await browser.close();
    if (preview) preview.kill();
  }

  if (failures.length) {
    console.error(`\n${failures.length} of ${targets.length} examples failed verification.`);
    process.exit(1);
  }
  console.log(`\nAll ${targets.length} examples verified and captured.`);
}

main().catch((err) => {
  console.error('Error capturing screenshots:', err);
  process.exit(1);
});
