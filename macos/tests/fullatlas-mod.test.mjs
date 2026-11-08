import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { test } from 'node:test';

const reference = 'custom-assets/mod-example/我的 贴图.png';
const localPath = '/Users/example/Library/Application Support/app/custom-atlas/mod-example/assets/我的 贴图.png';
const account = {
  character_skin: { 1: { url: 'official/body.png' } },
  character_skin_atlas: { 2: { height_light_id: 1, url: 'official/icon.png' } },
  accessory: {}, accessory_atlas: {}, emote: {}, emote_atlas: {},
};
const officialAtlas = [{ asset_id: 2, asset_type: 'character_skin_atlas', asset_name: 'Official' }];

for (const platform of ['macos', 'windows']) {
  const file = platform === 'macos' ? '../fullatlas-source/local_atlas_controller.js' : '../../windows/src/src/local_atlas_controller.js';
  const source = await readFile(new URL(file, import.meta.url), 'utf8');
  const factory = vm.runInNewContext(source + '\ncreateLocalAtlasController', { URL, AbortController });

  function harness() {
    const backend = { enabled: false, installed: false, reject: false };
    const values = new Map(), windows = [];
    function data() {
      const catalog = structuredClone(account), atlas = structuredClone(officialAtlas);
      if (backend.installed) {
        for (const [kind, id] of [['character_skin', 1000000001], ['accessory', 1000000003], ['emote', 1000000005]]) {
          catalog[kind][id] = { url: reference, game_role_id: 0, ...(kind === 'character_skin' ? { body1: { url: reference }, head1: { url: reference } } : {}) };
          catalog[kind + '_atlas'][id + 1] = { height_light_id: id, url: reference };
          atlas.push({ asset_id: id + 1, asset_type: kind + '_atlas', asset_name: 'My Mod ' + kind });
        }
      }
      return { catalog, atlas, atlas_count: atlas.length, customAssetPaths: backend.installed ? { [reference]: localPath } : {} };
    }
    function open(options = {}) {
      const listeners = new Map();
      let applied = structuredClone(account);
      const marks = new Set();
      const win = {
        location: { href: 'tauri://localhost/', origin: 'tauri://localhost' },
        document: {
          documentElement: {
            setAttribute: name => marks.add(name), removeAttribute: name => marks.delete(name),
            hasAttribute: name => marks.has(name),
          },
          getElementById: () => null,
          createElement: () => ({ set textContent(value) {}, append() {} }),
          head: { append() {} },
        },
        fetch: async (url, options) => ({ ok: true, json: () => request(new URL(url).pathname, options?.body ? JSON.parse(options.body) : undefined) }), navigator: {},
        __TAURI_INTERNALS__: { convertFileSrc: path => 'asset://localhost/' + encodeURIComponent(path) },
        addEventListener: (name, callback) => listeners.set(name, callback),
        dispatchEvent() {}, CustomEvent: class {}, setInterval: () => 1, clearInterval() {},
        setTimeout: () => 1,
      };
      async function request(route, body) {
        if (route === '/status') return { ready: true, enabled: backend.enabled, mode_verified: true, network_policy: 'rooms-only' };
        if (route === '/mode') { backend.enabled = body.enabled; return { enabled: body.enabled, mode_verified: true, network_policy: 'rooms-only' }; }
        if (route === '/offline-data') return data();
        if (route === '/custom/import') { if (backend.reject) throw new Error('Invalid ZIP'); backend.installed = true; return { id: 'mod-example', name: 'Example', item_count: 3 }; }
        if (route === '/custom/remove') { backend.installed = false; return { removed: body.id }; }
        throw new Error('Unexpected route ' + route);
      }
      const controller = factory({
        window: win, ref: value => ({ value }), origin: 'http://127.0.0.1:43229',
        token: 'test',
        storage: {
          getItem: key => values.get(key) ?? null,
          setItem(key, value) {
            if (values.get(key) === value) return;
            values.set(key, value);
            for (const other of windows) if (other !== listeners) other.get('storage')?.({ key });
          },
        },
        request,
        customAssetUrl: options.sharedDefault ? undefined : (platform === 'macos'
          ? (reference, file) => 'asset://localhost/' + encodeURIComponent(file)
          : reference => 'http://127.0.0.1:43229/' + reference.split('/').map(encodeURIComponent).join('/')),
        normalizeAtlasList: value => value,
        resolvePetAssetIdFromAppearanceId: id => Object.values(applied).find(group => group[id]?.height_light_id)?.[id].height_light_id ?? id,
        getAppearanceAssetById: id => ['character_skin', 'accessory', 'emote'].map(kind => applied[kind][id]).find(Boolean),
        snapshotCatalog: () => structuredClone(applied),
        applyOfflineCatalog: catalog => { applied = catalog; }, restoreCatalog: catalog => { applied = catalog; },
        onError: error => { throw error; },
      });
      windows.push(listeners);
      return { controller, win, catalog: () => applied };
    }
    return { open, backend, values };
  }

  test(platform + ': official / full atlas + Mod, image mapping and restart', async () => {
    const app = harness(), first = app.open();
    await first.controller.ready;
    const officialUser = { character: { id: 1 }, accessory: null, emote: [] }, bag = [{ asset_id: 1 }];
    assert.equal(first.controller.inventory(officialAtlas, bag), bag);
    await first.controller.importCustomResources('/tmp/mod.zip');
    assert.equal(first.controller.enabled.value, false, 'import preserves official mode');
    await first.controller.setEnabled(true);
    assert.equal(first.controller.inventory([], bag).length, 4);
    const expectedURL = platform === 'macos' ? 'asset://localhost/' + encodeURIComponent(localPath) : 'http://127.0.0.1:43229/' + reference.split('/').map(encodeURIComponent).join('/');
    assert.equal(first.catalog().character_skin[1000000001].url, expectedURL);
    assert.equal(first.win.document.documentElement.hasAttribute('data-wuwa-mod'), true, '本地模式下才挂 Mod 标记');
    assert.equal(first.catalog().character_skin[1000000001].body1.url, expectedURL);
    assert.equal(first.catalog().character_skin[1].url, account.character_skin[1].url);
    assert.equal(first.controller.offlineData.value.catalog.character_skin[1000000001].url, reference, 'runtime conversion leaves raw data intact');
    first.controller.select({ asset_type: 'character_skin', asset_id: 1000000001, asset_name: 'Hero' }, officialUser, 8);
    first.controller.select({ asset_type: 'accessory', asset_id: 1000000003, asset_name: 'Ears' }, officialUser, 8);
    first.controller.select({ asset_type: 'emote', asset_id: 1000000005, asset_name: 'Happy' }, officialUser, 8);
    const restarted = app.open();
    await restarted.controller.ready;
    assert.equal(restarted.controller.viewUser(officialUser).character.id, 1000000001);
    await first.controller.removeCustomResource('mod-example');
    await new Promise(resolve => setImmediate(resolve));
    for (const current of [first, restarted]) {
      assert.equal(current.controller.enabled.value, true, 'removing final Mod keeps full atlas enabled');
      assert.equal(current.controller.inventory([], bag).length, 1);
      const user = current.controller.viewUser(officialUser);
      assert.equal(user.character.id, 1);
      assert.equal(user.accessory, null);
      assert.equal(user.emote.length, 0);
    }
    await first.controller.setEnabled(false);
    assert.equal(first.win.document.documentElement.hasAttribute('data-wuwa-mod'), false, '官方模式不能留 Mod 标记');
    assert.equal(first.controller.viewUser(officialUser), officialUser);
    assert.equal(first.controller.inventory([], bag), bag);
    assert.equal(JSON.stringify(first.catalog()), JSON.stringify(account));
    first.controller.dispose(); restarted.controller.dispose();
  });

  test(platform + ': full atlas works without Mods and failed import preserves state', async () => {
    const app = harness(), { controller } = app.open();
    await controller.ready;
    await controller.setEnabled(true);
    const before = JSON.stringify(controller.offlineData.value);
    app.backend.reject = true;
    await assert.rejects(controller.importCustomResources('/tmp/bad.zip'), /Invalid ZIP/);
    assert.equal(controller.enabled.value, true);
    assert.equal(JSON.stringify(controller.offlineData.value), before);
    assert.equal(controller.inventory([], []).length, officialAtlas.length);
    controller.dispose();
  });

  test(platform + ': 注入器没有提供 customAssetUrl 时不拼出必然 404 的地址', async () => {
    const app = harness(), opened = app.open({ sharedDefault: true });
    await opened.controller.ready;
    app.backend.installed = true;
    await opened.controller.setEnabled(true);
    assert.equal(opened.controller.assetUrl(reference), '', '缺省实现只能放弃这张图，不能猜地址');
    assert.equal(opened.catalog().character_skin[1000000001].url, '', 'catalog 同样不能带坏地址');
    opened.controller.dispose();
  });
}
