/**
 * 前端副本的漂移守卫
 *
 * `shared/frontend/` 是唯一源，`macos/fullatlas-source/` 与 `windows/src/src/`
 * 里的同名文件都是 `npm run sync:frontend` 的产物。以前两侧手工同步，靠
 * `platform-parity.test.mjs` 数差异行数兜底——那只挡得住"改漏一侧"，挡不住
 * "两侧都改但改法不同"。
 *
 * 这里改成严格比对：副本必须逐字节等于渲染结果。想改功能只改唯一源；
 * 直接改副本会在这里红。
 */
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { MODULES, TARGETS, VERSION_MODULE, render, syncFrontend } from '../../scripts/sync-frontend.mjs';

const repoRoot = path.resolve(import.meta.dirname, '..', '..');

test('两侧前端副本与唯一源逐字节一致', async () => {
  const { drifted } = await syncFrontend({ projectRoot: repoRoot, check: true });
  assert.deepEqual(drifted, [], '副本落后于唯一源，请运行 npm run sync:frontend');
});

test('shared/frontend 没有多余文件（新增模块要同时登记到 sync-frontend.mjs）', async () => {
  const names = (await readdir(path.join(repoRoot, 'shared/frontend'))).filter(name => name.endsWith('.js')).sort();
  assert.deepEqual(names, [...MODULES].sort());
});

test('唯一源里不留平台字面量，否则生成器替换不到', async () => {
  for (const name of MODULES) {
    const source = await readFile(path.join(repoRoot, 'shared/frontend', name), 'utf8');
    for (const target of Object.values(TARGETS)) {
      assert.ok(!source.includes(target.brand), `${name} 直接写死了「${target.brand}」，应改用 __WUWA_BRAND__`);
    }
    assert.ok(!/macOS input queue|Windows input queue/.test(source), `${name} 直接写死了平台名，应改用 __WUWA_PLATFORM__`);
  }
});

test('渲染结果只差品牌行、平台注释与 VERSION 三处', () => {
  const source = [
    "el('div','__WUWA_BRAND__',eyebrow);",
    '/* Main-renderer driver. No signal enters the __WUWA_PLATFORM__ input queue. */',
    "const VERSION='1.3.0-alpha.3', updateRequired=ref(false);",
  ].join('\n');
  const mac = render(VERSION_MODULE, source, TARGETS.mac);
  const win = render(VERSION_MODULE, source, TARGETS.win);
  assert.match(mac, /WUWA TAPPO FULLATLAS \/ MAC/);
  assert.match(mac, /No signal enters the macOS input queue/);
  assert.match(mac, /const VERSION='1\.3\.0-alpha\.3'/);
  assert.match(win, /WUWA TAPPO \/ 后台助手/);
  assert.match(win, /No signal enters the Windows input queue/);
  assert.match(win, /const VERSION='1\.2'/);
});

test('VERSION 只在 desktop_assistant.js 里被覆盖，别的模块不受影响', () => {
  const source = "const VERSION='9.9.9';";
  const other = "const other='keep';";
  assert.match(render(VERSION_MODULE, source, TARGETS.win), /const VERSION='1\.2'/);
  assert.equal(render('assistant_dashboard.js', other, TARGETS.win), other);
});
