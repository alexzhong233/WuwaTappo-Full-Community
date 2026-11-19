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
const hostSource =
  readFileSync(join(root, 'shared', 'backend', 'src', 'local_gallery_host.rs'), 'utf8') +
  '\n' +
  readFileSync(join(root, 'shared', 'backend', 'src', 'lib.rs'), 'utf8');

const macSource =
  readFileSync(join(root, 'macos', 'src-tauri', 'src', 'fullatlas.rs'), 'utf8') +
  '\n' +
  readFileSync(join(root, 'macos', 'src-tauri', 'src', 'mod_assets.rs'), 'utf8') +
  '\n' +
  readFileSync(join(root, 'shared', 'backend', 'src', 'custom_resources.rs'), 'utf8') +
  '\n' +
  readFileSync(join(root, 'shared', 'backend', 'src', 'lib.rs'), 'utf8');

// 调用方：跨端共享前端、共享注入器以及桌面壳层桥接代码。
const callers = [
  'shared/frontend/local_atlas_controller.js',
  'shared/frontend/desktop_assistant.js',
  'shared/frontend/assistant_dashboard.js',
  'shared/tools/fullatlas_core.py',
  'shared/tools/assistant_core.py',
  'windows/src/src/desktop_context_menu.js',
  'windows/src/tools/integrate_assistant.py',
];

const WINDOWS_ONLY_ROUTES = new Set([
  '/bootstrap',
  '/startup',
  '/assistant/context-menu',
  '/assets/',
]);

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

const missingWin = [];
const missingMac = [];
for (const [route, files] of requested) {
  // Windows 宿主里既可能是精确匹配（"/startup"），也可能是前缀分支（"/assets/"）。
  const winDeclared = hostSource.includes(`"${route}"`) || hostSource.includes(`'${route}'`);
  if (!winDeclared) missingWin.push(`${route}  ← ${[...files].join(', ')}`);

  if (!WINDOWS_ONLY_ROUTES.has(route)) {
    const macPrefix = route.replace(/^\/+/, '');
    const macAction = macPrefix.replace(/\/+$/, '');
    const assistantAction = macAction.startsWith('assistant/')
      ? macAction.slice('assistant/'.length)
      : macAction;
    const macDeclared =
      macSource.includes(`"${macPrefix}"`) ||
      macSource.includes(`"${macAction}"`) ||
      (macAction.startsWith('assistant/') && macSource.includes(`"${assistantAction}"`));
    if (!macDeclared) missingMac.push(`${route} (${macAction})  ← ${[...files].join(', ')}`);
  }
}

assert.deepEqual(
  missingWin,
  [],
  `以下接口被前端/注入器调用，但在 shared/backend/src/local_gallery_host.rs 里没有路由：\n  ${missingWin.join('\n  ')}`,
);
assert.deepEqual(
  missingMac,
  [],
  `以下跨端接口被前端/注入器调用，但在 macOS Rust 宿主里没有对应 action：\n  ${missingMac.join('\n  ')}`,
);

console.log(`本机接口双端对账通过（共 ${requested.size} 条路径）`);
