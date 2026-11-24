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
  // 线上装备的实时快照：由面板每帧回传（syncPanelState），角标、兼容性和装备栏
  // 都直接读它，而不是靠一次性写入的 marks 累加——否则切过的角色会一直挂着标签。
  const officialUser = d.ref(null), panelState = d.ref(null);
  const MARKS_KEY='wuwatappo-appearance-marks';
  const readMarks=()=>{try{const rows=JSON.parse(d.storage.getItem(MARKS_KEY)||'{}');return rows&&typeof rows==='object'&&!Array.isArray(rows)?rows:{};}catch{return {};}};
  const marks=d.ref(readMarks());
  const offlineData = d.ref(null), revision = d.ref(0);
  const modAssetIds = new Set();
  function isModAsset(value) {
    if (value && typeof value === 'object') {
      if (value.wuwa_local_only || value.custom_pack_id || String(value.rarity || '').toLowerCase() === 'mod') return true;
      const id = Number(value.asset_id ?? value.id ?? 0);
      return Number.isFinite(id) && id > 0 && modAssetIds.has(id);
    }
    const id = Number(value);
    return Number.isFinite(id) && id > 0 && modAssetIds.has(id);
  }
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
  // 显式 id 优先（面板可以给官方装备的挂饰调微调）；否则按「本地选中的 → 线上快照的」顺序回落。
  const currentAccessoryId = (user, explicitId) => {
    if (explicitId) return String(explicitId);
    return String((user ? viewUser(user)?.accessory?.id
      : (selection.value.accessory?.id || officialUser.value?.accessory?.id)) || '');
  };
  const defaultTransform = () => ({dx: 0, dy: 0, scale: 1, rotate: 0});
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
  // 官方挂饰在叠加层还没放开时也能先存微调，所以这里不再要求 enabled；只要求确实有一个挂饰。
  function setAccessoryTransform(patch, explicitId) {
    const id = currentAccessoryId(null, explicitId);
    if (!id) throw new Error('先选择一个挂饰');
    const row = {...defaultTransform(), ...(transforms.value[id] || {}), ...(patch || {})};
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
  function resetAccessoryTransform(explicitId) {
    const id = currentAccessoryId(null, explicitId);
    if (!id || !transforms.value[id]) return defaultTransform();
    const next = {...transforms.value};
    delete next[id];
    transforms.value = next;
    d.storage.setItem(XFORM_KEY, JSON.stringify(next));
    revision.value++;
    return defaultTransform();
  }
  /** 面板列表：把本机存过的微调按可视名称回显，方便逐条删掉。 */
  function listAccessoryTransforms() {
    const rows = [];
    for (const key of Object.keys(transforms.value || {})) {
      const row = transforms.value[key] || {};
      const assetId = Number(key) || 0;
      const meta = assetMeta(assetId);
      const bagMatch = assetId ? bagRows().find(item => Number(item.asset_id) === assetId) : null;
      const name = String(meta?.asset_name || meta?.file_name || meta?.name || bagMatch?.asset_name || '');
      rows.push({id: key, name: name || (key ? '#' + key : ''),
        dx: clampNum(row.dx, -400, 400, 0), dy: clampNum(row.dy, -400, 400, 0),
        scale: clampNum(row.scale, 0.3, 3, 1), rotate: clampNum(row.rotate, -180, 180, 0)});
    }
    return rows;
  }
  function deleteAccessoryTransform(id) {
    const key = String(id ?? '');
    if (!key || !transforms.value[key]) return defaultTransform();
    const next = {...transforms.value};
    delete next[key];
    transforms.value = next;
    d.storage.setItem(XFORM_KEY, JSON.stringify(next));
    revision.value++;
    return defaultTransform();
  }
  let accountBag = null;
  let choiceNode = null;
  const equipmentBars = new Set();
  const notifyEquipmentBars = () => { for (const bar of equipmentBars) bar?.sync ? bar.sync() : bar?.refresh?.(); };
  let refreshSourceBadges = null;
  let badgeObserver = null;
  const isPanelHash = () => String(d.window.location?.hash || '').startsWith('#/panel');
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
    const s = selection.value || {}, wanted = normalizedType(type), ids = relatedIds(id);
    const hit = value => value != null && ids.has(Number(value));
    if (wanted === 'character_skin') return hit(s.character?.id);
    if (wanted === 'accessory') return hit(s.accessory?.id);
    if (wanted === 'emote') return (s.emote || []).some(item => hit(item?.id));
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
  /** 某个类型里已经不再被本地选中的标记就地清掉，避免旧装备的「本地」标签滞留。 */
  function pruneLocalMarks(type) {
    const wanted = normalizedType(type);
    if (!wanted) return;
    const next = {...marks.value};
    let changed = false;
    for (const key of Object.keys(next)) {
      if (!key.startsWith(wanted + ':')) continue;
      const row = next[key];
      if (!row || row.local !== true) continue;
      const keyId = Number(key.slice(wanted.length + 1)) || 0;
      const keep = [...relatedIds(keyId)].some(alias => selectedLocal(alias, wanted));
      if (keep) continue;
      if (row.official === true) next[key] = {...row, local: false};
      else delete next[key];
      changed = true;
    }
    if (!changed) return;
    marks.value = next;
    d.storage.setItem(MARKS_KEY, JSON.stringify(next));
  }
  /** 线上这份装备（官方快照）里是否正挂着该资产；按别名兼容图鉴/背包两套 id。 */
  function officialEquipped(id, type, official = officialUser.value) {
    if (!official) return false;
    const wanted = normalizedType(type), ids = relatedIds(id);
    const hit = value => value != null && ids.has(Number(value));
    if (wanted === 'character_skin') return hit(official.character?.id);
    if (wanted === 'accessory') return hit(official.accessory?.id);
    if (wanted === 'emote') return (official.emote || []).some(item => hit(item?.id));
    return false;
  }
  /**
   * 角标来源：有线上快照时完全实时计算（选品一换立刻反映），
   * 只有离线单测等拿不到快照的场景才回退到持久化的 marks。
   */
  function sourceFor(id, type, official) {
    const live = official || officialUser.value;
    const local = selectedLocal(id, type);
    if (live) return {local, official: officialEquipped(id, type, live)};
    const fallback = markFor(id, type);
    return {local: local || fallback.local, official: fallback.official};
  }
  /** 表情能否装到当前线上角色：空 role_id/0 为全角色通用。 */
  function onlineEmoteCompatibility(item, official = officialUser.value) {
    const assetId = resolveItemId(item?.asset_id ?? item?.id);
    const meta = assetMeta(assetId);
    const rawRole = String(item?.role_id ?? item?.game_role_id ?? meta?.game_role_id ?? '');
    const roleId = rawRole && rawRole !== '0' ? rawRole : '';
    const onlineRoleId = String(official?.role_id || assetMeta(official?.character?.id)?.game_role_id || '');
    if (!roleId) return {compatible: true, universal: true, roleId: '', onlineRoleId, label: '全角色通用'};
    if (!onlineRoleId) return {compatible: true, universal: false, roleId, onlineRoleId, label: '当前线上角色未知'};
    if (roleId === onlineRoleId) return {compatible: true, universal: false, roleId, onlineRoleId, label: '匹配当前线上角色'};
    return {compatible: false, universal: false, roleId, onlineRoleId, label: '与当前线上角色不兼容'};
  }
  function clearLocal(type) {
    const wanted = normalizedType(type), s = {...selection.value};
    if (wanted === 'character_skin') delete s.character;
    else if (wanted === 'accessory') s.accessory = null;
    else if (wanted === 'emote') s.emote = [];
    selection.value = s;
    pruneLocalMarks(wanted);
    save(); revision.value++; refreshSourceBadges?.();
  }
  /** 面板快照回传：官方角色/挂饰/表情一有变化就同步进来。 */
  function syncPanelState(snap) {
    if (!snap || typeof snap !== 'object') return panelState.value;
    panelState.value = snap;
    if (Array.isArray(snap.bagList)) accountBag = snap.bagList;
    if (Object.hasOwn(snap, 'officialUserInfo')) {
      const info = snap.officialUserInfo;
      officialUser.value = info && typeof info === 'object' ? info : null;
    }
    if (Object.hasOwn(snap, 'localSelection') && snap.localSelection && typeof snap.localSelection === 'object' && !Array.isArray(snap.localSelection)) {
      selection.value = snap.localSelection;
    }
    revision.value++;
    refreshSourceBadges?.();
    notifyEquipmentBars();
    return panelState.value;
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
      html[data-wuwa-capture="green"] .cat-wrap{background:#00ff00!important;border-radius:16px!important;box-shadow:0 0 0 18px #00ff00!important}
      html[data-wuwa-panel-styled] .panel-window-root .module-content,
      html[data-wuwa-panel-styled] .wt-console:not([data-embedded=true]){opacity:var(--wuwa-panel-opacity,1)}
      html[data-wuwa-panel-bg] .panel-window-root .module-content__bg{background-image:var(--wuwa-panel-bg-image)!important;background-size:cover!important;background-position:center!important}
      html[data-wuwa-panel-bg] .panel-window-root .module-content__bg::before{opacity:0.15!important}
      html[data-wuwa-panel-bg] .wt-console:not([data-embedded=true]){background-image:var(--wuwa-panel-bg-image)!important;background-size:cover!important;background-position:center!important}
    `;
    doc.head.append(style);
    if (!isPanelHash()) return;
    const observe = () => {
      if (disposed || typeof doc.querySelectorAll !== 'function' || !atlasIndex.size) return;
      const handled = new Set();
      const decorate = (card, assetId, assetType) => {
        if (!card || handled.has(card)) return;
        handled.add(card);
        const source = sourceFor(assetId, assetType);
        const existing = Array.from(card.children || []).find(node => node.classList?.contains('wuwa-source-tags'));
        if (!source.local && !source.official) {
          existing?.remove();
          card.removeAttribute?.('data-wuwa-marked');
          return;
        }
        card.setAttribute?.('data-wuwa-marked', '');
        const tags = existing || doc.createElement('span');
        if (!existing) {
          tags.className = 'wuwa-source-tags';
          tags.setAttribute?.('aria-hidden', 'true');
          card.append(tags);
        }
        const signature = `${source.local ? 'l' : ''}${source.official ? 'o' : ''}`;
        if (tags.dataset?.signature === signature) return;
        if (tags.dataset) tags.dataset.signature = signature;
        tags.textContent = '';
        if (source.local) { const tag=doc.createElement('span'); tag.className='wuwa-source-tag wuwa-source-tag--local'; tag.textContent='本地'; tags.append(tag); }
        if (source.official) { const tag=doc.createElement('span'); tag.className='wuwa-source-tag wuwa-source-tag--official'; tag.textContent='官方'; tags.append(tag); }
      };
      const readAttr = (node, name) => {
        try { return node.getAttribute?.(name) ?? ''; } catch { return ''; }
      };
      // 面板给每个条目注入了 data-wuwa-asset-id/type，直接按 id 定位最准；
      // 旧版本没有这组属性时再回退到图片文件名匹配。
      for (const node of doc.querySelectorAll('[data-wuwa-asset-id][data-wuwa-asset-type]')) {
        const assetId = Number(readAttr(node, 'data-wuwa-asset-id')) || 0;
        const assetType = readAttr(node, 'data-wuwa-asset-type');
        if (!assetId || !assetType) continue;
        decorate(node, assetId, assetType);
      }
      for (const img of doc.querySelectorAll('img')) {
        // 条目根节点已经带 asset 属性时它会走上面那条精确路径，这里不再重复打标。
        if (img.closest?.('[data-wuwa-asset-id][data-wuwa-asset-type]')) continue;
        const raw = String(img.currentSrc || img.src || '').split(/[?#]/)[0];
        const base = decodeURIComponent(raw.split('/').pop() || '').replace(/\.(png|webp|gif|jpe?g)$/i, '').toLowerCase();
        const row = atlasIndex.get(base);
        if (!row) continue;
        const card = img.closest('[class*="atlas"],[class*="bag"],[class*="appearance"],[class*="preset"],[class*="item"]') || img.parentElement;
        if (!card || handled.has(card) || readAttr(card, 'data-wuwa-asset-id')) continue;
        decorate(card, row.asset_id, row.asset_type);
      }
    };
    let scheduled = false;
    const schedule = () => { if (disposed || scheduled) return; scheduled = true; d.window.setTimeout(() => { scheduled = false; observe(); }, 80); };
    refreshSourceBadges = schedule;
    const Observer = d.window.MutationObserver;
    if (typeof Observer === 'function' && doc.documentElement) {
      badgeObserver = new Observer(schedule);
      badgeObserver.observe(doc.documentElement, {childList:true,subtree:true,attributes:true,attributeFilter:['src','style']});
    }
    schedule();
  }
  function showAssetChoice(item, official, maxEmotes, applyOfficial, choose = null, forceOfficial = false) {
    const doc = d.window.document;
    if (!doc || typeof doc.createElement !== 'function' || !doc.body || (!choose && !enabled.value)) return false;
    choiceNode?.remove();
    const type = normalizedType(item.asset_type);
    const isMod = Boolean(item?.wuwa_local_only || item?.custom_pack_id || String(item?.rarity || '').toLowerCase() === 'mod');
    const owned = !isMod && (forceOfficial || item?.owned === true || officialOwned(item.asset_id, type));
    const compat = owned && type === 'emote' ? onlineEmoteCompatibility(item, officialUser.value || official) : null;
    const node = doc.createElement('section');
    if (typeof node.querySelector !== 'function' || !node.append) return false;
    node.className = 'wt-source-choice';
    const description = owned
      ? '可以只在本机使用，也可以设为官方联机外观；两个标签可以同时存在。'
        + (compat ? `线上兼容性：${compat.label}。` : '')
      : '该外观不在官方背包中，只能作为本机外观使用。';
    node.innerHTML = `<strong>${owned ? '官方已有外观' : '仅本地外观'}</strong><p>${description}</p><div class="row"></div>`;
    const row = node.querySelector('.row');
    const local = doc.createElement('button'); local.className='primary'; local.textContent='设为本地';
    local.onclick = () => {
      if (choose) { choose('local'); node.remove(); choiceNode=null; return; }
      try { select(item, official, maxEmotes, {quiet:true}); setMark(item.asset_id, type, 'local', true); }
      catch (error) { node.querySelector('p').textContent = error.message; return; }
      node.remove(); choiceNode=null;
    };
    const close = doc.createElement('button'); close.textContent='关闭'; close.onclick=()=>{node.remove();choiceNode=null;};
    row.append(local);
    if (owned) {
      const officialButton = doc.createElement('button'); officialButton.textContent='设为官方';
      if (compat && !compat.compatible) {
        // 表情只能装到对应角色：不兼容时保留「设为本地」，官方这一路直接禁用并说明原因。
        officialButton.disabled = true;
        officialButton.title = compat.label;
      }
      officialButton.onclick = async () => {
        if (compat && !compat.compatible) return;
        if (choose) { choose('official'); node.remove(); choiceNode=null; return; }
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
    return true;
  }
  function beginSelect(item, official, maxEmotes, applyOfficial, options = {}) {
    const type = normalizedType(item?.asset_type || item?.__wuwaAssetType || item?.__wuwaType || '');
    const id = Number(item?.asset_id) || 0;
    // 装备栏/官方 × 按钮：直接卸载目标那一路，绝不弹「官方已有外观」选择窗。
    if (item?.__wuwaRemove === true) {
      if (item.__wuwaTarget === 'official') {
        if (type === 'character_skin') return true;
        applyOfficial?.();
        return true;
      }
      const s = {...selection.value};
      if (type === 'character_skin') delete s.character;
      else if (type === 'accessory') s.accessory = null;
      else if (type === 'emote') {
        const removeIds = relatedIds(id);
        const currentList = Array.isArray(s.emote)
          ? s.emote
          : (Array.isArray(official?.emote) ? official.emote : (officialUser.value?.emote || []));
        let nextList = id > 0
          ? currentList.filter(entry => !removeIds.has(Number(entry?.id ?? entry?.asset_id)))
          : currentList.slice();
        if (nextList.length === currentList.length && Number.isInteger(item.__wuwaSlotIndex)
          && item.__wuwaSlotIndex >= 0 && item.__wuwaSlotIndex < currentList.length) {
          nextList = currentList.slice();
          nextList.splice(item.__wuwaSlotIndex, 1);
        }
        s.emote = nextList;
      }
      else return true;
      selection.value = s;
      pruneLocalMarks(type);
      save();
      refreshSourceBadges?.();
      notifyEquipmentBars();
      return true;
    }
    if (!enabled.value) return isModAsset(item);
    // 拖拽/装备栏投放：直达目标，不弹选择窗。
    if (item?.__wuwaTarget === 'local') {
      try {
        select(item, official, maxEmotes, {quiet: true, forceEquip: true, slotIndex: item.__wuwaSlotIndex});
      } catch {}
      return true;
    }
    if (item?.__wuwaTarget === 'official') {
      if (item.__wuwaReorder) return false;
      if (isModAsset(item) || item?.owned === false || !officialOwned(item.asset_id, type)) return true;
      if (type === 'emote' && !onlineEmoteCompatibility(item, officialUser.value || official).compatible) return true;
      if (typeof applyOfficial !== 'function') return true;
      applyOfficial();
      return true;
    }
    if (options?.noDialog === true) return false;
    showAssetChoice(item, official, maxEmotes, applyOfficial);
    return true;
  }
  function beginPanelSelect(item, emit, forceOfficial = false) {
    const opened = showAssetChoice(item, null, 8, null, choice => {
      emit({...item, wuwa_choice: choice});
    }, forceOfficial);
    if (!opened) emit(item);
    return opened;
  }
  const capture = d.ref((() => { try { const v=JSON.parse(d.storage.getItem(CAPTURE_KEY)||'{}'); return v&&typeof v==='object'?v:{mode:'off',panelOpacity:0.96}; } catch { return {mode:'off',panelOpacity:0.96}; } })());
  function applyCaptureStyle() {
    const root=d.window.document?.documentElement;
    if(!root||!root.style||typeof root.style.setProperty!=='function')return;
    // 旧配置里的 transparent 已经没有意义（不叠 body 背景），统一回落成关闭。
    const mode=capture.value.mode==='green'?'green':'off';
    if(mode==='green')root.setAttribute('data-wuwa-capture','green');else root.removeAttribute('data-wuwa-capture');
    const opacity=clampNum(capture.value.panelOpacity,0.35,1,0.96);
    root.style.setProperty('--wuwa-panel-opacity',String(opacity));
    if(opacity<0.999)root.setAttribute('data-wuwa-panel-styled','');else root.removeAttribute('data-wuwa-panel-styled');
    const background=String(capture.value.panelBackground||'').trim();
    if(background){
      root.style.setProperty('--wuwa-panel-bg-image',`url("${background.replace(/"/g,'%22')}")`);
      root.setAttribute('data-wuwa-panel-bg','');
    }else{
      root.style.setProperty('--wuwa-panel-bg-image','none');
      root.removeAttribute('data-wuwa-panel-bg');
    }
  }
  function setCapture(patch={}) {
    const next={...capture.value,...patch};
    try {
      d.storage.setItem(CAPTURE_KEY,JSON.stringify(next));
    } catch {
      throw new Error('背景图片过大，请选择 2 MB 以内的图片');
    }
    capture.value=next;
    applyCaptureStyle();
    revision.value++;
    return {...capture.value};
  }
  // ---- 房间成员布局 ----
  // 位置/隐藏/缩放只落本机 localStorage：房间成员是人家的，布局只影响自己这块屏幕。
  const ROOM_LAYOUT_KEY = 'wuwatappo-room-layout-v1';
  const ROOM_ACTION_KEY = 'wuwatappo-room-action-v1';
  const ROOM_LAYOUT_CSS = '.room-cat-shell{opacity:var(--wuwa-room-opacity,1)}'
    + '.room-cat-pet{transform:scale(var(--wuwa-room-scale,1));transform-origin:center bottom}'
    + '.cat-friend__nickname{transform:scale(var(--wuwa-room-name-scale,1));transform-origin:center top}';
  const readRoomLayout = () => {
    try {
      const raw = JSON.parse(d.storage.getItem(ROOM_LAYOUT_KEY) || '{}');
      const row = raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {};
      const positions = row.positions && typeof row.positions === 'object' && !Array.isArray(row.positions) ? row.positions : {};
      const hidden = row.hidden && typeof row.hidden === 'object' && !Array.isArray(row.hidden) ? row.hidden : {};
      return {
        positions: {...positions}, hidden: {...hidden},
        scale: clampNum(row.scale, 0.3, 2, 1),
        opacity: clampNum(row.opacity, 0.15, 1, 1),
        nameScale: clampNum(row.nameScale, 0.5, 2, 1),
        arrangeMode: row.arrangeMode === 'vertical' ? 'vertical' : 'horizontal',
      };
    } catch {
      return {positions: {}, hidden: {}, scale: 1, opacity: 1, nameScale: 1, arrangeMode: 'horizontal'};
    }
  };
  const roomLayout = d.ref(readRoomLayout());
  const copyRoomLayout = () => {
    const layout = roomLayout.value || {};
    return {...layout, positions: {...(layout.positions || {})}, hidden: {...(layout.hidden || {})}};
  };
  const persistRoomLayout = () => d.storage.setItem(ROOM_LAYOUT_KEY, JSON.stringify(roomLayout.value));
  function applyRoomLayoutStyle() {
    const doc = d.window?.document, root = doc?.documentElement;
    let style = doc?.getElementById?.('wuwa-room-layout-style');
    if (!style) {
      style = doc?.createElement?.('style');
      if (style) {
        style.id = 'wuwa-room-layout-style';
        style.textContent = ROOM_LAYOUT_CSS;
        doc?.head?.append?.(style);
      }
    }
    if (!root?.style || typeof root.style.setProperty !== 'function') return;
    const layout = roomLayout.value || {};
    root.style.setProperty('--wuwa-room-scale', String(clampNum(layout.scale, 0.3, 2, 1)));
    root.style.setProperty('--wuwa-room-opacity', String(clampNum(layout.opacity, 0.15, 1, 1)));
    root.style.setProperty('--wuwa-room-name-scale', String(clampNum(layout.nameScale, 0.5, 2, 1)));
  }
  function setRoomLayout(patch = {}) {
    const current = roomLayout.value || {};
    const pick = (key, low, high, fallback) => clampNum(
      Object.hasOwn(patch || {}, key) ? patch[key] : current[key], low, high, clampNum(current[key], low, high, fallback));
    const next = {
      // positions/hidden 只从现有状态复制，外部传进来的脏字段不会覆盖掉别人的布局。
      positions: {...(current.positions && typeof current.positions === 'object' ? current.positions : {})},
      hidden: {...(current.hidden && typeof current.hidden === 'object' ? current.hidden : {})},
      scale: pick('scale', 0.3, 2, 1),
      opacity: pick('opacity', 0.15, 1, 1),
      nameScale: pick('nameScale', 0.5, 2, 1),
      arrangeMode: Object.hasOwn(patch || {}, 'arrangeMode')
        ? (patch.arrangeMode === 'vertical' ? 'vertical' : 'horizontal')
        : (current.arrangeMode === 'vertical' ? 'vertical' : 'horizontal'),
    };
    roomLayout.value = next;
    persistRoomLayout();
    applyRoomLayoutStyle();
    revision.value++;
    return copyRoomLayout();
  }
  /** 拖动结束后记一次落点：同时存下当时的窗口尺寸，换窗口大小时按比例缩放。 */
  function saveRoomMemberPosition(id, left, top) {
    const key = String(id ?? '').trim();
    const x = Number(left), y = Number(top);
    if (!key || !Number.isFinite(x) || !Number.isFinite(y)) return null;
    const saved = {left: Math.round(x), top: Math.round(y),
      w: Number(d.window?.innerWidth) || 1280, h: Number(d.window?.innerHeight) || 720};
    const current = roomLayout.value || {};
    roomLayout.value = {...current, positions: {...(current.positions || {}), [key]: saved}};
    persistRoomLayout();
    return {...saved};
  }
  /** 恢复上次落点；和官方排位重叠超过 30% 就放弃，让官方自己的避让逻辑接手。 */
  function savedRoomMemberPosition(id, others = [], boxW = 160, boxH = 160) {
    const saved = roomLayout.value?.positions?.[String(id ?? '').trim()];
    if (!saved || typeof saved !== 'object') return null;
    const width = Number(boxW) || 160, height = Number(boxH) || 160;
    const savedW = Number(saved.w) || 1280, savedH = Number(saved.h) || 720;
    const left = Math.round((Number(saved.left) || 0) * ((Number(d.window?.innerWidth) || savedW) / savedW));
    const top = Math.round((Number(saved.top) || 0) * ((Number(d.window?.innerHeight) || savedH) / savedH));
    for (const item of Array.isArray(others) ? others : []) {
      const otherLeft = Number(item?.left), otherTop = Number(item?.top);
      if (!Number.isFinite(otherLeft) || !Number.isFinite(otherTop)) continue;
      const overlapW = Math.max(0, Math.min(left + width, otherLeft + width) - Math.max(left, otherLeft));
      const overlapH = Math.max(0, Math.min(top + height, otherTop + height) - Math.max(top, otherTop));
      if ((overlapW * overlapH) / (width * height) > 0.3) return null;
    }
    return {left, top};
  }
  function toggleRoomMemberHidden(id, hidden) {
    const key = String(id ?? '').trim();
    if (!key) return false;
    const current = roomLayout.value || {};
    const nextHidden = {...(current.hidden || {})};
    const hide = hidden === undefined ? !nextHidden[key] : Boolean(hidden);
    if (hide) nextHidden[key] = true; else delete nextHidden[key];
    roomLayout.value = {...current, hidden: nextHidden};
    persistRoomLayout();
    revision.value++;
    return hide;
  }
  const isRoomMemberHidden = id => Boolean(roomLayout.value?.hidden?.[String(id ?? '').trim()]);
  function clearRoomPositions() {
    roomLayout.value = {...(roomLayout.value || {}), positions: {}};
    persistRoomLayout();
    d.storage.setItem(ROOM_ACTION_KEY, JSON.stringify({action: 'reset-positions', ts: Date.now()}));
    revision.value++;
  }
  function requestRoomArrange(mode) {
    setRoomLayout({arrangeMode: mode});
    d.storage.setItem(ROOM_ACTION_KEY, JSON.stringify({
      action: 'arrange', mode: roomLayout.value.arrangeMode, ts: Date.now(),
    }));
  }
  /**
   * 纯计算：把成员摆进视口的网格里，保证任意两格不重叠（间距恒为 gap）。
   * horizontal 从左上按行铺开；vertical 贴着右侧边按列铺开。
   */
  function arrangeRoomMembers(members, mode = 'horizontal', viewport = {}) {
    const rows = (Array.isArray(members) ? members : []).filter(item => item && String(item.id ?? '') !== '');
    if (!rows.length) return [];
    const vw = Number(viewport?.width) || 1280, vh = Number(viewport?.height) || 720;
    const cellW = 132, cellH = 152, gap = 16, margin = 24;
    const vertical = mode === 'vertical';
    const perLine = Math.max(1, Math.floor(((vertical ? vh : vw) - margin * 2 + gap) / ((vertical ? cellH : cellW) + gap)));
    const maxLeft = Math.max(0, vw - cellW), maxTop = Math.max(0, vh - cellH);
    return rows.map((item, index) => {
      const line = Math.floor(index / perLine), slot = index % perLine;
      const left = vertical
        ? Math.min(maxLeft, Math.max(0, vw - margin - cellW - line * (cellW + gap)))
        : Math.min(maxLeft, Math.max(0, margin + slot * (cellW + gap)));
      const top = vertical
        ? Math.min(maxTop, Math.max(0, margin + slot * (cellH + gap)))
        : Math.min(maxTop, Math.max(0, margin + line * (cellH + gap)));
      return {id: item.id, left, top};
    });
  }
  // ---- 低开销渲染 ----
  const PERF_KEY = 'wuwatappo-perf-v1';
  const readPerf = () => {
    try {
      const raw = JSON.parse(d.storage.getItem(PERF_KEY) || '{}');
      const row = raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {};
      const cap = Number(row.fpsCap);
      return {
        fpsCap: [0, 30, 60].includes(cap) ? cap : 60,
        idlePause: row.idlePause !== false,
        hideMainPet: row.hideMainPet === true,
        reduceMotion: row.reduceMotion === true,
      };
    } catch { return {fpsCap: 60, idlePause: true, hideMainPet: false, reduceMotion: false}; }
  };
  const perf = d.ref(readPerf());
  const PERF_CSS = 'html[data-wuwa-hide-pet] .cat-stack>img,html[data-wuwa-hide-pet] .local-emotion-bubble-layer{display:none!important}'
    + 'html[data-wuwa-reduce-motion] .local-emotion-bubble,html[data-wuwa-reduce-motion] .cat-mainbox,html[data-wuwa-reduce-motion] .room-cat-shell{animation-duration:0.01ms!important;transition-duration:0.01ms!important}';
  function applyPerfStyle() {
    const doc = d.window?.document, root = doc?.documentElement;
    let style = doc?.getElementById?.('wuwa-perf-style');
    if (!style) {
      style = doc?.createElement?.('style');
      if (style) {
        style.id = 'wuwa-perf-style';
        style.textContent = PERF_CSS;
        doc?.head?.append?.(style);
      }
    }
    if (!root || typeof root.setAttribute !== 'function') return;
    if (perf.value?.hideMainPet) root.setAttribute('data-wuwa-hide-pet', '');
    else root.removeAttribute?.('data-wuwa-hide-pet');
    if (perf.value?.reduceMotion) root.setAttribute('data-wuwa-reduce-motion', '');
    else root.removeAttribute?.('data-wuwa-reduce-motion');
  }
  function setPerf(patch = {}) {
    const current = perf.value || {};
    const pick = (key, fallback) => Object.hasOwn(patch || {}, key) ? Boolean(patch[key]) : Boolean(current[key] ?? fallback);
    const cap = Object.hasOwn(patch || {}, 'fpsCap') ? Number(patch.fpsCap) : Number(current.fpsCap);
    const next = {
      fpsCap: [0, 30, 60].includes(cap) ? cap : 60,
      idlePause: pick('idlePause', true),
      hideMainPet: pick('hideMainPet', false),
      reduceMotion: pick('reduceMotion', false),
    };
    perf.value = next;
    d.storage.setItem(PERF_KEY, JSON.stringify(next));
    applyPerfStyle();
    revision.value++;
    return {...next};
  }
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
    notifyEquipmentBars();
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
    localMeta.clear(); atlasIndex.clear(); petIdByAtlasId.clear(); atlasIdsByPetId.clear(); modAssetIds.clear();
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
      if (String(row?.rarity || '').toLowerCase() === 'mod' || Boolean(row?.custom_pack_id)) {
        modAssetIds.add(id);
        const primaryId = Number(catalog[row.asset_type]?.[row.asset_id]?.height_light_id ?? petIdByAtlasId.get(id) ?? 0);
        if (primaryId > 0) modAssetIds.add(primaryId);
      }
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
  /**
   * 背包展示层：把本机 Mod 以「数量 1、稀有度 mod」的伪条目前置合入，
   * 只在本地叠加开启时生效。accountBag 本身保持纯官方数据，绝不被污染。
   */
  function mergeBag(officialBag) {
    if (!enabled.value || !offlineData.value) return officialBag;
    const modRows = [], seen = new Set();
    for (const entry of offlineData.value.atlas || []) {
      if (!entry) continue;
      const isMod = String(entry.rarity || '').toLowerCase() === 'mod' || Boolean(entry.custom_pack_id);
      if (!isMod) continue;
      const primaryId = resolveItemId(entry.asset_id) || Number(entry.asset_id) || 0;
      if (!primaryId) continue;
      const type = normalizedType(entry.asset_type);
      const key = markKey(primaryId, type);
      if (seen.has(key)) continue;
      seen.add(key);
      const meta = assetMeta(primaryId);
      modRows.push({
        asset_id: primaryId,
        asset_code: entry.asset_code || '',
        asset_type: type,
        asset_name: entry.asset_name || entry.name || meta?.asset_name || '',
        rarity: 'mod',
        role_id: String(entry.role_id ?? entry.game_role_id ?? meta?.game_role_id ?? '0'),
        series_name: '',
        star_level: 0,
        quantity: 1,
        is_collect: (favorites.value[primaryId] ?? entry.is_collect) ? 1 : 0,
        region_code: entry.region_code || '',
        wuwa_local_only: true,
        custom_pack_id: entry.custom_pack_id || '',
      });
    }
    if (!modRows.length) return officialBag;
    return [...modRows, ...(Array.isArray(officialBag) ? officialBag : [])];
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
  function rewriteModCatalogUrls(value, paths) {
    if (Array.isArray(value)) return value.map(item => rewriteModCatalogUrls(item, paths));
    if (!value || typeof value !== 'object') return value;
    const out = {};
    for (const [key, item] of Object.entries(value)) {
      if (key === 'url' && typeof item === 'string' && paths[item]) out[key] = customAssetUrl(item, paths[item]);
      else out[key] = rewriteModCatalogUrls(item, paths);
    }
    return out;
  }
  function modCatalogPayload(data) {
    const catalog = data?.catalog || {};
    const paths = data?.customAssetPaths || {};
    const payload = {
      character_skin: {}, accessory: {}, emote: {},
      character_skin_atlas: {}, accessory_atlas: {}, emote_atlas: {},
    };
    const seen = new Set();
    for (const entry of data?.atlas || []) {
      if (!entry || (String(entry.rarity || '').toLowerCase() !== 'mod' && !entry.custom_pack_id)) continue;
      const atlasKind = String(entry.asset_type || '');
      const primaryKind = atlasKind.replace(/_atlas$/, '');
      const atlasId = Number(entry.asset_id) || 0;
      const atlasRow = catalog[atlasKind]?.[atlasId];
      if (atlasId > 0 && payload[atlasKind] && atlasRow) {
        const key = `${atlasKind}:${atlasId}`;
        if (!seen.has(key)) {
          seen.add(key);
          payload[atlasKind][atlasId] = rewriteModCatalogUrls(atlasRow, paths);
        }
      }
      const primaryId = Number(atlasRow?.height_light_id ?? resolveItemId(atlasId)) || 0;
      const primaryRow = catalog[primaryKind]?.[primaryId];
      if (primaryId > 0 && payload[primaryKind] && primaryRow) {
        const key = `${primaryKind}:${primaryId}`;
        if (!seen.has(key)) {
          seen.add(key);
          payload[primaryKind][primaryId] = rewriteModCatalogUrls(primaryRow, paths);
        }
      }
    }
    return payload;
  }
  function syncModCatalog() {
    d.mergeModCatalog?.(enabled.value && offlineData.value ? modCatalogPayload(offlineData.value) : null, resolveAssetsHost());
  }
  function applyData(data) { registerModNames(data); indexLocalData(data); syncModCatalog(); }
  async function applyMode(on) {
    if (on) {
      enabled.value = true;
      installOfflinePolicy();
      if(!modBase.value)await loadModBase();
      const data = await prepare();
      applyData(data);
    } else {
      enabled.value = false;
      syncModCatalog();
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
    if (!enabled.value && !options.allowDisabled) throw new Error(cacheReady.value ? '本地外观尚未就绪' : cacheProgressMessage());
    const user = viewUser(official), s = {...selection.value};
    const type = normalizedType(item.asset_type);
    const id = resolveItemId(item.asset_id);
    const meta = assetMeta(id);
    const asset = {id, name: item.asset_name || item.name || meta?.asset_name || meta?.file_name || ''};
    const itemRole = String(meta?.game_role_id || '');
    // 表情不做角色限制：本地预览本来就可以自由试装。
    if (type === 'accessory' && itemRole && itemRole !== '0' && itemRole !== String(user.role_id)) throw new Error('该配件只能用于对应角色');
    if (type === 'character_skin') {
      s.character = asset;
      const role = String(assetMeta(asset.id)?.game_role_id || '');
      // 表情保留不动；只有确实不适用新角色的挂饰才摘掉。
      const accessory = user.accessory;
      if (role && accessory && assetMeta(accessory.id)?.game_role_id
        && String(assetMeta(accessory.id)?.game_role_id) !== role) s.accessory = null;
    } else if (type === 'accessory') {
      s.accessory = user.accessory?.id === asset.id ? null : asset;
    } else if (type === 'emote') {
      const limit = Number(maxEmotes) > 0 ? Number(maxEmotes) : 5;
      const list = Array.isArray(user.emote) ? user.emote.slice() : [];
      const index = list.findIndex(e => Number(e?.id) === Number(asset.id));
      const slot = Number.isInteger(options.slotIndex)
        ? Math.max(0, Math.min(limit - 1, options.slotIndex))
        : -1;
      if (index >= 0) {
        if (!options.forceEquip) list.splice(index, 1);
        else if (slot >= 0 && slot !== index) {
          list.splice(index, 1);
          list.splice(Math.min(slot, list.length), 0, asset);
        }
      } else if (list.length >= limit) {
        throw new Error('已达到可选表情数量上限');
      } else if (slot >= 0 && slot < list.length) {
        list[slot] = asset;
      } else {
        list.push(asset);
      }
      s.emote = list;
    }
    selection.value = s;
    // 同类型里被换下的旧外观不再挂「本地」标签。
    pruneLocalMarks(type);
    setMark(id, type, 'local', true);
    save(); refreshSourceBadges?.();
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
    const targetId = Number(id);
    if (targetId > 0) throw new Error('账号预设只能在官方预设列表中删除');
    const rows = readPresets();
    if (!rows.some(p => Number(p.preset_id) === targetId)) throw new Error('本地预设不存在');
    savePresets(rows.filter(p => Number(p.preset_id) !== targetId));
  }
  // 账号预设（官方那份）与本地预设并排展示：本地 id 为负，账号 id 为正。
  // 点账号那条＝把它的搭配复制成本地外观，账号本身不动、也不联网写回。
  let accountPresets = [];
  function shouldCreateLocalPreset(official) {
    const user = viewUser(official);
    const assets = [user?.character, user?.accessory, ...(user?.emote || [])].filter(Boolean);
    return assets.some(item => isModAsset(item.id) || !officialOwned(item.id, assetMeta(item.id)?.asset_type));
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
    for (const type of ['character_skin', 'accessory', 'emote']) pruneLocalMarks(type);
    for (const item of [character, accessory, ...emote].filter(Boolean)) setMark(item.id, assetMeta(item.id)?.asset_type, 'official', true);
    save(); refreshSourceBadges?.();
    return selection.value;
  }
  function applyLocalPreset(id) {
    if (!enabled.value) throw new Error('本地外观尚未就绪');
    const targetId = Number(id);
    const row = readPresets().find(p => Number(p.preset_id) === targetId);
    if (!row) throw new Error('本地预设不存在');
    const character = presetAsset(row.character_asset_id, 'character_skin');
    const accessory = row.accessory_asset_id ? presetAsset(row.accessory_asset_id, 'accessory') : null;
    const emote = (row.emotes || []).map(e => presetAsset(e.asset_id, 'emote'));
    const role = String(assetMeta(character.id)?.game_role_id || '');
    if (emote.length > 5 || new Set(emote.map(e => e.id)).size !== emote.length) throw new Error('预设表情数量异常');
    // 表情允许跨角色试装，只有挂饰仍需要角色匹配。
    if (accessory) {
      const assetRole = String(assetMeta(accessory.id)?.game_role_id || '');
      if (assetRole && assetRole !== '0' && assetRole !== role) throw new Error('预设中的配件不适用此角色');
    }
    character.name = row.character_asset_name || character.name;
    if (accessory) accessory.name = row.accessory_asset_name || accessory.name;
    emote.forEach((e, i) => e.name = row.emotes[i].asset_name || e.name);
    const before = selection.value;
    selection.value = {character, accessory, emote};
    for (const type of ['character_skin', 'accessory', 'emote']) pruneLocalMarks(type);
    for (const item of [character, accessory, ...emote].filter(Boolean)) setMark(item.id, assetMeta(item.id)?.asset_type, 'local', true);
    try { save(); refreshSourceBadges?.(); } catch (error) { selection.value = before; throw error; }
  }
  function reorder(ids, official) {
    const list = viewUser(official).emote || [], byId = new Map(list.map(e => [e.id, e]));
    if (ids.length !== list.length || new Set(ids).size !== ids.length || ids.some(id => !byId.has(id))) return;
    selection.value = {...selection.value, emote: ids.map(id => byId.get(id))}; save();
  }
  /**
   * 批量合成只处理官方背包里「数量 > 1」的多余副本（保留本体），
   * 本地/Mod 条目不参与；按 类型:稀有度 分组后整批提交（同阶 5、升阶 10），
   * 不足一批的余数留在背包里，避免官方接口因批大小不合法报错。
   */
  async function synthesizeDuplicates(kind = 'same-level') {
    const startedAt = Date.now();
    // 合成一次要连打好几批官方接口，失败时只看到一句报错很难查。
    // 这里把批次结果收齐，成功/失败都在 finally 里报一条诊断（不含任何 token/cookie）。
    let beforeExcess = 0, batchesAttempted = 0, synthError = null;
    const results = [];
    const result = {count: 0, batches: 0, groups: []};
    try {
      if (typeof d.fetchBag === 'function') {
        try { const fresh = await d.fetchBag(); if (Array.isArray(fresh)) accountBag = fresh; }
        catch { /* 拉取失败就沿用当前背包 */ }
      }
      const rows = bagRows();
      if (!rows.length) throw new Error('官方背包尚未加载，请先登录并进入背包刷新一次');
      const isUpgrade = kind === 'upgrade' || kind === 'all';
      const batchSize = isUpgrade ? 10 : 5;
      const call = isUpgrade ? d.synthesisAtlas : d.synthesisAtlasSame;
      if (typeof call !== 'function') throw new Error('官方合成接口不可用，请更新客户端');
      const groups = new Map();
      for (const row of rows) {
        if (row.wuwa_local_only || row.custom_pack_id) continue;
        const rarity = String(row.rarity || '').trim().toLowerCase();
        if (rarity === 'mod') continue;
        const id = Number(row.asset_id) || 0;
        if (!id) continue;
        const excess = Math.max(0, Math.floor(Number(row.quantity) || 1) - 1);
        if (excess <= 0) continue;
        if (isUpgrade && rarity === 'epic') continue;
        beforeExcess += excess;
        const type = normalizedType(row.asset_type) || 'unknown';
        const key = `${type}:${rarity}`;
        if (!groups.has(key)) groups.set(key, []);
        const bucket = groups.get(key);
        for (let i = 0; i < excess; i++) bucket.push(id);
      }
      for (const [key, ids] of groups) {
        const usable = Math.floor(ids.length / batchSize) * batchSize;
        for (let offset = 0; offset < usable; offset += batchSize) {
          const batch = ids.slice(offset, offset + batchSize);
          batchesAttempted++;
          try { await call(batch); }
          catch (error) {
            results.push({key, count: batch.length, ok: false, error: error?.message || String(error)});
            throw error;
          }
          results.push({key, count: batch.length, ok: true});
          result.batches++;
          result.count += batch.length;
        }
        if (usable > 0) {
          const [type, rarity] = key.split(':');
          result.groups.push({key, type, rarity, count: usable});
        }
      }
      if (result.batches > 0) {
        await d.refreshBag?.();
        await d.onSynthesisComplete?.();
        if (typeof d.fetchBag === 'function') {
          try { const fresh = await d.fetchBag(); if (Array.isArray(fresh)) accountBag = fresh; }
          catch { /* 背包稍后由主窗口的刷新覆盖 */ }
        }
      }
      return result;
    } catch (error) {
      synthError = error?.message || String(error);
      throw error;
    } finally {
      localRequest('/diagnostic/synth', {
        kind, startedAt, durationMs: Date.now() - startedAt, beforeExcess, batchesAttempted,
        consumed: result.count, batches: result.batches, groups: result.groups, results,
        ...(synthError ? {error: synthError} : {}),
      }).catch?.(() => {});
    }
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
  const EQUIP_WING_PX = 472;
  let sessionEquipDockOpen = false;
  let appliedWingPx = 0;
  let wingQueue = Promise.resolve();
  let wingCollapseTimer = null;
  function clearWingStyle(root) {
    root?.removeAttribute?.('data-wuwa-equip-wing');
    if (typeof root?.style?.removeProperty === 'function') root.style.removeProperty('--wuwa-equip-wing');
    else root?.style?.setProperty?.('--wuwa-equip-wing', '0px');
  }
  function applyPanelWing(targetPx) {
    wingQueue = wingQueue.catch(() => {}).then(async () => {
      if (disposed && targetPx > 0) targetPx = 0;
      if (targetPx === appliedWingPx) return;
      const doc = d.window.document;
      const root = doc?.documentElement;
      const invoke = d.window.__TAURI_INTERNALS__?.invoke;
      const label = d.window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label || 'panel';
      if (typeof invoke !== 'function') {
        appliedWingPx = 0;
        clearWingStyle(root);
        return;
      }
      const delta = targetPx - appliedWingPx;
      try {
        const [phys, scale] = await Promise.all([
          invoke('plugin:window|inner_size', {label}),
          invoke('plugin:window|scale_factor', {label}),
        ]);
        const sf = Number(scale) > 0 ? Number(scale) : 1;
        const physW = Number(phys?.Physical?.width ?? phys?.width) || 0;
        const physH = Number(phys?.Physical?.height ?? phys?.height) || 0;
        const curW = physW / sf;
        const curH = Math.round(physH / sf);
        if (curW <= 0 || curH <= 0) return;
        const nextW = Math.max(600, Math.round(curW + delta));
        if (targetPx > 0) {
          root?.style?.setProperty?.('--wuwa-equip-wing', targetPx + 'px');
          root?.setAttribute?.('data-wuwa-equip-wing', '1');
          await invoke('plugin:window|set_size', {label, value: {Logical: {width: nextW, height: curH}}});
          appliedWingPx = targetPx;
        } else {
          await invoke('plugin:window|set_size', {label, value: {Logical: {width: nextW, height: curH}}});
          appliedWingPx = 0;
          clearWingStyle(root);
        }
      } catch {
        if (appliedWingPx === 0) clearWingStyle(root);
      }
    });
    return wingQueue;
  }
  function setPanelWing(wantOpen) {
    if (wingCollapseTimer !== null) {
      d.window.clearTimeout?.(wingCollapseTimer);
      wingCollapseTimer = null;
    }
    return applyPanelWing(wantOpen ? EQUIP_WING_PX : 0);
  }
  d.window.addEventListener?.('pagehide', () => {
    if (wingCollapseTimer !== null) {
      d.window.clearTimeout?.(wingCollapseTimer);
      wingCollapseTimer = null;
    }
    if (appliedWingPx > 0) applyPanelWing(0);
  });
  /**
   * 图鉴/背包内的「本地 vs 线上」装备管理栏。
   * 本地一栏直接改 selection（仅本机可见）；线上栏只负责发起官方外观操作。
   * 悬浮模式挂到窗口外层（document.body），不插入或挤压官方内容区。
   * 拖拽用 Pointer Events + 8px 阈值，避免把普通点击误判成拖动。
   */
  function mountEquipmentBar(mount, options = {}) {
    const doc = d.window.document;
    const host = typeof mount === 'string' ? doc?.querySelector?.(mount) : mount;
    if (!doc || !host || typeof doc.createElement !== 'function' || !host.append) return () => {};
    if (host.__wuwaEquipBar && typeof host.__wuwaEquipBar.destroy === 'function') host.__wuwaEquipBar.destroy();
    const isFloating = options.floating === true;
    if (isFloating && wingCollapseTimer !== null) {
      d.window.clearTimeout?.(wingCollapseTimer);
      wingCollapseTimer = null;
    }
    let dockOpen = isFloating ? sessionEquipDockOpen : true;
    let dragAutoOpen = false;
    let autoCloseTimer = null;
    const emit = payload => {
      if (typeof options.emitSelect !== 'function') return undefined;
      try { return options.emitSelect(payload); }
      catch (error) { status(error?.message || String(error), 'error'); return undefined; }
    };
    const typeLabel = type => type === 'character_skin' ? '角色' : (type === 'accessory' ? '挂饰' : '表情');
    const liveOfficial = () => officialUser.value || panelState.value?.officialUserInfo || null;
    let statusTimer = null;

    const ensureStyle = () => {
      if (doc.getElementById?.('wuwa-equip-bar-style')) return;
      const style = doc.createElement('style');
      style.id = 'wuwa-equip-bar-style';
      style.textContent = `
        html[data-wuwa-equip-wing] .panel-window-root{width:calc(100% - var(--wuwa-equip-wing,0px))!important}
        html[data-wuwa-equip-wing] .wt-source-choice{right:calc(var(--wuwa-equip-wing,0px) + 22px)}
        .wuwa-equip-dock{position:fixed;right:6px;top:50%;transform:translateY(-50%);z-index:2147483640;max-height:calc(100vh - 28px);display:flex;flex-direction:row;align-items:center;gap:8px;pointer-events:none}
        html[data-wuwa-equip-wing] .wuwa-equip-dock{left:calc(100% - var(--wuwa-equip-wing,472px) + 8px);right:8px;width:auto}
        .wuwa-equip-toggle{pointer-events:auto;flex-shrink:0;writing-mode:vertical-rl;text-orientation:mixed;letter-spacing:2px;border:1px solid #d9c8aa;background:#fffdf8f5;color:#7d5329;border-radius:12px;padding:14px 8px;font:700 13px/1.2 -apple-system,"Segoe UI",sans-serif;cursor:pointer;box-shadow:0 6px 20px #2c1f1229;backdrop-filter:blur(6px)}
        .wuwa-equip-toggle:hover{background:#fff6e5;border-color:#c9a66b}
        .wuwa-equip-dock .wuwa-equip-bar{pointer-events:auto;margin:0;width:min(416px,calc(100vw - 56px));max-height:calc(100vh - 32px);overflow-y:auto;padding:16px;border-radius:16px;box-shadow:0 16px 40px #2c1f123d;background:#fffdf8fa;backdrop-filter:blur(10px);display:flex;flex-direction:column;gap:12px}
        .wuwa-equip-bar{display:flex;flex-direction:column;gap:12px;padding:16px;margin:6px 0;border-radius:16px;background:#fffdf8cc;border:1px solid #e4d9c7;color:#3d352d;font:13px/1.35 -apple-system,"Segoe UI",sans-serif;flex-shrink:0;box-sizing:border-box}
        .wuwa-equip-status{min-height:18px;font-size:12px;color:#7d7163}
        .wuwa-equip-status:empty{display:none}
        .wuwa-equip-status.is-error{color:#b5443a}
        .wuwa-equip-grid{display:flex;flex-direction:column;gap:14px}
        .wuwa-equip-bar h4{margin:0 0 10px;font-size:13px;font-weight:700;letter-spacing:.3px;color:#65513d}
        .wuwa-equip-row{min-width:0}
        .wuwa-equip-row:first-of-type{border-bottom:1px dashed #e4d9c7;padding-bottom:14px}
        .wuwa-equip-slots{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px}
        .wuwa-equip-slot{position:relative;min-width:0;min-height:106px;border-radius:12px;border:1px dashed #d9c8aa;background:#fff;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:5px;padding:8px 6px;box-sizing:border-box;overflow:hidden}
        .wuwa-equip-slot.is-filled{border-style:solid;border-color:#dfbf83}
        .wuwa-equip-slot.is-follow{opacity:.72}
        .wuwa-equip-slot .wuwa-equip-thumb{max-width:100%;max-height:58px;border-radius:7px;object-fit:contain}
        .wuwa-equip-slot .wuwa-equip-initial{font:700 22px/1 -apple-system,"Segoe UI",sans-serif;color:#8a6d3b}
        .wuwa-equip-slot .wuwa-equip-name{max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px;line-height:1.25;color:#5c5145}
        .wuwa-equip-slot .wuwa-equip-remove{position:absolute;right:3px;top:3px;z-index:4;width:22px;height:22px;display:flex;align-items:center;justify-content:center;line-height:1;border-radius:50%;border:1px solid #e0c9a6;background:#fffdf8;color:#8a5a2b;font:700 14px/1 -apple-system,"Segoe UI",sans-serif;cursor:pointer;padding:0}
        .wuwa-equip-slot .wuwa-equip-remove:hover{background:#f8e8d2;border-color:#c9a66b}
        .wuwa-equip-slot .wuwa-equip-follow{position:absolute;left:3px;bottom:3px;z-index:4;border:1px solid #d9c8aa;background:#fff;border-radius:6px;font-size:10px;padding:1px 5px;cursor:pointer;color:#7d7163}
        .wuwa-equip-slot .wuwa-equip-compat{position:absolute;left:3px;top:3px;z-index:2;border-radius:5px;padding:1px 5px;font-size:10px;color:#fff;background:#534ab7}
        .wuwa-equip-slot .wuwa-equip-compat.is-bad{background:#b5443a}
        .wuwa-equip-slot.wuwa-drop-hover{outline:2px solid #534ab7;outline-offset:1px}
        .wuwa-equip-slot.wuwa-drop-bad{outline:2px solid #b5443a;outline-offset:1px}
        .wuwa-drag-ghost{position:fixed;z-index:2147483647;pointer-events:none;width:56px;height:56px;border-radius:12px;background:#fffdf8dd;border:1px solid #dfbf83;box-shadow:0 8px 24px #2c1f1240;display:flex;align-items:center;justify-content:center;font:700 18px/1 -apple-system,"Segoe UI",sans-serif;color:#8a6d3b;transform:translate(-50%,-50%)}
        .wuwa-drag-ghost .wuwa-equip-thumb{max-width:100%;max-height:100%;object-fit:contain;border-radius:8px}
      `;
      (doc.head || doc.documentElement)?.append?.(style);
    };
    ensureStyle();

    const node = doc.createElement('section');
    node.className = 'wuwa-equip-bar';
    node.setAttribute?.('data-wuwa-equip-bar', '');
    const statusNode = doc.createElement('div');
    statusNode.className = 'wuwa-equip-status';
    statusNode.setAttribute?.('role', 'status');
    const slotsWrap = doc.createElement('div');
    slotsWrap.className = 'wuwa-equip-grid';
    node.append(statusNode, slotsWrap);

    let dock = null, toggleBtn = null;
    const syncDock = () => {
      if (!isFloating) return;
      const expanded = dockOpen || dragAutoOpen;
      node.hidden = !expanded;
      if (node.style) node.style.display = expanded ? 'flex' : 'none';
      if (toggleBtn) {
        toggleBtn.textContent = expanded ? '收起 ◂' : '装备栏 ▸';
        toggleBtn.setAttribute?.('aria-expanded', expanded ? 'true' : 'false');
      }
      setPanelWing(expanded);
    };
    if (isFloating) {
      dock = doc.createElement('section');
      dock.className = 'wuwa-equip-dock';
      dock.setAttribute?.('data-wuwa-equip-dock', '');
      toggleBtn = doc.createElement('button');
      toggleBtn.type = 'button';
      toggleBtn.className = 'wuwa-equip-toggle';
      toggleBtn.onclick = () => {
        if (autoCloseTimer !== null) { d.window.clearTimeout?.(autoCloseTimer); autoCloseTimer = null; }
        dragAutoOpen = false;
        dockOpen = !dockOpen;
        sessionEquipDockOpen = dockOpen;
        syncDock();
      };
      dock.append(toggleBtn, node);
      syncDock();
    }

    function status(message, tone) {
      if (!statusNode) return;
      statusNode.textContent = message || '';
      statusNode.className = 'wuwa-equip-status' + (tone === 'error' ? ' is-error' : '');
      if (statusTimer !== null) d.window.clearTimeout?.(statusTimer);
      if (message) statusTimer = d.window.setTimeout?.(() => {
        if (statusNode) statusNode.textContent = '';
        statusTimer = null;
      }, 4500) ?? null;
    }

    const thumbFor = (asset, kind = '') => {
      const id = Number(asset?.id ?? asset?.asset_id) || 0;
      const meta = assetMeta(resolveItemId(id)) || assetMeta(id);
      const bagMatch = id ? bagRows().find(row => Number(row.asset_id) === id) : null;
      const rawType = normalizedType(kind || asset?.asset_type || meta?.asset_type || bagMatch?.asset_type || '');
      const assetName = String(asset?.asset_name || asset?.name || meta?.asset_name || meta?.file_name || bagMatch?.asset_name || '');
      const name = assetName || (id ? '#' + id : '');
      const isMod = Boolean(meta?.custom_pack_id) || String(meta?.rarity || '').toLowerCase() === 'mod';
      let src = '';
      if (isMod && meta?.url) { try { src = assetUrl(meta.url); } catch { src = ''; } }
      if (!src && assetName && typeof d.resolveAssetUrl === 'function') {
        try {
          src = d.resolveAssetUrl({asset_id: id, asset_type: rawType, asset_name: assetName}, panelState.value?.assetsCdnHost || '') || '';
        } catch { src = ''; }
      }
      if (!src && assetName && panelState.value?.assetsCdnHost) {
        const stem = assetName.replace(/\.png$/i, '').trim();
        const folder = rawType === 'character_skin' ? 'character' : (rawType === 'accessory' ? 'accessory' : (rawType === 'emote' ? 'emote_atlas' : ''));
        if (stem && folder) src = String(panelState.value.assetsCdnHost).replace(/\/+$/, '') + '/' + folder + '/' + encodeURIComponent(stem) + '.png';
      }
      return {id, assetName, name, src};
    };

    const makeSlot = (side, type, index, asset, flags = {}) => {
      const slot = doc.createElement('div');
      slot.className = 'wuwa-equip-slot' + (asset ? ' is-filled' : '') + (flags.follow ? ' is-follow' : '');
      const drop = `${side}:${type}${type === 'emote' ? ':' + index : ''}`;
      slot.setAttribute?.('data-wuwa-drop', drop);
      slot.setAttribute?.('data-wuwa-slot-side', side);
      slot.setAttribute?.('data-wuwa-slot-type', type);
      if (type === 'emote') slot.setAttribute?.('data-wuwa-slot-index', String(index));
      const thumb = thumbFor(asset, type);
      if (asset && thumb.src) {
        const img = doc.createElement('img');
        img.className = 'wuwa-equip-thumb';
        img.src = thumb.src;
        img.alt = thumb.name;
        slot.append(img);
      } else if (asset) {
        const initial = doc.createElement('span');
        initial.className = 'wuwa-equip-initial';
        initial.textContent = (thumb.name || '?').slice(0, 1).toUpperCase();
        slot.append(initial);
      } else {
        const empty = doc.createElement('span');
        empty.className = 'wuwa-equip-name';
        empty.textContent = type === 'emote' ? '表情 ' + (index + 1) : '空';
        slot.append(empty);
      }
      if (asset && thumb.name) {
        const label = doc.createElement('span');
        label.className = 'wuwa-equip-name';
        label.textContent = flags.follow ? '跟随：' + thumb.name : thumb.name;
        slot.append(label);
      }
      if (asset && type === 'emote' && side === 'official') {
        const compat = onlineEmoteCompatibility(asset, liveOfficial());
        const badge = doc.createElement('span');
        badge.className = 'wuwa-equip-compat' + (compat.compatible ? '' : ' is-bad');
        badge.textContent = compat.universal ? '通用' : (compat.compatible ? '匹配' : '不兼容');
        badge.title = compat.label;
        slot.append(badge);
      }
      if (asset && type === 'emote') {
        slot.setAttribute?.('data-wuwa-emote-source', side);
        slot.setAttribute?.('data-wuwa-emote-index', String(index));
        slot.setAttribute?.('data-wuwa-emote-id', String(thumb.id));
        if (thumb.assetName) slot.setAttribute?.('data-wuwa-emote-name', thumb.assetName);
      }
      if (asset && type !== 'character_skin') {
        const remove = doc.createElement('button');
        remove.type = 'button';
        remove.className = 'wuwa-equip-remove';
        remove.textContent = '×';
        remove.title = '卸下' + typeLabel(type);
        remove.onmousedown = event => event?.stopPropagation?.();
        remove.onclick = event => {
          event?.stopPropagation?.();
          event?.preventDefault?.();
          emit({asset_id: thumb.id, asset_type: type, asset_name: thumb.assetName, __wuwaRemove: true, __wuwaTarget: side, __wuwaSlotIndex: index});
          status(side === 'official' ? '已请求卸下线上' + typeLabel(type) : '已卸下本地' + typeLabel(type));
          scheduleRender(true);
        };
        slot.append(remove);
      }
      if (side === 'local' && type === 'character_skin' && !flags.follow) {
        const follow = doc.createElement('button');
        follow.type = 'button';
        follow.className = 'wuwa-equip-follow';
        follow.textContent = '跟随线上';
        follow.title = '恢复跟随线上角色';
        follow.onmousedown = event => event?.stopPropagation?.();
        follow.onclick = event => {
          event?.stopPropagation?.();
          event?.preventDefault?.();
          emit({asset_id: thumb.id || 0, asset_type: 'character_skin', asset_name: thumb.assetName, __wuwaRemove: true, __wuwaTarget: 'local'});
          status('已恢复跟随线上角色');
          scheduleRender(true);
        };
        slot.append(follow);
      }
      return slot;
    };

    const slotList = side => {
      const sel = selection.value || {};
      const official = liveOfficial();
      const offChar = official?.character || null;
      const offAcc = official?.accessory || null;
      const offEmotes = Array.isArray(official?.emote) ? official.emote : [];
      if (side === 'official') {
        return [
          makeSlot('official', 'character_skin', -1, offChar),
          makeSlot('official', 'accessory', -1, offAcc),
          ...Array.from({length: 5}, (_, index) => makeSlot('official', 'emote', index, offEmotes[index] || null)),
        ];
      }
      const hasCharacter = Boolean(sel.character);
      const hasAccessory = Object.hasOwn(sel, 'accessory');
      const localEmotes = Array.isArray(sel.emote) ? sel.emote : offEmotes;
      return [
        makeSlot('local', 'character_skin', -1, sel.character || offChar, {follow: !hasCharacter}),
        makeSlot('local', 'accessory', -1, hasAccessory ? sel.accessory : offAcc, {follow: !hasAccessory}),
        ...Array.from({length: 5}, (_, index) => makeSlot('local', 'emote', index, localEmotes[index] || null, {
          follow: !Array.isArray(sel.emote) && Boolean(localEmotes[index]),
        })),
      ];
    };

    function render() {
      if (disposed) return;
      observedSignature = signature();
      const rows = [];
      const localRow = doc.createElement('div');
      localRow.className = 'wuwa-equip-row';
      const localTitle = doc.createElement('h4'); localTitle.textContent = '本地装备（仅本机显示）';
      const localSlots = doc.createElement('div'); localSlots.className = 'wuwa-equip-slots';
      for (const slot of slotList('local')) localSlots.append(slot);
      localRow.append(localTitle, localSlots);
      const officialRow = doc.createElement('div');
      officialRow.className = 'wuwa-equip-row';
      const officialTitle = doc.createElement('h4'); officialTitle.textContent = '线上装备（联机可见）';
      const officialSlots = doc.createElement('div'); officialSlots.className = 'wuwa-equip-slots';
      for (const slot of slotList('official')) officialSlots.append(slot);
      officialRow.append(officialTitle, officialSlots);
      rows.push(localRow, officialRow);
      slotsWrap.textContent = '';
      slotsWrap.append(...rows);
    }

    let renderScheduled = false, renderForced = false;
    function scheduleRender(force = false) {
      if (disposed) return;
      if (force) renderForced = true;
      if (renderScheduled) return;
      renderScheduled = true;
      const run = () => {
        renderScheduled = false;
        const doForce = renderForced;
        renderForced = false;
        if (!doForce && signature() === observedSignature) return;
        render();
      };
      if (typeof d.window.setTimeout === 'function') d.window.setTimeout(run, 0); else run();
    }

    // ---- Pointer Events 拖拽 ----
    let drag = null, ghost = null, suppressClickUntil = 0;
    const threshold = 8;
    const parseDrop = value => {
      const parts = String(value || '').split(':');
      const side = parts[0];
      if (side !== 'local' && side !== 'official') return null;
      if (parts[1] === 'character_skin' || parts[1] === 'accessory') {
        return {side, type: parts[1], index: -1};
      }
      if (parts[1] === 'emote') {
        const index = Number(parts[2]);
        return {side, type: 'emote', index: Number.isInteger(index) ? Math.max(0, Math.min(4, index)) : 0};
      }
      return null;
    };
    const clearHighlights = () => {
      for (const slot of node.querySelectorAll?.('[data-wuwa-drop]') || []) {
        slot.classList?.remove('wuwa-drop-hover');
        slot.classList?.remove('wuwa-drop-bad');
      }
    };
    const removeGhost = () => { ghost?.remove?.(); ghost = null; };
    const makeGhost = item => {
      ghost = doc.createElement('div');
      ghost.className = 'wuwa-drag-ghost';
      const thumb = thumbFor(item);
      if (thumb.src) {
        const img = doc.createElement('img');
        img.className = 'wuwa-equip-thumb';
        img.src = thumb.src;
        img.alt = thumb.name;
        ghost.append(img);
      } else {
        ghost.textContent = (thumb.name || '?').slice(0, 1).toUpperCase();
      }
      doc.body?.append(ghost);
    };
    const moveGhost = (x, y) => { if (ghost) ghost.style.transform = `translate(${x}px, ${y}px) translate(-50%, -50%)`; };

    const canDrop = (item, target) => {
      if (!target || normalizedType(item.asset_type) !== target.type) return false;
      if (target.side === 'local') return true;
      if (!officialOwned(item.asset_id, target.type)) return false;
      if (target.type === 'emote') return onlineEmoteCompatibility(item, liveOfficial()).compatible;
      const meta = assetMeta(resolveItemId(item.asset_id));
      const role = String(meta?.game_role_id || '');
      const official = liveOfficial();
      const onlineRole = String(official?.role_id || assetMeta(official?.character?.id)?.game_role_id || '');
      return !role || role === '0' || !onlineRole || role === onlineRole;
    };

    const reorderLocalEmotes = (from, to) => {
      if (from === to) return false;
      const sel = selection.value || {};
      const base = Array.isArray(sel.emote) ? sel.emote.slice() : [...(liveOfficial()?.emote || [])];
      if (from < 0 || from >= base.length || to < 0 || to >= 5) return false;
      const [moved] = base.splice(from, 1);
      base.splice(Math.min(to, base.length), 0, moved);
      selection.value = {...sel, emote: base};
      save();
      return true;
    };

    const reorderOfficialEmotes = (from, to) => {
      const current = (liveOfficial()?.emote || []).map(entry => Number(entry?.id)).filter(n => n > 0);
      if (from < 0 || from >= current.length || to < 0 || to >= 5 || from === to) return false;
      const [moved] = current.splice(from, 1);
      current.splice(Math.min(to, current.length), 0, moved);
      if (typeof options.emitReorder === 'function') options.emitReorder(current);
      else emit({__wuwaReorder: true, __wuwaTarget: 'official', asset_type: 'emote', emoteAssetIds: current});
      return true;
    };

    const dropItem = (item, target) => {
      const type = normalizedType(item.asset_type);
      if (type !== target.type) { status('槽位类型不匹配：请放到对应类型的槽位', 'error'); return false; }
      const itemId = resolveItemId(item.asset_id) || Number(item.asset_id) || 0;
      const meta = assetMeta(itemId);
      const assetName = String(item.asset_name || meta?.asset_name || meta?.file_name || '');
      const fullItem = {...item, asset_id: itemId, asset_type: type, ...(assetName ? {asset_name: assetName} : {})};
      if (target.side === 'local') {
        emit({...fullItem, __wuwaTarget: 'local', __wuwaSlotIndex: target.type === 'emote' ? target.index : undefined});
        status('已放入本地' + typeLabel(target.type));
        scheduleRender();
        return true;
      }
      if (!officialOwned(itemId, target.type)) { status('未拥有该官方外观，不能放到线上', 'error'); return false; }
      if (target.type === 'emote') {
        const compat = onlineEmoteCompatibility(fullItem, liveOfficial());
        if (!compat.compatible) { status('不兼容当前线上角色，不能放到线上', 'error'); return false; }
        const offList = liveOfficial()?.emote || [];
        const existingIndex = offList.findIndex(entry => Number(entry?.id) === itemId);
        if (existingIndex >= 0) {
          reorderOfficialEmotes(existingIndex, target.index);
          status('已调整线上表情顺序');
          scheduleRender();
          return true;
        }
      } else {
        const role = String(meta?.game_role_id || '');
        const official = liveOfficial();
        const onlineRole = String(official?.role_id || assetMeta(official?.character?.id)?.game_role_id || '');
        if (role && role !== '0' && onlineRole && role !== onlineRole) {
          status('与当前线上角色不兼容，不能放到线上', 'error');
          return false;
        }
      }
      emit({...fullItem, __wuwaTarget: 'official', __wuwaSlotIndex: target.type === 'emote' ? target.index : undefined});
      status('已同步到线上' + typeLabel(target.type));
      scheduleRender();
      return true;
    };

    const hitDropNode = event => {
      const pointed = typeof doc.elementFromPoint === 'function' && Number.isFinite(event?.clientX) && Number.isFinite(event?.clientY)
        ? doc.elementFromPoint(event.clientX, event.clientY)
        : null;
      return pointed?.closest?.('[data-wuwa-drop]') || event?.target?.closest?.('[data-wuwa-drop]') || null;
    };

    const onPointerDown = event => {
      if (disposed || !event || event.button > 0) return;
      const target = event.target;
      if (target?.closest?.('.wuwa-equip-remove,.wuwa-equip-follow,.wuwa-equip-toggle,.atlas-collect')) return;
      const emoteSource = target?.closest?.('[data-wuwa-emote-source]');
      const atlasItem = target?.closest?.('.atlas-item[data-wuwa-asset-id]');
      const source = emoteSource || atlasItem;
      if (!source) return;
      const panel = host.closest?.('.panel-content');
      if (panel && source !== emoteSource && !panel.contains?.(source)) return;
      if (source !== emoteSource && atlasItem?.closest?.('.panel-content') === null && typeof atlasItem.closest === 'function') {
        // Ignore items outside panel content only if inside another unrelated container with .panel-content present.
        if (doc.querySelector?.('.panel-content')) return;
      }
      if (emoteSource) {
        const id = Number(emoteSource.getAttribute?.('data-wuwa-emote-id')) || 0;
        if (!id) return;
        drag = {
          x: event.clientX, y: event.clientY, active: false,
          item: {asset_id: id, asset_type: 'emote', asset_name: emoteSource.getAttribute?.('data-wuwa-emote-name') || ''},
          slot: {
            side: emoteSource.getAttribute?.('data-wuwa-emote-source') || 'local',
            index: Number(emoteSource.getAttribute?.('data-wuwa-emote-index')) || 0,
          },
        };
        return;
      }
      const id = Number(atlasItem.getAttribute?.('data-wuwa-asset-id')) || 0;
      const type = atlasItem.getAttribute?.('data-wuwa-asset-type') || '';
      if (!id || !type) return;
      drag = {
        x: event.clientX, y: event.clientY, active: false,
        item: {asset_id: id, asset_type: normalizedType(type), asset_name: atlasItem.getAttribute?.('data-wuwa-asset-name') || ''},
        slot: null,
      };
    };

    const onPointerMove = event => {
      if (!drag) return;
      const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
      if (!drag.active) {
        if (Math.sqrt(dx * dx + dy * dy) < threshold) return;
        drag.active = true;
        makeGhost(drag.item);
        if (isFloating && !dockOpen) {
          if (autoCloseTimer !== null) { d.window.clearTimeout?.(autoCloseTimer); autoCloseTimer = null; }
          dragAutoOpen = true;
          syncDock();
        }
      }
      event.preventDefault?.();
      moveGhost(event.clientX, event.clientY);
      const dropNode = hitDropNode(event);
      const target = dropNode ? parseDrop(dropNode.getAttribute?.('data-wuwa-drop')) : null;
      clearHighlights();
      if (dropNode && target) {
        const ok = drag.slot
          ? target.type === 'emote' && target.side === drag.slot.side
          : canDrop(drag.item, target);
        dropNode.classList?.add(ok ? 'wuwa-drop-hover' : 'wuwa-drop-bad');
      }
      drag.hoverDrop = dropNode || null;
    };

    const scheduleAutoCollapse = (ms = 1400) => {
      if (!isFloating || !dragAutoOpen) return;
      if (autoCloseTimer !== null) d.window.clearTimeout?.(autoCloseTimer);
      autoCloseTimer = d.window.setTimeout?.(() => {
        autoCloseTimer = null;
        dragAutoOpen = false;
        syncDock();
      }, ms) ?? null;
    };

    const finishDrag = event => {
      if (!drag) return;
      const current = drag;
      drag = null;
      clearHighlights();
      removeGhost();
      if (!current.active) return;
      suppressClickUntil = Date.now() + 500;
      const dropNode = current.hoverDrop || hitDropNode(event);
      const target = dropNode ? parseDrop(dropNode.getAttribute?.('data-wuwa-drop')) : null;
      if (!target) {
        status('拖到本地或线上槽位才能放置', 'error');
        scheduleAutoCollapse(900);
        return;
      }
      if (current.slot && target.type === 'emote' && target.side === current.slot.side) {
        if (target.side === 'local') {
          if (reorderLocalEmotes(current.slot.index, target.index)) status('已调整本地表情顺序');
          else status('表情排序未变化');
        } else {
          if (reorderOfficialEmotes(current.slot.index, target.index)) status('已请求调整线上表情顺序');
          else status('表情排序未变化');
        }
        scheduleAutoCollapse(1400);
        return;
      }
      dropItem(current.item, target);
      scheduleAutoCollapse(1400);
    };

    const onPointerUp = event => finishDrag(event);
    const onPointerCancel = () => { if (drag) { drag = null; clearHighlights(); removeGhost(); scheduleAutoCollapse(0); } };
    // 成功拖放后浏览器还会补一个 click，用捕获阶段吞掉，避免又弹一次选择窗。
    const onClickCapture = event => {
      if (Date.now() >= suppressClickUntil) return;
      event?.stopPropagation?.();
      event?.preventDefault?.();
    };

    doc.addEventListener?.('pointerdown', onPointerDown, true);
    doc.addEventListener?.('pointermove', onPointerMove, true);
    doc.addEventListener?.('pointerup', onPointerUp, true);
    doc.addEventListener?.('pointercancel', onPointerCancel, true);
    doc.addEventListener?.('click', onClickCapture, true);
    const refreshHandler = () => scheduleRender();
    d.window.addEventListener?.('storage', refreshHandler);

    let observedSignature = '';
    const signature = () => {
      const sel = selection.value || {}, off = liveOfficial() || {};
      return JSON.stringify([
        sel.character?.id || 0, sel.accessory?.id || 0, Object.hasOwn(sel, 'accessory'),
        Array.isArray(sel.emote), (sel.emote || []).map(item => item?.id || 0),
        off.character?.id || 0, off.accessory?.id || 0, (off.emote || []).map(item => item?.id || 0),
      ]);
    };
    const tick = () => {
      if (disposed) return;
      const next = signature();
      if (next === observedSignature) return;
      observedSignature = next;
      render();
    };
    const timer = d.window.setInterval?.(tick, 1200) ?? null;
    host.append(dock || node);
    observedSignature = signature();
    render();

    let destroyed = false;
    const destroy = () => {
      if (destroyed) return;
      destroyed = true;
      if (timer !== null) d.window.clearInterval?.(timer);
      if (statusTimer !== null) d.window.clearTimeout?.(statusTimer);
      if (autoCloseTimer !== null) d.window.clearTimeout?.(autoCloseTimer);
      statusTimer = null;
      autoCloseTimer = null;
      doc.removeEventListener?.('pointerdown', onPointerDown, true);
      doc.removeEventListener?.('pointermove', onPointerMove, true);
      doc.removeEventListener?.('pointerup', onPointerUp, true);
      doc.removeEventListener?.('pointercancel', onPointerCancel, true);
      doc.removeEventListener?.('click', onClickCapture, true);
      d.window.removeEventListener?.('storage', refreshHandler);
      removeGhost();
      (dock || node).remove?.();
      equipmentBars.delete(api);
      if (isFloating && equipmentBars.size === 0) {
        if (typeof d.window.setTimeout === 'function') {
          wingCollapseTimer = d.window.setTimeout(() => {
            wingCollapseTimer = null;
            if (equipmentBars.size === 0) applyPanelWing(0);
          }, 120);
        } else {
          applyPanelWing(0);
        }
      }
      if (host.__wuwaEquipBar === api) host.__wuwaEquipBar = null;
    };
    const api = {element: dock || node, bar: node, sync: () => scheduleRender(false), refresh: () => scheduleRender(true), destroy};
    host.__wuwaEquipBar = api;
    equipmentBars.add(api);
    return destroy;
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
  applyRoomLayoutStyle();
  applyPerfStyle();
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
    if (event.key === CAPTURE_KEY) {
      // /panel 里改绿幕/面板外观，main 小窗要立刻跟上，不用等重开窗口。
      try {
        const value = JSON.parse(d.storage.getItem(CAPTURE_KEY) || '{}');
        capture.value = value && typeof value === 'object' ? value : {mode: 'off', panelOpacity: 0.96};
      } catch { capture.value = {mode: 'off', panelOpacity: 0.96}; }
      applyCaptureStyle();
      revision.value++;
      return;
    }
    if (event.key === XFORM_KEY) { transforms.value = readTransforms(); revision.value++; return; }
    if (event.key === ROOM_LAYOUT_KEY) {
      roomLayout.value = readRoomLayout();
      applyRoomLayoutStyle();
      revision.value++;
      return;
    }
    if (event.key === PERF_KEY) { perf.value = readPerf(); applyPerfStyle(); revision.value++; return; }
    if (event.key === MARKS_KEY) { marks.value = readMarks(); revision.value++; refreshSourceBadges?.(); return; }
    if (event.key !== KEY) return;
    const state = read();
    if (state.marks && typeof state.marks === 'object' && !Array.isArray(state.marks)) marks.value = state.marks;
    selection.value = state.selection || {}; favorites.value = state.favorites || {};
    // Serialize mode changes so a stale async enable cannot overwrite a later disable.
    pending = pending.catch(() => {}).then(() => applyMode(state.enabled === true)).catch(d.onError);
    revision.value++;
    refreshSourceBadges?.();
    notifyEquipmentBars();
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
    let idleStatusTicks = 0;
    statusTimer=d.window.setInterval(async () => {
      if (checkingStatus || busy.value) return;
      if (cacheReady.value && syncStatus.value?.status !== 'running' && (!desiredLocal || enabled.value)) {
        idleStatusTicks = (idleStatusTicks + 1) % 6;
        if (idleStatusTicks !== 0) return;
      } else {
        idleStatusTicks = 0;
      }
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
  function dispose(){disposed=true;if(statusTimer!==null)d.window.clearInterval?.(statusTimer);for(const bar of [...equipmentBars])bar?.destroy?.();equipmentBars.clear();if(wingCollapseTimer!==null){d.window.clearTimeout?.(wingCollapseTimer);wingCollapseTimer=null;}if(appliedWingPx>0)applyPanelWing(0);badgeObserver?.disconnect();badgeObserver=null;choiceNode?.remove();}
  return {enabled, busy, selection, favorites, marks, revision, ready, cacheReady, setEnabled, inventory, viewUser, select, mergeAtlas, mergeBag, canSelect,
    favorite, reorder, prepare, offlineData, get assetsHost() { return resolveAssetsHost(); }, offlineError, checkInvoke, syncStatus, syncCache, importCustomResources, removeCustomResource, assetUrl, request: localRequest, dispose,
    presetList, createLocalPreset, deleteLocalPreset, applyLocalPreset, collectionProgress,
    mergePresets, applyPreset,
    ownedBag, accessoryTransform, setAccessoryTransform, resetAccessoryTransform,
    listAccessoryTransforms, deleteAccessoryTransform, activateLocalIfReady, startOverlay,
    officialOwned, sourceFor, markFor, setMark, beginSelect, beginPanelSelect, afterOfficialSelect, clearLocal, shouldCreateLocalPreset,
    isModAsset, syncModCatalog,
    officialUser, panelState, syncPanelState, onlineEmoteCompatibility, mountEquipmentBar,
    synthesizeDuplicates, capture, setCapture, applyCaptureStyle,
    roomLayout, setRoomLayout, saveRoomMemberPosition, savedRoomMemberPosition, toggleRoomMemberHidden,
    isRoomMemberHidden, clearRoomPositions, requestRoomArrange, arrangeRoomMembers, applyRoomLayoutStyle,
    perf, setPerf, applyPerfStyle,
    restricted,normalCumulative:()=>d.getAccountCumulative?.()??0,normalPending:()=>d.getAccountPending?.()??0};
}
