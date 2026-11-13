/**
 * 仓库结构守卫
 *
 * macos/ 与 windows/ 各放一套平台实现；跨平台源代码和文档放在 shared/、docs/，
 * 根目录保留统一命令入口与法律文件。
 * 这条边界靠人记住不可靠——目录一旦又长回仓库根，下一次「这文件是哪个平台的」就得重新考古，
 * 所以把它落成断言。
 */
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const repoRoot = path.resolve(import.meta.dirname, '..', '..');

// 这些名字都属于 macOS 平台实现，出现目录根就说明有人绕过 macos/ 直接改了根目录。
const PLATFORM_ENTRIES = ['src-tauri', 'tests', 'tools', 'fullatlas-source', 'frontend', 'frontend-global'];
const PLATFORM_FILES = ['upstream-cn.json', 'upstream-global.json', 'rust-toolchain.toml'];

test('平台实现只留在 macos/ 与 windows/，不散落到仓库根', () => {
  const strays = [...PLATFORM_ENTRIES, ...PLATFORM_FILES].filter(name => existsSync(path.join(repoRoot, name)));
  assert.deepEqual(strays, [], `这些应当放在 macos/ 下：${strays.join('、')}`);
  assert.equal(existsSync(path.join(repoRoot, 'macos/docs')), false, '文档统一维护在仓库根 docs/');
});

test('两个平台目录与跨平台入口都在位', () => {
  for (const name of ['macos/src-tauri/Cargo.toml', 'macos/scripts/app.mjs', 'macos/tests', 'macos/fullatlas-source',
    'windows/src', 'windows/tools', 'windows/upstream-win.json']) {
    assert.ok(existsSync(path.join(repoRoot, name)), `缺少 ${name}`);
  }
  for (const name of ['package.json', 'scripts/build-platform.mjs', 'LICENSE', 'THIRD_PARTY_NOTICES.md']) {
    assert.ok(existsSync(path.join(repoRoot, name)), `缺少跨平台入口或仓库级文件 ${name}`);
  }
  // 两平台共用的前端源码与文档有唯一归属；平台目录里只保留构建副本和专属说明。
  assert.ok(existsSync(path.join(repoRoot, 'shared/frontend/assistant_dashboard.js')), '缺少共用的前端源');
  assert.ok(existsSync(path.join(repoRoot, 'scripts/sync-frontend.mjs')), '缺少前端生成器');
  assert.ok(existsSync(path.join(repoRoot, 'docs/ARCHITECTURE.md')), '缺少共享架构文档');
  assert.ok(existsSync(path.join(repoRoot, 'docs/MODS.md')), '通用 Mod 文档应放在 docs/ 根目录');
  assert.ok(existsSync(path.join(repoRoot, 'mod-examples/demo-animated-pet.zip')), 'Mod 演示包应在仓库根目录');
  assert.equal(existsSync(path.join(repoRoot, 'docs/mod-examples')), false, 'Mod 演示素材不应放在 docs/');
  assert.equal(existsSync(path.join(repoRoot, 'docs/macos/MODS.md')), false, '通用文档不应按平台分目录');
  assert.ok(existsSync(path.join(repoRoot, 'docs/macos/INPUT_PERMISSION.md')), '缺少 Mac 专属权限说明');
  assert.ok(existsSync(path.join(repoRoot, 'docs/windows/BUILD.md')), '缺少 Windows 专属构建说明');
  // 注入器的公共核也只有一份：锚点工具与公共补丁在 inject_core，助手主流程在 assistant_core。
  for (const name of ['shared/tools/inject_core.py', 'shared/tools/assistant_core.py']) {
    assert.ok(existsSync(path.join(repoRoot, name)), `缺少注入器公共核 ${name}`);
  }
});
