import assert from 'node:assert/strict';
import vm from 'node:vm';
import { test } from 'node:test';
import { renderPlatformModule } from '../../scripts/sync-frontend.mjs';

const source = await renderPlatformModule('local_atlas_controller.js', 'mac');
const factory = vm.runInNewContext(`${source}\ncreateLocalAtlasController`, { URL, AbortController, console });

function controller({ document, appearance, synthesisSame, refreshBag } = {}) {
  const calls = [];
  const storage = new Map();
  const win = {
    fetch: async () => { throw new Error('unexpected fetch'); },
    addEventListener() {}, dispatchEvent() {}, setInterval: () => 1, clearInterval() {},
    setTimeout: callback => { callback(); return 1; }, clearTimeout() {},
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
    location: { href: 'tauri://localhost/' }, navigator: {}, ...(document ? { document } : {}),
  };
  const instance = factory({
    ref: value => ({ value }), window: win,
    storage: { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) },
    origin: 'https://mc-wuwatappo.kurogames-global.com', atlasCount: 1, catalogConsumer: false,
    normalizeAtlasList: value => value,
    resolvePetAssetIdFromAppearanceId: id => id,
    snapshotCatalog: () => ({}), applyOfflineCatalog: () => {}, restoreCatalog: () => {},
    getAppearanceAssetById: appearance || (id => ({ asset_id: id, asset_type: 'character_skin' })),
    synthesisAtlasSame: synthesisSame || (async ids => { calls.push(['synthesis', ids]); return {}; }),
    refreshBag: refreshBag || (async () => { calls.push(['refresh-bag', {}]); }),
    request: async (route, body) => {
      calls.push([route, body]);
      if (route === '/status') return { ready: true, enabled: true, mode_verified: true, network_policy: 'hybrid' };
      return {};
    },
  });
  return { instance, calls };
}

test('keeps official multiplayer and appearance writes available in hybrid mode', () => {
  const { instance } = controller();
  assert.equal(instance.restricted('/api/frontend/rooms', 'POST'), false);
  assert.equal(instance.restricted('/api/frontend/rooms/123', 'DELETE'), false);
  assert.equal(instance.restricted('/api/frontend/assets/favorites', 'POST'), false);
  assert.equal(instance.restricted('/api/frontend/user/me/appearance', 'PUT'), false);
  assert.equal(instance.restricted('/api/frontend/atlas', 'GET'), false);
  assert.equal(instance.restricted('/api/frontend/user/me', 'GET'), false);
  instance.dispose();
});

test('routes controller requests through the native bridge and never fetches localhost', async () => {
  const { instance, calls } = controller();
  await instance.ready;
  await instance.request('/assistant/state', {});
  // /asset-base 在启动时取一次本机取图前缀，其余请求照旧全部走原生桥。
  const routes = calls.map(([route]) => route);
  assert.deepEqual(routes.slice(0, 2), ['/status', '/asset-base']);
  assert.ok(routes.includes('/assistant/state'));
  instance.dispose();
});

test('加载遮罩延迟 300ms 才显形，短请求不会在屏幕上闪一下', () => {
  const injected = [];
  const head = { append: element => injected.push(element), setAttribute() {}, removeAttribute() {} };
  const document = {
    getElementById: () => null,
    createElement: () => ({ id: '', textContent: '' }),
    head, documentElement: head,
  };
  const { instance } = controller({ document });
  const style = injected.find(element => element.id === 'wuwa-loading-fade');
  assert.ok(style, '控制器启动时必须挂上 #my-loading 的延迟规则');
  // 官方 useLoading() 每次 show 都新建元素、hide 就 remove：动画延迟结束前元素已消失，
  // 圈根本不会显形；visibility 一并延后，透明遮罩也不会在延迟窗口里吃掉点击。
  assert.match(style.textContent, /#my-loading\{animation:[^}]*300ms both\}/);
  assert.match(style.textContent, /from\{opacity:0;visibility:hidden\}/);
  instance.dispose();
});

test('hybrid source labels, cloud preset decision, synthesis and capture stay shared', async () => {
  const appearance = id => id === 2
    ? { asset_id: 2, asset_type: 'accessory', game_role_id: 0 }
    : { asset_id: id, asset_type: 'character_skin', game_role_id: 0 };
  const { instance, calls } = controller({ appearance });
  await instance.ready;
  instance.enabled.value = true;
  const atlas = [
    { asset_id: 1, asset_type: 'character_skin_atlas', asset_name: 'Official' },
    { asset_id: 2, asset_type: 'accessory_atlas', asset_name: 'Local ears' },
  ];
  instance.inventory(atlas, [{ asset_id: 1, asset_type: 'character_skin', quantity: 3 }]);
  assert.equal(instance.officialOwned(1, 'character_skin'), true);
  assert.equal(instance.officialOwned(2, 'accessory'), false);
  instance.select({ asset_id: 1, asset_type: 'character_skin', asset_name: 'Official' }, null, 8, { quiet: true });
  assert.equal(instance.sourceFor(1, 'character_skin'), 'both');
  assert.equal(instance.shouldCreateLocalPreset(null), false);
  instance.select({ asset_id: 2, asset_type: 'accessory', asset_name: 'Local ears' }, null, 8, { quiet: true });
  assert.equal(instance.shouldCreateLocalPreset(null), true);
  instance.setAccessoryTransform({ rotate: 45, scale: 1.25, dx: 12 });
  const transform = instance.accessoryTransform(null);
  assert.equal(transform.dx, 12); assert.equal(transform.dy, 0);
  assert.equal(transform.scale, 1.25); assert.equal(transform.rotate, 45);
  await instance.synthesizeDuplicates('same-level');
  assert.deepEqual(Array.from(calls.find(([route]) => route === 'synthesis')[1]), [1, 1, 1]);
  instance.setCapture({ mode: 'green', panelOpacity: 0.7 });
  assert.equal(instance.capture.value.mode, 'green');
  assert.equal(instance.capture.value.panelOpacity, 0.7);
  instance.dispose();
});
