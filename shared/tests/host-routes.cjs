/**
 * 注入进官方客户端的页面与注入器请求的本机接口，必须在 Rust 宿主里真的有路由。
 *
 * 为什么需要这道检查：Windows 宿主从 Python 换成 shared/backend 的 Rust 之后，
 * `/startup`（开机自启托管）和 `/assistant/context-menu`（桌宠右键原生菜单）
 * 没跟着搬过去，前端照旧调用，服务端直接 404 / 400 —— 功能静默失效，跑构建也看不出来。
 * 这类缺口只能在「调用方」和「实现方」之间做一次文本级对账才挡得住。
 */
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const assert = require('node:assert/strict');

const root = join(__dirname, '..', '..');
const hostSource = readFileSync(
  join(root, 'shared', 'backend', 'src', 'local_gallery_host.rs'),
  'utf8',
);

// 调用方：注入到官方前端里的模块，以及改写官方界面的注入器。
const callers = [
  'windows/src/src/desktop_context_menu.js',
  'windows/src/src/desktop_assistant.js',
  'windows/src/src/assistant_dashboard.js',
  'windows/src/src/local_atlas_controller.js',
  'windows/src/tools/integrate_assistant.py',
];

// 只认本机服务的路径前缀；官方游戏接口（/api/frontend/...）不属于这里。
const ROUTE = /["'`]\/(?:assistant|startup|bootstrap|status|sync|mode|custom|diagnostic|asset-base|assets|offline-data|community|official|update|settings)[a-zA-Z0-9_\-/]*["'`]/g;

const requested = new Map();
for (const file of callers) {
  const source = readFileSync(join(root, file), 'utf8');
  for (const match of source.match(ROUTE) ?? []) {
    const route = match.slice(1, -1);
    if (!requested.has(route)) requested.set(route, new Set());
    requested.get(route).add(file);
  }
}

assert.ok(requested.size > 0, '没有从调用方解析出任何接口路径，检查文件列表是否过期');

const missing = [];
for (const [route, files] of requested) {
  // 宿主里既可能是精确匹配（"/startup"），也可能是前缀分支（"/assets/"）。
  const declared = hostSource.includes(`"${route}"`) || hostSource.includes(`'${route}'`);
  if (!declared) missing.push(`${route}  ← ${[...files].join(', ')}`);
}

assert.deepEqual(
  missing,
  [],
  `以下接口被前端/注入器调用，但在 shared/backend/src/local_gallery_host.rs 里没有路由：\n  ${missing.join('\n  ')}`,
);

console.log(`本机接口对账通过（${requested.size} 条路径）`);
