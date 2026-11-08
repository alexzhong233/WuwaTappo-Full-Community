import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { test } from 'node:test';

const source = await readFile(new URL('../fullatlas-source/local_atlas_controller.js', import.meta.url), 'utf8');
const factory = vm.runInNewContext(`${source}\ncreateLocalAtlasController`, { URL, AbortController, console });

function controller({ document } = {}) {
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
    request: async (route, body) => {
      calls.push([route, body]);
      if (route === '/status') return { ready: true, enabled: false, mode_verified: true, network_policy: 'rooms-only' };
      return {};
    },
  });
  return { instance, calls };
}

test('blocks room and account appearance writes while allowing ordinary account reads', () => {
  const { instance } = controller();
  assert.equal(instance.restricted('/api/frontend/rooms', 'POST'), true);
  assert.equal(instance.restricted('/api/frontend/rooms/123', 'DELETE'), true);
  assert.equal(instance.restricted('/api/frontend/assets/favorites', 'POST'), true);
  assert.equal(instance.restricted('/api/frontend/user/me/appearance', 'PUT'), true);
  assert.equal(instance.restricted('/api/frontend/atlas', 'GET'), false);
  assert.equal(instance.restricted('/api/frontend/user/me', 'GET'), false);
  instance.dispose();
});

test('routes controller requests through the native bridge and never fetches localhost', async () => {
  const { instance, calls } = controller();
  await instance.ready;
  await instance.request('/assistant/state', {});
  // /asset-base 在启动时取一次本机取图前缀，其余请求照旧全部走原生桥。
  assert.deepEqual(calls.map(([route]) => route), ['/status', '/asset-base', '/assistant/state']);
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
