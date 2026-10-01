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
  const base = `http://127.0.0.1:${server.address().port}/?story=ux-downloads--acceptance&mode=preview`;
  const context = await browser.newContext({ viewport: { width: 1100, height: 900 } });
  const page = await context.newPage(); await page.goto(base);
  await page.getByRole('button',{name:phase==='after'?'下载与安装':'下载队列',exact:true}).click();
  await page.getByText('Fixture: Failed modpack',{exact:true}).waitFor();
  if(phase==='after') {
    await page.getByText('安装未完成',{exact:true}).waitFor();
    await page.getByText('任务已完成',{exact:true}).waitFor();
    await page.getByText('如有缺失文件提示,请先完成手动下载。',{exact:true}).waitFor();
    await page.getByText('请返回发起安装的页面重试。',{exact:true}).waitFor();
    await page.getByText('查看错误详情',{exact:true}).click();
    await page.getByText('Fixture: HTTP 503 while downloading modpack',{exact:true}).waitFor();
  }
  await page.screenshot({path:`${out}/downloads-mixed-${phase}.png`});
  await page.getByRole('button',{name:phase==='after'?'清除已结束记录':'清除已完成',exact:true}).click();
  assert.equal(await page.getByText('Fixture: Failed modpack',{exact:true}).count(),0);
  assert.equal(await page.getByText('Fixture: Installed modpack',{exact:true}).count(),0);
  assert.equal(await page.getByText('Fixture: Installing modpack',{exact:true}).count(),1);
  assert.equal(await page.getByText('Fixture: Queued mod',{exact:true}).count(),1);
  await context.close(); await new Promise(r=>server.close(r));
}
await browser.close();
console.log('PASS: actual mixed-status DownloadQueue, failure details/recovery, clear ended records preserves active and queued');
