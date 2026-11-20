/**
 * 共享前端唯一源与双端渲染守卫
 *
 * `shared/frontend/` 是唯一源；构建期由 Rust 注入器直接按平台渲染，
 * 不再保留 `macos/fullatlas-source/` 与 `windows/src/src/` 的生成副本。
 */
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { MODULES, TARGETS, VERSION_MODULE, render, renderPlatformModule, syncFrontend } from '../../scripts/sync-frontend.mjs';

const repoRoot = path.resolve(import.meta.dirname, '..', '..');

test('共享前端无残留副本且所有模块按双端渲染有效', async () => {
  const { drifted } = await syncFrontend({ projectRoot: repoRoot, check: true });
  assert.deepEqual(drifted, [], '共享前端存在未清洗的占位符或残余生成副本');
});

test('shared/frontend 没有多余文件（新增模块要同时登记到 sync-frontend.mjs）', async () => {
  const names = (await readdir(path.join(repoRoot, 'shared/frontend'))).filter(name => name.endsWith('.js')).sort();
  assert.deepEqual(names, [...MODULES].sort());
});

test('唯一源里不留平台字面量，否则渲染器替换不到', async () => {
  for (const name of MODULES) {
    const source = await readFile(path.join(repoRoot, 'shared/frontend', name), 'utf8');
    for (const target of Object.values(TARGETS)) {
      assert.ok(!source.includes(target.brand), `${name} 直接写死了「${target.brand}」，应改用 __WUWA_BRAND__`);
    }
    assert.ok(!/macOS input queue|Windows input queue/.test(source), `${name} 直接写死了平台名，应改用 __WUWA_PLATFORM__`);
  }
});

test('渲染结果只差品牌行、平台注释与默认配置，两平台统一使用唯一源版本号', async () => {
  const source = [
    "el('div','__WUWA_BRAND__',eyebrow);",
    '/* Main-renderer driver. No signal enters the __WUWA_PLATFORM__ input queue. */',
    "const VERSION='1.3.0-alpha.3', defaults=__WUWA_ASSISTANT_DEFAULTS__;",
  ].join('\n');
  const mac = render(VERSION_MODULE, source, TARGETS.mac);
  const win = render(VERSION_MODULE, source, TARGETS.win);
  assert.match(mac, /WUWA TAPPO FULLATLAS \/ MAC/);
  assert.match(mac, /No signal enters the macOS input queue/);
  assert.match(mac, /const VERSION='1\.3\.0-alpha\.3'/);
  assert.match(win, /WUWA TAPPO \/ 后台助手/);
  assert.match(win, /No signal enters the Windows input queue/);
  assert.match(win, /const VERSION='1\.3\.0-alpha\.3'/);

  const realMac = await renderPlatformModule(VERSION_MODULE, 'mac', repoRoot);
  const realWin = await renderPlatformModule(VERSION_MODULE, 'win', repoRoot);
  assert.ok(realMac.includes('macOS input queue'));
  assert.ok(realWin.includes('Windows input queue'));
});

test('所有共用模块按平台只替换品牌、平台名与默认配置占位符', () => {
  const source = "const VERSION='9.9.9', cfg=__WUWA_ASSISTANT_DEFAULTS__;";
  const other = "const other='keep';";
  assert.match(render(VERSION_MODULE, source, TARGETS.win), /const VERSION='9\.9\.9'/);
  assert.equal(render('assistant_dashboard.js', other, TARGETS.win), other);
});
