// Rasterize `public/favicon.svg` into the PNG/ICO fallbacks.
//
// The SVG is the source of truth; this produces the raster sizes that Safari
// and legacy `/favicon.ico` probes need. Re-run after editing the SVG:
//
//   node scripts/make-icons.mjs

import { readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import puppeteer from 'puppeteer';

const APP = fileURLToPath(new URL('..', import.meta.url));
const PUBLIC = path.join(APP, 'public');
const svg = await readFile(path.join(PUBLIC, 'favicon.svg'), 'utf8');

const chromePath = '/usr/bin/google-chrome';
const browser = await puppeteer.launch({
  headless: 'new',
  args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
  ...(existsSync(chromePath) ? { executablePath: chromePath } : {})
});

async function render(size) {
  const page = await browser.newPage();
  await page.setViewport({ width: size, height: size, deviceScaleFactor: 1 });
  await page.setContent(
    `<html><body style="margin:0">
       <div style="width:${size}px;height:${size}px">${svg}</div>
     </body></html>`,
    { waitUntil: 'load' }
  );
  const buf = await page.screenshot({ type: 'png', omitBackground: true });
  await page.close();
  return Buffer.from(buf);
}

// ICO with a PNG payload (supported since Windows Vista); avoids hand-rolling
// a BMP encoder for a single 32x32 entry.
function icoWrap(png, size) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(1, 4); // one image
  const entry = Buffer.alloc(16);
  entry.writeUInt8(size === 256 ? 0 : size, 0); // width
  entry.writeUInt8(size === 256 ? 0 : size, 1); // height
  entry.writeUInt8(0, 2); // palette
  entry.writeUInt8(0, 3); // reserved
  entry.writeUInt16LE(1, 4); // colour planes
  entry.writeUInt16LE(32, 6); // bits per pixel
  entry.writeUInt32LE(png.length, 8);
  entry.writeUInt32LE(header.length + entry.length, 12);
  return Buffer.concat([header, entry, png]);
}

const png32 = await render(32);
const png180 = await render(180);
await browser.close();

await writeFile(path.join(PUBLIC, 'favicon-32.png'), png32);
await writeFile(path.join(PUBLIC, 'apple-touch-icon.png'), png180);
await writeFile(path.join(PUBLIC, 'favicon.ico'), icoWrap(png32, 32));

console.log(`wrote favicon-32.png (${png32.length} B), apple-touch-icon.png (${png180.length} B), favicon.ico`);
