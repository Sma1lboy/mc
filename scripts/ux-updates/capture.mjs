import { createRequire } from 'node:module';
import { mkdir, readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { resolve, extname } from 'node:path';
import assert from 'node:assert/strict';
const { chromium } = createRequire(`${process.env.UX_TOOLS_DIR}/package.json`)('playwright');
const out = resolve('ux-evidence'); await mkdir(out, { recursive: true });
const browser = await chromium.launch();
for (const [phase, root] of [['before', process.env.UX_BASE_DIR], ['after', process.cwd()]]) {
  const server = createServer(async (req, res) => {
    try { const url = new URL(req.url, 'http://localhost'); const file = resolve(root, 'desktop/build', url.pathname === '/' ? 'index.html' : url.pathname.slice(1));
      res.setHeader('Content-Type', { '.html':'text/html; charset=utf-8', '.js':'text/javascript', '.css':'text/css', '.json':'application/json' }[extname(file)] || 'application/octet-stream'); res.end(await readFile(file));
    } catch { res.statusCode=404;res.end(); }
  });
  await new Promise(r => server.listen(0,'127.0.0.1',r));
  const base = `http://127.0.0.1:${server.address().port}/?story=ux-updates--acceptance&mode=preview`;
  const context = await browser.newContext({ viewport: { width: 1100, height: 900 } });
  const page = await context.newPage();
  for(const view of ['library','mods']) {
    await page.goto(`${base}&uxView=${view}`);
    await page.getByRole('button',{name:'检查更新',exact:true}).click();
    const expected = phase==='after' ? (view==='library' ? '未发现可用更新。部分内容可能未被检查。' : '未发现兼容更新。部分 Mod 可能未被检查。') : (view==='library' ? '所有实例都已是最新' : '全部 mod 已是最新');
    await page.getByText(expected,{exact:true}).waitFor();
    if(phase==='after') {
      assert.equal(await page.getByText(/全部 mod 已是最新|所有实例都已是最新/).count(),0);
      await page.getByText(view==='library' ? /检查 Modrinth 整合包/ : /仅检查 Modrinth 可识别/).waitFor();
    }
    await page.waitForTimeout(450); // Finish the real toast entrance animation before capture.
    await page.screenshot({path:`${out}/updates-${view}-${phase}.png`});
  }
  await context.close(); await new Promise(r=>server.close(r));
}
await browser.close();
console.log('PASS: actual Library and ModsTab empty results with honest scope, paired baseline/head screenshots');
