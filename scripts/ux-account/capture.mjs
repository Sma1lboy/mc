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
  const base = `http://127.0.0.1:${server.address().port}/?story=ux-account--acceptance&mode=preview`;
  const context = await browser.newContext({ viewport: { width: 1100, height: 900 } });
  const page = await context.newPage();
  async function open(scenario, lang='zh') {
    await context.addInitScript(l => localStorage.setItem('mc-launcher.locale', l), lang);
    await page.goto(`${base}&uxCase=${scenario}`); await page.getByRole('button', { name: /微软账号|Microsoft Account/ }).waitFor();
  }
  await open('denied');
  await page.getByRole('button', { name: /微软账号/ }).click();
  await page.getByText('TEST-CODE', { exact: true }).waitFor();
  if (phase === 'after') {
    await page.getByText('未能复制。请选中上方代码,手动复制。').waitFor();
    await page.getByText('未能打开浏览器。请手动访问上方验证地址。').waitFor();
    assert.equal(await page.getByText(/已打开微软登录页并复制代码/).count(),0);
  } else await page.getByText(/已打开微软登录页并复制代码/).waitFor();
  await page.screenshot({ path: `${out}/account-denied-${phase}.png` });
  await open('start-error'); await page.getByRole('button', { name:/微软账号/ }).click();
  await page.getByText('Fixture: login service unavailable').waitFor();
  await page.screenshot({ path:`${out}/account-start-error-${phase}.png` });
  if(phase==='after') {
    assert.equal(await page.getByText('正在获取登录代码…').count(),0);
    await page.evaluate(()=>window.uxScenario='healthy');
    await page.getByRole('button',{name:'重新获取代码'}).click();
    await page.getByText('已复制登录代码',{exact:true}).waitFor();
  } else assert.equal(await page.getByText('正在获取登录代码…').count(),1);
  await open('delayed'); await page.getByRole('button',{name:/微软账号/}).click();
  await page.getByRole('button',{name:'关闭',exact:true}).click();
  await page.waitForTimeout(900);
  const calls=await page.evaluate(()=>window.uxCalls);
  if(phase==='after') { assert.equal(calls['plugin:shell|open']??0,0); assert.equal(calls['msa_login_poll']??0,0); }
  else assert.equal(calls['plugin:shell|open'],1);
  if(phase==='after') {
    await open('poll-error'); await page.getByRole('button',{name:/微软账号/}).click();
    await page.getByText('Fixture: code expired').waitFor(); await page.getByRole('button',{name:'返回',exact:true}).click();
    await page.getByRole('button',{name:/离线账号/}).click();
    await page.getByLabel('离线用户名',{exact:true}).fill('FixturePlayer');
    assert.equal(await page.getByRole('button',{name:'添加离线账号',exact:true}).isEnabled(),true);
    await page.getByRole('button',{name:'返回',exact:true}).click();
    await page.getByRole('button',{name:/外置登录/}).click();
    for(const name of ['皮肤站 API 地址','邮箱或用户名','密码']) await page.getByLabel(name,{exact:true}).waitFor();
    await page.screenshot({path:`${out}/account-external-labels-after.png`});
    await open('denied','en'); await page.getByRole('button',{name:/Microsoft Account/}).click();
    await page.getByText("Couldn't copy. Select the code above and copy it manually.").waitFor();
    await page.screenshot({path:`${out}/account-denied-en-after.png`});
  }
  await context.close(); await new Promise(r=>server.close(r));
}
await browser.close();
console.log('PASS: actual AccountDialog, denied copy/open, start failure, retry, expired code/back, close pending, offline action, external labels, English');
