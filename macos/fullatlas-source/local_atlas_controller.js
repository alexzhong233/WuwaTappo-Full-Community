// This factory is shared by every WebView and exercised independently in tests.
function createLocalAtlasController(d) {
  const KEY = 'wuwatappo-local-gallery-v1';
  const CACHE_KEY='wuwatappo-local-cache-generation';
  // 官方预设存在账号上（走 api/frontend/presets），本地模式一条请求都不发，
  // 所以预设改存本机：最多 5 个，字段与官方 normalizePresetList 的输出对齐，
  // 面板里的「我的预设」不用改渲染就能直接用。
  const PRESET_KEY='wuwatappo-local-presets-v1';
  const readPresets=()=>{try{const rows=JSON.parse(d.storage.getItem(PRESET_KEY)||'[]');return Array.isArray(rows)?rows.slice(0,5):[];}catch{return [];}};
  const presets=d.ref(readPresets());
  const enabled = d.ref(false), busy = d.ref(false), selection = d.ref({}), favorites = d.ref({});
  const offlineData = d.ref(null), revision = d.ref(0);
  const syncStatus = d.ref({status: 'idle', done: 0, total: 0});
  // 发行包不带素材缓存（近百 MB），所以「装完第一次启动」必然是 ready=false。
  // 这是正常状态，不是错误：先按联机模式把界面拉起来，后台补齐素材再放开开关。
  const cacheReady = d.ref(false);
  let cacheBootstrapStarted = false;
  let originalCatalog = null, pending = Promise.resolve();
  let statusTimer=null, disposed=false;
  let offlinePolicyInstalled = false;
  const sockets = new Set(), xhrs = new Set(), fetches = new Set();
  const rawFetch = d.window.fetch.bind(d.window);
  const origin = d.origin;
  // 本地图片地址统一走宿主的本机 HTTP 路由：/asset-base 下发前缀，
  // custom-assets/<包>/<文件> 是相对键。官方渲染器只把 http(s)/asset 原样交给
  // <img>，而 Mac 的 asset:// 在这个 WebView 里取不到图（表现为 404），所以不用它。
  // 前缀拿不到就退回宿主自带的转换函数，再不行给空串——宁可这张图不显示，
  // 也不拼出一个必然 404 的地址。
  const modBase = d.ref('');
  const customBase = () => String(modBase.value || offlineData.value?.customAssetBase || '');
  const customAssetUrl = (reference, file) => {
    const base = customBase();
    if (base) return base + String(reference || '').split('/').map(encodeURIComponent).join('/');
    return typeof d.customAssetUrl === 'function'
      ? d.customAssetUrl(reference, file)
      : (/^https?:/i.test(String(reference || '')) ? reference : '');
  };
  const offlineError = () => new Error('本地全图鉴已开启：不能进入联机或把本地图鉴外观写入账号。真实点击和开箱正常。');
  const read = () => { try { return JSON.parse(d.storage.getItem(KEY) || '{}'); } catch { return {}; } };
  const remote = value => {
    try {
      const u = new URL(typeof value === 'string' || value instanceof URL ? String(value) : value.url, d.window.location.href);
      return !(['data:', 'blob:', 'tauri:', 'asset:'].includes(u.protocol) || u.origin === origin || u.origin === d.window.location.origin);
    } catch { return true; }
  };
  function restricted(value, method='GET', socket=false) {
    try {
      const input=typeof value==='string'||value instanceof URL?String(value):value.url;
      const url=new URL(input,d.window.location.href), path=decodeURIComponent(url.pathname).replace(/^\/+/, '');
      if (/^api\/frontend\/rooms(?:\/|$)/i.test(path) || /(?:^|\/)socket\.io(?:\/|$)/i.test(path))return true;
      if(socket && /^(?:ws|wss):$/.test(url.protocol) && url.hostname==='api-mc-wuwatappo.kurogames-global.com')return true;
      if(String(method).toUpperCase()==='GET'||String(method).toUpperCase()==='HEAD')return false;
      return /^api\/frontend\/(?:assets\/(?:synthesis|favorites)|user\/me\/appearance|presets)(?:\/|$)/i.test(path);
    }catch{return false;}
  }
  const save = () => {
    d.storage.setItem(KEY, JSON.stringify({enabled: enabled.value, selection: selection.value, favorites: favorites.value}));
    revision.value++;
  };
  let bootstrapPromise;
  async function ensureLocalToken() {
    if (d.token) return;
    if (!bootstrapPromise) bootstrapPromise=rawFetch(origin+'/bootstrap').then(async response=>{
      if(!response.ok)throw new Error('本地服务身份验证失败');
      d.token=(await response.json()).token;
    }).catch(error=>{bootstrapPromise=null;throw error;});
    await bootstrapPromise;
  }
  async function localRequest(path, body) {
    if (d.request) return d.request(path, body);
    await ensureLocalToken();
    const response = await rawFetch(origin + path, body === undefined ? {} : {
      method: 'POST', headers: {'Content-Type': 'application/json', 'X-Local-Token': d.token}, body: JSON.stringify(body)
    });
    const value = await response.json();
    if (!response.ok) throw new Error(value.error || '本地离线服务不可用');
    return value;
  }
  function stopConnections() {
    for (const socket of sockets) { try { socket.close(); } catch {} }
    for (const xhr of xhrs) { try { xhr.abort(); } catch {} }
    for (const controller of fetches) controller.abort();
    sockets.clear(); xhrs.clear(); fetches.clear();
    d.window.dispatchEvent(new d.window.CustomEvent('wuwatappo-local-mode', {detail: true}));
  }
  function reportDiagnostic(kind, detail) {
    if (!enabled.value) return;
    if (d.request) { d.request('/diagnostic', {kind}).catch(() => {}); return; }
    rawFetch(origin + '/diagnostic', {method: 'POST', headers: {'Content-Type': 'application/json', 'X-Local-Token': d.token},
      body: JSON.stringify({kind, detail, origin: d.window.location.origin, time: Date.now()})}).catch(() => {});
  }
  function installOfflinePolicy() {
    // A local view no longer installs a blanket network CSP. This flag only
    // requests a clean ordinary-account view when the mode is turned off.
    offlinePolicyInstalled = true;
  }
  function installNetworkGuards() {
    d.window.fetch = async (input, options = {}) => {
      const isRemote = restricted(input,options.method||input?.method||'GET');
      if (enabled.value && isRemote) throw offlineError();
      const controller = new AbortController();
      if (options.signal?.aborted) controller.abort();
      const abort = () => controller.abort();
      options.signal?.addEventListener('abort', abort, {once: true});
      if (isRemote) fetches.add(controller);
      try { return await rawFetch(input, {...options, signal: controller.signal}); }
      finally { fetches.delete(controller); options.signal?.removeEventListener('abort', abort); }
    };
    const XHR = d.window.XMLHttpRequest;
    if (XHR) {
      const open = XHR.prototype.open, send = XHR.prototype.send;
      XHR.prototype.open = function(method, url, ...rest) { this.__localRemote = restricted(url,method); return open.call(this, method, url, ...rest); };
      XHR.prototype.send = function(...args) {
        if (enabled.value && this.__localRemote) throw offlineError();
        if (this.__localRemote) { xhrs.add(this); this.addEventListener('loadend', () => xhrs.delete(this), {once: true}); }
        return send.apply(this, args);
      };
    }
    for (const name of ['WebSocket', 'EventSource']) {
      const Original = d.window[name];
      if (!Original) continue;
      const Wrapped = function(url, ...args) {
        const blocked=restricted(url,'GET',name==='WebSocket');
        if (enabled.value && blocked) throw offlineError();
        const instance = new Original(url, ...args);
        if (blocked) { sockets.add(instance); instance.addEventListener?.('close', () => sockets.delete(instance), {once: true}); }
        return instance;
      };
      Object.setPrototypeOf(Wrapped, Original); Wrapped.prototype = Original.prototype;
      d.window[name] = Wrapped;
    }
    if (d.window.navigator?.sendBeacon) {
      const send = d.window.navigator.sendBeacon.bind(d.window.navigator);
      d.window.navigator.sendBeacon = (url, data) => enabled.value && restricted(url,'POST') ? false : send(url, data);
    }
  }
  // Tauri defines its injected invoke property as non-writable. The app's two
  // existing invoke wrapper functions call this guard instead of replacing it.
  function checkInvoke(command, args) {
    if(!enabled.value)return;
    if(!/^plugin:(?:http|websocket)\|/.test(command))return;
    const addresses=[],seen=new WeakSet();let method='GET';
    function visit(value) {
      if(typeof value==='string'){if(/^(?:https?:|wss?:|\/?api\/)/i.test(value))addresses.push(value);return;}
      if(!value||typeof value!=='object'||seen.has(value))return;
      seen.add(value);
      for(const [key,item]of Object.entries(value)){if(key==='method'&&typeof item==='string')method=item;else visit(item);}
    }
    visit(args);
    if(addresses.some(url=>restricted(url,method,/^plugin:websocket\|/.test(command))))throw offlineError();
  }
  async function prepare(refresh = false) {
    if (!offlineData.value || refresh) {
      const data = await localRequest('/offline-data');
      if (!data.catalog || !Array.isArray(data.atlas) || !data.atlas.length || data.atlas.length !== (data.atlas_count ?? d.atlasCount ?? data.atlas.length)) throw new Error('本地图鉴缓存不完整');
      offlineData.value = data;
    }
    return offlineData.value;
  }
  function runtimeCatalog(data) {
    const paths = data.customAssetPaths;
    if (!paths || !Object.keys(paths).length) return data.catalog;
    const catalog = JSON.parse(JSON.stringify(data.catalog));
    function visit(value) {
      if (Array.isArray(value)) { for (const item of value) visit(item); return; }
      if (!value || typeof value !== 'object') return;
      for (const [key, item] of Object.entries(value)) {
        if (key === 'url' && typeof item === 'string' && paths[item]) value[key] = customAssetUrl(item, paths[item]);
        else visit(item);
      }
    }
    visit(catalog);
    return catalog;
  }
  // 背包/图鉴里的 Mod 条目用专属稀有度 class 打标：样式只在本地模式挂到 html 上，
  // 切回官方后立刻失效，账号背包里不会留下任何 Mod 痕迹。
  // 官方 useLoading() 每次 show 都新建 #my-loading、请求结束立刻 remove，而静默更新检查
  // 用的也是同一套 loading：几百毫秒的往返落到屏幕上就是一帧闪光。把出圈压到 300ms 之后，
  // 短请求全程不可见（visibility 一并延后，透明遮罩也不抢点击），真卡住时照旧出圈。
  function installLoadingFade() {
    const doc = d.window.document;
    if (!doc || doc.getElementById('wuwa-loading-fade')) return;
    const style = doc.createElement('style');
    style.id = 'wuwa-loading-fade';
    style.textContent = '#my-loading{animation:wuwa-loading-reveal 1ms linear 300ms both}'
      + '@keyframes wuwa-loading-reveal{from{opacity:0;visibility:hidden}to{opacity:1;visibility:visible}}';
    (doc.head || doc.documentElement).append(style);
  }
  function installModBadge() {
    const doc = d.window.document;
    if (!doc || doc.getElementById('wuwa-mod-badge')) return;
    const style = doc.createElement('style');
    style.id = 'wuwa-mod-badge';
    style.textContent = 'html[data-wuwa-mod] .atlas-thumb--mod{background-image:none;outline:2px dashed #7f77dd;outline-offset:-2px}'
      + 'html[data-wuwa-mod] .atlas-thumb--mod::after{content:"MOD";position:absolute;left:3px;bottom:3px;z-index:2;font:600 9px/1 -apple-system,"Segoe UI",sans-serif;letter-spacing:.3px;padding:2px 4px;border-radius:4px;background:#534ab7;color:#fff}'
      // 排序保持官方原样：之前用 flex order 把 Mod 提到最前，等于在筛选之外又造了一个
      // 官方没有的「MOD 稀有度分组」，和背包的稀有度下拉框对不上，这里撤回。
      ;
    doc.head.append(style);
  }
  function paintModFlag() {
    const root = d.window.document?.documentElement;
    if (!root) return;
    if (enabled.value) root.setAttribute('data-wuwa-mod', '');
    else root.removeAttribute('data-wuwa-mod');
  }
  function assetUrl(reference) {
    const path = offlineData.value?.customAssetPaths?.[reference];
    // 面板缩略图不依赖 offline-data，只要有本机前缀就能按相对键取图。
    return path || customBase() ? customAssetUrl(reference, path || '') : '';
  }
  // 官方名字来自远端字典（按 asset_name 查），本机 Mod 的名字不在字典里，
  // 图鉴/背包会渲染成空或被占位符顶掉。把 Mod 条目的名字登记出来，名字解析处直接回退。
  // 背包「我的预设」左边的预览又是另一条链路：它按名字拼 CDN 地址（character/<名字>.png），
  // Mod 的图只在本地，拼出来必然 404，界面上就是那个「？」。所以这里同时登记名字 → 本机地址，
  // 共享块的 buildAssetUrl 命中后直接返回本机路由，拼不到的名字照旧走官方逻辑。
  function registerModNames(data) {
    const names = new Set(), images = new Map();
    const catalog = data.catalog || {};
    for (const entry of data.atlas || []) {
      // Mod 条目同时带 rarity: "MOD" 和 custom_pack_id；只认其中一个都不够稳，
      // 素材清单换版本时这两个字段未必一起出现。
      if (!entry || !entry.asset_name || (entry.rarity !== 'MOD' && !entry.custom_pack_id)) continue;
      const name = String(entry.asset_name).trim();
      if (!name) continue;
      names.add(name);
      const atlasKind = String(entry.asset_type || '');
      const atlasEntry = catalog[atlasKind]?.[entry.asset_id];
      const mapped = atlasEntry?.height_light_id;
      const primary = mapped == null ? null : catalog[atlasKind.replace(/_atlas$/, '')]?.[mapped];
      const atlasUrl = assetUrl(atlasEntry?.url);
      images.set(name, {atlas: atlasUrl, preview: assetUrl(primary?.url) || atlasUrl});
    }
    d.window.__WUWA_MOD_NAMES__ = names;
    d.window.__WUWA_MOD_ASSET_PATH__ = reference => {
      const raw = String(reference || '').trim().replace(/\.png$/i, '');
      const slash = raw.indexOf('/');
      if (slash < 0) return '';
      const row = images.get(raw.slice(slash + 1));
      if (!row) return '';
      return /_atlas$/.test(raw.slice(0, slash).toLowerCase()) ? row.atlas : row.preview;
    };
  }
  function applyData(data) { registerModNames(data); d.applyOfflineCatalog(runtimeCatalog(data), origin); }
  async function applyMode(on) {
    if (on) {
      enabled.value = true;
      installOfflinePolicy();
      stopConnections();
      if(d.catalogConsumer===false){revision.value++;return;}
      const data = await prepare();
      if (!originalCatalog) originalCatalog = d.snapshotCatalog();
      applyData(data);
    } else {
      enabled.value = false;
      if (originalCatalog) { d.restoreCatalog(originalCatalog); originalCatalog = null; }
      // Restore reactively; reloading would discard a running background driver.
    }
    revision.value++;
    paintModFlag();
  }
  function cacheProgressMessage() {
    const s = syncStatus.value || {};
    if (s.status === 'failed') return '本地素材下载失败：' + (s.error || '请检查网络后点「检查并同步」重试');
    if (s.status === 'running') return '本地素材正在下载 ' + (s.done || 0) + '/' + (s.total || '…') + '，完成后开关自动可用';
    return '本地素材还没准备好，点「检查并同步」开始下载';
  }
  async function setEnabled(on) {
    if (busy.value) return;
    // 素材没就绪时开关必须给出可执行的理由，而不是抛一句「未就绪」。
    if (on && !cacheReady.value) throw new Error(cacheProgressMessage());
    busy.value = true;
    try {
      await d.window.__WUWA_LOCAL_ATLAS__?.assistant?.flushBeforeSwitch(Boolean(on));
      if (on) await prepare(true);
      const response = await localRequest('/mode', {enabled: Boolean(on)});
      if (on && (!response.enabled || !response.mode_verified || response.network_policy!=='rooms-only')) throw new Error('本地模式保护未就绪，未开启本地图鉴');
      await applyMode(Boolean(on));
      save();
    } finally { busy.value = false; }
  }
  function inventory(atlas, bag) {
    revision.value;
    if (!enabled.value) return bag;
    const rows = d.normalizeAtlasList(offlineData.value?.atlas || atlas);
    return rows.map(entry => {
      const type = entry.asset_type.replace(/_atlas$/, '');
      const id = d.resolvePetAssetIdFromAppearanceId(entry.asset_id);
      const meta = d.getAppearanceAssetById(id);
      return {...entry, asset_id: id, asset_type: type,
        role_id: String(meta?.game_role_id || entry.role_id || ''), quantity: 1,
        is_collect: favorites.value[id] ? 1 : 0, owned: true};
    });
  }
  // 图鉴顶部的分类收集进度：只统计真实账号里 owned 的条目，
  // 本地模式全部显示为已拥有，所以面板那边在本地模式下整块隐藏。
  function collectionProgress(rows, loading = false) {
    const all = new Set(), owned = new Set();
    if (!loading) for (const row of Array.isArray(rows) ? rows : []) {
      if (!Number.isSafeInteger(row.asset_id) || row.asset_id <= 0) continue;
      const key = String(row.asset_type) + ':' + row.asset_id;
      all.add(key);
      if (row.owned === true) owned.add(key);
    }
    const total = all.size, collected = owned.size;
    return {collected, total, percent: total ? Math.max(0, Math.min(100, collected / total * 100)) : 0, loading};
  }
  function viewUser(official) {
    if (!enabled.value) return official;
    const s = selection.value;
    let character = s.character || official?.character || null;
    // A clean install has no account appearance yet. Pick a real cached
    // character instead of the official hard-coded online fallback sprite.
    if (!d.getAppearanceAssetById(character?.id)) {
      const first = inventory(offlineData.value?.atlas || [], []).find(item=>item.asset_type==='character_skin');
      if (first) character = {id:first.asset_id,name:first.asset_name};
    }
    return {...official, character,
      accessory: (() => { const item = Object.hasOwn(s, 'accessory') ? s.accessory : official?.accessory; return d.getAppearanceAssetById(item?.id) ? item : null; })(),
      emote: (s.emote || official?.emote || []).filter(item => d.getAppearanceAssetById(item.id)),
      role_id: String(d.getAppearanceAssetById(character?.id)?.game_role_id || official?.role_id || '')};
  }
  function select(item, official, maxEmotes) {
    if (!enabled.value) throw new Error('本地模式尚未开启');
    const user = viewUser(official), s = {...selection.value};
    const asset = {id: item.asset_id, name: item.asset_name};
    const itemRole = String(d.getAppearanceAssetById(item.asset_id)?.game_role_id || '');
    if (item.asset_type !== 'character_skin' && itemRole && itemRole !== '0' && itemRole !== String(user.role_id)) throw new Error('该配件或表情只能用于对应角色');
    if (item.asset_type === 'character_skin') {
      const oldRole = user.role_id;
      s.character = asset;
      const role = String(d.getAppearanceAssetById(asset.id)?.game_role_id || '');
      if (oldRole !== role) {
        s.emote = (user.emote || []).filter(e => !d.getAppearanceAssetById(e.id)?.game_role_id);
        const accessory = user.accessory;
        if (accessory && d.getAppearanceAssetById(accessory.id)?.game_role_id && String(d.getAppearanceAssetById(accessory.id)?.game_role_id) !== role) s.accessory = null;
      }
    } else if (item.asset_type === 'accessory') {
      s.accessory = user.accessory?.id === asset.id ? null : asset;
    } else if (item.asset_type === 'emote') {
      const list = user.emote || [], existing = list.some(e => e.id === asset.id);
      if (!existing && list.length >= maxEmotes) throw new Error('已达到可选表情数量上限');
      s.emote = existing ? list.filter(e => e.id !== asset.id) : [...list, asset];
    }
    selection.value = s; save();
  }
  function favorite(id, value) { favorites.value = {...favorites.value, [id]: Boolean(value)}; save(); }
  function presetAsset(id, type) {
    const meta = d.getAppearanceAssetById(id);
    if (!Number.isSafeInteger(id) || id <= 0 || !meta || meta.asset_type !== type) throw new Error('预设中的素材不可用，请重新选择');
    return {id, name: meta.file_name || meta.asset_name || meta.name || ''};
  }
  function savePresets(rows) {
    d.storage.setItem(PRESET_KEY, JSON.stringify(rows));
    presets.value = rows;
    revision.value++;
  }
  function presetList() { revision.value; return JSON.parse(JSON.stringify(presets.value)); }
  function createLocalPreset(official, limit = 5) {
    if (!enabled.value) throw new Error('本地模式尚未开启');
    const rows = readPresets();
    if (rows.length >= Math.min(5, limit)) throw new Error('本地预设已满，请先删除一个');
    const user = viewUser(official), c = presetAsset(user.character?.id, 'character_skin');
    const a = user.accessory ? presetAsset(user.accessory.id, 'accessory') : null;
    const emotes = (user.emote || []).map(e => presetAsset(e.id, 'emote'));
    const id = Math.min(-Date.now(), ...rows.map(p => p.preset_id - 1));
    const row = {preset_id: id, character_asset_id: c.id, character_asset_name: user.character.name || c.name,
      accessory_asset_id: a?.id || 0, accessory_asset_name: user.accessory?.name || a?.name || '',
      emotes: emotes.map((e, i) => ({asset_id: e.id, asset_name: user.emote[i].name || e.name}))};
    savePresets([...rows, row]);
    return row;
  }
  function deleteLocalPreset(id) {
    if (!enabled.value) throw new Error('本地模式尚未开启');
    const rows = readPresets();
    if (!rows.some(p => p.preset_id === id)) throw new Error('本地预设不存在');
    savePresets(rows.filter(p => p.preset_id !== id));
  }
  function applyLocalPreset(id) {
    if (!enabled.value) throw new Error('本地模式尚未开启');
    const row = readPresets().find(p => p.preset_id === id);
    if (!row) throw new Error('本地预设不存在');
    const character = presetAsset(row.character_asset_id, 'character_skin');
    const accessory = row.accessory_asset_id ? presetAsset(row.accessory_asset_id, 'accessory') : null;
    const emote = (row.emotes || []).map(e => presetAsset(e.asset_id, 'emote'));
    const role = String(d.getAppearanceAssetById(character.id)?.game_role_id || '');
    if (emote.length > 5 || new Set(emote.map(e => e.id)).size !== emote.length) throw new Error('预设表情数量异常');
    for (const asset of [accessory, ...emote].filter(Boolean)) {
      const assetRole = String(d.getAppearanceAssetById(asset.id)?.game_role_id || '');
      if (assetRole && assetRole !== '0' && assetRole !== role) throw new Error('预设中的配件或表情不适用此角色');
    }
    character.name = row.character_asset_name || character.name;
    if (accessory) accessory.name = row.accessory_asset_name || accessory.name;
    emote.forEach((e, i) => e.name = row.emotes[i].asset_name || e.name);
    const before = selection.value;
    selection.value = {character, accessory, emote};
    try { save(); } catch (error) { selection.value = before; throw error; }
  }
  function reorder(ids, official) {
    const list = viewUser(official).emote || [], byId = new Map(list.map(e => [e.id, e]));
    if (ids.length !== list.length || new Set(ids).size !== ids.length || ids.some(id => !byId.has(id))) return;
    selection.value = {...selection.value, emote: ids.map(id => byId.get(id))}; save();
  }
  async function syncCache(force = true) {
    syncStatus.value = await localRequest('/sync', {force});
    while (syncStatus.value.status === 'running') {
      await new Promise(resolve => d.window.setTimeout(resolve, 600));
      syncStatus.value = await localRequest('/sync/status');
    }
    if (syncStatus.value.status === 'failed') throw new Error(syncStatus.value.error || '图鉴同步失败，原缓存保留');
    const status = await localRequest('/status');
    cacheReady.value = status.ready === true;
    if(enabled.value){const data=await prepare(true);applyData(data);revision.value++;}
    if(syncStatus.value.result?.changed){
      const generation=String(syncStatus.value.result.assets_json_ts||syncStatus.value.checked_at);
      if(d.storage.getItem(CACHE_KEY)!==generation)d.storage.setItem(CACHE_KEY,generation);
    }
    return syncStatus.value;
  }
  let resourceRevision = 0;
  async function refreshCustomResources() {
    offlineData.value = null;
    d.storage.setItem(CACHE_KEY, 'mod:' + Date.now() + ':' + (++resourceRevision));
    if (enabled.value && d.catalogConsumer !== false) applyData(await prepare(true));
    revision.value++;
  }
  async function importCustomResources(path) {
    const result = await localRequest('/custom/import', {path});
    await refreshCustomResources();
    return result;
  }
  async function removeCustomResource(id) {
    const result = await localRequest('/custom/remove', {id});
    await refreshCustomResources();
    return result;
  }
  const initial = read();
  enabled.value = initial.enabled === true;
  selection.value = initial.selection || {}; favorites.value = initial.favorites || {};
  installLoadingFade();
  installModBadge();
  installNetworkGuards();
  d.window.addEventListener('securitypolicyviolation', event => reportDiagnostic('policy', {
    directive: event.violatedDirective, blocked: String(event.blockedURI).slice(0, 600)
  }));
  d.window.addEventListener('error', event => {
    if (event.target?.tagName === 'IMG') reportDiagnostic('image_error', {src: String(event.target.currentSrc || event.target.src).slice(0, 600)});
  }, true);
  d.window.addEventListener('storage', event => {
    if(disposed)return;
    if(event.key===CACHE_KEY){
      if(enabled.value&&d.catalogConsumer!==false)prepare(true).then(applyData).then(()=>{revision.value++;}).catch(d.onError);
      else offlineData.value=null;
      return;
    }
    if(event.key===PRESET_KEY){presets.value=readPresets();revision.value++;return;}
    if (event.key !== KEY) return;
    const state = read();
    selection.value = state.selection || {}; favorites.value = state.favorites || {};
    // Serialize mode changes so a stale async enable cannot overwrite a later disable.
    pending = pending.catch(() => {}).then(() => applyMode(state.enabled === true)).catch(d.onError);
    revision.value++;
  });
  async function loadModBase() {
    try { const info = await localRequest('/asset-base'); if (info?.base) modBase.value = String(info.base); } catch {}
  }
  async function waitForHost() {
    let last;
    for(let attempt=0;attempt<40;attempt++) {
      try{const status=await localRequest('/status');await loadModBase();return status;}catch(error){last=error;}
      await new Promise(resolve=>d.window.setTimeout(resolve,500));
    }
    throw last;
  }
  function kickCacheBootstrap() {
    // 宿主启动时已经会自动补；这里再补一刀兜底，正在跑的话宿主会直接返回当前进度。
    if (cacheBootstrapStarted) return;
    cacheBootstrapStarted = true;
    localRequest('/sync', {force: false}).then(status => { syncStatus.value = status; }).catch(() => {});
  }
  const ready = waitForHost().then(async status => {
    syncStatus.value = status.sync || syncStatus.value;
    cacheReady.value = status.ready === true;
    const on = status.enabled === true && status.mode_verified === true && status.network_policy==='rooms-only';
    if (!cacheReady.value) {
      // 全新安装：发行包不带素材缓存（近百 MB），宿主此刻正在后台下载。
      // 这里绝对不能 throw —— ready 是主渲染进程的启动闸门，抛错会把整个界面
      // 替换成一行「请通过本地版启动器运行」，装完就完全没法用。
      if (enabled.value) { enabled.value = false; save(); }
      kickCacheBootstrap();
      return;
    }
    await applyMode(on);
    save();
    if (d.catalogConsumer!==false&&d.window.setTimeout) d.window.setTimeout(() => syncCache(false).catch(() => {}), 2000);
  });
  if (d.window.setInterval) {
    let checkingStatus = false;
    statusTimer=d.window.setInterval(async () => {
      if (checkingStatus || busy.value) return;
      checkingStatus = true;
      try {
        const status = await localRequest('/status');
        if (status.sync) syncStatus.value = status.sync;
        const wasReady = cacheReady.value;
        cacheReady.value = status.ready === true;
        const on = cacheReady.value && status.enabled === true && status.mode_verified === true && status.network_policy==='rooms-only';
        if (on !== enabled.value) { await applyMode(on); save(); }
        else if (!wasReady && cacheReady.value) revision.value++;
      } catch {}
      finally { checkingStatus = false; }
    }, d.catalogConsumer===false?15000:1500);
  }
  function dispose(){disposed=true;if(statusTimer!==null)d.window.clearInterval?.(statusTimer);}
  return {enabled, busy, selection, favorites, revision, ready, cacheReady, setEnabled, inventory, viewUser, select,
    favorite, reorder, prepare, offlineData, assetsHost: origin, offlineError, checkInvoke, syncStatus, syncCache, importCustomResources, removeCustomResource, assetUrl, request: localRequest, dispose,
    presetList, createLocalPreset, deleteLocalPreset, applyLocalPreset, collectionProgress,
    restricted,normalCumulative:()=>d.getAccountCumulative?.()??0,normalPending:()=>d.getAccountPending?.()??0};
}
