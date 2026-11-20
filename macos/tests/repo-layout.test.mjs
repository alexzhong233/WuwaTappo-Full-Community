/**
 * 仓库结构守卫
 *
 * macos/ 与 windows/ 各放一套平台实现；跨平台源代码和文档放在 shared/、docs/，
 * 根目录保留统一命令入口与法律文件。
 * 这条边界靠人记住不可靠——目录一旦又长回仓库根，下一次「这文件是哪个平台的」就得重新考古，
 * 所以把它落成断言。
 */
import assert from 'node:assert/strict';
import { existsSync, readdirSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const repoRoot = path.resolve(import.meta.dirname, '..', '..');

function findPythonFiles(dir) {
  const found = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === '.git' || entry.name === 'node_modules' || entry.name === '.work' || entry.name === 'target') {
      continue;
    }
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) found.push(...findPythonFiles(full));
    else if (entry.name.endsWith('.py')) found.push(path.relative(repoRoot, full));
  }
  return found;
}

// 这些名字都属于 macOS 平台实现，出现目录根就说明有人绕过 macos/ 直接改了根目录。
const PLATFORM_ENTRIES = ['src-tauri', 'tests', 'tools', 'fullatlas-source', 'frontend', 'frontend-global'];
const PLATFORM_FILES = ['rust-toolchain.toml'];

test('平台实现只留在 macos/ 与 windows/，不散落到仓库根', () => {
  const strays = [...PLATFORM_ENTRIES, ...PLATFORM_FILES].filter(name => existsSync(path.join(repoRoot, name)));
  assert.deepEqual(strays, [], `这些应当放在 macos/ 下：${strays.join('、')}`);
  assert.equal(existsSync(path.join(repoRoot, 'macos/docs')), false, '文档统一维护在仓库根 docs/');
});

test('两个平台目录与跨平台入口都在位，且无 Python 或生成副本残留', () => {
  for (const name of ['macos/src-tauri/Cargo.toml', 'macos/scripts/app.mjs', 'macos/tests',
    'windows/src', 'windows/tools', 'shared/upstream/cn.json', 'shared/upstream/global.json']) {
    assert.ok(existsSync(path.join(repoRoot, name)), `缺少 ${name}`);
  }
  assert.equal(existsSync(path.join(repoRoot, 'macos/fullatlas-source')), false, 'macos/fullatlas-source 生成副本应已移除');
  assert.equal(existsSync(path.join(repoRoot, 'windows/src/src')), false, 'windows/src/src 生成副本应已移除');
  assert.deepEqual(findPythonFiles(repoRoot), [], '仓库内不应再保留 .py 脚本');
  for (const name of ['package.json', 'scripts/build-platform.mjs', 'LICENSE', 'THIRD_PARTY_NOTICES.md']) {
    assert.ok(existsSync(path.join(repoRoot, name)), `缺少跨平台入口或仓库级文件 ${name}`);
  }
  // 两平台共用的前端源码与文档有唯一归属；平台目录里只保留构建与专属说明。
  assert.ok(existsSync(path.join(repoRoot, 'shared/frontend/assistant_dashboard.js')), '缺少共用的前端源');
  assert.ok(existsSync(path.join(repoRoot, 'shared/frontend/desktop_context_menu.js')), '缺少共用的右键菜单源');
  assert.ok(existsSync(path.join(repoRoot, 'scripts/sync-frontend.mjs')), '缺少前端生成器');
  assert.ok(existsSync(path.join(repoRoot, 'docs/ARCHITECTURE.md')), '缺少共享架构文档');
  assert.ok(existsSync(path.join(repoRoot, 'docs/MODS.md')), '通用 Mod 文档应放在 docs/ 根目录');
  assert.ok(existsSync(path.join(repoRoot, 'mod-examples/demo-animated-pet.zip')), 'Mod 演示包应在仓库根目录');
  assert.equal(existsSync(path.join(repoRoot, 'docs/mod-examples')), false, 'Mod 演示素材不应放在 docs/');
  assert.equal(existsSync(path.join(repoRoot, 'docs/macos/MODS.md')), false, '通用文档不应按平台分目录');
  assert.ok(existsSync(path.join(repoRoot, 'docs/macos/INPUT_PERMISSION.md')), '缺少 Mac 专属权限说明');
  assert.ok(existsSync(path.join(repoRoot, 'docs/windows/BUILD.md')), '缺少 Windows 专属构建说明');
  // 注入器、PE 重打包、快捷方式、服务核心与跨平台构建/同步入口只在根目录 shared/ 与 scripts/。
  for (const name of [
    'shared/backend/src/injector.rs',
    'shared/backend/src/pe_repack.rs',
    'shared/backend/src/lnk.rs',
    'shared/backend/src/demo_packs.rs',
    'shared/backend/src/service.rs',
    'shared/config/regions.json',
    'shared/contracts/release-assets.json',
    'shared/contracts/windows-update.json',
    'scripts/extract-assets.mjs',
    'scripts/fetch-official-win.mjs',
    'scripts/sync-upstream.mjs',
    'scripts/upstream-detect.mjs',
    'scripts/upstream-release.mjs',
    'scripts/installer-mirror.mjs',
    'scripts/installer-cache.mjs',
  ]) {
    assert.ok(existsSync(path.join(repoRoot, name)), `缺少跨平台文件 ${name}`);
  }
});
