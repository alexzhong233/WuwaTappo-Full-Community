/**
 * 平台一致性（platform parity）
 *
 * macOS 与 Windows 共用同一套 FullAtlas 前端模块。源只有一份，在 `shared/frontend/`；
 * `macos/fullatlas-source/` 与 `windows/src/src/` 下的同名文件都是
 * `npm run sync:frontend` 生成的副本，只差品牌文案、首行平台注释和 VERSION 常量。
 *
 * 严格比对由 `frontend-sync.test.mjs` 负责（副本必须逐字节等于渲染结果）。
 * 这里保留行级 diff，是给人和 CI 日志看的：万一有人绕过生成器直接抄了一份改，
 * 这几行数字会立刻变得不像话。
 *
 * 差异上限变化时需要人工确认：是生成器配置改了（品牌文案 / VERSION），还是漏跑了生成器。
 */
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

// macos/ 下是 Mac 侧实现，跨平台的对比对象（windows/、package.json）在仓库根。
const macosRoot = path.resolve(import.meta.dirname, '..');
const repoRoot = path.resolve(macosRoot, '..');
const MAC_DIR = 'fullatlas-source';
const WIN_DIR = 'windows/src/src';
const MODULES = ['assistant_dashboard.js', 'desktop_assistant.js', 'local_atlas_controller.js'];

/** 忽略行尾 CR 的行级 diff，返回 ['-旧行' | '+新行', ...]。 */
function lineDiff(left, right) {
  const a = left.split('\n').map(line => line.replace(/\r$/, ''));
  const b = right.split('\n').map(line => line.replace(/\r$/, ''));
  const n = a.length, m = b.length;
  const dp = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const out = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) { i += 1; j += 1; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) out.push(`-${a[i++]}`);
    else out.push(`+${b[j++]}`);
  }
  while (i < n) out.push(`-${a[i++]}`);
  while (j < m) out.push(`+${b[j++]}`);
  return out;
}

for (const name of MODULES) {
  test(`${name} 的平台差异不超过 4 行，生成器输出没有被手改`, async () => {
    const mac = await readFile(path.join(macosRoot, MAC_DIR, name), 'utf8');
    const win = await readFile(path.join(repoRoot, WIN_DIR, name), 'utf8');
    const diff = lineDiff(mac, win);
    const preview = diff.map(line => `  ${line.slice(0, 140)}`).join('\n');
    assert.ok(
      diff.length <= 4,
      `${name} 两侧差异 ${diff.length} 行，超过约定的 4 行：\n${preview}\n` +
        '这两个文件是 shared/frontend/ 的生成物，不要手改；改完唯一源跑 npm run sync:frontend。',
    );
  });
}

test('Mac 侧模块版本号与本仓库 package.json 保持一致', async () => {
  const pkg = JSON.parse(await readFile(path.join(repoRoot, 'package.json'), 'utf8'));
  const source = await readFile(path.join(macosRoot, MAC_DIR, 'desktop_assistant.js'), 'utf8');
  const version = /const VERSION='([^']+)'/.exec(source)?.[1];
  assert.equal(version, pkg.version, 'Mac 侧 VERSION 需跟随整合版发版号');
});

test('Windows 侧模块版本号跟随上游 FullAtlas，不随本仓库发版变动', async () => {
  const source = await readFile(path.join(repoRoot, WIN_DIR, 'desktop_assistant.js'), 'utf8');
  const version = /const VERSION='([^']+)'/.exec(source)?.[1];
  assert.equal(version, '1.2', 'Windows 侧沿用上游 FullAtlas 1.2 的版本号');
});
