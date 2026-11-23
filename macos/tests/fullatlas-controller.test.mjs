import assert from 'node:assert/strict';
import vm from 'node:vm';
import { test } from 'node:test';
import { renderPlatformModule } from '../../scripts/sync-frontend.mjs';

const source = await renderPlatformModule('local_atlas_controller.js', 'mac');
const factory = vm.runInNewContext(`${source}\ncreateLocalAtlasController`, { URL, AbortController, console });

function controller({ document, appearance, synthesisSame, refreshBag, offline } = {}) {
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
      if (route === '/mode') return { enabled: body.enabled, mode_verified: true, network_policy: 'hybrid', ready: true };
      if (route === '/offline-data') return offline;
      return {};
    },
  });
  return { instance, calls, storage };
}

function fakeDocument() {
  class Node {
    constructor(tag = 'div') { this.tagName=String(tag).toUpperCase(); this.children=[]; this.dataset={}; this.className=''; this.textContent=''; this.parent=null; this.classList={contains:()=>false}; }
    append(...nodes){ for(const node of nodes){ node.parent=this; this.children.push(node); } }
    remove(){ if(!this.parent)return; this.parent.children=this.parent.children.filter(node=>node!==this); this.parent=null; }
    setAttribute(name,value){ this[name]=String(value); }
    removeAttribute(name){ delete this[name]; }
    querySelector(selector){ if(selector==='.row')return this.row || null; if(selector==='p')return this.paragraph || null; return null; }
    set innerHTML(value){ this._innerHTML=String(value); if(value.includes('class="row"')){ this.row=new Node('div'); this.row.parent=this; } if(value.includes('<p>')){ this.paragraph=new Node('p'); this.paragraph.parent=this; } }
    get innerHTML(){ return this._innerHTML || ''; }
  }
  const body=new Node('body'), head=new Node('head'), root=new Node('html');
  const doc={body,head,documentElement:root,getElementById:()=>null,createElement:tag=>new Node(tag),querySelectorAll:()=>[],querySelector:()=>null};
  return {doc,body};
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

test('hybrid marks, merged atlas, cloud preset decision, official synthesis and capture stay shared', async () => {
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
  const bag = [{ asset_id: 1, asset_type: 'character_skin', quantity: 3 }];
  assert.equal(instance.inventory(atlas, bag), bag, 'the backpack stays the official account inventory');
  assert.equal(instance.officialOwned(1, 'character_skin'), true);
  assert.equal(instance.officialOwned(2, 'accessory'), false);
  assert.deepEqual({...instance.sourceFor(1, 'character_skin')}, {local:false, official:false});
  instance.setMark(1, 'character_skin', 'official', true);
  instance.select({ asset_id: 1, asset_type: 'character_skin', asset_name: 'Official' }, null, 8, { quiet: true });
  assert.deepEqual({...instance.sourceFor(1, 'character_skin')}, {local:true, official:true});
  assert.equal(instance.shouldCreateLocalPreset(null), false);
  instance.select({ asset_id: 2, asset_type: 'accessory', asset_name: 'Local ears' }, null, 8, { quiet: true });
  assert.deepEqual({...instance.sourceFor(2, 'accessory')}, {local:true, official:false});
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

test('local-only atlas entries are grey additions and local/official marks stay independent', async () => {
  const offline = {
    catalog: {
      character_skin: { 1: { url: 'assets/a.png' }, 3: { url: 'assets/c.png' } },
      character_skin_atlas: { 11: { height_light_id: 1, url: 'assets/a.png' }, 13: { height_light_id: 3, url: 'assets/c.png' } },
    },
    atlas: [
      { asset_id: 11, asset_type: 'character_skin_atlas', asset_name: 'Official' },
      { asset_id: 13, asset_type: 'character_skin_atlas', asset_name: 'Local only' },
    ],
    atlas_count: 2,
  };
  const { instance, storage } = controller({ offline });
  await instance.ready;
  await instance.setEnabled(true);
  const merged = instance.mergeAtlas([{ asset_id: 11, asset_type: 'character_skin_atlas', asset_name: 'Official' }]);
  assert.equal(merged.length, 2);
  assert.equal(merged[1].asset_id, 13);
  assert.equal(merged[1].owned, false);
  assert.equal(merged[1].wuwa_local_only, true);
  assert.equal(instance.canSelect(merged[1]), true, 'grey local-only entries remain selectable');
  instance.setMark(13, 'character_skin_atlas', 'local', true);
  assert.deepEqual({...instance.sourceFor(13, 'character_skin_atlas')}, {local:true, official:false});
  assert.deepEqual({...instance.sourceFor(3, 'character_skin')}, {local:true, official:false}, 'primary and atlas ids share the same marks');
  instance.setMark(13, 'character_skin_atlas', 'official', true);
  assert.deepEqual({...instance.sourceFor(13, 'character_skin_atlas')}, {local:true, official:true});
  const persisted = [...storage.values()].map(value => { try { return JSON.parse(value); } catch { return null; } })
    .find(value => value && value['character_skin:13']);
  assert.deepEqual(persisted['character_skin:13'], {local:true, official:true});
  instance.dispose();
});

test('batch same-level synthesis also runs from the official account inventory', async () => {
  const { instance, calls } = controller();
  await instance.ready;
  assert.equal(instance.enabled.value, false);
  instance.inventory([], [{ asset_id: 7, asset_type: 'accessory', quantity: 3 }]);
  const result = await instance.synthesizeDuplicates('same-level');
  assert.equal(result.count, 3);
  assert.deepEqual(Array.from(calls.find(([route]) => route === 'synthesis')[1]), [7, 7, 7]);
  assert.ok(calls.some(([route]) => route === 'refresh-bag'));
  instance.dispose();
});

test('click choices add independent local and official tags and local-only items cannot be official', async () => {
  const appearance = id => id === 2
    ? { asset_id: 2, asset_type: 'accessory', game_role_id: 0 }
    : { asset_id: id, asset_type: 'character_skin', game_role_id: 0 };
  const { doc, body } = fakeDocument();
  const { instance } = controller({ document:doc, appearance });
  await instance.ready;
  instance.enabled.value = true;
  instance.inventory([], [{ asset_id: 1, asset_type: 'character_skin', quantity: 1 }]);
  const item = { asset_id: 1, asset_type: 'character_skin', asset_name: 'Official' };
  let officialCalls = 0;
  instance.beginSelect(item, null, 8, async()=>{ officialCalls++; });
  let choice = body.children.at(-1);
  assert.equal(choice.row.children.length, 3, 'official item exposes local, official and close');
  choice.row.children[0].onclick();
  assert.deepEqual({...instance.sourceFor(1, 'character_skin')}, {local:true, official:false});
  instance.beginSelect(item, null, 8, async()=>{ officialCalls++; });
  choice = body.children.at(-1);
  await choice.row.children[1].onclick();
  assert.equal(officialCalls, 1);
  assert.deepEqual({...instance.sourceFor(1, 'character_skin')}, {local:true, official:true});
  const localOnly = { asset_id: 2, asset_type: 'accessory', asset_name: 'Local ears' };
  instance.beginSelect(localOnly, null, 8, async()=>{ throw new Error('must not run'); });
  choice = body.children.at(-1);
  assert.equal(choice.row.children.length, 2, 'local-only item exposes only local and close');
  choice.row.children[0].onclick();
  assert.deepEqual({...instance.sourceFor(2, 'accessory')}, {local:true, official:false});
  const emitted = [];
  instance.enabled.value = false;
  assert.equal(instance.beginPanelSelect(item, payload => emitted.push(payload)), true);
  choice = body.children.at(-1);
  choice.row.children[0].onclick();
  assert.equal(emitted.at(-1).wuwa_choice, 'local');
  assert.equal(instance.beginPanelSelect(item, payload => emitted.push(payload)), true);
  choice = body.children.at(-1);
  await choice.row.children[1].onclick();
  assert.equal(emitted.at(-1).wuwa_choice, 'official');
  instance.dispose();
});

test('non-panel views skip source badge DOM observation and quota failures throw clearly', async () => {
  let observed = false;
  const head = { append() {}, setAttribute() {}, removeAttribute() {} };
  const doc = {
    getElementById: () => null,
    createElement: () => ({ id: '', textContent: '' }),
    head, documentElement: head,
    querySelectorAll: () => { observed = true; return []; },
  };
  class FakeObserver { observe() { observed = true; } disconnect() {} }
  const calls = [];
  const win = {
    fetch: async () => { throw new Error('unexpected fetch'); },
    addEventListener() {}, dispatchEvent() {}, setInterval: () => 1, clearInterval() {},
    setTimeout: callback => { callback(); return 1; }, clearTimeout() {},
    MutationObserver: FakeObserver,
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
    location: { href: 'tauri://localhost/', hash: '#/' }, navigator: {}, document: doc,
  };
  const instance = factory({
    ref: value => ({ value }), window: win,
    storage: {
      getItem: () => null,
      setItem: key => { if (key === 'wuwatappo-capture-v1') throw new Error('QuotaExceededError'); },
    },
    origin: 'https://mc-wuwatappo.kurogames-global.com', atlasCount: 1, catalogConsumer: false,
    normalizeAtlasList: value => value, resolvePetAssetIdFromAppearanceId: id => id,
    snapshotCatalog: () => ({}), applyOfflineCatalog: () => {}, restoreCatalog: () => {},
    getAppearanceAssetById: id => ({ asset_id: id, asset_type: 'character_skin' }),
    request: async route => { calls.push(route); return route === '/status' ? { ready: true, enabled: true, mode_verified: true, network_policy: 'hybrid' } : {}; },
  });
  await instance.ready;
  assert.equal(observed, false, 'main pet route must not run source badge DOM observer');
  assert.throws(() => instance.setCapture({ panelBackground: 'data:image/png;base64,abc' }), /2 MB 以内/);
  instance.dispose();
});

test('applyLocalChoice enables overlay on demand for unowned Atlas items and marks both IDs', async () => {
  const offline = {
    catalog: {
      character_skin: { 3: { url: 'assets/c.png' } },
      character_skin_atlas: { 13: { height_light_id: 3, url: 'assets/c.png' } },
    },
    atlas: [{ asset_id: 13, asset_type: 'character_skin_atlas', asset_name: 'Local only' }],
    atlas_count: 1,
  };
  const { instance, storage } = controller({ offline });
  await instance.ready;
  assert.equal(instance.enabled.value, false);
  await instance.applyLocalChoice({ asset_id: 13, asset_type: 'character_skin', asset_name: 'Local only', owned: false }, null, 8);
  assert.equal(instance.enabled.value, true);
  assert.equal(instance.selection.value.character.id, 3);
  assert.deepEqual({ ...instance.sourceFor(13, 'character_skin') }, { local: true, official: false });
  assert.deepEqual({ ...instance.sourceFor(3, 'character_skin') }, { local: true, official: false });
  const savedMarks = JSON.parse(storage.get('wuwatappo-appearance-marks'));
  assert.deepEqual(savedMarks['character_skin:13'], { local: true });
  assert.deepEqual(savedMarks['character_skin:3'], { local: true });
  instance.dispose();
});

test('panel cards render local and official badges via data attributes even without storage events', async () => {
  const sharedStorage = new Map();
  const makeCard = (id, type) => {
    const attrs = new Map([['data-wuwa-asset-id', String(id)], ['data-wuwa-asset-type', type]]);
    const el = {
      tagName: 'DIV', children: [], dataset: {}, className: 'atlas-item', textContent: '', parent: null,
      classList: { contains: cls => el.className.split(/\s+/).includes(cls) },
      append(...nodes) { for (const n of nodes) { n.parent = el; el.children.push(n); } },
      remove() { if (el.parent) { el.parent.children = el.parent.children.filter(x => x !== el); el.parent = null; } },
      setAttribute(k, v) { attrs.set(k, String(v)); },
      getAttribute(k) { return attrs.get(k) ?? null; },
      removeAttribute(k) { attrs.delete(k); },
    };
    return el;
  };
  const bagCard = makeCard(3, 'character_skin');
  const atlasCard = makeCard(13, 'character_skin_atlas');
  const cards = [bagCard, atlasCard];
  const { doc, body } = fakeDocument();
  const panelDoc = {
    body, head: doc.head, documentElement: doc.documentElement,
    getElementById: () => null,
    createElement: tag => {
      const n = doc.createElement(tag);
      let rawText = '';
      Object.defineProperty(n, 'textContent', {
        get() { return rawText; },
        set(v) { rawText = String(v); if (rawText === '') n.children = []; },
      });
      n.classList = { contains: cls => String(n.className).split(/\s+/).includes(cls) };
      return n;
    },
    querySelector: sel => sel.includes('panel') ? body : null,
    querySelectorAll: sel => sel === '.atlas-item[data-wuwa-asset-id]' ? cards : [],
    addEventListener() {}, removeEventListener() {},
  };
  const timers = [];
  class FakeObserver { observe() {} disconnect() {} }
  const offline = {
    catalog: {
      character_skin: { 3: { url: 'assets/c.png' } },
      character_skin_atlas: { 13: { height_light_id: 3, url: 'assets/c.png' } },
    },
    atlas: [{ asset_id: 13, asset_type: 'character_skin_atlas', asset_name: 'Local only' }],
    atlas_count: 1,
  };
  const makeWin = (hash, document) => ({
    fetch: async () => { throw new Error('unexpected fetch'); },
    addEventListener() {}, removeEventListener() {}, dispatchEvent() {},
    setInterval: () => 1, clearInterval() {},
    setTimeout: cb => { timers.push(cb); return timers.length; }, clearTimeout() {},
    MutationObserver: FakeObserver,
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
    location: { href: `tauri://localhost/${hash}`, hash }, navigator: {}, ...(document ? { document } : {}),
  });
  const mainCtrl = factory({
    ref: value => ({ value }), window: makeWin('#/', null),
    storage: { getItem: k => sharedStorage.get(k) ?? null, setItem: (k, v) => sharedStorage.set(k, v) },
    origin: 'https://mc-wuwatappo.kurogames-global.com', atlasCount: 1, catalogConsumer: false,
    normalizeAtlasList: v => v,
    resolvePetAssetIdFromAppearanceId: id => (id === 13 ? 3 : id),
    snapshotCatalog: () => ({}), applyOfflineCatalog: () => {}, restoreCatalog: () => {},
    getAppearanceAssetById: id => ({ asset_id: id, asset_type: 'character_skin' }),
    request: async route => route === '/status' ? { ready: true, enabled: true, mode_verified: true, network_policy: 'hybrid' } : route === '/mode' ? { enabled: true, mode_verified: true, network_policy: 'hybrid', ready: true } : route === '/offline-data' ? offline : {},
  });
  const panelCtrl = factory({
    ref: value => ({ value }), window: makeWin('#/panel', panelDoc),
    storage: { getItem: k => sharedStorage.get(k) ?? null, setItem: (k, v) => sharedStorage.set(k, v) },
    origin: 'https://mc-wuwatappo.kurogames-global.com', atlasCount: 1, catalogConsumer: false,
    normalizeAtlasList: v => v,
    resolvePetAssetIdFromAppearanceId: id => (id === 13 ? 3 : id),
    snapshotCatalog: () => ({}), applyOfflineCatalog: () => {}, restoreCatalog: () => {},
    getAppearanceAssetById: id => ({ asset_id: id, asset_type: 'character_skin' }),
    request: async route => route === '/status' ? { ready: true, enabled: false, mode_verified: true, network_policy: 'hybrid' } : route === '/offline-data' ? offline : {},
  });
  const flushTimers = async () => {
    for (let i = 0; i < 6 && timers.length; i++) {
      const batch = timers.splice(0, timers.length);
      for (const cb of batch) cb();
      await Promise.resolve();
    }
  };
  await Promise.all([mainCtrl.ready, panelCtrl.ready]);
  const emitted = [];
  assert.equal(panelCtrl.beginPanelSelect({ asset_id: 3, asset_type: 'character_skin', asset_name: 'Local only' }, payload => emitted.push(payload), true), true);
  let choice = body.children.at(-1);
  assert.equal(choice.row.children.length, 3, 'bag cards force official button alongside local and close');
  await choice.row.children[1].onclick();
  assert.equal(emitted.at(-1).wuwa_choice, 'official');
  // Main window applies official mark without firing any cross-window storage event.
  mainCtrl.setMark(3, 'character_skin', 'official', true);
  await flushTimers();
  for (const card of cards) {
    const tags = card.children.find(c => c.className === 'wuwa-source-tags');
    assert.ok(tags, 'card must show wuwa-source-tags after official mark');
    assert.deepEqual(tags.children.map(t => t.textContent), ['官方']);
  }
  // Unowned Atlas card only shows local + close, then after main window runs applyLocalChoice both tags render.
  assert.equal(panelCtrl.beginPanelSelect({ asset_id: 13, asset_type: 'character_skin', owned: false, asset_name: 'Local only' }, payload => emitted.push(payload), false), true);
  choice = body.children.at(-1);
  assert.equal(choice.row.children.length, 2, 'unowned atlas cards expose local and close');
  choice.row.children[0].onclick();
  assert.equal(emitted.at(-1).wuwa_choice, 'local');
  await mainCtrl.applyLocalChoice(emitted.at(-1), null, 8);
  await flushTimers();
  for (const card of cards) {
    const tags = card.children.find(c => c.className === 'wuwa-source-tags');
    assert.deepEqual(tags.children.map(t => t.textContent), ['本地', '官方']);
  }
  mainCtrl.dispose();
  panelCtrl.dispose();
});

test('marks stay exclusive per slot, unequipping clears badges, and legacy marks purge on sync', async () => {
  const storage = new Map();
  storage.set('wuwatappo-local-gallery-v1', JSON.stringify({
    enabled: true, desired: true,
    selection: { character: { id: 1, name: 'Char1' }, accessory: null, emote: [] },
    favorites: {}, marks: {},
  }));
  storage.set('wuwatappo-appearance-marks', JSON.stringify({
    'character_skin:1': { local: true, official: true },
    'character_skin:99': { local: true, official: true },
    'accessory:88': { local: true, official: true },
    'emote:77': { local: true, official: true },
  }));
  const offline = {
    catalog: {
      character_skin: { 1: { url: 'a/1.png', game_role_id: 1 }, 3: { url: 'a/3.png', game_role_id: 1 } },
      character_skin_atlas: { 11: { height_light_id: 1, url: 'a/1.png' }, 13: { height_light_id: 3, url: 'a/3.png' } },
      accessory: { 5: { url: 'a/5.png', game_role_id: 0 } },
      accessory_atlas: { 15: { height_light_id: 5, url: 'a/5.png' } },
      emote: { 6: { url: 'a/6.png', game_role_id: 0 } },
      emote_atlas: { 16: { height_light_id: 6, url: 'a/6.png' } },
    },
    atlas: [
      { asset_id: 11, asset_type: 'character_skin_atlas', asset_name: 'Char1' },
      { asset_id: 13, asset_type: 'character_skin_atlas', asset_name: 'Char3' },
      { asset_id: 15, asset_type: 'accessory_atlas', asset_name: 'Acc5' },
      { asset_id: 16, asset_type: 'emote_atlas', asset_name: 'Emote6' },
    ],
    atlas_count: 4,
  };
  const win = {
    fetch: async () => { throw new Error('unexpected fetch'); },
    addEventListener() {}, dispatchEvent() {}, setInterval: () => 1, clearInterval() {},
    setTimeout: cb => { cb(); return 1; }, clearTimeout() {},
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
    location: { href: 'tauri://localhost/' }, navigator: {},
  };
  const instance = factory({
    ref: value => ({ value }), window: win,
    storage: { getItem: k => storage.get(k) ?? null, setItem: (k, v) => storage.set(k, v) },
    origin: 'https://mc-wuwatappo.kurogames-global.com', atlasCount: 4, catalogConsumer: false,
    normalizeAtlasList: v => v,
    resolvePetAssetIdFromAppearanceId: id => ({ 11: 1, 13: 3, 15: 5, 16: 6 }[id] || id),
    snapshotCatalog: () => ({}), applyOfflineCatalog: () => {}, restoreCatalog: () => {},
    getAppearanceAssetById: id => {
      for (const kind of ['character_skin', 'accessory', 'emote']) {
        if (offline.catalog[kind][id]) return { asset_id: id, asset_type: kind, ...offline.catalog[kind][id] };
      }
      return null;
    },
    request: async route => route === '/status'
      ? { ready: true, enabled: true, mode_verified: true, network_policy: 'hybrid' }
      : route === '/mode'
        ? { enabled: true, mode_verified: true, network_policy: 'hybrid', ready: true }
        : route === '/offline-data' ? offline : {},
  });
  // Legacy stale local marks are purged at controller startup.
  assert.deepEqual({ ...instance.sourceFor(99, 'character_skin') }, { local: false, official: true });
  assert.deepEqual({ ...instance.sourceFor(88, 'accessory') }, { local: false, official: true });
  assert.deepEqual({ ...instance.sourceFor(1, 'character_skin') }, { local: true, official: true });
  await instance.ready;
  await instance.setEnabled(true);
  // Official sync purges stale official marks without disturbing local selection.
  instance.syncOfficialMarks({ character: { id: 1 }, accessory: null, emote: [] });
  assert.deepEqual({ ...instance.sourceFor(99, 'character_skin') }, { local: false, official: false });
  assert.deepEqual({ ...instance.sourceFor(88, 'accessory') }, { local: false, official: false });
  assert.deepEqual({ ...instance.sourceFor(77, 'emote') }, { local: false, official: false });
  assert.deepEqual({ ...instance.sourceFor(1, 'character_skin') }, { local: true, official: true });
  // Switching official character replaces prior official mark only.
  instance.setMark(3, 'character_skin', 'official', true);
  assert.deepEqual({ ...instance.sourceFor(1, 'character_skin') }, { local: true, official: false });
  assert.deepEqual({ ...instance.sourceFor(3, 'character_skin') }, { local: false, official: true });
  // Switching local character replaces prior local mark only.
  instance.select({ asset_id: 13, asset_type: 'character_skin', asset_name: 'Char3' }, null, 8);
  assert.deepEqual({ ...instance.sourceFor(1, 'character_skin') }, { local: false, official: false });
  assert.deepEqual({ ...instance.sourceFor(3, 'character_skin') }, { local: true, official: true });
  // Equipping then unequipping accessory & emote removes their local badges cleanly.
  instance.select({ asset_id: 15, asset_type: 'accessory', asset_name: 'Acc5' }, null, 8);
  assert.deepEqual({ ...instance.sourceFor(5, 'accessory') }, { local: true, official: false });
  assert.deepEqual({ ...instance.sourceFor(15, 'accessory') }, { local: true, official: false });
  instance.select({ asset_id: 15, asset_type: 'accessory', asset_name: 'Acc5' }, null, 8);
  assert.deepEqual({ ...instance.sourceFor(5, 'accessory') }, { local: false, official: false });
  assert.deepEqual({ ...instance.sourceFor(15, 'accessory') }, { local: false, official: false });
  instance.select({ asset_id: 16, asset_type: 'emote', asset_name: 'Emote6' }, null, 8);
  assert.deepEqual({ ...instance.sourceFor(6, 'emote') }, { local: true, official: false });
  instance.select({ asset_id: 16, asset_type: 'emote', asset_name: 'Emote6' }, null, 8);
  assert.deepEqual({ ...instance.sourceFor(6, 'emote') }, { local: false, official: false });
  instance.dispose();
});

test('accessory transform works for official mode and syncs across windows via storage and events', async () => {
  const sharedStorage = new Map();
  const bus = new Map();
  const storageHandlers = [];
  const officialUser = { character: { id: 1 }, accessory: { id: 5, name: 'Halo' }, emote: [] };
  const makeCtrl = hash => {
    const listeners = new Map();
    const win = {
      fetch: async () => { throw new Error('unexpected fetch'); },
      addEventListener: (name, cb) => { if (name === 'storage') storageHandlers.push(cb); },
      removeEventListener() {}, dispatchEvent() {}, setInterval: () => 1, clearInterval() {},
      setTimeout: () => 1, clearTimeout() {},
      CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
      location: { href: `tauri://localhost/${hash}`, hash }, navigator: {},
    };
    return factory({
      ref: value => ({ value }), window: win,
      storage: {
        getItem: k => sharedStorage.get(k) ?? null,
        setItem: (k, v) => {
          sharedStorage.set(k, v);
          for (const cb of storageHandlers) cb({ key: k });
        },
      },
      origin: 'https://mc-wuwatappo.kurogames-global.com', atlasCount: 1, catalogConsumer: false,
      getOfficialUser: () => officialUser,
      emit: (name, payload) => {
        for (const cb of bus.get(name) || []) cb({ payload });
      },
      listen: (name, cb) => {
        if (!bus.has(name)) bus.set(name, new Set());
        bus.get(name).add(cb);
        listeners.set(name, cb);
        return () => bus.get(name)?.delete(cb);
      },
      normalizeAtlasList: v => v, resolvePetAssetIdFromAppearanceId: id => id,
      snapshotCatalog: () => ({}), applyOfflineCatalog: () => {}, restoreCatalog: () => {},
      getAppearanceAssetById: id => ({ asset_id: id, asset_type: id === 5 ? 'accessory' : 'character_skin', asset_name: id === 5 ? 'Halo' : 'Char' }),
      request: async route => route === '/status' ? { ready: true, enabled: false, mode_verified: true, network_policy: 'hybrid' } : {},
    });
  };
  const mainCtrl = makeCtrl('#/');
  const panelCtrl = makeCtrl('#/panel');
  await Promise.all([mainCtrl.ready, panelCtrl.ready]);
  assert.equal(panelCtrl.enabled.value, false);
  assert.equal(panelCtrl.currentAccessoryInfo().id, 5);
  panelCtrl.setAccessoryTransform({ dx: 24, dy: -16, scale: 1.5, rotate: 30 });
  assert.deepEqual({ ...mainCtrl.accessoryTransform(officialUser) }, { dx: 24, dy: -16, scale: 1.5, rotate: 30 });
  panelCtrl.resetAccessoryTransform();
  assert.equal(mainCtrl.accessoryTransform(officialUser), null);
  mainCtrl.dispose();
  panelCtrl.dispose();
});

test('mod catalog overlay augments runtime catalog without mutating raw cache and prunes removed mods', async () => {
  const storage = new Map();
  let modInstalled = true;
  const officialCatalog = { character_skin: { 1: { url: 'a/1.png', game_role_id: 1 } }, character_skin_atlas: { 11: { height_light_id: 1, url: 'a/1.png' } } };
  const modCatalog = {
    character_skin: { 1: { url: 'a/1.png', game_role_id: 1 }, 1000001: { url: 'custom-assets/pack1/hero.png', game_role_id: 0 } },
    character_skin_atlas: { 11: { height_light_id: 1, url: 'a/1.png' }, 1000002: { height_light_id: 1000001, url: 'custom-assets/pack1/icon.png' } },
  };
  const makeOffline = () => modInstalled
    ? {
        catalog: structuredClone(modCatalog),
        atlas: [
          { asset_id: 11, asset_type: 'character_skin_atlas', asset_name: 'Official' },
          { asset_id: 1000002, asset_type: 'character_skin_atlas', asset_name: 'HeroMod', rarity: 'MOD', custom_pack_id: 'pack1' },
        ],
        atlas_count: 2,
        customAssetBase: 'http://127.0.0.1:43229/',
        customAssetPaths: {
          'custom-assets/pack1/hero.png': 'C:/mods/pack1/assets/hero.png',
          'custom-assets/pack1/icon.png': 'C:/mods/pack1/assets/icon.png',
        },
      }
    : {
        catalog: structuredClone(officialCatalog),
        atlas: [{ asset_id: 11, asset_type: 'character_skin_atlas', asset_name: 'Official' }],
        atlas_count: 1,
      };
  let activeOverlay = null, clearCalls = 0;
  const win = {
    fetch: async () => { throw new Error('unexpected fetch'); },
    addEventListener() {}, removeEventListener() {}, dispatchEvent() {},
    setInterval: () => 1, clearInterval() {}, setTimeout: () => 1, clearTimeout() {},
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init.detail; } },
    location: { href: 'tauri://localhost/' }, navigator: {},
  };
  const instance = factory({
    ref: value => ({ value }), window: win,
    storage: { getItem: k => storage.get(k) ?? null, setItem: (k, v) => storage.set(k, v) },
    origin: 'https://mc-wuwatappo.kurogames-global.com', atlasCount: 1, catalogConsumer: true,
    normalizeAtlasList: v => v,
    resolvePetAssetIdFromAppearanceId: id => id === 1000002 ? 1000001 : id === 11 ? 1 : id,
    augmentCatalog: payload => { activeOverlay = payload; },
    clearCatalogOverlay: () => { clearCalls++; activeOverlay = null; },
    snapshotCatalog: () => ({}), applyOfflineCatalog: () => {}, restoreCatalog: () => {},
    getAppearanceAssetById: id => activeOverlay?.character_skin?.[id] ? { asset_id: id, asset_type: 'character_skin', ...activeOverlay.character_skin[id] } : officialCatalog.character_skin[id] ? { asset_id: id, asset_type: 'character_skin', ...officialCatalog.character_skin[id] } : null,
    request: async route => {
      if (route === '/status') return { ready: true, enabled: true, mode_verified: true, network_policy: 'hybrid' };
      if (route === '/mode') return { enabled: true, mode_verified: true, network_policy: 'hybrid', ready: true };
      if (route === '/offline-data') return makeOffline();
      if (route === '/custom/remove') { modInstalled = false; return { removed: true }; }
      return {};
    },
  });
  await instance.ready;
  await instance.setEnabled(true);
  assert.equal(activeOverlay.character_skin[1000001].url, 'http://127.0.0.1:43229/custom-assets/pack1/hero.png');
  assert.equal(instance.offlineData.value.catalog.character_skin[1000001].url, 'custom-assets/pack1/hero.png');
  instance.select({ asset_id: 1000002, asset_type: 'character_skin', asset_name: 'HeroMod' }, null, 8);
  assert.deepEqual({ ...instance.sourceFor(1000001, 'character_skin') }, { local: true, official: false });
  await instance.removeCustomResource('pack1');
  assert.equal(instance.selection.value.character, undefined);
  assert.deepEqual({ ...instance.sourceFor(1000001, 'character_skin') }, { local: false, official: false });
  await instance.setEnabled(false);
  assert.ok(clearCalls >= 1);
  assert.equal(activeOverlay, null);
  instance.dispose();
});

test('canApplyOfficial, collectionProgress, and shouldCreateLocalPreset isolate official writes from local/mod state', async () => {
  const { instance } = controller();
  await instance.ready;
  instance.enabled.value = true;
  instance.inventory([], [
    { asset_id: 1, asset_type: 'character_skin', quantity: 1 },
    { asset_id: 2, asset_type: 'character_skin', quantity: 1 },
  ]);

  assert.equal(instance.canApplyOfficial({ asset_id: 1, asset_type: 'character_skin', owned: true }), true);
  assert.equal(instance.canApplyOfficial({ asset_id: 3, asset_type: 'character_skin' }), false);
  assert.equal(instance.canApplyOfficial({ asset_id: 1, asset_type: 'character_skin', owned: false }), false);
  assert.equal(instance.canApplyOfficial({ asset_id: 1, asset_type: 'character_skin', unowned: true }), false);
  assert.equal(instance.canApplyOfficial({ asset_id: 1, asset_type: 'character_skin', wuwa_local_only: true }), false);
  assert.equal(instance.canApplyOfficial({ asset_id: 1, asset_type: 'character_skin', rarity: 'MOD' }), false);
  assert.equal(instance.canApplyOfficial({ asset_id: 1, asset_type: 'character_skin', custom_pack_id: 'pack1' }), false);
  assert.equal(instance.canApplyOfficial({ asset_id: 1000000001, asset_type: 'character_skin' }), false);

  const progress = instance.collectionProgress([
    { asset_id: 1, asset_type: 'character_skin', owned: true },
    { asset_id: 3, asset_type: 'character_skin', owned: false },
    { asset_id: 99, asset_type: 'character_skin', owned: false, wuwa_local_only: true },
    { asset_id: 1000000001, asset_type: 'character_skin_atlas', owned: false, rarity: 'MOD' },
  ]);
  assert.deepEqual({ collected: progress.collected, total: progress.total }, { collected: 1, total: 2 });

  const officialUser = { character: { id: 1 }, accessory: null, emote: [] };
  instance.select({ asset_id: 1, asset_type: 'character_skin', asset_name: 'One' }, officialUser, 8);
  assert.equal(instance.shouldCreateLocalPreset(officialUser), false);
  instance.select({ asset_id: 2, asset_type: 'character_skin', asset_name: 'Two' }, officialUser, 8);
  assert.equal(instance.shouldCreateLocalPreset(officialUser), true);
  instance.dispose();
});
