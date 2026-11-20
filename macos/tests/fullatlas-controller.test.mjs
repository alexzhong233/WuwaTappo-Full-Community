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
  instance.dispose();
});
