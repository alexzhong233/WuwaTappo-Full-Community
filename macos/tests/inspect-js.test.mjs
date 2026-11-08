/**
 * 结构索引器的冒烟测试
 *
 * `shared/tools/inspect_js.cjs` 是两平台共用的解包分析器（macOS 的
 * `fullatlas-frontend.mjs`、Windows 的 `build.py` 与两个 ps1 都调它），
 * 输出 js_structure.json 供注入器按结构定位锚点。它原来是两份拷贝，
 * 合并后路径与 acorn 来源都变过，所以这里用最小夹具直接跑一遍：
 * 走不到就等于两个平台的注入链都断在第一步。
 */
import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';

const repoRoot = path.resolve(import.meta.dirname, '..', '..');
const tool = path.join(repoRoot, 'shared/tools/inspect_js.cjs');

// 官方压缩产物用的是 `export{a as b}` 形式，不是 `export function`——索引器只认前者，
// 夹具照抄真实形状，否则测的是我们自己编的语法。
const SHARED = [
  'function applyCatalog(a,b){}',
  'const helper=1;',
  'function localOnly(){}',
  'export{applyCatalog as applyCatalog, helper as helper, localOnly as zzLocal};',
].join('\n');

const MAIN = [
  "import { applyCatalog as apply, helper } from './index-shared.js';",
  'function widget(){',
  '  return { __name: "Widget", setup(){ const v = 1; return () => v; } };',
  '}',
  'const snapshot = { streamId: 1, bagList: two, atlasList: three };',
  'function route(state){ return state === "room" ? a : b; }',
].join('\n');

test('索引器产出 js_structure.json，并解析出函数、组件、快照与导出表', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'wuwa-inspect-'));
  try {
    const assets = path.join(root, 'readable_js/assets');
    await mkdir(assets, { recursive: true });
    await writeFile(path.join(assets, 'index-shared.js'), SHARED);
    await writeFile(path.join(assets, 'index-main.js'), MAIN);
    const result = spawnSync(process.execPath, [tool, root, 'index-shared.js'], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /indexed 2 modules/);

    const structure = JSON.parse(await readFile(path.join(root, 'js_structure.json'), 'utf8'));
    assert.equal(structure.shared, 'index-shared.js');
    assert.deepEqual(structure.exports, { applyCatalog: 'applyCatalog', helper: 'helper', zzLocal: 'localOnly' });
    assert.equal(structure.files['index-main.js'].imports.applyCatalog, 'apply');

    const main = structure.files['index-main.js'];
    assert.ok(main.functions.some(fn => fn.name === 'widget'));
    assert.equal(main.components[0].name, 'Widget');
    assert.equal(main.snapshots.length, 1);
    assert.equal(main.roomConditions.length, 1);
    // 偏移要落在原文的字符位置上，否则注入器会切到别的地方。
    const body = MAIN.slice(main.functions.find(fn => fn.name === 'widget').start);
    assert.match(body, /^function widget\(\)/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('缺少参数时打印用法并以非零码退出', () => {
  const result = spawnSync(process.execPath, [tool], { encoding: 'utf8' });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /用法/);
});
