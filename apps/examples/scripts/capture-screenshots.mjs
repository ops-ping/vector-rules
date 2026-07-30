import { existsSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import puppeteer from 'puppeteer';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '../../..');

const targets = [
  {
    id: 'address',
    route: '#/address',
    output: resolve(repoRoot, 'docs/examples-address.png'),
    viewport: { width: 1200, height: 950 },
    prepare: async (page) => {
      await page.waitForSelector('.pill', { timeout: 30000 });
      await new Promise((r) => setTimeout(r, 1500));
    }
  },
  {
    id: 'semantic',
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
    route: '#/prove',
    output: resolve(repoRoot, 'docs/examples-proof.png'),
    viewport: { width: 1200, height: 950 },
    prepare: async (page) => {
      await page.waitForSelector('.out', { timeout: 30000 });
      await new Promise((r) => setTimeout(r, 1000));
    }
  }
];

async function main() {
  console.log('Launching headless browser to capture all example screenshots...');

  const chromePath = '/usr/bin/google-chrome';
  const launchOptions = {
    headless: 'new',
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage']
  };
  if (existsSync(chromePath)) {
    launchOptions.executablePath = chromePath;
  }

  const browser = await puppeteer.launch(launchOptions);
  try {
    for (const t of targets) {
      console.log(`\n--- Capturing ${t.id} (${t.route}) ---`);
      const page = await browser.newPage();
      page.on('console', (msg) => console.log('  PAGE LOG:', msg.text()));
      page.on('pageerror', (err) => console.error('  PAGE ERROR:', err));

      await page.setViewport({ width: t.viewport.width, height: t.viewport.height, deviceScaleFactor: 2 });

      const url = `http://localhost:5173/${t.route}`;
      console.log(`Navigating to ${url}...`);
      await page.goto(url, { waitUntil: 'networkidle0' });

      console.log(`Preparing UI state for ${t.id}...`);
      await t.prepare(page);

      const section = await page.$('main section.stage');
      if (!section) {
        throw new Error(`Could not find main section.stage for ${t.id}`);
      }

      console.log(`Saving screenshot to ${t.output}...`);
      await section.screenshot({
        path: t.output,
        type: 'png'
      });

      await page.close();
    }

    console.log('\nAll screenshots captured successfully!');
  } finally {
    await browser.close();
  }
}

main().catch((err) => {
  console.error('Error capturing screenshots:', err);
  process.exit(1);
});
