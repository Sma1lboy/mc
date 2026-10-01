import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import assert from 'node:assert/strict';
import test from 'node:test';
const source = readFileSync(new URL('../../landing/copy-command.js', import.meta.url), 'utf8');
function fixture(writeText) {
  let click;
  const elements = { 'copy-brew': { disabled: false, addEventListener: (_, fn) => { click = fn; } }, 'copy-status': { textContent: '' }, 'brew-cmd': { textContent: 'brew install --cask sma1lboy/tap/kobemc' } };
  runInNewContext(source, { document: { getElementById: (id) => elements[id] }, navigator: { clipboard: { writeText } } });
  return { click: () => click(), status: elements['copy-status'], button: elements['copy-brew'] };
}
test('copy success is shown only after clipboard resolves; repeat clicks are gated', async () => {
  let resolve, calls = 0;
  const f = fixture(() => { calls++; return new Promise(r => { resolve = r; }); });
  const pending = f.click();
  assert.equal(f.status.textContent, '正在复制…');
  await f.click(); assert.equal(calls, 1);
  resolve(); await pending;
  assert.equal(f.status.textContent, '已复制安装命令');
  assert.equal(f.button.disabled, false);
});
test('clipboard rejection provides manual recovery and supports retry', async () => {
  let failed = true;
  const f = fixture(async () => { if (failed) throw new Error('denied'); });
  await f.click(); assert.match(f.status.textContent, /未能复制/); assert.equal(f.button.disabled, false);
  failed = false; await f.click(); assert.equal(f.status.textContent, '已复制安装命令');
});
test('primary CTA describes download navigation and absolute guarantees are absent', () => {
  const html = readFileSync(new URL('../../landing/index.html', import.meta.url), 'utf8');
  assert.match(html, /class="btn-play" href="#download">下载启动器/);
  assert.doesNotMatch(html, /满速|再也不会缺前置|想玩什么 Mod 都行|全程不用你动手/);
  assert.match(html, /安装包链接会打开 GitHub Releases/);
});
