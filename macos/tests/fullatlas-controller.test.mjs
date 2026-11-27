import assert from 'node:assert/strict';
import vm from 'node:vm';
import { test } from 'node:test';
import { renderPlatformModule } from '../../scripts/sync-frontend.mjs';

const source = await renderPlatformModule('local_atlas_controller.js', 'mac');
const factory = vm.runInNewContext(`${source}\ncreateLocalAtlasController`, { URL, AbortController, console });

function controller({ document, appearance, synthesisSame, synthesisAtlas, refreshBag, fetchBag, onSynthesisComplete, offline, winProps, catalogConsumer = false, applyOfflineCatalog, restoreCatalog } = {}) {
  const calls = [];
  const mergedCatalogs = [];
  const storage = new Map();
  const listeners = new Map();
  const win = {
    fetch: async () => { throw new Error('unexpected fetch'); },
    addEventListener(type, handler) {
      const list = listeners.get(type) || [];
      if (typeof handler === 'function' && !list.includes(handler)) list.push(handler);
      listeners.set(type, list);
    },
    removeEventListener(type, handler) {
      listeners.set(type, (listeners.get(type) || []).filter(fn => fn !== handler));
    },
    dispatchEvent(event) {
      for (const handler of [...(listeners.get(event?.type) || [])]) handler(event);
      return true;
    },
    setInterval: () => 1, clearInterval() {},
    setTimeout: callback => { callback(); return 1; }, clearTimeout() {},
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init?.detail; } },
    location: { href: 'tauri://localhost/' }, navigator: {}, ...(document ? { document } : {}),
    ...(winProps || {}),
  };
  const instance = factory({
    ref: value => ({ value }), window: win,
    storage: { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) },
    origin: 'https://mc-wuwatappo.kurogames-global.com', atlasCount: 1, catalogConsumer,
    normalizeAtlasList: value => value,
    resolvePetAssetIdFromAppearanceId: id => id,
    snapshotCatalog: () => ({}), applyOfflineCatalog: applyOfflineCatalog || (() => {}), mergeModCatalog: payload => mergedCatalogs.push(payload), restoreCatalog: restoreCatalog || (() => {}),
    getAppearanceAssetById: appearance || (id => ({ asset_id: id, asset_type: 'character_skin' })),
    synthesisAtlasSame: synthesisSame || (async ids => { calls.push(['synthesis', ids]); return {}; }),
    synthesisAtlas: synthesisAtlas || (async ids => { calls.push(['synthesis-upgrade', ids]); return {}; }),
    refreshBag: refreshBag || (async () => { calls.push(['refresh-bag', {}]); }),
    ...(typeof fetchBag === 'function' ? { fetchBag } : {}),
    ...(typeof onSynthesisComplete === 'function' ? { onSynthesisComplete } : {}),
    request: async (route, body) => {
      calls.push([route, body]);
      if (route === '/status') return { ready: true, enabled: true, mode_verified: true, network_policy: 'hybrid' };
      if (route === '/mode') return { enabled: body.enabled, mode_verified: true, network_policy: 'hybrid', ready: true };
      if (route === '/offline-data') return offline;
      return {};
    },
  });
  // 本机 storage 事件在真实 WebView 里由另一个窗口触发；测试里直接重放同样的回调。
  const emitStorage = key => {
    for (const handler of [...(listeners.get('storage') || [])]) handler({ key });
  };
  return { instance, calls, storage, listeners, emitStorage, win, mergedCatalogs };
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

// 装备栏要跑真正的 Pointer Events 拖拽，需要 classList/closest/querySelectorAll/elementFromPoint
// 这些最小实现；只服务于 mountEquipmentBar 的用例，不影响上面轻量的 fakeDocument。
function matchesSimpleSelector(node, selector) {
  const tokens = String(selector || '').match(/[.#]?[A-Za-z0-9_-]+|\[[^\]]+\]/g) || [];
  if (!tokens.length) return false;
  return tokens.every(token => {
    if (token[0] === '.') return node.classList.contains(token.slice(1));
    if (token[0] === '#') return node.id === token.slice(1);
    if (token[0] === '[') {
      const body = token.slice(1, -1), eq = body.indexOf('=');
      const name = eq < 0 ? body : body.slice(0, eq);
      if (!node.hasAttribute(name)) return false;
      if (eq < 0) return true;
      return String(node.getAttribute(name)) === body.slice(eq + 1).replace(/^["']|["']$/g, '');
    }
    return node.tagName === token.toUpperCase();
  });
}
function matchesSelector(node, selector) {
  return String(selector || '').split(',').some(part => matchesSimpleSelector(node, part.trim()));
}
function richDocument() {
  class Node {
    constructor(tag = 'div') {
      this.tagName = String(tag).toUpperCase(); this.children = []; this.parent = null;
      this._attrs = {}; this._classes = new Set(); this.dataset = {}; this._text = '';
      this.style = {_props: {}, setProperty(name, value) { this._props[name] = String(value); }, getPropertyValue(name) { return this._props[name] || ''; }};
    }
    get id() { return this._attrs.id || ''; }
    set id(value) { this._attrs.id = String(value); }
    get className() { return [...this._classes].join(' '); }
    set className(value) { this._classes = new Set(String(value || '').split(/\s+/).filter(Boolean)); }
    get classList() {
      const self = this;
      return {
        add: (...names) => names.forEach(name => self._classes.add(name)),
        remove: (...names) => names.forEach(name => self._classes.delete(name)),
        contains: name => self._classes.has(name),
        toggle: (name, on) => { if ((on === undefined ? self._classes.has(name) : !on)) self._classes.delete(name); else self._classes.add(name); },
      };
    }
    get textContent() { return this._text; }
    set textContent(value) { this._text = String(value ?? ''); this.children.forEach(child => { child.parent = null; }); this.children = []; }
    get innerHTML() { return this._text; }
    set innerHTML(value) { this._text = String(value ?? ''); this.children.forEach(child => { child.parent = null; }); this.children = []; }
    append(...nodes) { for (const node of nodes) { if (!node) continue; node.parent = this; this.children.push(node); } return this; }
    remove() { if (!this.parent) return; this.parent.children = this.parent.children.filter(child => child !== this); this.parent = null; }
    setAttribute(name, value) { this._attrs[name] = String(value); if (name === 'class') this.className = value; }
    getAttribute(name) { return name in this._attrs ? this._attrs[name] : null; }
    hasAttribute(name) { return name in this._attrs; }
    removeAttribute(name) { delete this._attrs[name]; }
    contains(node) { let walk = node; while (walk) { if (walk === this) return true; walk = walk.parent; } return false; }
    closest(selector) { let walk = this; while (walk) { if (matchesSelector(walk, selector)) return walk; walk = walk.parent; } return null; }
    querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
    querySelectorAll(selector) { const out = []; const walk = node => { for (const child of node.children) { if (matchesSelector(child, selector)) out.push(child); walk(child); } }; walk(this); return out; }
  }
  const head = new Node('head'), body = new Node('body'), root = new Node('html');
  const doc = {
    head, body, documentElement: root, _listeners: {},
    createElement: tag => new Node(tag),
    getElementById(id) {
      for (const base of [root, head, body]) {
        if (base.id === id) return base;
        const found = base.querySelector(`[id="${id}"]`);
        if (found) return found;
      }
      return null;
    },
    querySelector(selector) {
      for (const base of [root, head, body]) { const found = base.querySelector(selector); if (found) return found; }
      return null;
    },
    querySelectorAll(selector) { return [root, head, body].flatMap(base => base.querySelectorAll(selector)); },
    addEventListener(type, handler) { (this._listeners[type] = this._listeners[type] || []).push(handler); },
    removeEventListener(type, handler) { this._listeners[type] = (this._listeners[type] || []).filter(fn => fn !== handler); },
    fire(type, event) { for (const handler of [...(this._listeners[type] || [])]) handler(event); },
  };
  return {doc, head, body, root};
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
  const bag = [{ asset_id: 1, asset_type: 'character_skin', rarity: 'SR', quantity: 6 }];
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
  const synthesized = await instance.synthesizeDuplicates('same-level');
  // quantity 6 keeps one base copy and synthesizes the 5 excess copies in one same-level chunk.
  assert.equal(synthesized.count, 5);
  assert.deepEqual(Array.from(calls.find(([route]) => route === 'synthesis')[1]), [1, 1, 1, 1, 1]);
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

test('same-level synthesis only consumes excess copies and batches by type and rarity', async () => {
  let fetched = 0;
  const bag = [
    { asset_id: 1, asset_type: 'character_skin', rarity: 'SR', quantity: 7 }, // 6 excess -> one chunk of 5, 1 leftover
    { asset_id: 2, asset_type: 'accessory', rarity: 'SR', quantity: 3 }, // 2 excess -> below one chunk, retained
    { asset_id: 3, asset_type: 'emote', rarity: 'SR', quantity: 9, wuwa_local_only: true },
    { asset_id: 4, asset_type: 'emote', rarity: 'mod', quantity: 9 },
    { asset_id: 5, asset_type: 'emote', rarity: 'SR', quantity: 9, custom_pack_id: 'custom-pack' },
  ];
  const { instance, calls } = controller({
    fetchBag: async () => { fetched++; return bag; },
    onSynthesisComplete: async () => { calls.push(['synthesis-complete', {}]); },
  });
  await instance.ready;
  assert.equal(instance.enabled.value, false);
  instance.inventory([], bag);
  const result = await instance.synthesizeDuplicates('same-level');
  assert.equal(fetched, 2, 'fetchBag runs before the pass and again after a real synthesis');
  assert.equal(result.count, 5);
  assert.equal(result.batches, 1);
  assert.deepEqual(Array.from(result.groups, group => ({ ...group })), [{ key: 'character_skin:sr', type: 'character_skin', rarity: 'sr', count: 5 }]);
  const chunks = calls.filter(([route]) => route === 'synthesis').map(([, ids]) => Array.from(ids));
  assert.deepEqual(chunks, [[1, 1, 1, 1, 1]]);
  assert.equal(calls.some(([route]) => route === 'synthesis-upgrade'), false, 'same-level never uses the upgrade endpoint');
  const fed = chunks.flat();
  assert.ok(!fed.includes(3) && !fed.includes(4) && !fed.includes(5), 'local/mod rows never reach the official synthesis call');
  assert.ok(!fed.includes(2), 'fewer than five excess copies stay in the bag');
  assert.equal(bag[0].quantity, 7, 'the official inventory is not mutated');
  assert.ok(calls.some(([route]) => route === 'refresh-bag'));
  assert.ok(calls.some(([route]) => route === 'synthesis-complete'));
  instance.dispose();
});

test('upgrade synthesis skips epics and submits chunks of ten grouped by type', async () => {
  let fetched = 0;
  const bag = [
    { asset_id: 10, asset_type: 'character_skin', rarity: 'Epic', quantity: 20 }, // epics are skipped on upgrade
    { asset_id: 11, asset_type: 'character_skin', rarity: 'SR', quantity: 12 }, // 11 excess -> one chunk of 10
    { asset_id: 12, asset_type: 'accessory', rarity: 'SR', quantity: 11 }, // 10 excess -> one chunk of 10
    { asset_id: 13, asset_type: 'emote', rarity: 'R', quantity: 4 }, // 3 excess -> below one chunk
  ];
  const { instance, calls } = controller({
    fetchBag: async () => { fetched++; return bag; },
    onSynthesisComplete: async () => { calls.push(['synthesis-complete', {}]); },
  });
  await instance.ready;
  instance.inventory([], bag);
  const result = await instance.synthesizeDuplicates('upgrade');
  assert.equal(fetched, 2);
  const chunks = calls.filter(([route]) => route === 'synthesis-upgrade').map(([, ids]) => Array.from(ids));
  assert.deepEqual(chunks, [Array(10).fill(11), Array(10).fill(12)]);
  assert.equal(calls.some(([route]) => route === 'synthesis'), false, 'upgrade never uses the same-level endpoint');
  assert.equal(result.count, 20);
  assert.equal(result.batches, 2);
  assert.deepEqual(Array.from(result.groups, group => group.key), ['character_skin:sr', 'accessory:sr']);
  assert.ok(!chunks.flat().includes(10), 'epic duplicates are never sent to upgrade synthesis');
  assert.ok(!chunks.flat().includes(13), 'remainders below ten are left in the bag');
  assert.ok(calls.some(([route]) => route === 'refresh-bag'));
  assert.ok(calls.some(([route]) => route === 'synthesis-complete'));
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
    snapshotCatalog: () => ({}), applyOfflineCatalog: () => {}, mergeModCatalog: () => {}, restoreCatalog: () => {},
    getAppearanceAssetById: id => ({ asset_id: id, asset_type: 'character_skin' }),
    request: async route => { calls.push(route); return route === '/status' ? { ready: true, enabled: true, mode_verified: true, network_policy: 'hybrid' } : {}; },
  });
  await instance.ready;
  assert.equal(observed, false, 'main pet route must not run source badge DOM observer');
  assert.throws(() => instance.setCapture({ panelBackground: 'data:image/png;base64,abc' }), /2 MB 以内/);
  instance.dispose();
});

test('OBS green capture paints .cat-wrap and syncs across windows without a transparent mode', async () => {
  const { doc, root } = richDocument();
  const { instance, storage, emitStorage } = controller({ document: doc });
  const style = doc.getElementById('wuwa-source-badges');
  assert.ok(style, 'capture CSS is injected once the controller boots');
  assert.match(style.textContent, /html\[data-wuwa-capture="green"\] \.cat-wrap\{background:#00ff00/);
  assert.doesNotMatch(style.textContent, /transparent/i, 'transparent backdrop is no longer a supported mode');

  instance.setCapture({ mode: 'green', panelOpacity: 0.9 });
  assert.equal(instance.capture.value.mode, 'green');
  assert.equal(root.getAttribute('data-wuwa-capture'), 'green');

  instance.setCapture({ mode: 'transparent' });
  assert.equal(root.getAttribute('data-wuwa-capture'), null, 'transparent falls back to the off state');

  // The /panel window flips the shared capture record; main follows on the storage event.
  storage.set('wuwatappo-capture-v1', JSON.stringify({ mode: 'green', panelOpacity: 0.8 }));
  emitStorage('wuwatappo-capture-v1');
  assert.equal(instance.capture.value.mode, 'green');
  assert.equal(instance.capture.value.panelOpacity, 0.8);
  assert.equal(root.getAttribute('data-wuwa-capture'), 'green');
  instance.dispose();
});

test('mergeBag prepends mod rows without mutating the official account bag', async () => {
  const offline = {
    catalog: {},
    atlas: [
      { asset_id: 101, asset_type: 'character_skin_atlas', asset_name: 'Mod hero', rarity: 'MOD' },
      { asset_id: 102, asset_type: 'accessory_atlas', asset_name: 'Pack ears', custom_pack_id: 'custom-pack' },
    ],
    atlas_count: 2,
  };
  const { instance } = controller({ offline });
  await instance.ready;
  await instance.setEnabled(true);
  const officialBag = [{ asset_id: 1, asset_type: 'character_skin', quantity: 1 }];
  instance.inventory([], officialBag);

  const merged = instance.mergeBag(officialBag);
  assert.notEqual(merged, officialBag);
  assert.equal(merged.length, 3);
  assert.equal(merged[0].asset_id, 101);
  assert.equal(merged[0].rarity, 'mod');
  assert.equal(merged[0].wuwa_local_only, true);
  assert.equal(merged[1].asset_id, 102);
  assert.equal(merged[1].rarity, 'mod');
  assert.equal(merged[1].wuwa_local_only, true);
  assert.equal(merged[2], officialBag[0]);
  assert.equal(instance.ownedBag(), officialBag, 'the account bag reference is untouched');
  assert.equal(officialBag.length, 1);
  assert.equal(officialBag[0].wuwa_local_only, undefined);
  instance.dispose();
});

test('sourceFor follows live selection and official snapshots and drops stale marks', async () => {
  const { instance } = controller();
  await instance.ready;
  instance.enabled.value = true;

  instance.select({ asset_id: 1, asset_type: 'character_skin', asset_name: 'A' }, null, 8, { quiet: true });
  assert.deepEqual({ ...instance.sourceFor(1, 'character_skin') }, { local: true, official: false });
  instance.select({ asset_id: 2, asset_type: 'character_skin', asset_name: 'B' }, null, 8, { quiet: true });
  assert.deepEqual({ ...instance.sourceFor(1, 'character_skin') }, { local: false, official: false }, 'the replaced local choice loses its badge');
  assert.deepEqual({ ...instance.sourceFor(2, 'character_skin') }, { local: true, official: false });

  instance.syncPanelState({ officialUserInfo: { role_id: '1001', character: { id: 2 }, accessory: null, emote: [] } });
  assert.deepEqual({ ...instance.sourceFor(2, 'character_skin') }, { local: true, official: true });
  assert.deepEqual({ ...instance.sourceFor(1, 'character_skin') }, { local: false, official: false });

  instance.syncPanelState({ officialUserInfo: { role_id: '2002', character: { id: 1 }, accessory: null, emote: [] } });
  assert.deepEqual({ ...instance.sourceFor(2, 'character_skin') }, { local: true, official: false });
  assert.deepEqual({ ...instance.sourceFor(1, 'character_skin') }, { local: false, official: true });

  // A persisted official mark for id 1 must not survive once the live snapshot says id 2.
  instance.setMark(1, 'character_skin', 'official', true);
  instance.syncPanelState({ officialUserInfo: { role_id: '2001', character: { id: 2 }, accessory: null, emote: [] } });
  assert.deepEqual({ ...instance.sourceFor(1, 'character_skin') }, { local: false, official: false }, 'live snapshot wins over a stale mark');
  instance.dispose();
});

test('local emotes have no role restrictions across select, character switch and preset apply', async () => {
  const appearance = id => {
    if (id === 1) return { asset_id: 1, asset_type: 'character_skin', game_role_id: '1001' };
    if (id === 2) return { asset_id: 2, asset_type: 'character_skin', game_role_id: '2002' };
    if (id === 31) return { asset_id: 31, asset_type: 'emote', game_role_id: '1001' };
    if (id === 32) return { asset_id: 32, asset_type: 'emote', game_role_id: '2002' };
    return { asset_id: id, asset_type: 'emote', game_role_id: '0' };
  };
  const { instance, storage } = controller({ appearance });
  await instance.ready;
  instance.enabled.value = true;

  instance.select({ asset_id: 1, asset_type: 'character_skin', asset_name: 'Hero A' }, null, 8, { quiet: true });
  assert.doesNotThrow(() => instance.select({ asset_id: 32, asset_type: 'emote', asset_name: 'B emote' }, null, 8, { quiet: true }));
  assert.deepEqual(Array.from(instance.selection.value.emote || [], item => item.id), [32]);

  // Switching to a character from another role keeps the emote equipped.
  instance.select({ asset_id: 2, asset_type: 'character_skin', asset_name: 'Hero B' }, null, 8, { quiet: true });
  assert.equal(instance.selection.value.character.id, 2);
  assert.deepEqual(Array.from(instance.selection.value.emote || [], item => item.id), [32]);

  storage.set('wuwatappo-local-presets-v1', JSON.stringify([{
    preset_id: -1, character_asset_id: 1, character_asset_name: 'Hero A',
    accessory_asset_id: 0, accessory_asset_name: '',
    emotes: [{ asset_id: 31, asset_name: 'A emote' }, { asset_id: 32, asset_name: 'B emote' }],
  }]));
  assert.doesNotThrow(() => instance.applyLocalPreset(-1));
  assert.equal(instance.selection.value.character.id, 1);
  assert.deepEqual(Array.from(instance.selection.value.emote || [], item => item.id), [31, 32]);
  instance.dispose();
});

test('online compatibility flags cross-role emotes and disables the official choice', async () => {
  const appearance = id => {
    if (id === 1) return { asset_id: 1, asset_type: 'character_skin', game_role_id: '1001' };
    if (id === 31) return { asset_id: 31, asset_type: 'emote', game_role_id: '1001' };
    if (id === 32) return { asset_id: 32, asset_type: 'emote', game_role_id: '2002' };
    if (id === 33) return { asset_id: 33, asset_type: 'emote', game_role_id: '0' };
    return { asset_id: id, asset_type: 'emote', game_role_id: '0' };
  };
  const { doc, body } = fakeDocument();
  const { instance } = controller({ document: doc, appearance });
  await instance.ready;
  instance.enabled.value = true;
  instance.inventory([], [{ asset_id: 32, asset_type: 'emote' }, { asset_id: 33, asset_type: 'emote' }]);
  instance.syncPanelState({ officialUserInfo: { role_id: '1001', character: { id: 1 }, accessory: null, emote: [] } });

  const cross = instance.onlineEmoteCompatibility({ asset_id: 32, asset_type: 'emote' });
  assert.equal(cross.compatible, false);
  assert.match(cross.label, /不兼容/);
  const universal = instance.onlineEmoteCompatibility({ asset_id: 33, asset_type: 'emote' });
  assert.equal(universal.compatible, true);
  assert.equal(universal.universal, true);

  let officialCalls = 0;
  instance.beginSelect({ asset_id: 32, asset_type: 'emote', asset_name: 'Cross emote' }, null, 8, async () => { officialCalls++; });
  const choice = body.children.at(-1);
  assert.equal(choice.className, 'wt-source-choice');
  assert.equal(choice.row.children.length, 3);
  const officialButton = choice.row.children[1];
  assert.equal(officialButton.disabled, true, 'the official button is disabled for incompatible emotes');
  assert.match(officialButton.title, /不兼容/);
  await officialButton.onclick();
  assert.equal(officialCalls, 0);
  choice.row.children[0].onclick();
  assert.deepEqual({ ...instance.sourceFor(32, 'emote') }, { local: true, official: false });
  instance.dispose();
});

test('__wuwaRemove and __wuwaTarget bypass the choice dialog', async () => {
  const { doc, body } = fakeDocument();
  const { instance } = controller({ document: doc });
  await instance.ready;
  instance.enabled.value = true;
  instance.inventory([], [{ asset_id: 1, asset_type: 'character_skin' }]);
  const official = { role_id: '1001', character: { id: 1 }, accessory: null, emote: [{ id: 31 }, { id: 32 }] };
  const before = body.children.length;

  assert.equal(instance.beginSelect({ asset_id: 31, asset_type: 'emote', __wuwaRemove: true, __wuwaTarget: 'local' }, official, 8, () => {}), true);
  assert.deepEqual(Array.from(instance.selection.value.emote || [], item => item.id), [32]);
  assert.equal(body.children.length, before, 'removing a local emote opens no dialog');

  let applied = 0;
  assert.equal(instance.beginSelect({ asset_id: 2, asset_type: 'accessory', __wuwaRemove: true, __wuwaTarget: 'official' }, official, 8, () => { applied++; }), true);
  assert.equal(applied, 1);
  assert.equal(body.children.length, before, 'removing an official accessory opens no dialog');

  assert.equal(instance.beginSelect({ asset_id: 1, asset_type: 'character_skin', __wuwaTarget: 'local' }, official, 8, () => {}), true);
  assert.equal(instance.selection.value.character.id, 1);
  assert.equal(body.children.length, before, 'a direct local drop opens no dialog');

  let appliedOfficial = 0;
  assert.equal(instance.beginSelect({ asset_id: 1, asset_type: 'character_skin', __wuwaTarget: 'official' }, official, 8, () => { appliedOfficial++; }), true);
  assert.equal(appliedOfficial, 1);
  assert.equal(body.children.length, before, 'a direct official drop opens no dialog');
  instance.dispose();
});

test('equipment bar renders local and official slots and handles pointer drags and reordering', async () => {
  const appearance = id => {
    if (id === 1) return { asset_id: 1, asset_type: 'character_skin', game_role_id: '1001' };
    if (id === 9) return { asset_id: 9, asset_type: 'character_skin', game_role_id: '2002' };
    if (id === 2) return { asset_id: 2, asset_type: 'accessory', game_role_id: '1001' };
    if (id === 31) return { asset_id: 31, asset_type: 'emote', game_role_id: '1001' };
    if (id === 32) return { asset_id: 32, asset_type: 'emote', game_role_id: '2002' };
    return { asset_id: id, asset_type: 'emote', game_role_id: '0' };
  };
  const { doc, body } = richDocument();
  const { instance } = controller({ document: doc, appearance });
  await instance.ready;
  instance.enabled.value = true;
  instance.inventory([], [
    { asset_id: 1, asset_type: 'character_skin' },
    { asset_id: 9, asset_type: 'character_skin' },
    { asset_id: 2, asset_type: 'accessory' },
    { asset_id: 31, asset_type: 'emote' },
    { asset_id: 32, asset_type: 'emote' },
  ]);
  instance.syncPanelState({ officialUserInfo: { role_id: '1001', character: { id: 1 }, accessory: { id: 2 }, emote: [{ id: 31 }] } });

  const host = doc.createElement('div');
  const emitted = [];
  const destroy = instance.mountEquipmentBar(host, { emitSelect: payload => { emitted.push(payload); return true; } });
  const slots = () => host.querySelectorAll('[data-wuwa-drop]');
  const slot = drop => slots().find(node => node.getAttribute('data-wuwa-drop') === drop);
  const atlasItem = (id, type, name) => {
    const node = doc.createElement('div');
    node.className = 'atlas-item';
    node.setAttribute('data-wuwa-asset-id', String(id));
    node.setAttribute('data-wuwa-asset-type', type);
    node.setAttribute('data-wuwa-asset-name', name || '');
    return node;
  };
  const dragFromAtlas = (item, drop, distance = 12) => {
    doc.fire('pointerdown', { button: 0, clientX: 100, clientY: 100, target: item });
    const target = slot(drop);
    assert.ok(target, `slot ${drop} exists`);
    doc.fire('pointermove', { clientX: 100 + distance, clientY: 100, target });
    doc.fire('pointerup', { clientX: 100 + distance, clientY: 100, target });
    return target;
  };

  assert.equal(host.querySelectorAll('[data-wuwa-slot-side="local"]').length, 7, 'local character + accessory + five emotes');
  assert.equal(host.querySelectorAll('[data-wuwa-slot-side="official"]').length, 7, 'official character + accessory + five emotes');
  assert.equal(slots().filter(node => node.getAttribute('data-wuwa-slot-type') === 'emote').length, 10);
  assert.deepEqual(host.querySelectorAll('h4').map(node => node.textContent), ['本地装备（仅本机显示）', '线上装备（联机可见）']);
  assert.equal(slot('local:character_skin').classList.contains('is-follow'), true, 'the local character follows the official one until overridden');

  // A sub-threshold gesture is a click, not a drag.
  emitted.length = 0;
  const tiny = atlasItem(1, 'character_skin', 'Hero');
  doc.fire('pointerdown', { button: 0, clientX: 100, clientY: 100, target: tiny });
  doc.fire('pointermove', { clientX: 105, clientY: 100, target: slot('local:character_skin') });
  doc.fire('pointerup', { clientX: 105, clientY: 100, target: slot('local:character_skin') });
  assert.equal(emitted.length, 0, 'less than 8px stays a click');

  emitted.length = 0;
  dragFromAtlas(atlasItem(1, 'character_skin', 'Hero'), 'local:character_skin');
  assert.equal(emitted.length, 1);
  assert.equal(emitted[0].__wuwaTarget, 'local');
  assert.equal(emitted[0].asset_id, 1);
  assert.equal(emitted[0].asset_type, 'character_skin');

  emitted.length = 0;
  dragFromAtlas(atlasItem(31, 'emote', 'Emote'), 'local:emote:2');
  assert.equal(emitted.length, 1);
  assert.equal(emitted[0].__wuwaTarget, 'local');
  assert.equal(emitted[0].__wuwaSlotIndex, 2);

  emitted.length = 0;
  dragFromAtlas(atlasItem(1, 'character_skin', 'Hero'), 'official:character_skin');
  assert.equal(emitted.length, 1);
  assert.equal(emitted[0].__wuwaTarget, 'official');

  emitted.length = 0;
  dragFromAtlas(atlasItem(9, 'character_skin', 'OtherHero'), 'official:character_skin');
  assert.equal(emitted.length, 1, 'owned character_skin with different role can switch official character');
  assert.equal(emitted[0].__wuwaTarget, 'official');

  emitted.length = 0;
  dragFromAtlas(atlasItem(99, 'character_skin', 'Ghost'), 'official:character_skin');
  assert.equal(emitted.length, 0, 'unowned official assets are rejected');

  emitted.length = 0;
  dragFromAtlas(atlasItem(32, 'emote', 'Cross'), 'official:emote:0');
  assert.equal(emitted.length, 0, 'cross-role emotes are rejected on the official side');

  emitted.length = 0;
  dragFromAtlas(atlasItem(1, 'character_skin', 'Hero'), 'local:emote:0');
  assert.equal(emitted.length, 0, 'mismatched slot types are rejected');

  emitted.length = 0;
  doc.fire('pointerdown', { button: 0, clientX: 100, clientY: 100, target: atlasItem(1, 'character_skin', 'Hero') });
  doc.fire('pointermove', { clientX: 112, clientY: 100, target: body });
  doc.fire('pointerup', { clientX: 112, clientY: 100, target: body });
  assert.equal(emitted.length, 0, 'dropping outside a slot is rejected');

  // Local emote reordering keeps the same asset list but changes the order.
  instance.selection.value = { emote: [{ id: 31 }, { id: 32 }, { id: 1 }] };
  host.__wuwaEquipBar.refresh();
  doc.fire('pointerdown', { button: 0, clientX: 100, clientY: 100, target: slot('local:emote:0') });
  doc.fire('pointermove', { clientX: 112, clientY: 100, target: slot('local:emote:2') });
  doc.fire('pointerup', { clientX: 112, clientY: 100, target: slot('local:emote:2') });
  assert.deepEqual(Array.from(instance.selection.value.emote, item => item.id), [32, 1, 31]);

  // Official emote reordering is delegated to the official apply pipeline.
  instance.syncPanelState({ officialUserInfo: { role_id: '1001', character: { id: 1 }, accessory: { id: 2 }, emote: [{ id: 31 }, { id: 32 }, { id: 1 }] } });
  emitted.length = 0;
  doc.fire('pointerdown', { button: 0, clientX: 100, clientY: 100, target: slot('official:emote:0') });
  doc.fire('pointermove', { clientX: 112, clientY: 100, target: slot('official:emote:2') });
  doc.fire('pointerup', { clientX: 112, clientY: 100, target: slot('official:emote:2') });
  assert.equal(emitted.length, 1);
  assert.equal(emitted[0].__wuwaReorder, true);
  assert.equal(emitted[0].__wuwaTarget, 'official');
  assert.deepEqual(Array.from(emitted[0].emoteAssetIds), [32, 1, 31]);

  destroy();
  assert.equal(host.__wuwaEquipBar, null);
  instance.dispose();
});

test('deleteLocalPreset removes negative local ids and refuses official presets', async () => {
  const { instance, storage } = controller();
  await instance.ready;
  instance.enabled.value = true;
  storage.set('wuwatappo-local-presets-v1', JSON.stringify([
    { preset_id: -10, character_asset_id: 1 },
    { preset_id: -20, character_asset_id: 2 },
  ]));
  instance.deleteLocalPreset(-10);
  assert.deepEqual(Array.from(instance.presetList(), row => row.preset_id), [-20]);
  assert.deepEqual(JSON.parse(storage.get('wuwatappo-local-presets-v1')).map(row => row.preset_id), [-20]);
  assert.throws(() => instance.deleteLocalPreset(5), /账号预设只能在官方预设列表中删除/);
  assert.throws(() => instance.deleteLocalPreset(-30), /本地预设不存在/);
  instance.dispose();
});

test('room layout clamps scale, opacity and name scale and mirrors CSS variables across windows', async () => {
  const { doc, root } = richDocument();
  const { instance, storage, emitStorage } = controller({ document: doc });
  await instance.ready;

  const high = instance.setRoomLayout({ scale: 9, opacity: 0, nameScale: 0.1, arrangeMode: 'vertical' });
  assert.equal(high.scale, 2);
  assert.equal(high.opacity, 0.15);
  assert.equal(high.nameScale, 0.5);
  assert.equal(high.arrangeMode, 'vertical');
  assert.equal(root.style.getPropertyValue('--wuwa-room-scale'), '2');
  assert.equal(root.style.getPropertyValue('--wuwa-room-opacity'), '0.15');
  assert.equal(root.style.getPropertyValue('--wuwa-room-name-scale'), '0.5');
  assert.deepEqual({...JSON.parse(storage.get('wuwatappo-room-layout-v1'))},
    { positions: {}, hidden: {}, scale: 2, opacity: 0.15, nameScale: 0.5, arrangeMode: 'vertical' });

  const low = instance.setRoomLayout({ scale: 0.3, opacity: 1, nameScale: 2 });
  assert.equal(low.scale, 0.3);
  assert.equal(low.opacity, 1);
  assert.equal(low.nameScale, 2);
  assert.equal(root.style.getPropertyValue('--wuwa-room-scale'), '0.3');
  assert.equal(root.style.getPropertyValue('--wuwa-room-opacity'), '1');
  assert.equal(root.style.getPropertyValue('--wuwa-room-name-scale'), '2');

  // The /panel window edits the shared record; main follows the storage event.
  storage.set('wuwatappo-room-layout-v1', JSON.stringify({
    positions: {}, hidden: {}, scale: 1.5, opacity: 0.5, nameScale: 1.8, arrangeMode: 'vertical',
  }));
  emitStorage('wuwatappo-room-layout-v1');
  assert.equal(instance.roomLayout.value.scale, 1.5);
  assert.equal(instance.roomLayout.value.opacity, 0.5);
  assert.equal(instance.roomLayout.value.nameScale, 1.8);
  assert.equal(instance.roomLayout.value.arrangeMode, 'vertical');
  assert.equal(root.style.getPropertyValue('--wuwa-room-scale'), '1.5');
  assert.equal(root.style.getPropertyValue('--wuwa-room-opacity'), '0.5');
  assert.equal(root.style.getPropertyValue('--wuwa-room-name-scale'), '1.8');
  instance.dispose();
});

test('room member positions rescale to the new viewport and yield to heavy overlap', async () => {
  const { instance, win } = controller();
  await instance.ready;
  const saved = instance.saveRoomMemberPosition('201', 100, 50);
  assert.deepEqual({...saved}, { left: 100, top: 50, w: 1280, h: 720 });

  // Restore after the window grows from 1280x720 to 1920x1080: coordinates scale 1.5x.
  win.innerWidth = 1920;
  win.innerHeight = 1080;
  assert.deepEqual({...instance.savedRoomMemberPosition('201')}, { left: 150, top: 75 });
  assert.equal(instance.savedRoomMemberPosition('201', [{ left: 150, top: 75 }]), null,
    'more than 30% overlap with an official member is rejected');
  assert.deepEqual({...instance.savedRoomMemberPosition('201', [{ left: 800, top: 600 }])}, { left: 150, top: 75 });
  assert.equal(instance.savedRoomMemberPosition('missing'), null);
  instance.dispose();
});

test('room hidden state persists, clearRoomPositions resets, and requestRoomArrange broadcasts', async () => {
  const { instance, storage } = controller();
  await instance.ready;
  instance.saveRoomMemberPosition('201', 40, 60);

  assert.equal(instance.toggleRoomMemberHidden('201'), true);
  assert.equal(instance.isRoomMemberHidden('201'), true);
  assert.equal(JSON.parse(storage.get('wuwatappo-room-layout-v1')).hidden['201'], true);

  assert.equal(instance.toggleRoomMemberHidden('201', false), false);
  assert.equal(instance.isRoomMemberHidden('201'), false);
  assert.equal(Object.hasOwn(JSON.parse(storage.get('wuwatappo-room-layout-v1')).hidden, '201'), false);

  instance.clearRoomPositions();
  assert.deepEqual({...instance.roomLayout.value.positions}, {});
  assert.deepEqual(JSON.parse(storage.get('wuwatappo-room-layout-v1')).positions, {});
  assert.equal(JSON.parse(storage.get('wuwatappo-room-action-v1')).action, 'reset-positions');

  instance.requestRoomArrange('vertical');
  assert.equal(instance.roomLayout.value.arrangeMode, 'vertical');
  const arrange = JSON.parse(storage.get('wuwatappo-room-action-v1'));
  assert.equal(arrange.action, 'arrange');
  assert.equal(arrange.mode, 'vertical');
  instance.dispose();
});

test('arrangeRoomMembers lays out non-overlapping cells inside the viewport for both modes', async () => {
  const { instance } = controller();
  await instance.ready;
  const cellW = 132, cellH = 152;
  const viewport = { width: 1280, height: 720 };
  const check = positions => {
    for (const row of positions) {
      assert.ok(row.left >= 0 && row.top >= 0, `${row.id} stays inside the viewport`);
      assert.ok(row.left + cellW <= viewport.width && row.top + cellH <= viewport.height,
        `${row.id} fits within the viewport`);
    }
    for (let i = 0; i < positions.length; i++) {
      for (let j = i + 1; j < positions.length; j++) {
        const a = positions[i], b = positions[j];
        const overlapW = Math.max(0, Math.min(a.left + cellW, b.left + cellW) - Math.max(a.left, b.left));
        const overlapH = Math.max(0, Math.min(a.top + cellH, b.top + cellH) - Math.max(a.top, b.top));
        assert.equal(overlapW * overlapH, 0, `${a.id} and ${b.id} must not overlap`);
      }
    }
  };
  const members = ['a', 'b', 'c', 'd', 'e'].map(id => ({ id }));
  const horizontal = instance.arrangeRoomMembers(members, 'horizontal', viewport);
  assert.equal(horizontal.length, 5);
  check(horizontal);
  const vertical = instance.arrangeRoomMembers(members, 'vertical', viewport);
  assert.equal(vertical.length, 5);
  check(vertical);
  assert.deepEqual(Array.from(instance.arrangeRoomMembers([], 'horizontal', viewport)), []);
  instance.dispose();
});

test('perf toggles data attributes and syncs from storage', async () => {
  const { doc, root } = richDocument();
  const { instance, storage, emitStorage } = controller({ document: doc });
  await instance.ready;
  assert.equal(root.getAttribute('data-wuwa-hide-pet'), null);
  assert.equal(root.getAttribute('data-wuwa-reduce-motion'), null);

  const perf = instance.setPerf({ fpsCap: 30, idlePause: false, hideMainPet: true, reduceMotion: true });
  assert.equal(perf.fpsCap, 30);
  assert.equal(perf.idlePause, false);
  assert.equal(perf.hideMainPet, true);
  assert.equal(perf.reduceMotion, true);
  assert.equal(root.getAttribute('data-wuwa-hide-pet'), '');
  assert.equal(root.getAttribute('data-wuwa-reduce-motion'), '');
  assert.deepEqual(JSON.parse(storage.get('wuwatappo-perf-v1')),
    { fpsCap: 30, idlePause: false, hideMainPet: true, reduceMotion: true });

  // An unsupported cap falls back to 60 without disturbing the other switches.
  assert.equal(instance.setPerf({ fpsCap: 45 }).fpsCap, 60);

  instance.setPerf({ hideMainPet: false, reduceMotion: false });
  assert.equal(root.getAttribute('data-wuwa-hide-pet'), null);
  assert.equal(root.getAttribute('data-wuwa-reduce-motion'), null);

  // External window update arrives through the storage event.
  storage.set('wuwatappo-perf-v1', JSON.stringify({ fpsCap: 0, idlePause: true, hideMainPet: true, reduceMotion: false }));
  emitStorage('wuwatappo-perf-v1');
  assert.equal(instance.perf.value.fpsCap, 0);
  assert.equal(instance.perf.value.idlePause, true);
  assert.equal(instance.perf.value.hideMainPet, true);
  assert.equal(instance.perf.value.reduceMotion, false);
  assert.equal(root.getAttribute('data-wuwa-hide-pet'), '');
  assert.equal(root.getAttribute('data-wuwa-reduce-motion'), null);
  instance.dispose();
});

test('accessory transforms work for official and explicit ids, list names, delete one and sync', async () => {
  const appearance = id => ({ asset_id: id, asset_type: 'accessory', asset_name: `Accessory-${id}` });
  const { instance, storage, emitStorage } = controller({ appearance });
  await instance.ready;
  assert.equal(instance.enabled.value, false);
  instance.selection.value = { accessory: null };
  instance.syncPanelState({ officialUserInfo: { role_id: '1001', character: { id: 1 }, accessory: { id: 2 } } });

  // An official accessory gets a transform without local mode or a local selection.
  const saved = instance.setAccessoryTransform({ dx: 10, dy: -6, scale: 1.2, rotate: 15 });
  assert.deepEqual({...saved}, { dx: 10, dy: -6, scale: 1.2, rotate: 15 });
  assert.deepEqual(JSON.parse(storage.get('wuwatappo-accessory-transform-v1'))['2'],
    { dx: 10, dy: -6, scale: 1.2, rotate: 15 });

  // Explicit ids are accepted too, and all-default patches never leave a redundant row.
  instance.setAccessoryTransform({ dy: 9 }, '5');
  instance.setAccessoryTransform({ dx: 0, dy: 0, scale: 1, rotate: 0 }, '9');
  assert.equal(Object.hasOwn(JSON.parse(storage.get('wuwatappo-accessory-transform-v1')), '9'), false);

  const listed = instance.listAccessoryTransforms().map(row => ({...row}));
  const byId = Object.fromEntries(listed.map(row => [row.id, row]));
  assert.deepEqual(Object.keys(byId).sort(), ['2', '5']);
  assert.equal(byId['2'].name, 'Accessory-2');
  assert.equal(byId['2'].dx, 10);
  assert.equal(byId['2'].dy, -6);
  assert.equal(byId['2'].scale, 1.2);
  assert.equal(byId['2'].rotate, 15);
  assert.equal(byId['5'].name, 'Accessory-5');
  assert.equal(byId['5'].dy, 9);

  instance.deleteAccessoryTransform('2');
  assert.deepEqual(Array.from(instance.listAccessoryTransforms(), row => row.id), ['5']);
  assert.equal(Object.hasOwn(JSON.parse(storage.get('wuwatappo-accessory-transform-v1')), '2'), false);

  assert.deepEqual({...instance.resetAccessoryTransform('5')}, { dx: 0, dy: 0, scale: 1, rotate: 0 });
  assert.equal(instance.listAccessoryTransforms().length, 0);

  // /panel writes a transform; main picks it up through the storage event.
  storage.set('wuwatappo-accessory-transform-v1', JSON.stringify({ '2': { dx: 7, dy: 8, scale: 1.1, rotate: 9 } }));
  emitStorage('wuwatappo-accessory-transform-v1');
  instance.enabled.value = true;
  instance.selection.value = {};
  assert.deepEqual({...instance.accessoryTransform(instance.officialUser.value)}, { dx: 7, dy: 8, scale: 1.1, rotate: 9 });
  instance.dispose();
});

test('synthesizeDuplicates reports a same-level diagnostic snapshot after success', async () => {
  const bag = [{ asset_id: 1, asset_type: 'character_skin', rarity: 'SR', quantity: 6 }];
  const { instance, calls } = controller({ fetchBag: async () => bag });
  await instance.ready;
  instance.inventory([], bag);
  const result = await instance.synthesizeDuplicates('same-level');
  assert.equal(result.count, 5);
  assert.equal(result.batches, 1);
  const entry = calls.filter(([route]) => route === '/diagnostic/synth').pop();
  assert.ok(entry, 'finally always posts a /diagnostic/synth snapshot');
  const payload = entry[1];
  assert.equal(payload.kind, 'same-level');
  assert.equal(payload.consumed, 5);
  assert.equal(payload.batches, 1);
  assert.ok(Array.isArray(payload.groups) && payload.groups.length > 0);
  assert.ok(Array.isArray(payload.results) && payload.results.length > 0);
  assert.equal(payload.results[0].ok, true);
  assert.equal('error' in payload, false);
  instance.dispose();
});

test('synthesizeDuplicates rethrows a failed synthesis and still reports the diagnostic snapshot', async () => {
  const bag = [{ asset_id: 1, asset_type: 'character_skin', rarity: 'SR', quantity: 6 }];
  const { instance, calls } = controller({
    fetchBag: async () => bag,
    synthesisSame: async () => { throw new Error('network busy'); },
  });
  await instance.ready;
  instance.inventory([], bag);
  await assert.rejects(() => instance.synthesizeDuplicates('same-level'), /network busy/);
  const entry = calls.filter(([route]) => route === '/diagnostic/synth').pop();
  assert.ok(entry, 'the failure snapshot is still posted');
  const payload = entry[1];
  assert.equal(payload.kind, 'same-level');
  assert.equal(payload.consumed, 0);
  assert.equal(payload.error, 'network busy');
  assert.ok(Array.isArray(payload.results) && payload.results.length > 0);
  assert.equal(payload.results[0].ok, false);
  assert.equal(payload.results[0].error, 'network busy');
  instance.dispose();
});

test('beginPanelSelect resolves source choices inside panel and main noDialog never spawns pet popup', async () => {
  const { doc, body } = fakeDocument();
  const { instance } = controller({ document: doc });
  await instance.ready;
  instance.enabled.value = true;

  const picked = [];
  assert.equal(
    instance.beginPanelSelect({ asset_id: 1, asset_type: 'character_skin', asset_name: 'Hero' }, item => picked.push(item), true),
    true,
    'bag card opens choice popup inside panel',
  );
  let choice = body.children.at(-1);
  assert.equal(choice.className, 'wt-source-choice');
  assert.equal(choice.row.children.length, 3, 'forceOfficial bag card offers local, official and close');
  choice.row.children[1].onclick();
  assert.equal(picked.length, 1);
  assert.equal(picked[0].wuwa_choice, 'official');

  // Mod items in bag must never offer official even when forceOfficial = true.
  picked.length = 0;
  instance.beginPanelSelect(
    { asset_id: 99, asset_type: 'character_skin', asset_name: 'ModHero', rarity: 'mod', wuwa_local_only: true },
    item => picked.push(item),
    true,
  );
  choice = body.children.at(-1);
  assert.equal(choice.row.children.length, 2, 'mod card offers only local and close');
  choice.row.children[0].onclick();
  assert.equal(picked[0].wuwa_choice, 'local');

  // Main renderer passes { noDialog: true }, so clicking must never render .wt-source-choice on the pet window.
  const before = body.children.length;
  assert.equal(
    instance.beginSelect({ asset_id: 1, asset_type: 'character_skin', asset_name: 'Hero' }, null, 8, () => {}, { noDialog: true }),
    false,
  );
  assert.equal(body.children.length, before, 'main renderer must not open .wt-source-choice overlay');
  instance.dispose();
});

test('floating equipment dock mounts externally, auto-expands on drag, and removes local & official slots cleanly', async () => {
  const appearance = id => {
    if (id === 1) return { asset_id: 1, asset_type: 'character_skin', game_role_id: '1001', asset_name: 'Hero' };
    if (id === 2) return { asset_id: 2, asset_type: 'accessory', game_role_id: '1001', asset_name: 'Hat' };
    return { asset_id: id, asset_type: 'emote', game_role_id: '0', asset_name: `Emote-${id}` };
  };
  const { doc, body } = richDocument();
  const { instance } = controller({ document: doc, appearance });
  await instance.ready;
  instance.enabled.value = true;
  instance.selection.value = {
    character: { id: 1, name: 'Hero' },
    accessory: { id: 2, name: 'Hat' },
    emote: [{ id: 31, name: 'Emote-31' }, { id: 32, name: 'Emote-32' }],
  };
  instance.syncPanelState({
    officialUserInfo: { role_id: '1001', character: { id: 1 }, accessory: { id: 2 }, emote: [{ id: 31 }] },
  });

  const panelContent = doc.createElement('div');
  panelContent.className = 'panel-content';
  const card = doc.createElement('div');
  card.className = 'atlas-item';
  card.setAttribute('data-wuwa-asset-id', '33');
  card.setAttribute('data-wuwa-asset-type', 'emote');
  card.setAttribute('data-wuwa-asset-name', 'Emote-33');
  panelContent.append(card);
  body.append(panelContent);

  const emitted = [];
  const destroy = instance.mountEquipmentBar(body, {
    floating: true,
    emitSelect: payload => {
      emitted.push(payload);
      return instance.beginSelect(payload, instance.officialUser.value, 5, () => {}, { noDialog: true });
    },
  });

  const dock = body.querySelector('.wuwa-equip-dock');
  const toggle = dock.querySelector('.wuwa-equip-toggle');
  const bar = dock.querySelector('.wuwa-equip-bar');
  const equipStyle = doc.getElementById('wuwa-equip-bar-style');
  assert.ok(dock, 'floating equipment dock mounts to body outside .panel-content');
  assert.ok(equipStyle, 'equipment dock CSS is injected once');
  assert.match(equipStyle.textContent, /right:6px/);
  assert.match(equipStyle.textContent, /html\[data-wuwa-equip-wing\]/);
  assert.match(equipStyle.textContent, /min-height:106px/);
  assert.match(equipStyle.textContent, /max-height:58px/);
  assert.equal(panelContent.querySelector('.wuwa-equip-dock'), null, 'panelContent stays clean');
  assert.equal(bar.hidden, true, 'floating bar starts collapsed by default');
  assert.equal(toggle.textContent, '装备栏 ▸', 'collapsed toggle invites opening');

  // Dragging an item auto-expands the floating dock before pointerup.
  doc.fire('pointerdown', { button: 0, clientX: 50, clientY: 50, target: card });
  doc.fire('pointermove', { clientX: 65, clientY: 50, target: body });
  assert.equal(bar.hidden, false, 'dragging a panel card auto-expands the floating dock');
  doc.fire('pointerup', { clientX: 65, clientY: 50, target: body });

  toggle.onclick();
  assert.equal(bar.hidden, false, 'clicking toggle opens the floating dock persistently');
  assert.equal(toggle.textContent, '收起 ◂', 'opened toggle invites collapsing');

  // Remove local accessory via × button.
  const localAccSlot = bar.querySelector('[data-wuwa-drop="local:accessory"]');
  // An unchanged equipment snapshot with a bumped localClickCount must not re-render the bar
  // mid-click (the WebView2 "× click swallowed" regression).
  const accRemoveBefore = localAccSlot.querySelector('.wuwa-equip-remove');
  instance.syncPanelState({
    localClickCount: 999,
    officialUserInfo: { role_id: '1001', character: { id: 1 }, accessory: { id: 2 }, emote: [{ id: 31 }] },
  });
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(
    bar.querySelector('[data-wuwa-drop="local:accessory"]')?.querySelector('.wuwa-equip-remove'),
    accRemoveBefore,
    'unchanged snapshot tick (e.g. localClickCount) must preserve existing × button DOM node mid-click',
  );
  localAccSlot.querySelector('.wuwa-equip-remove').onclick({});
  assert.equal(instance.selection.value.accessory, null, 'clicking × unequips local accessory');

  // Remove local emote even if asset_id was zero (slotIndex fallback).
  instance.beginSelect({ asset_id: 0, asset_type: 'emote', __wuwaRemove: true, __wuwaTarget: 'local', __wuwaSlotIndex: 1 }, instance.officialUser.value, 5, () => {}, { noDialog: true });
  assert.deepEqual(Array.from(instance.selection.value.emote, item => item.id), [31]);

  // Remove official emote via × button routes to official pipeline.
  const officialEmoteSlot = bar.querySelector('[data-wuwa-drop="official:emote:0"]');
  officialEmoteSlot.querySelector('.wuwa-equip-remove').onclick({});
  const officialRemove = emitted.at(-1);
  assert.equal(officialRemove.__wuwaRemove, true);
  assert.equal(officialRemove.__wuwaTarget, 'official');
  assert.equal(officialRemove.asset_type, 'emote');
  assert.equal(officialRemove.asset_id, 31);

  // Main IPC can sync the local selection directly; the dock re-renders from it.
  instance.syncPanelState({
    localSelection: { character: { id: 1, name: 'Hero' }, accessory: null, emote: [{ id: 31, name: 'Emote-31' }] },
    officialUserInfo: { role_id: '1001', character: { id: 1 }, accessory: { id: 2 }, emote: [{ id: 31 }] },
  });
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(instance.selection.value.accessory, null, 'syncPanelState syncs localSelection from Main IPC snapshot');

  destroy();
  assert.equal(body.querySelector('.wuwa-equip-dock'), null);
  instance.dispose();
});

test('floating equipment dock expands and restores panel window width externally on Tauri', async () => {
  const tick = () => new Promise(resolve => setTimeout(resolve, 20));
  // Each case gets its own document/window so the applied wing width and dock-open
  // session flag never leak between the happy path and the resize-rejection case.
  const harness = (rejectSetSize = false) => {
    const { doc, body, root } = richDocument();
    let curW = 1100;
    const sizeCalls = [];
    const invoke = async (cmd, args) => {
      if (cmd === '__simulateSidebarCollapse') { curW -= 142; return null; }
      if (cmd === 'plugin:window|inner_size') return { width: curW * 1.25, height: 760 * 1.25 };
      if (cmd === 'plugin:window|scale_factor') return 1.25;
      if (cmd === 'plugin:window|set_size') {
        if (rejectSetSize) throw new Error('set_size denied');
        curW = args.value.Logical.width;
        sizeCalls.push({ label: args.label, width: curW, height: args.value.Logical.height });
        return null;
      }
      throw new Error('unexpected invoke ' + cmd);
    };
    const { instance } = controller({
      document: doc,
      winProps: {
        __TAURI_INTERNALS__: {
          metadata: { currentWindow: { label: 'panel' } },
          invoke,
        },
      },
    });
    return { body, root, instance, sizeCalls, getCurW: () => curW, invoke };
  };

  // Happy path: widen the transparent OS window to the right, keep the official
  // panel un-squished, and restore the exact original width on collapse/unmount.
  const { body, root, instance, sizeCalls, getCurW, invoke } = harness();
  await instance.ready;
  instance.enabled.value = true;
  const destroy = instance.mountEquipmentBar(body, { floating: true });
  const toggle = body.querySelector('.wuwa-equip-dock')?.querySelector('.wuwa-equip-toggle');

  toggle.onclick();
  await tick();
  assert.equal(getCurW(), 1100 + 472, 'expanding widens the OS window by the 472px wing');
  assert.equal(sizeCalls.at(-1).label, 'panel');
  assert.equal(root.getAttribute('data-wuwa-equip-wing'), '1');
  assert.equal(root.style.getPropertyValue('--wuwa-equip-wing'), '472px');

  toggle.onclick();
  await tick();
  assert.equal(getCurW(), 1100, 'collapsing restores the original OS window width');
  assert.equal(root.getAttribute('data-wuwa-equip-wing'), null);

  toggle.onclick();
  await tick();
  assert.equal(getCurW(), 1572, 're-expanding widens the OS window again');
  // Sidebar collapse while wing is open (-142px): closing wing must subtract only 472px -> 958px.
  invoke('__simulateSidebarCollapse');
  toggle.onclick();
  await tick();
  assert.equal(getCurW(), 1100 - 142, 'collapsing after sidebar toggle preserves the -142px sidebar delta');
  toggle.onclick();
  await tick();
  assert.equal(getCurW(), 1430);
  destroy();
  await new Promise(resolve => setTimeout(resolve, 150));
  assert.equal(getCurW(), 958, 'closing the dock restores collapsed window width');
  instance.dispose();

  // A failed OS resize must never leave the official panel squished: the wing
  // style is rolled back even though the toggle reports "open".
  const rejected = harness(true);
  await rejected.instance.ready;
  const rejectDestroy = rejected.instance.mountEquipmentBar(rejected.body, { floating: true });
  const rejectToggle = rejected.body.querySelector('.wuwa-equip-dock')?.querySelector('.wuwa-equip-toggle');
  rejectToggle.onclick();
  await tick();
  assert.equal(rejected.root.getAttribute('data-wuwa-equip-wing'), null,
    'the official panel is never squished when the OS resize fails');
  rejectDestroy();
  rejected.instance.dispose();
});

test('Mod catalog merges via mergeModCatalog, mergeBag includes asset_code, and beginSelect/isModAsset block official fallback', async () => {
  const offline = {
    customAssetBase: 'http://127.0.0.1:41234/tok/',
    customAssetPaths: {
      'custom-assets/pack-a/hero.png': 'custom-assets/pack-a/hero.png',
      'custom-assets/pack-a/body.png': 'custom-assets/pack-a/body.png',
      'custom-assets/pack-a/head.png': 'custom-assets/pack-a/head.png',
      'custom-assets/pack-a/ears.png': 'custom-assets/pack-a/ears.png',
    },
    catalog: {
      character_skin: {
        900000001: {
          url: 'custom-assets/pack-a/hero.png',
          game_role_id: 1001,
          body1: { url: 'custom-assets/pack-a/body.png', offset_x: 0, offset_y: 0, my_offset_x: 0, my_offset_y: 0 },
          head1: { url: 'custom-assets/pack-a/head.png', offset_x: 0, offset_y: 0, my_offset_x: 0, my_offset_y: 0 },
        },
      },
      character_skin_atlas: {
        900000002: { url: 'custom-assets/pack-a/hero.png', height_light_id: 900000001, game_role_id: 1001 },
      },
      accessory: {
        900000003: { url: 'custom-assets/pack-a/ears.png', game_role_id: 2002 },
      },
      accessory_atlas: {
        900000004: { url: 'custom-assets/pack-a/ears.png', height_light_id: 900000003, game_role_id: 2002 },
      },
    },
    atlas: [
      { asset_id: 900000002, asset_code: 'mod:pack-a:hero', asset_type: 'character_skin_atlas', asset_name: 'ModHero', rarity: 'MOD', custom_pack_id: 'pack-a' },
      { asset_id: 900000004, asset_code: 'mod:pack-a:ears', asset_type: 'accessory_atlas', asset_name: 'ModEars', rarity: 'MOD', custom_pack_id: 'pack-a' },
    ],
    atlas_count: 2,
  };
  const { instance, mergedCatalogs } = controller({ offline });
  await instance.ready;
  await instance.setEnabled(true);

  const lastMerged = mergedCatalogs.at(-1);
  assert.ok(lastMerged, 'mergeModCatalog runs when local mode enables');
  assert.equal(
    lastMerged.character_skin[900000001].body1.url,
    'http://127.0.0.1:41234/tok/custom-assets/pack-a/body.png',
  );
  assert.equal(lastMerged.character_skin_atlas[900000002].height_light_id, 900000001);

  const bag = instance.mergeBag([]);
  assert.equal(bag[0].asset_code, 'mod:pack-a:hero');
  assert.equal(bag[1].asset_code, 'mod:pack-a:ears');

  assert.equal(instance.isModAsset(900000001), true);
  assert.equal(instance.isModAsset(900000002), true);
  assert.equal(instance.isModAsset({ asset_id: 900000003 }), true);
  assert.equal(instance.isModAsset(1), false);

  let officialCalled = 0;
  // Dropping a Mod item onto official target or an incompatible Mod accessory onto local target must be consumed (true).
  assert.equal(
    instance.beginSelect({ asset_id: 900000001, asset_type: 'character_skin', __wuwaTarget: 'official' }, null, 5, () => { officialCalled++; }),
    true,
  );
  instance.select({ asset_id: 900000001, asset_type: 'character_skin' }, null, 5);
  assert.equal(instance.shouldCreateLocalPreset(null), true, 'equipped Mod requires local preset creation');
  assert.equal(
    instance.beginSelect({ asset_id: 900000003, asset_type: 'accessory', __wuwaTarget: 'local' }, null, 5, () => { officialCalled++; }),
    true,
  );
  assert.equal(officialCalled, 0, 'blocked drops never invoke official API callback');

  await instance.setEnabled(false);
  assert.equal(mergedCatalogs.at(-1), null, 'disabling local mode clears injected Mod catalog entries');
  instance.dispose();
});

test('Mod preview resolves filename stems, disambiguates active Mod, avoids badge overlap, and keeps UX fixes', async () => {
  const { doc } = fakeDocument();
  const offline = {
    customAssetBase: 'http://127.0.0.1:41234/tok/',
    customAssetPaths: {
      'custom-assets/pack-a/character/icon.png': 'custom-assets/pack-a/character/icon.png',
      'custom-assets/pack-a/character/body1.png': 'custom-assets/pack-a/character/body1.png',
      'custom-assets/pack-b/character/icon.png': 'custom-assets/pack-b/character/icon.png',
    },
    catalog: {
      character_skin: {
        900000001: {
          url: 'custom-assets/pack-a/character/icon.png',
          game_role_id: 1001,
          body1: { url: 'custom-assets/pack-a/character/body1.png' },
        },
        900000011: {
          url: 'custom-assets/pack-b/character/icon.png',
          game_role_id: 1002,
        },
        101: { url: 'assets/c101.png', game_role_id: 1 },
        102: { url: 'assets/c102.png', game_role_id: 2 },
      },
      character_skin_atlas: {
        900000002: { url: 'custom-assets/pack-a/character/icon.png', height_light_id: 900000001, game_role_id: 1001 },
        900000012: { url: 'custom-assets/pack-b/character/icon.png', height_light_id: 900000011, game_role_id: 1002 },
        201: { url: 'assets/c101.png', height_light_id: 101, game_role_id: 1 },
        202: { url: 'assets/c102.png', height_light_id: 102, game_role_id: 2 },
      },
      accessory: {
        301: { url: 'assets/a301.png', game_role_id: 0 },
      },
      emote: {
        401: { url: 'assets/e401.png', game_role_id: 0 },
        402: { url: 'assets/e402.png', game_role_id: 0 },
        403: { url: 'assets/e403.png', game_role_id: 0 },
        404: { url: 'assets/e404.png', game_role_id: 0 },
        405: { url: 'assets/e405.png', game_role_id: 0 },
        406: { url: 'assets/e406.png', game_role_id: 0 },
      },
    },
    atlas: [
      { asset_id: 900000002, asset_type: 'character_skin_atlas', asset_name: '小家伙（演示）', rarity: 'MOD', custom_pack_id: 'pack-a' },
      { asset_id: 900000012, asset_type: 'character_skin_atlas', asset_name: '第二只（演示）', rarity: 'MOD', custom_pack_id: 'pack-b' },
      { asset_id: 201, asset_type: 'character_skin_atlas', asset_name: 'Role1', rarity: 'common' },
      { asset_id: 202, asset_type: 'character_skin_atlas', asset_name: 'Role2', rarity: 'common' },
      { asset_id: 301, asset_type: 'accessory_atlas', asset_name: 'UnivAcc', rarity: 'common' },
      { asset_id: 401, asset_type: 'emote_atlas', asset_name: 'E1', rarity: 'common' },
      { asset_id: 402, asset_type: 'emote_atlas', asset_name: 'E2', rarity: 'common' },
      { asset_id: 403, asset_type: 'emote_atlas', asset_name: 'E3', rarity: 'common' },
      { asset_id: 404, asset_type: 'emote_atlas', asset_name: 'E4', rarity: 'common' },
      { asset_id: 405, asset_type: 'emote_atlas', asset_name: 'E5', rarity: 'common' },
      { asset_id: 406, asset_type: 'emote_atlas', asset_name: 'E6', rarity: 'common' },
    ],
    atlas_count: 11,
  };
  let bagRows = [
    { asset_id: 101, asset_type: 'character_skin', asset_name: 'Role1' },
    { asset_id: 102, asset_type: 'character_skin', asset_name: 'Role2' },
  ];
  const { instance, win } = controller({
    document: doc,
    offline,
    catalogConsumer: true,
    appearance: id => {
      const n = Number(id);
      if (n === 101 || n === 102) return { asset_id: n, asset_type: 'character_skin', game_role_id: n === 101 ? 1 : 2 };
      if (n === 301) return { asset_id: 301, asset_type: 'accessory', game_role_id: 0 };
      if (n >= 401 && n <= 406) return { asset_id: n, asset_type: 'emote', game_role_id: 0 };
      return null;
    },
  });
  await instance.ready;
  await instance.setEnabled(true);

  const badgeStyle = doc.head.children.find(n => n.id === 'wuwa-mod-badge')?.textContent || '';
  assert.match(badgeStyle, /\.atlas-thumb--mod::after\{[^}]*right:3px;bottom:3px/);
  assert.doesNotMatch(badgeStyle, /\.atlas-thumb--mod::after\{[^}]*left:3px;bottom:3px/);

  // Stem lookup for character/icon.png resolves even when asset_name differs from filename,
  // and disambiguates between two Mods sharing icon.png by active equipped character.
  instance.select({ asset_id: 900000011, asset_type: 'character_skin' }, null, 5);
  assert.equal(win.__WUWA_MOD_ASSET_PATH__('character/icon.png'), 'http://127.0.0.1:41234/tok/custom-assets/pack-b/character/icon.png');
  assert.equal(win.__WUWA_MOD_ASSET_PATH__('character/icon.webp.png'), 'http://127.0.0.1:41234/tok/custom-assets/pack-b/character/icon.png');
  instance.select({ asset_id: 900000001, asset_type: 'character_skin' }, null, 5);
  assert.equal(win.__WUWA_MOD_ASSET_PATH__('character/icon.png'), 'http://127.0.0.1:41234/tok/custom-assets/pack-a/character/icon.png');
  assert.equal(win.__WUWA_MOD_ASSET_PATH__('character/%E5%B0%8F%E5%AE%B6%E4%BC%99%EF%BC%88%E6%BC%94%E7%A4%BA%EF%BC%89.png'), 'http://127.0.0.1:41234/tok/custom-assets/pack-a/character/icon.png');
  assert.equal(win.__WUWA_MOD_ASSET_PATH__('character/小家伙（演示）_body_1.png'), 'http://127.0.0.1:41234/tok/custom-assets/pack-a/character/body1.png');

  // Favorites reflect on mergeAtlas and mergeBag regardless of whether primaryId or atlasId was toggled.
  instance.favorite(900000001, true);
  let atlasMerged = instance.mergeAtlas([{ asset_id: 202, asset_type: 'character_skin_atlas', asset_name: 'Role2', owned: false }]);
  assert.equal(atlasMerged.find(r => r.asset_id === 900000002)?.is_collect, 1);
  assert.equal(instance.mergeBag([]).find(r => r.asset_id === 900000001)?.is_collect, 1);
  instance.favorite(900000002, false);
  atlasMerged = instance.mergeAtlas([{ asset_id: 202, asset_type: 'character_skin_atlas', asset_name: 'Role2', owned: false }]);
  assert.equal(atlasMerged.find(r => r.asset_id === 900000002)?.is_collect, 0);
  assert.equal(instance.mergeBag([]).find(r => r.asset_id === 900000001)?.is_collect, 0);

  // Universal accessory (role 0) stays equipped when switching character_skin.
  instance.select({ asset_id: 301, asset_type: 'accessory' }, { character: { id: 101 }, role_id: '1' }, 5);
  instance.select({ asset_id: 102, asset_type: 'character_skin' }, { character: { id: 101 }, role_id: '1' }, 5);
  assert.equal(instance.selection.value.accessory?.id, 301);

  // Full 5/5 emote bar allows slot replacement on drop without hitting limit error.
  for (const id of [401, 402, 403, 404, 405]) {
    instance.select({ asset_id: id, asset_type: 'emote' }, null, 5);
  }
  instance.select({ asset_id: 406, asset_type: 'emote' }, null, 5, { slotIndex: 2 });
  assert.equal(instance.selection.value.emote.length, 5);
  assert.equal(instance.selection.value.emote[2].id, 406);

  // applyPreset returns numeric meta asset_id instead of undefined.
  instance.mergePresets([], [{ preset_id: 88, character_asset_id: 101, accessory_asset_id: 301, emotes: [{ asset_id: 401 }] }]);
  const applied = instance.applyPreset(88);
  assert.equal(applied.character.id, 101);
  assert.equal(applied.accessory.id, 301);

  // Disabling mode clears Mod name & path resolvers cleanly.
  await instance.setEnabled(false);
  assert.equal(win.__WUWA_MOD_NAMES__.size, 0);
  assert.equal(win.__WUWA_MOD_ASSET_PATH__, null);

  // Verify removeCustomResource prunes removed Mod IDs from selection.
  await instance.setEnabled(true);
  instance.select({ asset_id: 900000001, asset_type: 'character_skin' }, null, 5);
  assert.equal(instance.selection.value.character?.id, 900000001);
  offline.atlas = offline.atlas.filter(r => r.asset_id !== 900000002);
  offline.atlas_count = offline.atlas.length;
  delete offline.catalog.character_skin[900000001];
  delete offline.catalog.character_skin_atlas[900000002];
  await instance.removeCustomResource('pack-a');
  assert.equal(instance.selection.value.character, undefined, 'removed Mod is pruned from selection');
  instance.dispose();
  void bagRows;
});
