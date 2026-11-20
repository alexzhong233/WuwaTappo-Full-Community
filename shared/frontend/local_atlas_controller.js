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
  const MARKS_KEY='wuwatappo-appearance-marks';
  const readMarks=()=>{try{const rows=JSON.parse(d.storage.getItem(MARKS_KEY)||'{}');return rows&&typeof rows==='object'&&!Array.isArray(rows)?rows:{};}catch{return {};}};
  const marks=d.ref(readMarks());
  const offlineData = d.ref(null), revision = d.ref(0);
  const syncStatus = d.ref({status: 'idle', done: 0, total: 0});
  // 发行包不带素材缓存（近百 MB），所以「装完第一次启动」必然是 ready=false。
  // 这是正常状态，不是错误：先按联机模式把界面拉起来，后台补齐素材再放开开关。
  const cacheReady = d.ref(false);
  let cacheBootstrapStarted = false;
  let pending = Promise.resolve();
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
  const resolveAssetsHost = () => String(customBase() || offlineData.value?.assets_host || origin).replace(/\/+$/, '');
  const customAssetUrl = (reference, file) => {
    const clean = String(reference || '').split(/[?#]/)[0].replace(/^\/+/, '');
    const base = customBase();
    if (base && clean) return base + clean.split('/').map(encodeURIComponent).join('/');
    return typeof d.customAssetUrl === 'function'
      ? d.customAssetUrl(reference, file)
      : (/^https?:/i.test(String(reference || '')) ? reference : '');
  };
  const offlineError = () => new Error('本地外观只影响本机显示；官方背包、联机和云端预设仍可正常使用。');
  const CAPTURE_KEY = 'wuwatappo-capture-v1';
  // 挂饰微调：每个挂饰在本机记一组位移与缩放，只在本地模式叠加到官方的 translate 上。
  // 数据只落 localStorage，不进账号、不进房间请求——别人看到的仍是账号里的官方外观。
  const XFORM_KEY = 'wuwatappo-accessory-transform-v1';
  const readTransforms = () => {
    try {
      const rows = JSON.parse(d.storage.getItem(XFORM_KEY) || '{}');
      return rows && typeof rows === 'object' && !Array.isArray(rows) ? rows : {};
    } catch { return {}; }
  };
  const transforms = d.ref(readTransforms());
  const clampNum = (value, low, high, fallback) => {
    const number = Number(value);
    return Number.isFinite(number) ? Math.min(high, Math.max(low, number)) : fallback;
  };
  const currentAccessoryId = user => String(
    (user ? viewUser(user)?.accessory?.id : selection.value.accessory?.id) || '');
  /** 官方渲染层每帧算 translate 时来问这里：返回 null 表示没有本地调整。 */
  function accessoryTransform(user) {
    // 读一次 revision：调用方是官方的 computed，靠这次读取订阅本机改动，
    // 拖动滑杆才能立刻重算，而不是等下一次切页。
    revision.value;
    if (!enabled.value) return null;
    const id = currentAccessoryId(user);
    const row = id ? transforms.value[id] : null;
    if (!row) return null;
    return {dx: clampNum(row.dx, -400, 400, 0), dy: clampNum(row.dy, -400, 400, 0),
      scale: clampNum(row.scale, 0.3, 3, 1), rotate: clampNum(row.rotate, -180, 180, 0)};
  }
  function setAccessoryTransform(patch) {
    if (!enabled.value) throw new Error('本地外观尚未就绪');
    const id = String(selection.value.accessory?.id || '');
    if (!id) throw new Error('先选择一个本地挂饰');
    const row = {...{dx: 0, dy: 0, scale: 1, rotate: 0}, ...(transforms.value[id] || {}), ...(patch || {})};
    const saved = {dx: Math.round(clampNum(row.dx, -400, 400, 0)),
      dy: Math.round(clampNum(row.dy, -400, 400, 0)),
      scale: Math.round(clampNum(row.scale, 0.3, 3, 1) * 100) / 100,
      rotate: Math.round(clampNum(row.rotate, -180, 180, 0))};
    const next = {...transforms.value};
    // 回到默认值就把这条删掉，免得配置里堆一堆全零。
    if (!saved.dx && !saved.dy && saved.scale === 1 && !saved.rotate) delete next[id];
    else next[id] = saved;
    transforms.value = next;
    d.storage.setItem(XFORM_KEY, JSON.stringify(next));
    revision.value++;
    return saved;
  }
  function resetAccessoryTransform() {
    const id = String(selection.value.accessory?.id || '');
    if (!id || !transforms.value[id]) return {dx: 0, dy: 0, scale: 1, rotate: 0};
    const next = {...transforms.value};
    delete next[id];
    transforms.value = next;
    d.storage.setItem(XFORM_KEY, JSON.stringify(next));
    revision.value++;
    return {dx: 0, dy: 0, scale: 1, rotate: 0};
  }
  let accountBag = null;
  let choiceNode = null;
  let refreshSourceBadges = null;
  const localMeta = new Map();
  const atlasIndex = new Map();
  const petIdByAtlasId = new Map();
  const atlasIdsByPetId = new Map();
  const assetMeta = id => localMeta.get(Number(id) || 0) || d.getAppearanceAssetById(Number(id) || 0);
  const resolveItemId = id => petIdByAtlasId.get(Number(id) || 0) || d.resolvePetAssetIdFromAppearanceId(Number(id) || 0);
  const normalizedType = value => String(value || '').replace(/_atlas$/, '');
  const markKey = (id, type) => normalizedType(type) + ':' + (Number(id) || 0);
  function relatedIds(id) {
    const raw = Number(id) || 0, ids = new Set([raw]);
    const primary = petIdByAtlasId.get(raw);
    if (primary) ids.add(primary);
    for (const alias of atlasIdsByPetId.get(raw) || []) ids.add(alias);
    return ids;
  }
  const bagRows = () => Array.isArray(accountBag) ? accountBag : [];
  function officialOwned(id, type) {
    const assetId = resolveItemId(id);
    if (!assetId) return false;
    const wanted = normalizedType(type || assetMeta(assetId)?.asset_type);
    return bagRows().some(row => Number(row.asset_id) === assetId
      && (!wanted || normalizedType(row.asset_type) === wanted));
  }
  function selectedLocal(id, type) {
    const s = selection.value || {}, wanted = normalizedType(type);
    if (wanted === 'character_skin') return Number(s.character?.id) === Number(id);
    if (wanted === 'accessory') return Number(s.accessory?.id) === Number(id);
    if (wanted === 'emote') return (s.emote || []).some(item => Number(item.id) === Number(id));
    return false;
  }
  function markFor(id, type) {
    const result = {local:false, official:false};
    for (const alias of relatedIds(id)) {
      const row = marks.value[markKey(alias, type)] || {};
      result.local ||= row.local === true;
      result.official ||= row.official === true;
    }
    return result;
  }
  function setMark(id, type, source, value = true) {
    const next = {...marks.value};
    for (const alias of relatedIds(id)) {
      const key = markKey(alias, type), row = {...(next[key] || {})};
      row[source] = Boolean(value);
      if (!row.local && !row.official) delete next[key];
      else next[key] = row;
    }
    marks.value = next;
    d.storage.setItem(MARKS_KEY, JSON.stringify(next));
    revision.value++;
    refreshSourceBadges?.();
    return markFor(id, type);
  }
  function sourceFor(id, type) {
    return markFor(id, type);
  }
  function clearLocal(type) {
    const wanted = normalizedType(type), s = {...selection.value};
    if (wanted === 'character_skin') delete s.character;
    else if (wanted === 'accessory') s.accessory = null;
    else if (wanted === 'emote') s.emote = [];
    selection.value = s; save(); revision.value++; refreshSourceBadges?.();
  }
  function installSourceBadges() {
    const doc = d.window.document;
    if (!doc || typeof doc.createElement !== 'function' || doc.getElementById('wuwa-source-badges')) return;
    const style = doc.createElement('style');
    style.id = 'wuwa-source-badges';
    style.textContent = `
      [data-wuwa-marked]{position:relative!important}
      .wuwa-source-tags{position:absolute;left:4px;top:4px;z-index:8;display:flex;gap:3px;pointer-events:none}
      .wuwa-source-tag{padding:2px 5px;border-radius:5px;color:#fff;font:700 9px/1.2 -apple-system,"Segoe UI",sans-serif;letter-spacing:.3px;box-shadow:0 1px 4px #0003}
      .wuwa-source-tag--official{background:#8a6d3b}
      .wuwa-source-tag--local{background:#534ab7}
      .wt-source-choice{position:fixed;z-index:2147483647;right:22px;bottom:22px;width:min(320px,calc(100vw - 44px));padding:14px;border-radius:16px;background:#fffdf8;border:1px solid #e4d9c7;box-shadow:0 14px 44px #2c1f1233;font:13px/1.5 -apple-system,"Segoe UI",sans-serif;color:#3d352d}
      .wt-source-choice strong{display:block;font-size:15px;margin-bottom:3px}.wt-source-choice p{margin:0 0 10px;color:#7d7163}.wt-source-choice .row{display:flex;gap:8px;flex-wrap:wrap}.wt-source-choice button{border:1px solid #d9c8aa;background:#fff;border-radius:9px;padding:7px 12px;cursor:pointer}.wt-source-choice button.primary{background:#efd8ac;border-color:#dfbf83}.wt-source-choice button:disabled{opacity:.45;cursor:not-allowed}
      html[data-wuwa-capture="green"] body{background:#00ff00!important}
      html[data-wuwa-capture="transparent"] body{background:transparent!important}
    `;
    doc.head.append(style);
    const observe = () => {
      if (typeof doc.querySelectorAll !== 'function') return;
      for (const img of doc.querySelectorAll('img')) {
        const raw = String(img.currentSrc || img.src || '').split(/[?#]/)[0];
        const base = decodeURIComponent(raw.split('/').pop() || '').replace(/\.(png|webp|gif|jpe?g)$/i, '').toLowerCase();
        const row = atlasIndex.get(base);
        if (!row) continue;
        const card = img.closest('[class*="atlas"],[class*="bag"],[class*="appearance"],[class*="preset"],[class*="item"]') || img.parentElement;
        if (!card) continue;
        const source = sourceFor(row.asset_id, row.asset_type);
        const existing = Array.from(card.children || []).find(node => node.classList?.contains('wuwa-source-tags'));
        if (!source.local && !source.official) {
          existing?.remove();
          card.removeAttribute('data-wuwa-marked');
          continue;
        }
        card.setAttribute('data-wuwa-marked', '');
        const tags = existing || doc.createElement('span');
        if (!existing) { tags.className='wuwa-source-tags'; tags.setAttribute('aria-hidden','true'); card.append(tags); }
        const signature = `${source.local ? 'l' : ''}${source.official ? 'o' : ''}`;
        if (tags.dataset.signature === signature) continue;
        tags.dataset.signature = signature;
        tags.textContent = '';
        if (source.local) { const tag=doc.createElement('span'); tag.className='wuwa-source-tag wuwa-source-tag--local'; tag.textContent='本地'; tags.append(tag); }
        if (source.official) { const tag=doc.createElement('span'); tag.className='wuwa-source-tag wuwa-source-tag--official'; tag.textContent='官方'; tags.append(tag); }
      }
    };
    let scheduled = false;
    const schedule = () => { if (scheduled) return; scheduled = true; d.window.setTimeout(() => { scheduled = false; observe(); }, 80); };
    refreshSourceBadges = schedule;
    const Observer = d.window.MutationObserver;
    if (typeof Observer === 'function' && doc.documentElement) {
      new Observer(schedule).observe(doc.documentElement, {childList:true,subtree:true,attributes:true,attributeFilter:['src','style']});
    }
    schedule();
  }
  function showAssetChoice(item, official, maxEmotes, applyOfficial) {
    const doc = d.window.document;
    if (!doc || typeof doc.createElement !== 'function' || !doc.body || !enabled.value) return;
    choiceNode?.remove();
    const type = normalizedType(item.asset_type);
    const owned = officialOwned(item.asset_id, type);
    const node = doc.createElement('section');
    if (typeof node.querySelector !== 'function' || !node.append) return;
    node.className = 'wt-source-choice';
    node.innerHTML = `<strong>${owned ? '官方已有外观' : '仅本地外观'}</strong><p>${owned ? '可以只在本机使用，也可以设为官方联机外观；两个标签可以同时存在。' : '该外观不在官方背包中，只能作为本机外观使用。'}</p><div class="row"></div>`;
    const row = node.querySelector('.row');
    const local = doc.createElement('button'); local.className='primary'; local.textContent='设为本地';
    local.onclick = () => {
      try { select(item, official, maxEmotes, {quiet:true}); setMark(item.asset_id, type, 'local', true); }
      catch (error) { node.querySelector('p').textContent = error.message; return; }
      node.remove(); choiceNode=null;
    };
    const close = doc.createElement('button'); close.textContent='关闭'; close.onclick=()=>{node.remove();choiceNode=null;};
    row.append(local);
    if (owned) {
      const officialButton = doc.createElement('button'); officialButton.textContent='设为官方';
      officialButton.onclick = async () => {
        if (typeof applyOfficial !== 'function') { node.querySelector('p').textContent='当前官方界面不支持设置外观'; return; }
        officialButton.disabled = true;
        try { await applyOfficial(); setMark(item.asset_id, type, 'official', true); }
        catch (error) { officialButton.disabled=false; node.querySelector('p').textContent=error?.message || String(error); return; }
        node.remove(); choiceNode=null;
      };
      row.append(officialButton);
    }
    row.append(close);
    doc.body.append(node); choiceNode=node;
  }
  function beginSelect(item, official, maxEmotes, applyOfficial) {
    if (!enabled.value) return false;
    showAssetChoice(item, official, maxEmotes, applyOfficial);
    return true;
  }
  const capture = d.ref((() => { try { const v=JSON.parse(d.storage.getItem(CAPTURE_KEY)||'{}'); return v&&typeof v==='object'?v:{mode:'off',panelOpacity:0.96}; } catch { return {mode:'off',panelOpacity:0.96}; } })());
  function applyCaptureStyle() {
    const root=d.window.document?.documentElement; if(!root||!root.style)return;
    const mode=['green','transparent'].includes(capture.value.mode)?capture.value.mode:'off';
    if(mode==='off')root.removeAttribute('data-wuwa-capture');else root.setAttribute('data-wuwa-capture',mode);
    const opacity=clampNum(capture.value.panelOpacity,0.35,1,0.96);
    root.style.setProperty('--wuwa-panel-opacity',String(opacity));
    root.style.setProperty('--wuwa-panel-bg-image',capture.value.panelBackground?`url("${String(capture.value.panelBackground).replace(/"/g,'%22')}")`:'none');
  }
  function setCapture(patch={}) { capture.value={...capture.value,...patch}; d.storage.setItem(CAPTURE_KEY,JSON.stringify(capture.value)); applyCaptureStyle(); revision.value++; return {...capture.value}; }
  const read = () => { try { return JSON.parse(d.storage.getItem(KEY) || '{}'); } catch { return {}; } };
  const remote = value => {
    try {
      const u = new URL(typeof value === 'string' || value instanceof URL ? String(value) : value.url, d.window.location.href);
      return !(['data:', 'blob:', 'tauri:', 'asset:'].includes(u.protocol) || u.origin === origin || u.origin === d.window.location.origin);
    } catch { return true; }
  };
  // Hybrid mode keeps the official network and multiplayer protocols intact.
  // Local-only appearance is an overlay and is never written into these requests.
  function restricted() { return false; }
  const save = () => {
    d.storage.setItem(KEY, JSON.stringify({enabled: enabled.value, desired: desiredLocal, selection: selection.value, favorites: favorites.value, marks: marks.value}));
    d.storage.setItem(MARKS_KEY, JSON.stringify(marks.value));
    revision.value++;
  };
  // 宿主刚起来、或后台正在跑素材同步时，本机 HTTP 会有短暂不响应。
  // fetch 不设超时的话，这个 pending 会把 waitForHost、面板挂载、更新检查
  // 一起拖死——界面就永远停在「读取中」，看起来像卡 loading。
  function fetchWithTimeout(input, options, timeoutMs = 15000) {
    const Abort = d.window.AbortController;
    if (!Abort) return rawFetch(input, options);
    const controller = new Abort();
    const timer = d.window.setTimeout(() => controller.abort(), timeoutMs);
    return rawFetch(input, {...(options || {}), signal: controller.signal})
      .catch(error => {
        const aborted = error && (error.name === 'AbortError' || error.code === 20);
        throw aborted ? new Error('本机服务响应超时') : error;
      })
      .finally(() => d.window.clearTimeout(timer));
  }
  let bootstrapPromise;
  async function ensureLocalToken() {
    if (d.token) return;
    if (!bootstrapPromise) bootstrapPromise=fetchWithTimeout(origin+'/bootstrap', undefined, 8000).then(async response=>{
      if(!response.ok)throw new Error('本地服务身份验证失败');
      d.token=(await response.json()).token;
    }).catch(error=>{bootstrapPromise=null;throw error;});
    await bootstrapPromise;
  }
  async function localRequest(path, body) {
    if (d.request) return d.request(path, body);
    await ensureLocalToken();
    const response = await fetchWithTimeout(origin + path, body === undefined ? {} : {
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
    if (d.request) { d.request('/diagnostic', {kind, detail}).catch(() => {}); return; }
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
      indexLocalData(data);
    }
    return offlineData.value;
  }
  function imageStem(value) {
    try {
      return decodeURIComponent(String(value || '').split(/[?#]/)[0].split('/').pop() || '')
        .replace(/\.(png|webp|gif|jpe?g)$/i, '').toLowerCase();
    } catch { return ''; }
  }
  function indexLocalData(data) {
    localMeta.clear(); atlasIndex.clear(); petIdByAtlasId.clear(); atlasIdsByPetId.clear();
    const catalog = data?.catalog || {};
    const collectUrls = (value, out, seen = new Set()) => {
      if (!value || typeof value !== 'object' || seen.has(value)) return;
      seen.add(value);
      for (const [key, item] of Object.entries(value)) {
        if (key === 'url' && typeof item === 'string') out.push(item);
        else collectUrls(item, out, seen);
      }
    };
    for (const [rawType, group] of Object.entries(catalog)) {
      if (!group || typeof group !== 'object') continue;
      const type = normalizedType(rawType);
      for (const [rawId, metadata] of Object.entries(group)) {
        const id = Number(rawId) || 0;
        if (!id || !metadata || typeof metadata !== 'object') continue;
        localMeta.set(id, {...metadata, asset_id:id, asset_type:type});
        const primaryId = Number(metadata.height_light_id) || 0;
        if (rawType.endsWith('_atlas') && primaryId > 0) {
          petIdByAtlasId.set(id, primaryId);
          const aliases = atlasIdsByPetId.get(primaryId) || [];
          aliases.push(id); atlasIdsByPetId.set(primaryId, aliases);
        }
      }
    }
    for (const row of data?.atlas || []) {
      const id = Number(row.asset_id) || 0;
      if (!id) continue;
      const type = normalizedType(row.asset_type);
      const meta = {...(catalog[row.asset_type]?.[row.asset_id] || catalog[type]?.[id] || {}), ...row,
        asset_id: id, asset_type: type};
      localMeta.set(id, meta);
      for (const name of [row.asset_name, row.file_name, row.name]) {
        const stem = imageStem(name);
        if (stem) atlasIndex.set(stem, row);
      }
      const urls = [];
      collectUrls(catalog[row.asset_type]?.[row.asset_id], urls);
      collectUrls(catalog[type]?.[id], urls);
      for (const url of urls) {
        const stem = imageStem(url);
        if (stem) atlasIndex.set(stem, row);
      }
    }
  }
  function mergeAtlas(officialAtlas) {
    const rows = Array.isArray(officialAtlas) ? officialAtlas.slice() : [];
    if (!enabled.value || !offlineData.value) return rows;
    const seen = new Set(rows.map(row => markKey(row.asset_id, row.asset_type)));
    for (const row of offlineData.value.atlas || []) {
      const key = markKey(row.asset_id, row.asset_type);
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push({...row, wuwa_local_only:true, owned:false});
    }
    return rows;
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
      // 「我的预设」两类卡片各打一个角标：账号那份是服务器的，本地这份只在本机。
      + 'html[data-wuwa-mod] .preset-item--account,html[data-wuwa-mod] .preset-item--local{position:relative}'
      + 'html[data-wuwa-mod] .preset-item--account::after,html[data-wuwa-mod] .preset-item--local::after{position:absolute;left:3px;top:3px;z-index:3;font:600 9px/1 -apple-system,"Segoe UI",sans-serif;letter-spacing:.3px;padding:2px 4px;border-radius:4px;color:#fff}'
      + 'html[data-wuwa-mod] .preset-item--account::after{content:"官方";background:#8a6d3b}'
      + 'html[data-wuwa-mod] .preset-item--local::after{content:"本地";background:#534ab7}'
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
  function applyData(data) { registerModNames(data); indexLocalData(data); }
  async function applyMode(on) {
    if (on) {
      enabled.value = true;
      installOfflinePolicy();
      if(!modBase.value)await loadModBase();
      const data = await prepare();
      applyData(data);
    } else {
      enabled.value = false;
      // Restore reactively; reloading would discard a running background driver.
    }
    revision.value++;
    paintModFlag();
  }
  function cacheProgressMessage() {
    const s = syncStatus.value || {};
    if (s.status === 'failed') return '本地素材下载失败：' + (s.error || '请检查网络后点「检查并同步」重试');
    if (s.status === 'running') return '本地素材正在下载 ' + (s.done || 0) + '/' + (s.total || '…') + '，完成后会自动启用本地补充';
    return '本地素材还没准备好，点「检查并同步」开始下载';
  }
  async function setEnabled(on) {
    if (busy.value) return;
    desiredLocal = Boolean(on);
    // 素材没就绪时开关必须给出可执行的理由，而不是抛一句「未就绪」。
    if (on && !cacheReady.value) throw new Error(cacheProgressMessage());
    busy.value = true;
    try {
      await d.window.__WUWA_LOCAL_ATLAS__?.assistant?.flushBeforeSwitch(Boolean(on));
      if (on) await prepare(true);
      const response = await localRequest('/mode', {enabled: Boolean(on)});
      if (on && (!response.enabled || !response.mode_verified || !['rooms-only','hybrid'].includes(response.network_policy))) throw new Error('本地外观模式未就绪');
      await applyMode(Boolean(on));
      save();
    } finally { busy.value = false; }
  }
  // 账号背包是「官方拥有」的唯一判据。图鉴页自己把官方图鉴与这份背包
  // 合并成 owned/unowned；本地条目只补充在官方图鉴缺失的位置。
  function inventory(atlas, bag) {
    revision.value;
    if (Array.isArray(bag)) accountBag = bag;
    return bag;
  }
  /** 面板合并图鉴时用的归属索引；取不到就返回 null，调用方回退到官方 bagList。 */
  function ownedBag() { revision.value; return accountBag; }
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
    if (!assetMeta(character?.id)) {
      const first = (offlineData.value?.atlas || []).find(item=>normalizedType(item.asset_type)==='character_skin');
      if (first) character = {id:resolveItemId(first.asset_id),name:first.asset_name};
    }
    return {...official, character,
      accessory: (() => { const item = Object.hasOwn(s, 'accessory') ? s.accessory : official?.accessory; return assetMeta(item?.id) ? item : null; })(),
      emote: (s.emote || official?.emote || []).filter(item => assetMeta(item.id)),
      role_id: String(assetMeta(character?.id)?.game_role_id || official?.role_id || '')};
  }
  function canSelect(item) {
    return Boolean(enabled.value && item?.asset_id);
  }
  function select(item, official, maxEmotes, options={}) {
    if (!enabled.value) throw new Error('本地外观尚未就绪');
    const user = viewUser(official), s = {...selection.value};
    const type = normalizedType(item.asset_type);
    const id = resolveItemId(item.asset_id);
    const meta = assetMeta(id);
    const asset = {id, name: item.asset_name || item.name || meta?.asset_name || meta?.file_name || ''};
    const itemRole = String(meta?.game_role_id || '');
    if (type !== 'character_skin' && itemRole && itemRole !== '0' && itemRole !== String(user.role_id)) throw new Error('该配件或表情只能用于对应角色');
    if (type === 'character_skin') {
      const oldRole = user.role_id;
      s.character = asset;
      const role = String(assetMeta(asset.id)?.game_role_id || '');
      if (oldRole !== role) {
        s.emote = (user.emote || []).filter(e => !assetMeta(e.id)?.game_role_id);
        const accessory = user.accessory;
        if (accessory && assetMeta(accessory.id)?.game_role_id && String(assetMeta(accessory.id)?.game_role_id) !== role) s.accessory = null;
      }
    } else if (type === 'accessory') {
      s.accessory = user.accessory?.id === asset.id ? null : asset;
    } else if (type === 'emote') {
      const list = user.emote || [], existing = list.some(e => e.id === asset.id);
      if (!existing && list.length >= maxEmotes) throw new Error('已达到可选表情数量上限');
      s.emote = existing ? list.filter(e => e.id !== asset.id) : [...list, asset];
    }
    selection.value = s; setMark(id, type, 'local', true); save(); refreshSourceBadges?.();
    return selection.value;
  }
  function afterOfficialSelect(item, official, maxEmotes, applyOfficial) { return beginSelect(item, official, maxEmotes, applyOfficial); }
  function favorite(id, value) { favorites.value = {...favorites.value, [id]: Boolean(value)}; save(); }
  function presetAsset(id, type) {
    const meta = assetMeta(id);
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
    if (!enabled.value) throw new Error('本地外观尚未就绪');
    const rows = readPresets();
    if (rows.length >= Math.min(5, limit)) throw new Error('本地预设已满，请先删除一个');
    const user = viewUser(official), c = presetAsset(user.character?.id, 'character_skin');
    const a = user.accessory ? presetAsset(user.accessory.id, 'accessory') : null;
    const emotes = (user.emote || []).map(e => presetAsset(e.id, 'emote'));
    const id = Math.min(-Date.now(), ...rows.map(p => p.preset_id - 1));
    const row = {preset_id: id, character_asset_id: c.id, character_asset_name: user.character.name || c.name,
      accessory_asset_id: a?.id || 0, accessory_asset_name: user.accessory?.name || a?.name || '',
      emotes: emotes.map((e, i) => ({asset_id: e.id, asset_name: user.emote[i].name || e.name}))};
    row.wuwa_source = 'local';
    savePresets([...rows, row]);
    return row;
  }
  function deleteLocalPreset(id) {
    if (!enabled.value) throw new Error('本地外观尚未就绪');
    // 账号预设是真存在服务器上的，本地模式连的是只读允许清单，删不掉也不该删。
    if (Number(id) > 0) throw new Error('账号预设只能在官方预设列表中删除');
    const rows = readPresets();
    if (!rows.some(p => p.preset_id === id)) throw new Error('本地预设不存在');
    savePresets(rows.filter(p => p.preset_id !== id));
  }
  // 账号预设（官方那份）与本地预设并排展示：本地 id 为负，账号 id 为正。
  // 点账号那条＝把它的搭配复制成本地外观，账号本身不动、也不联网写回。
  let accountPresets = [];
  function shouldCreateLocalPreset(official) {
    const user = viewUser(official);
    const assets = [user?.character, user?.accessory, ...(user?.emote || [])].filter(Boolean);
    return assets.some(item => !officialOwned(item.id, assetMeta(item.id)?.asset_type));
  }
  function mergePresets(localRows, accountRows) {
    accountPresets = (Array.isArray(accountRows) ? accountRows : []).map(row => ({...row, wuwa_source: 'official'}));
    const locals = (Array.isArray(localRows) ? localRows : []).map(row => ({...row, wuwa_source: 'local'}));
    return [...locals, ...accountPresets];
  }
  function applyPreset(id) {
    if (Number(id) < 0) return applyLocalPreset(id);
    if (!enabled.value) throw new Error('本地外观尚未就绪');
    const row = accountPresets.find(item => item.preset_id === Number(id));
    if (!row) throw new Error('账号预设不存在，请先刷新列表');
    const pick = (assetId, name, type) => {
      const meta = assetMeta(Number(assetId) || 0);
      if (!meta || meta.asset_type !== type) throw new Error('该预设里的素材不在本机素材缓存里');
      return {id: meta.id, name: name || meta.file_name || meta.asset_name || ''};
    };
    const character = pick(row.character_asset_id, row.character_asset_name, 'character_skin');
    const accessory = Number(row.accessory_asset_id) > 0
      ? pick(row.accessory_asset_id, row.accessory_asset_name, 'accessory') : null;
    const emote = (Array.isArray(row.emotes) ? row.emotes : [])
      .map(item => Number(item.asset_id) || 0)
      .filter(id2 => assetMeta(id2)?.asset_type === 'emote')
      .slice(0, 5)
      .map(id2 => ({id: id2, name: (Array.isArray(row.emotes) ? row.emotes : [])
        .find(item => Number(item.asset_id) === id2)?.asset_name || ''}));
    selection.value = {character, accessory, emote};
    for (const item of [character, accessory, ...emote].filter(Boolean)) setMark(item.id, assetMeta(item.id)?.asset_type, 'official', true);
    save();
    return selection.value;
  }
  function applyLocalPreset(id) {
    if (!enabled.value) throw new Error('本地外观尚未就绪');
    const row = readPresets().find(p => p.preset_id === id);
    if (!row) throw new Error('本地预设不存在');
    const character = presetAsset(row.character_asset_id, 'character_skin');
    const accessory = row.accessory_asset_id ? presetAsset(row.accessory_asset_id, 'accessory') : null;
    const emote = (row.emotes || []).map(e => presetAsset(e.asset_id, 'emote'));
    const role = String(assetMeta(character.id)?.game_role_id || '');
    if (emote.length > 5 || new Set(emote.map(e => e.id)).size !== emote.length) throw new Error('预设表情数量异常');
    for (const asset of [accessory, ...emote].filter(Boolean)) {
      const assetRole = String(assetMeta(asset.id)?.game_role_id || '');
      if (assetRole && assetRole !== '0' && assetRole !== role) throw new Error('预设中的配件或表情不适用此角色');
    }
    character.name = row.character_asset_name || character.name;
    if (accessory) accessory.name = row.accessory_asset_name || accessory.name;
    emote.forEach((e, i) => e.name = row.emotes[i].asset_name || e.name);
    const before = selection.value;
    selection.value = {character, accessory, emote};
    for (const item of [character, accessory, ...emote].filter(Boolean)) setMark(item.id, assetMeta(item.id)?.asset_type, 'local', true);
    try { save(); } catch (error) { selection.value = before; throw error; }
  }
  function reorder(ids, official) {
    const list = viewUser(official).emote || [], byId = new Map(list.map(e => [e.id, e]));
    if (ids.length !== list.length || new Set(ids).size !== ids.length || ids.some(id => !byId.has(id))) return;
    selection.value = {...selection.value, emote: ids.map(id => byId.get(id))}; save();
  }
  async function synthesizeDuplicates(kind='same-level') {
    const rows = bagRows();
    if (!rows.length) throw new Error('官方背包尚未加载，请先进入背包刷新一次');
    const groups = new Map();
    for (const row of rows) {
      const id = Number(row.asset_id) || 0, quantity = Math.max(1, Number(row.quantity) || 1);
      if (!id || quantity < 2) continue;
      const type = normalizedType(row.asset_type) || 'unknown';
      if (!groups.has(type)) groups.set(type, []);
      groups.get(type).push(...Array(quantity).fill(id));
    }
    if (!groups.size) return {count:0,groups:[]};
    const completed=[];
    for (const [type, ids] of groups) {
      const call = kind === 'all' ? d.synthesisAtlas : d.synthesisAtlasSame;
      if (typeof call !== 'function') throw new Error('官方合成接口不可用，请更新客户端');
      await call(ids);
      completed.push({type,count:ids.length});
    }
    await d.refreshBag?.();
    return {count:completed.reduce((sum,row)=>sum+row.count,0),groups:completed};
  }
  let gameWasRunning=null, gameTimer=null;
  async function gameModeSetting() {
    const live=d.window.__WUWA_LOCAL_ATLAS__?.assistant?.state?.value?.settings;
    if(live&&Object.hasOwn(live,'auto_game_mode'))return live.auto_game_mode!==false;
    try{const state=await localRequest('/assistant/state',{});return state?.settings?.auto_game_mode!==false;}catch{return true;}
  }
  async function pollGameMode() {
    try {
      const status=await localRequest('/game/status');
      const running=status?.running===true, enabled=await gameModeSetting();
      if(!enabled){if(gameWasRunning===true)d.setGameMode?.(false);gameWasRunning=false;return;}
      if(running!==gameWasRunning){d.setGameMode?.(running);gameWasRunning=running;}
    } catch {}
  }
  function startGameWatch() {
    if(gameTimer!==null||!d.window.setInterval)return;
    pollGameMode();
    gameTimer=d.window.setInterval(pollGameMode,3000);
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
  let desiredLocal = initial.desired !== false && initial.enabled !== false;
  enabled.value = false;
  if (initial.marks && typeof initial.marks === 'object' && !Array.isArray(initial.marks)) marks.value = {...initial.marks, ...marks.value};
  selection.value = initial.selection || {}; favorites.value = initial.favorites || {};
  installLoadingFade();
  installModBadge();
  installSourceBadges();
  applyCaptureStyle();
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
    if (state.marks && typeof state.marks === 'object' && !Array.isArray(state.marks)) marks.value = state.marks;
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
  async function activateLocalIfReady() {
    try {
      const status = await localRequest('/status');
      syncStatus.value = status.sync || syncStatus.value;
      cacheReady.value = status.ready === true;
      if (!cacheReady.value) { kickCacheBootstrap(); return false; }
      await prepare(true);
      const response = await localRequest('/mode', {enabled:true});
      if (!response.enabled || !response.mode_verified || !['rooms-only','hybrid'].includes(response.network_policy)) {
        throw new Error('本地外观叠加层未就绪');
      }
      await applyMode(true);
      enabled.value = true;
      desiredLocal = true;
      save();
      startGameWatch();
      return true;
    } catch (error) {
      enabled.value = false;
      save();
      console.warn('[local-gallery] overlay disabled; official UI remains active', error);
      return false;
    }
  }
  const ready = waitForHost().then(status => {
    syncStatus.value = status.sync || syncStatus.value;
    cacheReady.value = status.ready === true;
    if (!cacheReady.value) kickCacheBootstrap();
    return status;
  });
  let overlayPromise = null;
  function startOverlay() {
    if (overlayPromise) return overlayPromise;
    overlayPromise = Promise.resolve()
      .then(() => desiredLocal && cacheReady.value ? activateLocalIfReady() : false)
      .finally(() => { overlayPromise = null; });
    return overlayPromise;
  }
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
        const on = cacheReady.value && status.enabled === true && status.mode_verified === true && ['rooms-only','hybrid'].includes(status.network_policy);
        if (cacheReady.value && desiredLocal && !enabled.value) { await startOverlay(); }
        else if (on !== enabled.value) { await applyMode(on); save(); }
        else if (!wasReady && cacheReady.value) revision.value++;
      } catch {}
      finally { checkingStatus = false; }
    }, d.catalogConsumer===false?15000:1500);
  }
  function dispose(){disposed=true;if(statusTimer!==null)d.window.clearInterval?.(statusTimer);if(gameTimer!==null)d.window.clearInterval?.(gameTimer);choiceNode?.remove();}
  return {enabled, busy, selection, favorites, marks, revision, ready, cacheReady, setEnabled, inventory, viewUser, select, mergeAtlas, canSelect,
    favorite, reorder, prepare, offlineData, get assetsHost() { return resolveAssetsHost(); }, offlineError, checkInvoke, syncStatus, syncCache, importCustomResources, removeCustomResource, assetUrl, request: localRequest, dispose,
    presetList, createLocalPreset, deleteLocalPreset, applyLocalPreset, collectionProgress,
    mergePresets, applyPreset,
    ownedBag, accessoryTransform, setAccessoryTransform, resetAccessoryTransform, activateLocalIfReady, startOverlay,
    officialOwned, sourceFor, markFor, setMark, beginSelect, afterOfficialSelect, clearLocal, shouldCreateLocalPreset,
    synthesizeDuplicates, capture, setCapture, applyCaptureStyle,
    restricted,normalCumulative:()=>d.getAccountCumulative?.()??0,normalPending:()=>d.getAccountPending?.()??0};
}
