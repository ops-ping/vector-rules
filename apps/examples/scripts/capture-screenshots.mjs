import { existsSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import puppeteer from 'puppeteer';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '../../..');
const outputPath = resolve(repoRoot, 'docs/examples-semantic.png');

async function main() {
  console.log('Launching headless browser to capture documentation screenshot...');

  // Use system chrome if available, otherwise puppeteer's bundled browser
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
    const page = await browser.newPage();
    await page.setViewport({ width: 1200, height: 1300, deviceScaleFactor: 2 });

    const targetUrl = 'http://localhost:5173/#/semantic';
    console.log(`Navigating to ${targetUrl}...`);
    await page.goto(targetUrl, { waitUntil: 'networkidle0' });

    // Wait for the WASM evaluation to finish (indicated by the .status or .chain element)
    console.log('Waiting for WebAssembly vector engine evaluations...');
    await page.waitForSelector('.chain', { timeout: 30000 });
    await page.waitForSelector('.status', { timeout: 30000 });

    // Expand the first row ('queen') to show its rule & evaluated vector algebra expression
    console.log('Expanding row details...');
    const vrow = await page.$('.vrow');
    if (vrow) await vrow.click();

    // Expand the forward chain element to show the 3-step rules and trace
    console.log('Expanding forward chain details...');
    const chain = await page.$('.chain');
    if (chain) await chain.click();

    // Wait a brief moment for layout/fonts and state transitions
    await new Promise((r) => setTimeout(r, 1000));

    // Capture the semantic example stage container
    const section = await page.$('main section.stage');
    if (!section) {
      throw new Error('Could not find main section.stage container');
    }

    console.log(`Saving screenshot to ${outputPath}...`);
    await section.screenshot({
      path: outputPath,
      type: 'png'
    });

    console.log('Screenshot captured successfully!');
  } finally {
    await browser.close();
  }
}

main().catch((err) => {
  console.error('Error capturing screenshot:', err);
  process.exit(1);
});
