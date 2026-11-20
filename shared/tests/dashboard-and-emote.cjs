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
      remember_monitor: false,
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

  let communityMode = 'error';
  let installMode = 'error';
  let installStatusOverride = null;
  let statusCalls = 0;
  const atlas = {
    enabled: { value: false },
    busy: { value: false },
    cacheReady: { value: true },
    assistant: { emoteStats: {} },
    prepare: async () => {},
    setEnabled: async v => { atlas.enabled.value = v; },
    async request(route, body) {
      requests.push({ route, body });
      if (route === '/community/version') return { current: '1.3.2-beta.3' };
      if (route === '/official/update-check') return { latestVersion: '1.0.4', currentVersion: '1.0.4', updateAvailable: false };
      if (route === '/custom/status') return { packs: [], atlas_count: 10, asset_count: 20 };
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
  for (const key of ['rewards', 'daily', 'rooms', 'mods', 'update', 'settings', 'overview']) {
    dash.showTab(key);
    await flush();
  }

  // Test `update` tab: no ReferenceError on fields, no stuck 检查更新 button, prerelease toggle re-checks
  dash.showTab('update');
  await flush();
  await flush();
  const checkBtn = doc.body.findAll(n => n.tagName === 'BUTTON' && n._text === '检查更新')[0];
  const preBox = doc.body.querySelector('[aria-label="包含预发布版本"]');
  assert.ok(checkBtn, '检查更新 button exists');
  assert.ok(preBox, '包含预发布版本 toggle exists');

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

  // Test `settings` tab: no Accessibility button, frequency input, 1-5 slot picker, live emote hint
  dash.showTab('settings');
  await flush();
  assert.equal(doc.body.querySelector('[data-accessibility-btn]'), null, 'Accessibility permission button must be removed');
  assert.equal(requests.some(r => r.route === '/accessibility/request'), false);

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
    remember_monitor: false,
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

  await stepDue();
  const lastTick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.deepEqual([...lastTick.body.emote.slots], ['slot1', 'slot2', 'slot3', '', '']);
  listeners.get('pagehide')?.();
  assert.equal(clickListener, null, 'click watcher cleaned up on pagehide');
}

(async () => {
  for (const platform of ['mac', 'win']) {
    await testDashboard(platform);
    await testClickDrivenAutoEmoteAndAutoClickAvoidance(platform);
  }
  console.log('dashboard and emote regression passed (macOS + Windows): 7 tabs, interval + 1-5 slot picker, no accessibility prompt, click-driven emote & F24 yield');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
