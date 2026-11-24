// Regression: dashboard update tab, frequency + 1-5 slot selection without accessibility prompt,
// click-count driven auto-emote, and virtual auto-click avoidance/yield in desktop_assistant.js.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const { renderFrontend } = require('./render-frontend.cjs');

const flush = () => new Promise(resolve => setImmediate(resolve));

function createMockWindow() {
  const timers = new Map();
  let nextTimerId = 1;

  function matchSelector(node, selector) {
    if (selector.startsWith('[') && selector.endsWith(']')) {
      const inner = selector.slice(1, -1);
      const eq = inner.indexOf('=');
      if (eq === -1) return node.attributes.has(inner);
      const key = inner.slice(0, eq);
      const val = inner.slice(eq + 1).replace(/^['"]|['"]$/g, '');
      return node.attributes.get(key) === val;
    }
    if (selector.startsWith('.')) return node.className.split(/\s+/).includes(selector.slice(1));
    if (selector.startsWith('#')) return node.id === selector.slice(1);
    return node.tagName.toLowerCase() === selector.toLowerCase();
  }

  function makeNode(tag) {
    const node = {
      tagName: String(tag).toUpperCase(),
      id: '',
      className: '',
      type: '',
      checked: false,
      disabled: false,
      hidden: false,
      value: '',
      style: {},
      attributes: new Map(),
      children: [],
      parentNode: null,
      _text: '',
      get textContent() {
        return this.children.length ? this.children.map(c => c.textContent).join('') : this._text;
      },
      set textContent(v) {
        this._text = String(v ?? '');
        if (this.children.length) this.children = [];
      },
      append(...items) {
        for (const item of items) {
          if (!item || typeof item !== 'object') continue;
          item.parentNode = this;
          this.children.push(item);
        }
      },
      replaceChildren(...items) {
        this.children = [];
        this.append(...items);
      },
      remove() {
        if (!this.parentNode) return;
        this.parentNode.children = this.parentNode.children.filter(c => c !== this);
        this.parentNode = null;
      },
      focus() { doc.activeElement = this; },
      setAttribute(k, v) { this.attributes.set(k, String(v)); },
      getAttribute(k) { return this.attributes.get(k) ?? null; },
      findAll(predicate, out = []) {
        if (predicate(this)) out.push(this);
        for (const child of this.children) child.findAll(predicate, out);
        return out;
      },
      querySelectorAll(selector) { return this.findAll(n => matchSelector(n, selector)); },
      querySelector(selector) { return this.querySelectorAll(selector)[0] || null; },
    };
    return node;
  }

  const head = makeNode('head');
  const body = makeNode('body');
  const doc = {
    head,
    body,
    activeElement: null,
    createElement: makeNode,
    getElementById: id => body.findAll(n => n.id === id)[0] || head.findAll(n => n.id === id)[0] || null,
  };

  const intervals = [];
  const win = {
    document: doc,
    setTimeout(fn, ms) {
      const id = nextTimerId++;
      const handle = global.setTimeout(() => { timers.delete(id); fn(); }, Math.min(ms, 20));
      timers.set(id, { kind: 'timeout', handle });
      return id;
    },
    clearTimeout(id) {
      const entry = timers.get(id);
      if (entry) { global.clearTimeout(entry.handle); timers.delete(id); }
    },
    setInterval(fn, ms) {
      const id = nextTimerId++;
      intervals.push({ id, fn, ms });
      return id;
    },
    clearInterval(id) {
      const idx = intervals.findIndex(item => item.id === id);
      if (idx !== -1) intervals.splice(idx, 1);
    },
  };
  return { win, doc, intervals };
}

async function testDashboard(platform) {
  const source = renderFrontend('assistant_dashboard.js', platform);
  const { win, doc, intervals } = createMockWindow();
  const requests = [];
  const state = {
    session: 12,
    session_chests: 1,
    rewards: [{ name: '测试奖励', asset_name: 'emote.png', time: 1700000000, side: 'left', source: 'manual', cost: 1000 }],
    rooms: [{ code: '123456', host: 'Alice', favorite: true }],
    weekly: { normal: [{ date: '2026-11-19', count: 5 }] },
    settings: {
      enabled: false,
      auto_click_balance: false,
      auto_click_below: 1000,
      auto_click_above: 2000,
      auto_chest: false,
      auto_emote: true,
      auto_emote_interval: 25,
      auto_emote_random: true,
      auto_emote_slots: 31,
      threshold: 1000,
      prerelease_updates: false,
      speed: 5,
    },
    runtime: {
      logged_in: true,
      balance: 3000,
      total: 5000,
      emote: { sent: 0, remaining: 22, reason: 'ready', slots: ['挥手', '点赞', '', '', ''] },
    },
  };

  const synthKinds = [];
  const perfCalls = [];
  const roomLayoutCalls = [];
  const arrangeCalls = [];
  const clearPositionCalls = [];
  const toggleHiddenCalls = [];
  const transformCalls = [];
  const transforms = [
    { id: '88', name: 'official_hat.png', dx: 12, dy: -4, scale: 1.2, rotate: 15 },
    { id: '99', name: 'local_star.png', dx: 0, dy: 8, scale: 0.9, rotate: -10 },
  ];
  let communityMode = 'error';
  let installMode = 'error';
  let installStatusOverride = null;
  let statusCalls = 0;
  const atlas = {
    enabled: { value: false },
    busy: { value: false },
    cacheReady: { value: true },
    assistant: { emoteStats: {} },
    panelState: { value: { roomMembers: [{ id: 101, nickname: '小猫甲' }, { id: 102, nickname: '小猫乙' }] } },
    officialUser: { value: { accessory: { id: 88, name: 'official_hat.png' } } },
    selection: { value: { accessory: null } },
    roomLayout: { value: { positions: {}, hidden: {}, scale: 1, opacity: 1, nameScale: 1, arrangeMode: 'horizontal' } },
    perf: { value: { fpsCap: 60, idlePause: true, reduceMotion: false, hideMainPet: false } },
    listAccessoryTransforms: () => transforms.map(row => ({ ...row })),
    accessoryTransform(id) {
      const targetId = id === undefined || id === null
        ? String(atlas.officialUser.value.accessory?.id ?? '')
        : String(id);
      return transforms.find(row => String(row.id) === targetId) || null;
    },
    setAccessoryTransform(patch, id) {
      transformCalls.push({ patch, id });
      const targetId = id === undefined || id === null
        ? String(atlas.officialUser.value.accessory?.id ?? '')
        : String(id);
      const row = transforms.find(item => String(item.id) === targetId);
      if (row) Object.assign(row, patch);
      else transforms.push({ id: targetId, ...patch });
    },
    resetAccessoryTransform(id) {
      transformCalls.push({ reset: true, id });
    },
    deleteAccessoryTransform(id) {
      transformCalls.push({ delete: true, id });
      const idx = transforms.findIndex(row => String(row.id) === String(id));
      if (idx !== -1) transforms.splice(idx, 1);
    },
    setRoomLayout(patch) {
      roomLayoutCalls.push(patch);
      Object.assign(atlas.roomLayout.value, patch);
    },
    toggleRoomMemberHidden(id) {
      toggleHiddenCalls.push(id);
      const hidden = atlas.roomLayout.value.hidden;
      hidden[id] = !hidden[id];
    },
    isRoomMemberHidden(id) {
      return !!atlas.roomLayout.value.hidden[id];
    },
    requestRoomArrange(mode) {
      arrangeCalls.push(mode);
      atlas.roomLayout.value.arrangeMode = mode;
    },
    clearRoomPositions() {
      clearPositionCalls.push(true);
      atlas.roomLayout.value.positions = {};
    },
    setPerf(patch) {
      perfCalls.push(patch);
      Object.assign(atlas.perf.value, patch);
    },
    prepare: async () => {},
    setEnabled: async v => { atlas.enabled.value = v; },
    synthesizeDuplicates: async kind => { synthKinds.push(kind); return { count: 3, batches: 1, groups: 1 }; },
    async request(route, body) {
      requests.push({ route, body });
      if (route === '/community/version') return { current: '1.3.2-beta.3' };
      if (route === '/official/update-check') return { latestVersion: '1.0.4', currentVersion: '1.0.4', updateAvailable: false };
      if (route === '/custom/status') return { packs: [], atlas_count: 10, asset_count: 20 };
      if (route === '/diagnostic/synth/list') return { entries: [{ time: '2026-11-23T08:00:00Z', entry: { kind: 'same-level', consumed: 5, batches: 1, durationMs: 42 } }] };
      if (route === '/update/status') {
        statusCalls++;
        if (installStatusOverride) {
          return { required: false, current: '1.3.2-beta.3', github_version: '1.3.2-beta.5', checked_at: 100, release_notes: 'notes', installation: installStatusOverride };
        }
        if (statusCalls <= 2) return { required: false, current: '1.3.2-beta.3', github_version: '', checked_at: 0, error: null, release_notes: '' };
        return { required: false, current: '1.3.2-beta.3', github_version: '1.3.2', checked_at: 100, release_notes: 'notes' };
      }
      if (route === '/update/check') throw new Error('offline');
      if (route === '/update/install') {
        if (installMode === 'ok') return { started: true, message: '已开始后台下载并校验安装包…' };
        throw new Error('network timeout');
      }
      if (route === '/community/update-check') {
        if (communityMode === 'error') return { current: '1.3.2-beta.3', latest: '', updateAvailable: false, error: 'rate limit', message: 'GitHub 接口限流' };
        if (communityMode === 'available-unwritable') return { current: '1.3.2-beta.3', latest: '1.3.2-beta.5', updateAvailable: true, installable: false, installableReason: '当前正直接在磁盘映像（DMG）中运行，请先将应用拖入「应用程序」文件夹后再打开' };
        if (communityMode === 'available-installable') return { current: '1.3.2-beta.3', latest: '1.3.2-beta.5', updateAvailable: true, installable: true };
        return { current: '1.3.2-beta.3', latest: '1.3.2', updateAvailable: false, releaseNotes: 'Fixed' };
      }
      return {};
    },
  };

  const ctx = vm.createContext({ window: win, Date, Number, Promise, console, Object, Math, Array, String });
  vm.runInContext(source + ';globalThis.createDashboard = createAssistantDashboard;', ctx);

  const dash = ctx.createDashboard({
    window: win,
    version: '1.3.2-beta.3',
    initialTab: 'overview',
    atlas,
    getState: async () => structuredClone(state),
    configure: async patch => {
      Object.assign(state.settings, patch);
      return structuredClone(state);
    },
    reset: async () => structuredClone(state),
    favorite: async () => {},
    join: async () => {},
    deleteRoom: async () => {},
  });

  await flush();
  await flush();
  const cardWithTitle = title => doc.body.findAll(n =>
    n.className.split(/\s+/).includes('wt-card') &&
    n.findAll(h => h.tagName === 'H3' && h._text === title).length > 0,
  )[0];

  // Six-tab console: top nav labels are the new canonical set.
  const nav = doc.body.findAll(n => n.tagName === 'NAV')[0];
  assert.ok(nav, 'console nav exists');
  assert.deepEqual(
    nav.findAll(n => n.tagName === 'BUTTON').map(n => n._text),
    ['概览', '自动助手', '统计与记录', '联机房间', '外观与 Mod', '系统与更新'],
    'top nav exposes the six reorganized tabs',
  );

  // Legacy tab keys keep working through the alias router.
  for (const [alias, canonical] of [
    ['rewards', 'records'], ['daily', 'records'], ['settings', 'automation'], ['update', 'system'],
  ]) {
    dash.showTab(alias);
    assert.equal(dash.currentTab(), canonical, alias + ' resolves to ' + canonical);
  }

  // Overview: live status pills and KPI/progress/recent cards, but no speed picker.
  dash.showTab('overview');
  await flush();
  const statCards = doc.body.findAll(n => n.className.split(/\s+/).includes('wt-stat'));
  assert.equal(statCards.length, 4, 'overview shows four KPI stat cards');
  assert.deepEqual(
    statCards.map(c => c.findAll(n => n.className === 'wt-label')[0]?._text),
    ['可用点数', '累计点数', '本次发送', '本次开箱'],
    'overview KPI labels',
  );
  const chestProgressCard = cardWithTitle('宝箱进度');
  assert.ok(chestProgressCard && chestProgressCard.textContent.includes('宝箱1') && chestProgressCard.textContent.includes('宝箱2'), '宝箱进度 card lists both chests');
  assert.ok(cardWithTitle('最近获得'), '最近获得 card exists on overview');
  const pills = doc.body.findAll(n => n.className.split(/\s+/).includes('wt-pill'));
  assert.equal(pills.length, 3, 'overview shows three live status pills');
  assert.ok(pills.some(p => p.textContent.includes('点击速度')), 'speed status pill exists');
  assert.ok(pills.some(p => p.textContent.includes('自动开箱')), 'chest status pill exists');
  assert.ok(pills.some(p => p.textContent.includes('打字表情')), 'emote status pill exists');
  assert.equal(doc.body.querySelector('.wt-speeds'), null, 'overview must not host the speed picker');

  // System tab (`update` alias): version/update behavior plus perf rendering and local assets.
  dash.showTab('update');
  assert.equal(dash.currentTab(), 'system');
  await flush();
  await flush();
  const checkBtn = doc.body.findAll(n => n.tagName === 'BUTTON' && n._text === '检查更新')[0];
  const preBox = doc.body.querySelector('[aria-label="包含预发布版本"]');
  assert.ok(checkBtn, '检查更新 button exists');
  assert.ok(preBox, '包含预发布版本 toggle exists');
  assert.deepEqual(
    checkBtn.parentNode.children.filter(n => n.tagName === 'BUTTON').map(n => n._text),
    ['打开 GitHub Releases', '打开官方桌宠下载页', '检查更新'],
    'update action row keeps GitHub / official site / check order',
  );

  await checkBtn.onclick();
  assert.equal(checkBtn.disabled, false, '检查更新 button re-enables after error response');
  const remoteVal = doc.body.findAll(n => n.className === 'wt-version-value')[2];
  assert.equal(remoteVal.textContent, '检查失败');

  communityMode = 'latest';
  preBox.checked = true;
  await preBox.onchange();
  assert.equal(state.settings.prerelease_updates, true);
  assert.equal(remoteVal.textContent, '1.3.2');
  const communityReqs = requests.filter(r => r.route === '/community/update-check');
  assert.ok(communityReqs.length >= 2);
  assert.equal(communityReqs.at(-1).body?.force, true);

  // Test confirmUpdate dialog: installable=true shows 一键更新 + fallback on error; installable=false shows installableReason
  communityMode = 'available-unwritable';
  await checkBtn.onclick();
  let gate = doc.body.querySelector('.wt-gate');
  assert.ok(gate, 'update modal shown when updateAvailable is true');
  assert.ok(gate.textContent.includes('拖入「应用程序」'), gate.textContent);
  assert.equal(gate.findAll(n => n.tagName === 'BUTTON' && n._text === '一键更新').length, 0);
  gate.findAll(n => n.tagName === 'BUTTON' && n._text === '稍后')[0].onclick();

  communityMode = 'available-installable';
  await checkBtn.onclick();
  gate = doc.body.querySelector('.wt-gate');
  assert.ok(gate);
  const oneClickBtn = gate.findAll(n => n.tagName === 'BUTTON' && n._text === '一键更新')[0];
  assert.ok(oneClickBtn, '一键更新 button shown when installable is true');
  await oneClickBtn.onclick();
  assert.equal(requests.some(r => r.route === '/update/install'), true);
  assert.ok(gate.querySelector('[data-github-fallback="true"]'), 'fallback GitHub button shown after synchronous install error');
  gate.findAll(n => n.tagName === 'BUTTON' && n._text === '稍后')[0].onclick();

  // When /update/install starts background download cleanly, modal closes immediately and returns to update tab with progress bar
  installMode = 'ok';
  installStatusOverride = { phase: 'download', percent: 45, message: '正在下载更新包（45%）' };
  await checkBtn.onclick();
  gate = doc.body.querySelector('.wt-gate');
  assert.ok(gate);
  await gate.findAll(n => n.tagName === 'BUTTON' && n._text === '一键更新')[0].onclick();
  await flush();
  assert.equal(doc.body.querySelector('.wt-gate'), null, 'update modal closes immediately and returns to update page');
  const progressBox = doc.body.querySelector('.wt-progress');
  assert.ok(progressBox && progressBox.hidden === false, 'update progress bar shown after returning to update tab');
  assert.ok(progressBox.textContent.includes('45%'), progressBox.textContent);
  const installFallbackBtn = doc.body.querySelector('[data-install-github-fallback="true"]');
  assert.ok(installFallbackBtn);
  assert.equal(installFallbackBtn.parentNode.hidden, true);

  // Background download failure surfaces on the update tab alongside a manual GitHub fallback button
  installStatusOverride = { phase: 'failed', percent: 0, message: '准备更新失败：SHA-256 校验失败' };
  await checkBtn.onclick();
  gate = doc.body.querySelector('.wt-gate');
  await gate.findAll(n => n.tagName === 'BUTTON' && n._text === '一键更新')[0].onclick();
  await flush();
  assert.ok(progressBox.textContent.includes('SHA-256'), progressBox.textContent);
  assert.equal(installFallbackBtn.parentNode.hidden, false, 'manual GitHub fallback shown on update tab when background install fails');
  installStatusOverride = null;

  // Perf card: FPS presets and render toggles go through atlas.setPerf.
  const perfCard = cardWithTitle('性能与省内存渲染');
  assert.ok(perfCard, '性能与省内存渲染 card lives on the system tab');
  const perfCheckboxes = doc.body.findAll(n => n.tagName === 'INPUT' && n.type === 'checkbox' && n.getAttribute('data-perf-setting') === 'true');
  assert.equal(perfCheckboxes.length, 3, 'system tab exposes exactly three render toggles');
  assert.deepEqual(
    perfCheckboxes.map(n => n.getAttribute('aria-label')),
    ['空闲停帧', '精简动画效果', '停止渲染桌面主角'],
    'perf toggles are idle pause, reduced motion and hide main pet',
  );
  const fps30 = perfCard.findAll(n => n.tagName === 'BUTTON' && n._text === '30 FPS（省电）')[0];
  assert.ok(fps30, '30 FPS preset exists');
  await fps30.onclick();
  assert.deepEqual({ ...perfCalls.at(-1) }, { fpsCap: 30 }, '30 FPS preset calls setPerf with fpsCap 30');
  assert.ok(fps30.className.includes('wt-primary'), '30 FPS preset becomes primary after selection');
  assert.equal(atlas.perf.value.fpsCap, 30, 'atlas.perf reflects the selected FPS cap');
  const perfBox = label => perfCard.findAll(n => n.tagName === 'INPUT' && n.getAttribute('data-perf-setting') === 'true' && n.getAttribute('aria-label') === label)[0];
  const idlePauseBox = perfBox('空闲停帧');
  const hidePetBox = perfBox('停止渲染桌面主角');
  assert.ok(idlePauseBox && hidePetBox, 'perf checkbox toggles exist');
  idlePauseBox.checked = false;
  await idlePauseBox.onchange();
  assert.deepEqual({ ...perfCalls.at(-1) }, { idlePause: false }, '空闲停帧 toggle calls setPerf with idlePause false');
  assert.equal(atlas.perf.value.idlePause, false, 'atlas.perf reflects idlePause off');
  hidePetBox.checked = true;
  await hidePetBox.onchange();
  assert.deepEqual({ ...perfCalls.at(-1) }, { hideMainPet: true }, '停止渲染桌面主角 toggle calls setPerf with hideMainPet true');
  assert.equal(atlas.perf.value.hideMainPet, true, 'atlas.perf reflects hideMainPet on');

  const assetsCard = cardWithTitle('本机素材资源');
  assert.ok(assetsCard, '本机素材资源 card lives on the system tab');
  assert.equal(assetsCard.findAll(n => n.tagName === 'BUTTON' && n._text === '检查并同步素材缓存').length, 1, '本机素材资源 offers the cache-sync button');

  // Automation tab (`settings` alias): emote triggers, balance policy and batch synthesis.
  dash.showTab('settings');
  await flush();
  assert.equal(dash.currentTab(), 'automation');
  assert.equal(doc.body.querySelector('[data-accessibility-btn]'), null, 'Accessibility permission button must be removed');
  assert.equal(requests.some(r => r.route === '/accessibility/request'), false);

  const speedGrid = doc.body.querySelector('.wt-speeds');
  assert.ok(speedGrid, 'speed presets live on automation, not overview');
  assert.deepEqual(
    speedGrid.findAll(n => n.tagName === 'BUTTON').map(b => b.findAll(s => s.tagName === 'STRONG')[0]?._text),
    ['轻缓 · 1 次/秒', '标准 · 5 次/秒', '快速 · 10 次/秒'],
    'three speed presets are listed with their labels',
  );

  const freqInput = doc.body.querySelector('[aria-label="自动表情平均间隔"]');
  assert.ok(freqInput, 'auto_emote_interval input exists');
  assert.equal(freqInput.value, 25);
  freqInput.value = '12';
  await freqInput.onchange();
  assert.equal(state.settings.auto_emote_interval, 12);

  const randBtn = doc.body.querySelector('[data-emote-random="true"]');
  assert.ok(randBtn, 'auto_emote_random toggle button exists');
  assert.equal(randBtn.textContent, '随机');
  assert.ok(randBtn.className.includes('active'), 'auto_emote_random starts active');
  await randBtn.onclick();
  assert.equal(state.settings.auto_emote_random, false);
  assert.ok(!randBtn.className.includes('active'), 'clicking 随机 turns fluctuation off');
  await randBtn.onclick();
  assert.equal(state.settings.auto_emote_random, true);
  assert.ok(randBtn.className.includes('active'), 'clicking 随机 restores fluctuation on');

  const slotAll = doc.body.querySelector('[data-emote-slot="all"]');
  const slot1 = doc.body.querySelector('[data-emote-slot="1"]');
  const slot2 = doc.body.querySelector('[data-emote-slot="2"]');
  const slot3 = doc.body.querySelector('[data-emote-slot="3"]');
  const slot4 = doc.body.querySelector('[data-emote-slot="4"]');
  const slot5 = doc.body.querySelector('[data-emote-slot="5"]');
  assert.equal(slotAll.textContent, '全部');
  assert.deepEqual(
    [slot1.textContent, slot2.textContent, slot3.textContent, slot4.textContent, slot5.textContent],
    ['1', '2', '3', '4', '5'],
    'slot buttons display only 1..5 without emote names',
  );
  assert.ok(slotAll.className.includes('active'));

  // Clicking slot 2 while 全部 (31) is active selects ONLY slot 2 (bit 2)
  await slot2.onclick();
  assert.equal(state.settings.auto_emote_slots, 2);
  assert.ok(slot2.className.includes('active'));
  assert.ok(!slot1.className.includes('active'));
  assert.ok(!slotAll.className.includes('active'));

  // Clicking slot 3 toggles bit 4 on -> 6 (multi-select slots 2 and 3)
  await slot3.onclick();
  assert.equal(state.settings.auto_emote_slots, 6);

  // Removing slot 2 leaves only empty slot 3 -> hint warns immediately
  await slot2.onclick();
  assert.equal(state.settings.auto_emote_slots, 4);
  const emptyHint = doc.body.querySelector('[data-emote-hint="true"]');
  assert.ok(emptyHint.textContent.includes('暂未装备表情'), emptyHint.textContent);

  // Clicking last remaining active slot (slot 3) resets back to 全部 (31)
  await slot3.onclick();
  assert.equal(state.settings.auto_emote_slots, 31);
  assert.ok(slotAll.className.includes('active'));

  state.runtime.emote = { sent: 4, remaining: 8, reason: 'sent', slots: ['挥手', '点赞', '', '', ''] };
  const refreshLoop = intervals.find(item => item.ms === 2000);
  assert.ok(refreshLoop);
  await refreshLoop.fn();
  const nextHint = doc.body.querySelector('[data-emote-hint="true"]');
  assert.ok(nextHint.textContent.includes('已触发 4 次'), nextHint.textContent);
  assert.ok(nextHint.textContent.includes('8 次'), nextHint.textContent);

  // Automation exposes exactly the five supported toggles; perf toggles now live on system.
  const settingCheckboxes = doc.body.findAll(n => n.tagName === 'INPUT' && n.type === 'checkbox');
  const settingToggles = settingCheckboxes.filter(n => n.getAttribute('data-perf-setting') !== 'true');
  const toggleLabels = settingToggles.map(n => String(n.getAttribute('aria-label') || ''));
  assert.equal(settingToggles.length, 5, 'automation page exposes only the five automation toggles');
  assert.deepEqual(
    toggleLabels,
    ['后台自动点击', '按可用点数自动启停', '自动开宝箱', '打字自动表情', 'Alt+1～5 快捷表情'],
    'automation toggles are enabled / balance / chest / emote / hotkeys',
  );
  assert.ok(
    !toggleLabels.some(label => /游戏模式|显示器|记住/.test(label)),
    'auto_game_mode / remember_monitor toggles must be removed',
  );

  const enabledToggle = doc.body.querySelector('[aria-label="后台自动点击"]');
  const balanceToggle = doc.body.querySelector('[aria-label="按可用点数自动启停"]');
  assert.ok(enabledToggle && balanceToggle, 'enabled and auto_click_balance toggles exist');
  enabledToggle.checked = true;
  await enabledToggle.onchange();
  assert.equal(state.settings.enabled, true);
  balanceToggle.checked = true;
  await balanceToggle.onchange();
  assert.equal(state.settings.auto_click_balance, true);
  enabledToggle.checked = false;
  await enabledToggle.onchange();
  assert.equal(state.settings.enabled, false);
  assert.equal(state.settings.auto_click_balance, true, 'turning off enabled keeps auto_click_balance decoupled');

  state.settings.enabled = true;
  state.runtime.balance_paused = true;
  await refreshLoop.fn();
  const badgeNode = doc.body.findAll(n => n.className.split(/\s+/).includes('wt-badge'))[0];
  assert.ok(badgeNode, 'status badge exists');
  assert.equal(badgeNode.textContent, '点数达上限 · 待机', 'badge shows the balance-paused idle state');
  state.settings.enabled = false;
  state.runtime.balance_paused = false;

  const synthCard = cardWithTitle('背包批量合成');
  assert.ok(synthCard, '背包批量合成 card exists');
  const sameLevelBtn = synthCard.findAll(n => n.tagName === 'BUTTON' && n._text === '批量同阶合成（5 合 1）')[0];
  const upgradeBtn = synthCard.findAll(n => n.tagName === 'BUTTON' && n._text === '批量升阶合成（10 合 1）')[0];
  assert.ok(sameLevelBtn && upgradeBtn, 'both batch synthesis buttons live inside the card');
  await sameLevelBtn.onclick();
  await upgradeBtn.onclick();
  assert.deepEqual(synthKinds, ['same-level', 'upgrade'], 'batch synthesis buttons call the right modes');

  // Automation search: filters setting rows and the 背包批量合成 card.
  const searchInput = doc.body.querySelector('[data-settings-search="true"]');
  assert.ok(searchInput, 'settings search input exists');
  assert.equal(cardWithTitle('快捷键速览'), undefined, '快捷键速览 card removed');
  const automationRow = doc.body.findAll(n => n.className === 'wt-setting' && n.textContent.includes('后台自动点击'))[0];
  assert.ok(automationRow, '后台自动点击 setting row exists');
  const clearSearchBtn = doc.body.findAll(n => n.tagName === 'BUTTON' && n._text === '清空')[0];
  assert.ok(clearSearchBtn, 'search clear button exists');
  assert.equal(clearSearchBtn.disabled, true, 'clear button starts disabled while the search box is empty');
  searchInput.value = '合成';
  await searchInput.oninput();
  assert.equal(synthCard.hidden, false, '背包批量合成 stays visible when searching 合成');
  assert.equal(automationRow.hidden, true, 'unrelated setting rows hide when searching 合成');
  searchInput.value = '不存在关键字xyz';
  await searchInput.oninput();
  const searchEmpty = doc.body.findAll(n => n.className === 'wt-empty' && n.textContent.includes('未找到匹配'))[0];
  assert.ok(searchEmpty, 'empty search hint exists');
  assert.equal(searchEmpty.hidden, false, 'empty search hint is shown when nothing matches');
  assert.equal(synthCard.hidden, true, '背包批量合成 hides when nothing matches');
  await clearSearchBtn.onclick();
  assert.equal(synthCard.hidden, false, 'clearing the search restores the synth card');
  assert.equal(automationRow.hidden, false, 'clearing the search restores setting rows');
  assert.equal(searchEmpty.hidden, true, 'clearing the search hides the empty hint');
  assert.equal(clearSearchBtn.disabled, true, 'clear button is disabled again after clearing');

  // Synth log: first click loads /diagnostic/synth/list and renders entries, second click collapses it.
  const synthLogBtn = doc.body.querySelector('[data-synth-log-btn="true"]');
  assert.ok(synthLogBtn, 'synth log button exists');
  const synthLog = doc.body.querySelector('.wt-synth-log');
  assert.ok(synthLog, 'synth log container exists');
  assert.equal(synthLog.hidden, true, 'synth log starts collapsed');
  await synthLogBtn.onclick();
  await flush();
  assert.ok(requests.some(r => r.route === '/diagnostic/synth/list'), 'opening the synth log requests /diagnostic/synth/list');
  assert.equal(synthLog.hidden, false, 'synth log is visible after opening');
  assert.ok(synthLog.textContent.includes('同阶合成'), synthLog.textContent);
  assert.ok(synthLog.textContent.includes('消耗 5 个'), synthLog.textContent);
  await synthLogBtn.onclick();
  assert.equal(synthLog.hidden, true, 'second click collapses the synth log');

  // Records tab: 7-day statistics and the real chest reward log.
  dash.showTab('records');
  await flush();
  assert.equal(dash.currentTab(), 'records');
  assert.ok(cardWithTitle('最近 7 天统计'), 'records tab has the 最近 7 天统计 card');
  assert.ok(cardWithTitle('真实开箱获得记录'), 'records tab has the 真实开箱获得记录 card');

  // Rooms tab: member layout card, hiding members and layout sliders/arrange buttons.
  dash.showTab('rooms');
  await flush();
  await flush();
  const layoutCard = doc.body.findAll(n => n.className.split(/\s+/).includes('wt-card') && n.findAll(h => h.tagName === 'H3' && h._text === '联机角色样式与布局').length > 0)[0];
  assert.ok(layoutCard, '联机角色样式与布局 card exists');
  assert.ok(layoutCard.textContent.includes('小猫甲'), 'room member 小猫甲 is listed');
  assert.ok(layoutCard.textContent.includes('小猫乙'), 'room member 小猫乙 is listed');
  const hideMemberBtn = layoutCard.findAll(n => n.tagName === 'BUTTON' && n._text === '隐藏')[0];
  assert.ok(hideMemberBtn, 'first room member has a 隐藏 button');
  await hideMemberBtn.onclick();
  assert.deepEqual(toggleHiddenCalls, [101], 'hiding the first member calls toggleRoomMemberHidden(101)');
  assert.ok(layoutCard.findAll(n => n.tagName === 'BUTTON' && n._text === '显示').length > 0, 'hidden member button switches to 显示');
  for (const [label, next] of [['联机角色大小', 1.5], ['联机角色透明度', 0.6], ['联机昵称大小', 1.3]]) {
    const slider = layoutCard.findAll(n => n.tagName === 'INPUT' && n.getAttribute('aria-label') === label)[0];
    assert.ok(slider, label + ' slider exists');
    slider.value = next;
    await slider.oninput();
  }
  assert.deepEqual(
    roomLayoutCalls.map(call => ({ ...call })),
    [{ scale: 1.5 }, { opacity: 0.6 }, { nameScale: 1.3 }],
    'layout sliders call setRoomLayout with the changed key',
  );
  await layoutCard.findAll(n => n.tagName === 'BUTTON' && n._text === '横向排列')[0].onclick();
  await layoutCard.findAll(n => n.tagName === 'BUTTON' && n._text === '纵向排列')[0].onclick();
  await layoutCard.findAll(n => n.tagName === 'BUTTON' && n._text === '重置位置')[0].onclick();
  assert.deepEqual(arrangeCalls, ['horizontal', 'vertical'], 'arrange buttons request horizontal and vertical layouts');
  assert.equal(clearPositionCalls.length, 1, '重置位置 calls clearRoomPositions');

  // Mods tab: transform editor, saved list, resource packs, OBS green screen and panel look.
  dash.showTab('mods');
  await flush();
  await flush();
  assert.equal(dash.currentTab(), 'mods');
  assert.equal(doc.body.findAll(n => n.tagName === 'BUTTON' && n._text === '检查并同步官方图鉴').length, 0, 'mods tab must not duplicate 检查并同步官方图鉴');
  const xformCard = cardWithTitle('挂饰位置 / 大小 / 角度');
  assert.ok(xformCard, '挂饰位置 / 大小 / 角度 card exists');
  assert.equal(xformCard.findAll(n => n.tagName === 'INPUT' && n.type === 'range').length, 4, 'four transform sliders exist');
  const packsCard = cardWithTitle('已安装的资源包');
  assert.ok(packsCard, '已安装的资源包 card exists');
  assert.equal(packsCard.findAll(n => n.tagName === 'BUTTON' && n._text === '导入 Mod ZIP').length, 1, 'resource pack import button exists');
  const obsCard = cardWithTitle('OBS 桌宠绿幕');
  assert.ok(obsCard, 'OBS 桌宠绿幕 card lives on mods');
  assert.deepEqual(obsCard.findAll(n => n.tagName === 'BUTTON').map(n => n._text), ['关闭', '开启绿幕'], 'OBS card only offers off/green');
  assert.ok(!obsCard.findAll(n => n.tagName === 'BUTTON').some(n => n._text.includes('透明')), 'OBS card no longer offers 透明');
  const lookCard = cardWithTitle('面板外观');
  assert.ok(lookCard, '面板外观 card exists');
  assert.notEqual(lookCard, obsCard, '面板外观 is a separate card');
  assert.equal(lookCard.findAll(n => n.tagName === 'BUTTON' && n._text === '清除背景图').length, 1, '面板外观 card offers 清除背景图');
  const accessoryLabel = doc.body.findAll(n => n.tagName === 'P' && n.textContent.includes('当前挂饰：'))[0];
  assert.ok(accessoryLabel, 'current accessory label exists');
  assert.ok(accessoryLabel.textContent.includes('当前挂饰：official_hat.png'), accessoryLabel.textContent);
  const savedSection = doc.body.findAll(n => n.className === 'wt-notes' && n.textContent.includes('已保存的挂饰微调'))[0];
  assert.ok(savedSection, '已保存的挂饰微调 section exists');
  const savedRows = () => savedSection.findAll(n => n.className === 'wt-version');
  assert.equal(savedRows().length, 2, 'saved accessory transform list has two entries');
  assert.ok(savedSection.textContent.includes('official_hat.png') && savedSection.textContent.includes('local_star.png'), 'saved list shows both accessories');
  const editTransformBtn = savedSection.findAll(n => n.tagName === 'BUTTON' && n._text === '加载编辑')[0];
  assert.ok(editTransformBtn, '加载编辑 button exists');
  await editTransformBtn.onclick();
  assert.ok(accessoryLabel.textContent.includes('编辑中：'), accessoryLabel.textContent);
  assert.ok(accessoryLabel.textContent.includes('official_hat.png'), accessoryLabel.textContent);
  const deleteTransformBtn = savedSection.findAll(n => n.tagName === 'BUTTON' && n._text === '删除')[0];
  assert.ok(deleteTransformBtn, '删除 button exists');
  await deleteTransformBtn.onclick();
  assert.ok(transformCalls.some(call => call.delete === true && String(call.id) === '88'), 'deleting calls deleteAccessoryTransform(88)');
  assert.equal(savedRows().length, 1, 'deleted accessory transform is removed from the list');
  assert.equal(savedSection.textContent.includes('official_hat.png'), false, 'deleted entry disappears from the saved list');

  dash.close();
}

async function testClickDrivenAutoEmoteAndAutoClickAvoidance(platform) {
  const source = renderFrontend('desktop_assistant.js', platform);
  let now = 200000;
  const tasks = [];
  const listeners = new Map();
  const sentEmotes = [];
  const requests = [];
  let clickListener = null;
  let f24Presses = 0;

  const win = {
    document: { querySelector: () => null },
    addEventListener(name, fn) { listeners.set(name, fn); },
    setTimeout(fn, ms) {
      const task = { id: tasks.length + 1, at: now + ms, fn };
      tasks.push(task);
      return task.id;
    },
    clearTimeout(id) {
      const idx = tasks.findIndex(t => t.id === id);
      if (idx !== -1) tasks.splice(idx, 1);
    },
    setInterval() { return 1; },
    clearInterval() {},
  };

  let allowed = false;
  const settings = {
    enabled: true,
    speed: 10,
    auto_chest: false,
    auto_emote: true,
    auto_emote_interval: 5,
    auto_emote_random: true,
    auto_emote_slots: 31,
    threshold: 1000,
  };
  const atlas = {
    enabled: { value: false },
    ready: Promise.resolve(),
    async request(route, body) {
      requests.push({ route, body });
      if (route === '/assistant/configure') Object.assign(settings, body.settings);
      if (route === '/assistant/tick') return { allowed, quiet: true, settings: { ...settings } };
      if (route === '/update/status') return { required: false };
      return { settings: { ...settings }, session: 0, confirmed: 0, rooms: [], rewards: [] };
    },
  };

  const FakeDate = class extends Date { static now() { return now; } };
  const ctx = vm.createContext({ window: win, Date: FakeDate, Math, Map, Number, Promise, Array, String, console });
  vm.runInContext(source + ';globalThis.make = createDesktopAssistant;', ctx);
  const assistant = ctx.make({ window: win, ref: v => ({ value: v }), atlas });
  const equipped = [
    { id: 101, name: 'slot1.png', src: 'asset://emote/slot1.png' },
    { id: 102, name: 'slot2.png', src: 'asset://emote/slot2.png' },
    { id: 103, name: 'slot3.png', src: 'asset://emote/slot3.png' },
  ];

  await assistant.startMain({
    total: () => f24Presses,
    balance: () => 0,
    account: () => 'acc',
    loggedIn: () => false,
    blocked: () => false,
    press(key) {
      assert.equal(key, 'F24');
      f24Presses++;
      // Virtual F24 synchronously increments store.localClickCount in the real app!
      clickListener?.(1);
    },
    release() {},
    chestCost: 1000,
    emotes: () => equipped,
    sendEmote: item => sentEmotes.push(item),
    currentMonitor: async () => null,
    listenClicks: cb => { clickListener = cb; return () => { clickListener = null; }; },
    listenRoom() {},
    listenJoin: async () => {},
  });

  async function stepDue() {
    tasks.sort((a, b) => a.at - b.at);
    const task = tasks.shift();
    assert.ok(task, 'expected scheduled task');
    now = Math.max(now, task.at);
    await task.fn();
  }

  // 1. Virtual F24 auto-clicks must NEVER count toward or trigger auto-emote
  allowed = true;
  for (let i = 0; i < 40; i++) {
    await stepDue();
  }
  assert.equal(f24Presses, 40, 'virtual auto-click runs normally');
  assert.equal(assistant.emoteStats.clicks, 0, 'virtual F24 must be excluded from emote click count');
  assert.equal(sentEmotes.length, 0, 'virtual F24 must never trigger emotes');

  // 2. Select only slot 2 via bitmask (2), sync via tick, then fire real clicks (interval=5 -> 3..7 clicks)
  settings.auto_emote_slots = 2;
  allowed = false;
  await stepDue();
  let realClicks = 0;
  for (let i = 0; i < 7 && sentEmotes.length === 0; i++) {
    clickListener(1);
    realClicks++;
  }
  assert.equal(assistant.emoteStats.clicks, realClicks);
  assert.equal(sentEmotes.length, 1, 'real clicks trigger emote around auto_emote_interval=5');
  assert.equal(sentEmotes[0].id, 102, 'only slot 2 (mask=2) is chosen');

  // 3. Yield window: virtual F24 auto-click pauses for 350ms right after an emote fires
  allowed = true;
  const pressesBeforeYield = f24Presses;
  await stepDue(); // fires +50ms after emote, still inside 350ms yield window
  assert.equal(f24Presses, pressesBeforeYield, 'virtual F24 yields immediately after emote fires');

  // Once 350ms yield window elapses, virtual F24 resumes cleanly
  for (let i = 0; i < 8 && f24Presses === pressesBeforeYield; i++) {
    await stepDue();
  }
  assert.equal(f24Presses, pressesBeforeYield + 1, 'virtual F24 resumes after yield window');

  // 4. Multiple triggers with mask=5 (slots 1 and 3) never choose slot 2
  settings.auto_emote_slots = 5;
  allowed = false;
  await stepDue();
  for (let round = 0; round < 30; round++) {
    now += 200;
    clickListener(1);
  }
  const postMask13 = sentEmotes.slice(1);
  assert.ok(postMask13.length >= 2);
  assert.ok(postMask13.every(item => item.id === 101 || item.id === 103), 'mask=5 picks only slots 1 and 3');

  // 5. Empty selected slot (slot 4 -> mask=8) never fires and sets slot-empty reason
  settings.auto_emote_slots = 8;
  await stepDue();
  const beforeEmpty = sentEmotes.length;
  for (let round = 0; round < 10; round++) {
    now += 200;
    clickListener(1);
  }
  assert.equal(sentEmotes.length, beforeEmpty);
  assert.equal(assistant.emoteStats.reason, 'slot-empty');

  // 6. Fixed interval (auto_emote_random = false): triggers strictly every 5 clicks (5th and 10th)
  settings.auto_emote_slots = 31;
  settings.auto_emote_random = false;
  settings.auto_emote_interval = 5;
  allowed = false;
  await stepDue();
  const baseCount = sentEmotes.length;
  for (let c = 1; c <= 10; c++) {
    now += 200;
    clickListener(1);
    const expectedTriggered = c < 5 ? 0 : c < 10 ? 1 : 2;
    assert.equal(
      sentEmotes.length - baseCount,
      expectedTriggered,
      `fixed interval=5 should fire at clicks 5 and 10 (click ${c})`,
    );
  }

  // 7. Global Alt+1~5 hotkeys trigger without DOM focus, support both Windows (Digit2) and macOS ('3'), and dedup with DOM keydown.
  settings.emote_hotkeys = false;
  await stepDue();
  const beforeDisabledHotkey = sentEmotes.length;
  assert.equal(assistant.onGlobalKey('AltLeft', true), false);
  assert.equal(assistant.onGlobalKey('Digit2', true), false);
  assistant.onGlobalKey('Digit2', false);
  assistant.onGlobalKey('AltLeft', false);
  assert.equal(sentEmotes.length, beforeDisabledHotkey, 'global hotkeys respect emote_hotkeys = false');

  settings.emote_hotkeys = true;
  await stepDue();
  assert.equal(assistant.onGlobalKey('Digit2', true), false, 'pressing Digit2 without Alt held does not fire');
  assert.equal(assistant.onGlobalKey('1', true), false, 'pressing bare 1 without Alt held does not fire');
  assert.equal(assistant.onGlobalKey('AltLeft', true), false);
  assert.equal(assistant.onGlobalKey('Digit2', true), true, 'global AltLeft+Digit2 triggers slot 2 without window focus');
  assert.equal(sentEmotes.at(-1).id, 102);
  let prevented = false;
  listeners.get('keydown')?.({
    code: 'Digit2',
    altKey: true,
    repeat: false,
    preventDefault() { prevented = true; },
    stopPropagation() {},
  });
  assert.equal(prevented, false, 'immediate DOM keydown within 120ms is deduplicated');
  assert.equal(sentEmotes.length, beforeDisabledHotkey + 1, 'no duplicate emote sent');
  assistant.onGlobalKey('Digit2', false);
  assistant.onGlobalKey('AltLeft', false);

  now += 200;
  assistant.onGlobalKey('AltRight', true);
  assert.equal(assistant.onGlobalKey('3', true), true, 'macOS style global AltRight+3 triggers slot 3');
  assert.equal(sentEmotes.at(-1).id, 103);
  assistant.onGlobalKey('3', false);
  assistant.onGlobalKey('AltRight', false);

  now += 200;
  assistant.onGlobalKey('Alt', true);
  assert.equal(assistant.onGlobalKey('Num1', true), true, 'Windows rdev Alt+Num1 triggers slot 1');
  assert.equal(sentEmotes.at(-1).id, 101);
  assistant.onGlobalKey('Num1', false);
  assistant.onGlobalKey('Alt', false);

  now += 200;
  assistant.onGlobalKey('AltGr', true);
  assert.equal(assistant.onGlobalKey('Kp3', true), true, 'Windows rdev AltGr+Kp3 triggers slot 3');
  assert.equal(sentEmotes.at(-1).id, 103);
  assistant.onGlobalKey('Kp3', false);
  assistant.onGlobalKey('AltGr', false);

  assistant.onGlobalKey('AltLeft', true);
  listeners.get('blur')?.();
  assert.equal(assistant.onGlobalKey('1', true), false, 'window blur clears heldAlt so bare 1 cannot fire');
  assistant.onGlobalKey('1', false);

  await stepDue();
  const lastTick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.deepEqual([...lastTick.body.emote.slots], ['slot1', 'slot2', 'slot3', '', '']);
  listeners.get('pagehide')?.();
  assert.equal(clickListener, null, 'click watcher cleaned up on pagehide');
}

async function testDriverBalanceHybridAndSoftUpdate(platform) {
  const source = renderFrontend('desktop_assistant.js', platform);
  let now = 500000;
  const tasks = [];
  const intervals = [];
  const requests = [];
  const storage = new Map();
  const listeners = new Map();
  let f24Presses = 0;
  let roomListener = null;

  function makeNode(tag) {
    return {
      tagName: String(tag).toUpperCase(),
      type: '',
      className: '',
      style: {},
      children: [],
      parentNode: null,
      _text: '',
      get textContent() {
        return this.children.length ? this.children.map(c => c.textContent).join('') : this._text;
      },
      set textContent(v) {
        this._text = String(v ?? '');
        this.children = [];
      },
      append(...items) {
        for (const item of items) {
          if (!item || typeof item !== 'object') continue;
          item.parentNode = this;
          this.children.push(item);
        }
      },
      remove() {
        if (!this.parentNode) return;
        this.parentNode.children = this.parentNode.children.filter(c => c !== this);
        this.parentNode = null;
      },
      setAttribute() {},
      getAttribute() { return null; },
      findAll(predicate, out = []) {
        if (predicate(this)) out.push(this);
        for (const child of this.children) child.findAll(predicate, out);
        return out;
      },
      querySelector(selector) {
        if (selector.startsWith('.') && !selector.includes(' ')) {
          return this.findAll(n => n.className.split(/\s+/).includes(selector.slice(1)))[0] || null;
        }
        return null;
      },
    };
  }

  const body = makeNode('body');
  const doc = {
    body,
    createElement: makeNode,
    querySelector: selector => body.querySelector(selector),
  };

  const win = {
    document: doc,
    localStorage: {
      getItem: key => (storage.has(key) ? storage.get(key) : null),
      setItem: (key, value) => { storage.set(key, String(value)); },
      removeItem: key => { storage.delete(key); },
    },
    addEventListener(name, fn) { listeners.set(name, fn); },
    removeEventListener(name) { listeners.delete(name); },
    setTimeout(fn, ms) {
      const task = { id: tasks.length + 1, at: now + ms, fn };
      tasks.push(task);
      return task.id;
    },
    clearTimeout(id) {
      const idx = tasks.findIndex(t => t.id === id);
      if (idx !== -1) tasks.splice(idx, 1);
    },
    setInterval(fn, ms) {
      const id = intervals.length + 1;
      intervals.push({ id, fn, ms });
      return id;
    },
    clearInterval(id) {
      const idx = intervals.findIndex(t => t.id === id);
      if (idx !== -1) intervals.splice(idx, 1);
    },
  };

  let allowed = true;
  let balanceValue = 3000;
  const settings = {
    enabled: true,
    auto_click_balance: true,
    auto_click_below: 1000,
    auto_click_above: 2000,
    auto_chest: false,
    auto_emote: false,
    auto_emote_interval: 25,
    auto_emote_random: true,
    auto_emote_slots: 31,
    threshold: 1000,
    emote_hotkeys: false,
  };
  const atlas = {
    enabled: { value: false },
    ready: Promise.resolve(),
    async request(route, payload) {
      requests.push({ route, body: payload });
      if (route === '/assistant/state') return { settings: { ...settings }, runtime: {}, update_required: false };
      if (route === '/assistant/renderer-start') return { settings: { ...settings } };
      if (route === '/assistant/tick') return { allowed, quiet: true, settings: { ...settings } };
      if (route === '/assistant/configure') { Object.assign(settings, payload.settings); return { settings: { ...settings } }; }
      if (route === '/community/update-check') return { updateAvailable: true, latest: '9.9.9', current: '1.4.0-beta.2' };
      if (route === '/update/status') return { required: false };
      return { settings: { ...settings }, session: 0, confirmed: 0, rooms: [], rewards: [] };
    },
  };

  const FakeDate = class extends Date { static now() { return now; } };
  const ctx = vm.createContext({ window: win, Date: FakeDate, Math, Map, Number, Promise, Array, String, JSON, console });
  vm.runInContext(source + ';globalThis.make = createDesktopAssistant;', ctx);
  const assistant = ctx.make({ window: win, ref: value => ({ value }), atlas });

  const startNow = now;
  await assistant.startMain({
    total: () => 0,
    balance: () => balanceValue,
    account: () => 'acc',
    loggedIn: () => true,
    blocked: () => false,
    press(key) {
      assert.equal(key, 'F24');
      f24Presses++;
    },
    release() {},
    chestCost: 1000,
    emotes: () => [],
    sendEmote() {},
    currentMonitor: async () => null,
    listenClicks: () => () => {},
    listenRoom: cb => { roomListener = cb; },
    listenJoin: async () => {},
  });

  async function stepDue() {
    tasks.sort((a, b) => a.at - b.at);
    const task = tasks.shift();
    assert.ok(task, 'expected scheduled task');
    now = Math.max(now, task.at);
    await task.fn();
  }

  // (a) Hysteresis: above the upper bound pauses virtual F24 without touching the enabled switch.
  await stepDue();
  let tick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.equal(tick.body.balance_paused, true, 'balance_paused is reported once balance exceeds the upper bound');
  assert.equal(f24Presses, 0, 'virtual F24 pauses while balance is above the upper bound');
  assert.equal(
    requests.some(r => r.route === '/assistant/configure' && r.body?.settings?.enabled === false),
    false,
    'auto-pause must not disable the enabled master switch through /assistant/configure',
  );

  balanceValue = 1500;
  await stepDue();
  tick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.equal(tick.body.balance_paused, true, 'the middle band preserves the paused state (hysteresis)');
  assert.equal(f24Presses, 0, 'virtual F24 stays paused inside the hysteresis band');

  balanceValue = 500;
  await stepDue();
  tick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.equal(tick.body.balance_paused, false, 'balance_paused clears below the lower bound');
  assert.equal(f24Presses, 1, 'virtual F24 resumes after balance drops below auto_click_below');

  // (b) Hybrid mode records room history through /assistant/room.
  atlas.enabled.value = true;
  assert.ok(roomListener, 'room listener registered');
  await roomListener('654321', 'Host');
  let roomReq = requests.filter(r => r.route === '/assistant/room').at(-1);
  assert.ok(roomReq, 'hybrid mode records the room');
  assert.equal(roomReq.body.code, '654321');
  assert.equal(roomReq.body.host, 'Host');
  assert.equal(roomReq.body.new_visit, true, 'first visit is marked new');
  await roomListener('654321', 'Host');
  roomReq = requests.filter(r => r.route === '/assistant/room').at(-1);
  assert.equal(roomReq.body.new_visit, false, 're-entering the same room is not a new visit');

  // (c) Periodic non-forced update check shows a dismissible soft banner.
  settings.enabled = false;
  allowed = false;
  const kickoff = tasks.find(t => t.at === startNow + 15000 && t.fn.name === 'checkCommunityUpdate');
  assert.ok(kickoff, 'the first update check is scheduled 15s after start');
  const updateInterval = intervals.find(i => i.ms === 8 * 3600 * 1000 && i.fn.name === 'checkCommunityUpdate');
  assert.ok(updateInterval, 'the update check repeats every 8h');
  await kickoff.fn();

  const updateCheck = requests.filter(r => r.route === '/community/update-check').at(-1);
  assert.ok(updateCheck, 'periodic check calls /community/update-check');
  assert.equal(updateCheck.body.force, false, 'the periodic update check is not forced');
  assert.equal(assistant.updateRequired.value, false, 'the soft banner never forces the required-update gate');
  const banner = () => body.querySelector('.wt-soft-update');
  assert.ok(banner(), '.wt-soft-update banner is shown');
  assert.ok(banner().textContent.includes('9.9.9'), banner().textContent);

  const dismiss = banner().findAll(n => n.tagName === 'BUTTON' && n._text === '忽略此版本')[0];
  assert.ok(dismiss, '忽略此版本 button exists');
  dismiss.onclick();
  assert.equal(storage.get('wuwatappo-dismissed-update-v1'), '9.9.9', 'dismissed version is stored in localStorage');
  assert.equal(banner(), null, 'banner is removed after dismissing');

  await updateInterval.fn();
  assert.equal(banner(), null, 'the same dismissed version is not shown again');
  assert.equal(assistant.updateRequired.value, false);

  listeners.get('pagehide')?.();
}

async function testBalanceInitialAndLoggedOutGuard(platform) {
  const source = renderFrontend('desktop_assistant.js', platform);
  let now = 5_000_000;
  let f24Presses = 0;
  let loggedInValue = false;
  let balanceValue = 0;
  const tasks = [];
  const intervals = [];
  const requests = [];
  const storage = new Map();
  const listeners = new Map();
  const body = { querySelector: () => null };
  const doc = { body, querySelector: () => null };
  const win = {
    document: doc,
    localStorage: {
      getItem: key => (storage.has(key) ? storage.get(key) : null),
      setItem: (key, value) => { storage.set(key, String(value)); },
      removeItem: key => { storage.delete(key); },
    },
    addEventListener(name, fn) { listeners.set(name, fn); },
    removeEventListener(name) { listeners.delete(name); },
    setTimeout(fn, ms) {
      const task = { id: tasks.length + 1, at: now + ms, fn };
      tasks.push(task);
      return task.id;
    },
    clearTimeout(id) {
      const idx = tasks.findIndex(t => t.id === id);
      if (idx !== -1) tasks.splice(idx, 1);
    },
    setInterval(fn, ms) {
      const id = intervals.length + 1;
      intervals.push({ id, fn, ms });
      return id;
    },
    clearInterval(id) {
      const idx = intervals.findIndex(t => t.id === id);
      if (idx !== -1) intervals.splice(idx, 1);
    },
  };
  const settings = {
    enabled: true,
    auto_click_balance: true,
    auto_click_below: 1000,
    auto_click_above: 2000,
    auto_chest: false,
    auto_emote: false,
    auto_emote_interval: 25,
    auto_emote_random: true,
    auto_emote_slots: 31,
    threshold: 1000,
    emote_hotkeys: false,
  };
  const atlas = {
    enabled: { value: false },
    ready: Promise.resolve(),
    async request(route, payload) {
      requests.push({ route, body: payload });
      if (route === '/assistant/state') return { settings: { ...settings }, runtime: {}, update_required: false };
      if (route === '/assistant/tick') return { allowed: true, quiet: true, settings: { ...settings } };
      if (route === '/update/status') return { required: false };
      return { settings: { ...settings } };
    },
  };
  const FakeDate = class extends Date { static now() { return now; } };
  const ctx = vm.createContext({ window: win, Date: FakeDate, Math, Map, Set, Number, Promise, Array, String, JSON, console });
  vm.runInContext(source + ';globalThis.make = createDesktopAssistant;', ctx);
  const assistant = ctx.make({ window: win, ref: value => ({ value }), atlas });

  await assistant.startMain({
    total: () => 0,
    balance: () => balanceValue,
    account: () => (loggedInValue ? 'acc' : ''),
    loggedIn: () => loggedInValue,
    blocked: () => false,
    press() { f24Presses++; },
    release() {},
    chestCost: 1000,
    emotes: () => [],
    sendEmote() {},
    currentMonitor: async () => null,
    listenClicks: () => () => {},
    listenRoom: () => {},
    listenJoin: async () => {},
  });

  async function stepDue() {
    tasks.sort((a, b) => a.at - b.at);
    const task = tasks.shift();
    assert.ok(task, 'expected scheduled task');
    now = Math.max(now, task.at);
    await task.fn();
  }

  // Not logged in with initial balance 0 must NOT start clicking.
  await stepDue();
  let tick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.equal(tick.body.balance_paused, true, 'stays paused before login even when balance is 0');
  assert.equal(f24Presses, 0);

  // Logging in inside the middle band [1000, 2000] must stay paused until balance drops below 1000.
  loggedInValue = true;
  balanceValue = 1500;
  await stepDue();
  tick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.equal(tick.body.balance_paused, true, 'middle band on initial login stays paused until below auto_click_below');
  assert.equal(f24Presses, 0);

  // Dropping strictly below auto_click_below starts auto-click and keeps going through middle band.
  balanceValue = 800;
  await stepDue();
  tick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.equal(tick.body.balance_paused, false);
  assert.equal(f24Presses, 1);

  balanceValue = 1500;
  await stepDue();
  tick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.equal(tick.body.balance_paused, false, 'continues auto-clicking across middle band after dropping below lower bound');
  assert.equal(f24Presses, 2);

  balanceValue = 2500;
  await stepDue();
  tick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.equal(tick.body.balance_paused, true, 'pauses again above upper bound');
  assert.equal(f24Presses, 2);

  listeners.get('pagehide')?.();
}

async function testDesktopContextMenu(platform) {
  const source = renderFrontend('desktop_context_menu.js', platform);
  const ctx = vm.createContext({ Map, Error, console });
  vm.runInContext(source + ';globalThis.createMenu = createDesktopContextMenu;', ctx);
  const emits = [];
  const requests = [];
  let nextAction = 'switch-display';
  const menu = ctx.createMenu({
    invoke: async (cmd, payload) => { emits.push({ cmd, payload }); },
    request: async (route, body) => {
      requests.push({ route, body });
      if (route === '/assistant/context-menu') return { action: nextAction };
      return { ok: true };
    },
    settings: () => ({ windowPassthrough: false, gameMode: true, windowOpacity: 0, petScale: 100 }),
    visible: () => true,
    labels: async () => ({ switchDisplay: 'Switch Display' }),
  });

  const actions = { openTab() {}, openConsole() {} };
  await menu.initialize(actions);
  await menu.popup(10, 20, actions);
  const menuItems = requests.find(r => r.route === '/assistant/context-menu').body.items;
  const flatKeys = menuItems.flatMap(item => item.items ? item.items.map(sub => sub.key) : [item.key]).filter(Boolean);
  assert.ok(flatKeys.includes('switch-display'), 'switch-display menu item exists');
  assert.ok(flatKeys.includes('opacity-0'), '0% opacity preset exists');
  assert.ok(flatKeys.includes('open-logs'), 'open-logs menu item exists');
  const hideIdx = menuItems.findIndex(item => item.key === 'hide');
  assert.equal(menuItems[hideIdx + 1]?.key, 'switch-display');
  const opacityMenu = menuItems.find(item => Array.isArray(item.items) && item.items.some(sub => sub.key === 'opacity-0'));
  assert.equal(opacityMenu.items.find(sub => sub.key === 'opacity-0')?.checked, true);
  assert.equal(emits.at(-1)?.cmd, 'plugin:event|emit');
  assert.equal(emits.at(-1)?.payload?.event, 'tray-switch-display');
  assert.equal(emits.at(-1)?.payload?.payload, null);

  nextAction = 'open-logs';
  await menu.popup(10, 20, actions);
  assert.ok(requests.some(r => r.route === '/diagnostic/open-logs'), 'open-logs posts to /diagnostic/open-logs');
}

(async () => {
  for (const platform of ['mac', 'win']) {
    await testDashboard(platform);
    await testClickDrivenAutoEmoteAndAutoClickAvoidance(platform);
    await testDriverBalanceHybridAndSoftUpdate(platform);
    await testBalanceInitialAndLoggedOutGuard(platform);
    await testDesktopContextMenu(platform);
  }
  console.log('dashboard, context menu, and emote regression passed (macOS + Windows): Mod tab rename, update action order, settings cards/decoupling, balance hysteresis, hybrid room history, soft update dismissal + menu parity');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
