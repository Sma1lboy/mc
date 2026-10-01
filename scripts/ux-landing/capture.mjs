import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import assert from 'node:assert/strict';
const require = createRequire(`${process.env.UX_TOOLS_DIR}/package.json`);
const { chromium } = require('playwright');
const out = resolve('ux-evidence'); await mkdir(out, { recursive: true });
const browser = await chromium.launch({ headless: true });
for (const [phase, root] of [['before', process.env.UX_BASE_DIR], ['after', process.cwd()]]) {
  const server = createServer(async (req, res) => {
    try { const path = resolve(root, 'landing', req.url === '/' ? 'index.html' : req.url.slice(1));
      res.setHeader('Content-Type', { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.png': 'image/png' }[extname(path)] || 'application/octet-stream');
      res.end(await readFile(path));
    } catch { res.statusCode = 404; res.end(); }
  });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const context = await browser.newContext({ viewport: { width: 1440, height: 1050 }, deviceScaleFactor: 1 });
  await context.addInitScript(() => Object.defineProperty(navigator, 'clipboard', { value: { writeText: async () => { throw new Error('Fixture: clipboard denied'); } } }));
  const page = await context.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}`, { waitUntil: 'networkidle' });
  await page.screenshot({ path: `${out}/landing-${phase}.png`, fullPage: true });
  await page.locator('.brew button').click();
  if (phase === 'after') {
    await page.waitForFunction(() => document.querySelector('#copy-status').textContent.includes('未能复制'));
    assert.match(await page.locator('#copy-status').textContent(), /手动复制/);
    assert.equal(await page.locator('.btn-play').textContent(), '下载启动器');
    await page.locator('.btn-play').click();
    assert.equal(new URL(page.url()).hash, '#download');
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.screenshot({ path: `${out}/landing-mobile-after.png`, fullPage: true });
  } else {
    assert.equal(await page.locator('.brew button').textContent(), 'OK!');
  }
  await page.setViewportSize({ width: 1440, height: 1050 });
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await page.locator('.brew button').click();
  if (phase === 'after') await page.waitForFunction(() => document.querySelector('#copy-status').textContent.includes('未能复制'));
  await page.screenshot({ path: `${out}/clipboard-denied-${phase}.png` });
  await context.close(); await new Promise(r => server.close(r));
}
await browser.close();
console.log('PASS: real landing renders, CTA destination, denied clipboard before/after, mobile overflow');
