// Regression: dashboard update tab, cross-window emote stats, macOS accessibility prompt,
// and slow Windows UIA focus probe (>500ms) in desktop_assistant.js.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

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

async function testDashboard(targetPath) {
  const source = fs.readFileSync(path.join(__dirname, targetPath), 'utf8');
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
      threshold: 1000,
      remember_monitor: false,
      prerelease_updates: false,
      speed: 5,
    },
    runtime: {
      logged_in: true,
      balance: 3000,
      total: 5000,
      accessibility_granted: false,
      emote: { sent: 0, reason: '' },
    },
  };

  let communityMode = 'error';
  let statusCalls = 0;
  let accessRequests = 0;
  const atlas = {
    enabled: { value: false },
    busy: { value: false },
    cacheReady: { value: true },
    assistant: { emoteStats: {} },
    prepare: async () => {},
    setEnabled: async v => { atlas.enabled.value = v; },
    async request(route, body) {
      requests.push({ route, body });
      if (route === '/community/version') return { current: '1.3.2-beta.2' };
      if (route === '/official/update-check') return { latestVersion: '1.0.4', currentVersion: '1.0.4', updateAvailable: false };
      if (route === '/custom/status') return { packs: [], atlas_count: 10, asset_count: 20 };
      if (route === '/update/status') {
        statusCalls++;
        if (statusCalls <= 2) return { required: false, current: '1.3.2-beta.2', github_version: '', checked_at: 0, error: null, release_notes: '' };
        return { required: false, current: '1.3.2-beta.2', github_version: '1.3.2', checked_at: 100, release_notes: 'notes' };
      }
      if (route === '/update/check') throw new Error('offline');
      if (route === '/community/update-check') {
        if (communityMode === 'error') return { current: '1.3.2-beta.2', latest: '', updateAvailable: false, error: 'rate limit', message: 'GitHub 接口限流' };
        return { current: '1.3.2-beta.2', latest: '1.3.2', updateAvailable: false, releaseNotes: 'Fixed' };
      }
      if (route === '/accessibility/request') {
        accessRequests++;
        state.runtime.accessibility_granted = true;
        return { ok: true, granted: true };
      }
      return {};
    },
  };

  const ctx = vm.createContext({ window: win, Date, Number, Promise, console, Object, Math });
  vm.runInContext(source + ';globalThis.createDashboard = createAssistantDashboard;', ctx);

  const dash = ctx.createDashboard({
    window: win,
    version: '1.3.2-beta.2',
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

  // Test `settings` tab: cross-window runtime.emote + accessibility_granted + button action
  dash.showTab('settings');
  await flush();
  const hint = doc.body.querySelector('[data-emote-hint="true"]');
  const accessBtn = doc.body.querySelector('[data-accessibility-btn="true"]');
  assert.ok(hint.textContent.includes('辅助功能'), hint.textContent);
  assert.equal(accessBtn.hidden, false);

  await accessBtn.onclick();
  assert.equal(accessRequests, 1);

  state.runtime.emote = { sent: 4, reason: 'sent' };
  const refreshLoop = intervals.find(item => item.ms === 2000);
  assert.ok(refreshLoop);
  await refreshLoop.fn();
  const nextHint = doc.body.querySelector('[data-emote-hint="true"]');
  const nextAccessBtn = doc.body.querySelector('[data-accessibility-btn="true"]');
  assert.equal(nextAccessBtn.hidden, true);
  assert.ok(nextHint.textContent.includes('已触发 4 次'), nextHint.textContent);

  dash.close();
}

async function testDelayedTypingContextAndTickEmote(targetPath) {
  const source = fs.readFileSync(path.join(__dirname, targetPath), 'utf8');
  let now = 200000;
  const tasks = [];
  const listeners = new Map();
  const sentEmotes = [];
  const requests = [];

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

  let resolveTypingProbe;
  const settings = {
    enabled: false,
    speed: 5,
    auto_chest: false,
    auto_emote: true,
    threshold: 1000,
    remember_monitor: false,
  };
  const atlas = {
    enabled: { value: false },
    ready: Promise.resolve(),
    async request(route, body) {
      requests.push({ route, body });
      if (route === '/assistant/typing-context') {
        return new Promise(resolve => { resolveTypingProbe = resolve; });
      }
      if (route === '/assistant/tick') return { allowed: false, quiet: true, settings };
      if (route === '/update/status') return { required: false };
      return { settings, session: 0, confirmed: 0, rooms: [], rewards: [] };
    },
  };

  const FakeDate = class extends Date { static now() { return now; } };
  const ctx = vm.createContext({ window: win, Date: FakeDate, Math, Map, Number, Promise, console });
  vm.runInContext(source + ';globalThis.make = createDesktopAssistant;', ctx);
  const assistant = ctx.make({ window: win, ref: v => ({ value: v }), atlas });
  await assistant.startMain({
    total: () => 0,
    balance: () => 0,
    account: () => 'acc',
    loggedIn: () => false,
    blocked: () => false,
    press() {},
    release() {},
    chestCost: 1000,
    emotes: () => [{ id: 7, name: 'wave.png', src: 'asset://emote/wave.png' }],
    sendEmote: item => sentEmotes.push(item),
    currentMonitor: async () => null,
    listenRoom() {},
    listenJoin: async () => {},
  });

  // Type two eligible physical keys 50ms apart
  assistant.physicalKey('KeyA');
  now += 50;
  assistant.physicalKey('KeyB');
  await flush();
  assert.ok(resolveTypingProbe, 'typing-context probe started after 2 keys');

  // Simulate 900ms UIA delay (> 500ms, < 3000ms)
  now += 900;
  resolveTypingProbe({ editable: true, reason: 'editable', source: 'uia-fallback' });
  await flush();
  await flush();
  assert.equal(sentEmotes.length, 1, 'Slow >500ms focus probe must still trigger emote');

  // Advance timer so `tick` fires and carries `emote` stats for panel synchronization
  now += 1100;
  const tickTask = tasks.find(t => t.at <= now);
  assert.ok(tickTask);
  await tickTask.fn();
  const lastTick = requests.filter(r => r.route === '/assistant/tick').at(-1);
  assert.equal(lastTick.body.emote.sent, 1);
  assert.equal(lastTick.body.emote.reason, 'sent');
  listeners.get('pagehide')?.();
}

(async () => {
  for (const rel of ['../../macos/fullatlas-source/assistant_dashboard.js', '../../windows/src/src/assistant_dashboard.js']) {
    await testDashboard(rel);
  }
  for (const rel of ['../../macos/fullatlas-source/desktop_assistant.js', '../../windows/src/src/desktop_assistant.js']) {
    await testDelayedTypingContextAndTickEmote(rel);
  }
  console.log('dashboard and emote regression passed (macOS + Windows): all 7 tabs, update tab prerelease/error recovery, cross-window emoteStats + accessibility prompt, >500ms UIA probe');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
