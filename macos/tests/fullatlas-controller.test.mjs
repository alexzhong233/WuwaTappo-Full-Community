import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { test } from 'node:test';

const source = await readFile(new URL('../fullatlas-source/local_atlas_controller.js', import.meta.url), 'utf8');
const factory = vm.runInNewContext(`${source}\ncreateLocalAtlasController`, { URL, AbortController, console });

function controller() {
  const calls = [];
  const storage = new Map();
  const win = {
    fetch: async () => { throw new Error('unexpected fetch'); },
    addEventListener() {}, dispatchEvent() {}, setInterval: () => 1, clearInterval() {},
    setTimeout: callback => { callback(); return 1; }, clearTimeout() {},
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
    location: { href: 'tauri://localhost/' }, navigator: {},
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
