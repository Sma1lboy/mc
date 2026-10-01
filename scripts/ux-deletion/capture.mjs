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
  const base = `http://127.0.0.1:${server.address().port}/?story=ux-deletion--acceptance&mode=preview`;
  const context = await browser.newContext({ viewport: { width: 1100, height: 900 } });
  const page = await context.newPage();
  for(const view of ['instance','world']) {
    await page.goto(`${base}&uxView=${view}`);
    if(view==='instance') {
      await page.getByRole('button',{name:'更多操作',exact:true}).click();
      await page.getByRole('menuitem',{name:'删除实例',exact:true}).click();
    } else await page.getByRole('button',{name:'删除',exact:true}).click();
    await page.getByRole('dialog').waitFor();
    if(phase==='after') await page.getByRole('dialog').getByText(/若失败,会永久删除且无法恢复。请先备份。/).waitFor();
    await page.screenshot({path:`${out}/deletion-${view}-${phase}.png`});
    await page.getByRole('button',{name:'取消',exact:true}).click();
    await page.getByRole('dialog').waitFor({state:'hidden'});
    assert.equal(await page.getByRole('dialog').isVisible(),false);
    assert.equal(await page.evaluate(()=>window.uxDeleteCalls),0);
    if(view==='world') {
      await page.getByRole('button',{name:'删除',exact:true}).click();
      await page.getByRole('dialog').waitFor();
      await page.getByRole('dialog').getByRole('button',{name:'取消',exact:true}).focus();
      await page.waitForTimeout(200); // Let Ark install its dismissable-layer listener after reopening.
      await page.keyboard.press('Escape');
      await page.getByRole('dialog').waitFor({state:'hidden'});
      assert.equal(await page.evaluate(()=>window.uxDeleteCalls),0);
    }
  }
  await context.close(); await new Promise(r=>server.close(r));
}
await browser.close();
console.log('PASS: real InstanceRow and WorldsPanel confirmation copy; Cancel/Escape; no deletion calls');
