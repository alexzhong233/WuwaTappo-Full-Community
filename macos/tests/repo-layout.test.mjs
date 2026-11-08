/**
 * 仓库结构守卫
 *
 * macos/ 与 windows/ 各放一套平台实现，仓库根只留跨平台入口（package.json、
 * scripts/build-platform.mjs）、两平台共用的产物目录 dist/ 以及仓库级法律文件。
 * 这条边界靠人记住不可靠——目录一旦又长回仓库根，下一次「这文件是哪个平台的」就得重新考古，
 * 所以把它落成断言。
 */
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const repoRoot = path.resolve(import.meta.dirname, '..', '..');

// 这些名字都属于 macOS 平台实现，出现目录根就说明有人绕过 macos/ 直接改了根目录。
const PLATFORM_ENTRIES = ['src-tauri', 'tests', 'tools', 'docs', 'fullatlas-source', 'frontend', 'frontend-global'];
const PLATFORM_FILES = ['upstream-cn.json', 'upstream-global.json', 'rust-toolchain.toml'];

test('平台实现只留在 macos/ 与 windows/，不散落到仓库根', () => {
  const strays = [...PLATFORM_ENTRIES, ...PLATFORM_FILES].filter(name => existsSync(path.join(repoRoot, name)));
  assert.deepEqual(strays, [], `这些应当放在 macos/ 下：${strays.join('、')}`);
});

test('两个平台目录与跨平台入口都在位', () => {
  for (const name of ['macos/src-tauri/Cargo.toml', 'macos/scripts/app.mjs', 'macos/tests', 'macos/fullatlas-source',
    'windows/src', 'windows/tools', 'windows/upstream-win.json']) {
    assert.ok(existsSync(path.join(repoRoot, name)), `缺少 ${name}`);
  }
  for (const name of ['package.json', 'scripts/build-platform.mjs', 'LICENSE', 'THIRD_PARTY_NOTICES.md']) {
    assert.ok(existsSync(path.join(repoRoot, name)), `缺少跨平台入口或仓库级文件 ${name}`);
  }
  // 两平台共用的前端源码只有一份，放在 shared/frontend/；两侧目录里的是生成物。
  assert.ok(existsSync(path.join(repoRoot, 'shared/frontend/assistant_dashboard.js')), '缺少共用的前端源');
  assert.ok(existsSync(path.join(repoRoot, 'scripts/sync-frontend.mjs')), '缺少前端生成器');
});
