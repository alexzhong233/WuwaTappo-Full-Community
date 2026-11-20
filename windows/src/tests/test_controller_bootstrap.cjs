// 全新安装（发行包不带素材缓存）时，控制器必须能正常启动并等宿主补齐素材。
// 老逻辑在 /status 返回 ready:false 时直接 throw，主渲染进程会把整个界面替换成
// 一行「请通过本地版启动器运行」，用户装完就完全用不了。
const vm = require('vm');
const assert = require('assert/strict');
const { renderFrontend } = require('../../../shared/tests/render-frontend.cjs');
const source = renderFrontend('local_atlas_controller.js', 'win');

const checks = [];
const record = (name, fn) => { fn(); checks.push(name); };
const state = { ready: false, enabled: false, sync: { status: 'running', done: 1, total: 3 } };
let modeCalls = 0, syncCalls = 0, applied = 0;
const intervals = [];
const storage = new Map();

const window = {
  location: { href: 'http://tauri.localhost/index.html', origin: 'http://tauri.localhost' },
  fetch: async (url) => {
    const target = String(url);
    let body = {};
    if (target.endsWith('/status')) body = { ready: state.ready, enabled: state.enabled, mode_verified: true, network_policy: 'rooms-only', sync: state.sync };
    else if (target.endsWith('/offline-data')) body = { catalog: { character_skin: {} }, atlas: [{ asset_id: 1, asset_type: 'character_skin_atlas', asset_name: 'a' }], atlas_count: 1 };
    else if (target.endsWith('/mode')) { modeCalls++; state.enabled = JSON.parse(arguments[1].body).enabled; body = { enabled: state.enabled, mode_verified: true, network_policy: 'rooms-only', ready: true }; }
    else if (target.endsWith('/sync')) { syncCalls++; body = state.sync; }
    else if (target.endsWith('/sync/status')) body = state.sync;
    else if (target.endsWith('/bootstrap')) body = { token: 'test' };
    return { ok: true, json: async () => body };
  },
  XMLHttpRequest: class { open() {} send() {} addEventListener() {} abort() {} },
  WebSocket: class { constructor() {} addEventListener() {} close() {} },
  EventSource: class { constructor() {} addEventListener() {} close() {} },
  navigator: { sendBeacon: () => true },
  addEventListener() {}, dispatchEvent() {}, CustomEvent: class { constructor(type, init) { this.type = type; Object.assign(this, init); } },
  setInterval(fn) { intervals.push(fn); return intervals.length; },
  clearInterval() {},
  setTimeout() { return 0; }
};

// fetch 的第二个参数在箭头函数里拿不到 arguments，单独包一层
const rawFetch = window.fetch;
window.fetch = async function (url, options) {
  if (String(url).endsWith('/mode')) {
    modeCalls++;
    state.enabled = JSON.parse(options.body).enabled;
    return { ok: true, json: async () => ({ enabled: state.enabled, mode_verified: true, network_policy: 'rooms-only', ready: true }) };
  }
  return rawFetch(url);
};

const context = { URL, AbortController, console };
vm.createContext(context);
vm.runInContext(source + '\nthis.factory=createLocalAtlasController;', context);
const controller = context.factory({
  ref: value => ({ value }),
  window,
  storage: { getItem: key => storage.get(key), setItem: (key, val) => storage.set(key, val) },
  origin: 'http://127.0.0.1:43219', token: 'test', atlasCount: 1,
  normalizeAtlasList: value => value,
  resolvePetAssetIdFromAppearanceId: id => id,
  getAppearanceAssetById: id => ({ asset_id: id, asset_type: 'character_skin' }),
  snapshotCatalog: () => ({}), applyOfflineCatalog: () => { applied++; }, restoreCatalog: () => {},
  catalogConsumer: true,
  onError: error => { throw error; }
});

(async () => {
  await controller.ready;
  record('缓存缺失时启动不再抛错', () => assert.equal(controller.cacheReady.value, false));
  record('缓存缺失时保持联机模式', () => assert.equal(controller.enabled.value, false));
  record('宿主自动开始下载', () => assert.ok(syncCalls >= 1));
  await assert.rejects(controller.setEnabled(true), /正在下载 1\/3/);
  record('未就绪时不允许开启本地模式', () => assert.equal(modeCalls, 0));

  state.ready = true;
  state.sync = { status: 'complete', done: 3, total: 3 };
  await intervals[0]();
  record('同步完成后自动变为可用', () => assert.equal(controller.cacheReady.value, true));
  await controller.setEnabled(true);
  record('就绪后可以开启本地模式', () => assert.ok(controller.enabled.value && modeCalls >= 1 && applied > 0));
  console.log(checks.map(name => 'ok - ' + name).join('\n'));
})().catch(error => { console.error(error); process.exit(1); });
