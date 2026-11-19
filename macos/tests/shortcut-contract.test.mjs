import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import vm from 'node:vm';
import { parse } from '../../shared/tools/acorn.mjs';
import { patchMacShortcutLabels } from '../scripts/patch-frontend.mjs';
import { contract, profile, root, findSource, hasExtractedFrontend, hasPreparedFrontend, requireFrontendAssets } from './frontend-context.mjs';

let cachedSources = null;
function sources() {
  if (!cachedSources) {
    cachedSources = {
      main: findSource(source => source.includes('const DEFAULT_APP_SHORTCUTS=')),
      view: findSource(source => source.includes('e==="togglePet"')),
      settings: findSource(source => source.includes('__name:"ShortcutInput"')),
    };
  }
  return cachedSources;
}

function nodes(source) {
  const result = [];
  function visit(node) {
    if (!node || typeof node !== 'object') return;
    result.push(node);
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) value.forEach(visit);
      else if (value && typeof value === 'object') visit(value);
    }
  }
  visit(parse(source, { sourceType: 'module', ecmaVersion: 'latest' }));
  return result;
}

function declaration(source, name, predicate = () => true) {
  const selected = nodes(source).filter(node => node.type === 'FunctionDeclaration' && node.id.name === name && predicate(node));
  assert.equal(selected.length, 1, `Expected one official ${name} function`);
  return source.slice(selected[0].start, selected[0].end);
}

function constant(source, name) {
  const selected = nodes(source).filter(node => node.type === 'VariableDeclarator' && node.id.name === name);
  assert.equal(selected.length, 1, `Expected one official ${name} binding`);
  return `const ${source.slice(selected[0].start, selected[0].end)};`;
}

test('official shortcut bindings retain VK values while macOS displays Option and Command', t => {
  if (!hasExtractedFrontend && !requireFrontendAssets) return t.skip('缺少已提取前端，跳过官方前端契约测试');
  const { main } = sources();
  const patched = patchMacShortcutLabels(main.source);
  const definitions = ['DEFAULT_APP_SHORTCUTS', 'namedKeys', 'keyLabels'].map(name => constant(patched, name)).join('\n');
  const functions = ['formatShortcut', 'formatModifiers', 'eventModifiers', 'keyFromEvent', 'formatPressedShortcut', 'shortcutFromEvent']
    .map(name => declaration(patched, name)).join('\n');
  const api = vm.runInNewContext(`${definitions}\n${functions}\n({ defaults: DEFAULT_APP_SHORTCUTS, formatShortcut, formatModifiers, shortcutFromEvent })`);
  const clone = value => JSON.parse(JSON.stringify(value));
  assert.deepEqual(clone(api.defaults), {
    gameMode: { modifiers: 0, key: 118 }, switchScreen: { modifiers: 0, key: 119 }, togglePet: { modifiers: 0, key: 120 },
  });
  assert.deepEqual(clone(api.formatModifiers(15)), ['Ctrl', 'Option', 'Shift', 'Cmd']);
  assert.equal(api.formatShortcut({ modifiers: 8, key: 75 }), 'Cmd+K');
  assert.equal(api.formatShortcut({ modifiers: 1, key: 118 }), 'Option+F7');
  assert.equal(api.formatShortcut({ modifiers: 0, key: 0 }), '');
  assert.deepEqual(clone(api.shortcutFromEvent({ code: 'KeyK', key: 'k', metaKey: true })), { modifiers: 8, key: 75 });
  assert.deepEqual(clone(api.shortcutFromEvent({ code: 'KeyP', key: 'P', ctrlKey: true, altKey: true, shiftKey: true })), { modifiers: 7, key: 80 });
  assert.deepEqual(clone(api.shortcutFromEvent({ code: 'F7', key: 'F7' })), { modifiers: 0, key: 118 });
  assert.equal(api.shortcutFromEvent({ code: 'KeyK', key: 'K', shiftKey: true }), null);
  assert.equal(api.shortcutFromEvent({ code: 'KeyK', key: 'k' }), null);
  assert.throws(() => patchMacShortcutLabels(''), /modifier labels changed/);
  assert.throws(() => patchMacShortcutLabels(main.source.replace('[8,"Win"]', '[8,"Meta"]')), /modifier labels changed/);
  if (hasPreparedFrontend || requireFrontendAssets) {
    const prepared = readFileSync(path.join(root, profile.preparedDir, 'assets', main.name), 'utf8');
    assert.ok(prepared.includes(declaration(patched, 'formatModifiers')));
  }
});

test('official settings save the complete shortcut map and retain actionable native errors', t => {
  if (!hasExtractedFrontend && !requireFrontendAssets) return t.skip('缺少已提取前端，跳过官方前端契约测试');
  const { settings } = sources();
  const saveNodes = nodes(settings.source).filter(node => node.type === 'FunctionDeclaration'
    && settings.source.slice(node.start, node.end).includes('set_app_shortcuts'));
  assert.equal(saveNodes.length, 1, 'Expected one official set_app_shortcuts function');
  const save = settings.source.slice(saveNodes[0].start, saveNodes[0].end);
  assert.match(save, /set_app_shortcuts/);
  assert.match(save, /setting_shortcut_/);
  for (const key of ['gameMode', 'switchScreen', 'togglePet']) {
    assert.ok(settings.source.includes(key), `Shortcut settings must reference ${key}`);
  }
});

test('official shortcut capture pairs focus lock and unlock calls cleanly', t => {
  if (!hasExtractedFrontend && !requireFrontendAssets) return t.skip('缺少已提取前端，跳过官方前端契约测试');
  const { settings } = sources();
  const decls = nodes(settings.source).filter(node => node.type === 'FunctionDeclaration');
  const slice = node => settings.source.slice(node.start, node.end);
  const focusNodes = decls.filter(node => /await \w+\(!0\)/.test(slice(node)));
  const blurNodes = decls.filter(node => /\w+\(!1\)\.catch\(\(\)=>\{\}\)/.test(slice(node)));
  assert.equal(focusNodes.length, 1, 'Expected one official capture-focus function');
  assert.equal(blurNodes.length, 1, 'Expected one official capture-blur function');
  const focusLock = /await (\w+)\(!0\)/.exec(slice(focusNodes[0]))[1];
  const blurLock = /(\w+)\(!1\)\.catch\(\(\)=>\{\}\)/.exec(slice(blurNodes[0]))[1];
  assert.equal(blurLock, focusLock);
});

test('official app-shortcut consumer dispatches exactly one enabled action and honors the lock', t => {
  if (!hasExtractedFrontend && !requireFrontendAssets) return t.skip('缺少已提取前端，跳过官方前端契约测试');
  const { view } = sources();
  // 压缩名每次官方重建都会漂移（1.0.3: Ch/Ie/ly/mc…，1.0.4: Ip/Pe/py/wc…），按分派形状解析。
  const match = /function ([\w$]+)\(e\)\{([\w$]+)\(\)\|\|\(e==="togglePet"&&([\w$]+)\(\)&&([\w$]+)\(\),e==="gameMode"&&([\w$]+)\(\)&&([\w$]+)\(\),e==="switchScreen"&&([\w$]+)\(\)&&([\w$]+)\(\)\)\}/.exec(view.source);
  assert.ok(match, 'app-shortcut dispatch contract');
  const [, name, guard, petEnabled, petAction, modeEnabled, modeAction, screenEnabled, screenAction] = match;
  for (const locked of [false, true]) for (const enabled of [false, true]) {
    const effects = [];
    const env = vm.createContext({
      [guard]: () => locked,
      [petEnabled]: () => enabled, [modeEnabled]: () => enabled, [screenEnabled]: () => enabled,
      [petAction]: () => effects.push('togglePet'),
      [modeAction]: () => effects.push('gameMode'),
      [screenAction]: () => effects.push('switchScreen'),
    });
    vm.runInContext(declaration(view.source, name), env);
    for (const action of ['togglePet', 'gameMode', 'switchScreen', 'unknown']) env[name](action);
    assert.deepEqual(effects, !locked && enabled ? ['togglePet', 'gameMode', 'switchScreen'] : []);
  }
  assert.deepEqual(contract.events.find(event => event.name === 'app-shortcut').payload.enum, ['gameMode', 'switchScreen', 'togglePet']);
});

test('official fullscreen automation follows transitions without clearing a manually enabled mode when disabled', t => {
  if (!hasExtractedFrontend && !requireFrontendAssets) return t.skip('缺少已提取前端，跳过官方前端契约测试');
  const { view } = sources();
  // 1.0.3: Ph/xt/ma/Qt/Ga + 内联通知；1.0.4: Op/Et/_a/tn/$a，通知下沉到 $a 内部。
  const match = /function ([\w$]+)\(e\)\{if\(!([\w$]+)\(\)\.autoGameModeOnFullscreen\)\{([\w$]+)=!1;return\}e!==\3&&\(\3=e,([\w$]+)\.value!==e&&([\w$]+)\(e\)\)\}/.exec(view.source);
  assert.ok(match, 'fullscreen automation contract');
  const [, name, settingsFn, state, ref, apply] = match;
  const settings = { autoGameModeOnFullscreen: true };
  const effects = [];
  const env = vm.createContext({
    [state]: false, [ref]: { value: false }, [settingsFn]: () => settings,
    [apply]: value => { env[ref].value = value; effects.push(value); },
  });
  const handler = declaration(view.source, name);
  vm.runInContext(handler, env);
  for (const fullscreen of [false, true, true, false, false]) env[name](fullscreen);
  assert.deepEqual(effects, [true, false]);
  settings.autoGameModeOnFullscreen = false;
  env[ref].value = true;
  env[name](false);
  assert.equal(env[ref].value, true);
  assert.deepEqual(effects, [true, false]);
});

test('official desktop-safe-area consumer keeps application fullscreen separate from desktop fullscreen', t => {
  if (!hasExtractedFrontend && !requireFrontendAssets) return t.skip('缺少已提取前端，跳过官方前端契约测试');
  const { source } = findSource(value => value.includes('foregroundApplicationFullscreen'));
  assert.match(source, /get_main_desktop_safe_area/);
  assert.match(source, /\bforegroundApplicationFullscreen\b/);
  assert.match(source, /\bforegroundFullscreen\b/);
});
