import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { parse } from 'acorn';
import { test } from 'node:test';
import { verifiedAdapterVersion } from '../scripts/fullatlas-frontend.mjs';
import { getRegionProfile } from '../scripts/regions.mjs';

const profile = getRegionProfile();
// FullAtlas 数据整合只在国服构建里启用（integrateFullAtlas 对国际服直接返回），
// 国际服没有对应的整合产物，下面三个依赖产物的用例无从比对。
const integratedOnly = profile.id === 'cn' ? {} : { skip: `FullAtlas 集成不参与${profile.label}构建` };

test('FullAtlas accepts only the reviewed official version and exact installer digest', async () => {
  const lock = JSON.parse(await readFile(new URL('../upstream-cn.json', import.meta.url), 'utf8'));
  assert.equal(verifiedAdapterVersion(lock), '1.0.3');
  assert.throws(() => verifiedAdapterVersion({ ...lock, version: '1.0.99' }), /no verified Mac adapter/);
  assert.throws(() => verifiedAdapterVersion({ ...lock, installer: { ...lock.installer, sha256: 'f'.repeat(64) } }), /no verified Mac adapter/);
  assert.throws(() => verifiedAdapterVersion({ ...lock, region: 'GLOBAL' }), /no verified Mac adapter/);
});

test('prepared FullAtlas factory stays inactive in login and other auxiliary routes', integratedOnly, async () => {
  const { readdir } = await import('node:fs/promises');
  const directory = new URL('../build/frontend/assets/', import.meta.url);
  const sources = await Promise.all((await readdir(directory)).filter(name => name.endsWith('.js')).map(name => readFile(new URL(name, directory), 'utf8')));
  const source = sources.find(text => text.includes('const localAtlasRoute='));
  assert.ok(source, 'prepare the CN frontend before testing the integrated adapter');
  const ast = parse(source, { sourceType: 'module', ecmaVersion: 'latest' });
  const names = new Set(['localAtlasRoute', 'localAtlasController']);
  const bindings = ast.body.filter(node => node.type === 'VariableDeclaration' && node.declarations.some(d => names.has(d.id.name)));
  assert.equal(bindings.length, 2);
  const code = bindings.map(node => source.slice(node.start, node.end)).join('\n') + '\nlocalAtlasController';
  for (const route of ['/login', '/about', '/oss-notice', '/update']) {
    const controller = vm.runInNewContext(code, {
      window: { location: { hash: '#' + route } },
      createLocalAtlasController: () => { throw new Error('Auxiliary route requested the FullAtlas bridge'); },
    });
    assert.equal(controller, null, route);
  }
});

test('every prepared module imports exports that its actual dependency provides', integratedOnly, async () => {
  const { readdir } = await import('node:fs/promises');
  const directory = new URL('../build/frontend/assets/', import.meta.url);
  const modules = new Map();
  for (const name of (await readdir(directory)).filter(name => name.endsWith('.js'))) {
    const text = await readFile(new URL(name, directory), 'utf8');
    const ast = parse(text, { sourceType: 'module', ecmaVersion: 'latest' });
    const exports = new Set();
    for (const node of ast.body) {
      if (node.type === 'ExportNamedDeclaration') for (const item of node.specifiers) exports.add(item.exported.name);
      if (node.type === 'ExportDefaultDeclaration') exports.add('default');
    }
    modules.set(name, { ast, exports });
  }
  for (const [name, { ast }] of modules) for (const node of ast.body) {
    if (node.type !== 'ImportDeclaration' || !node.source.value.startsWith('./') || !node.source.value.endsWith('.js')) continue;
    const target = modules.get(node.source.value.slice(2));
    assert.ok(target, `${name}: missing ${node.source.value}`);
    for (const item of node.specifiers) {
      if (item.type === 'ImportNamespaceSpecifier') continue;
      const binding = item.type === 'ImportDefaultSpecifier' ? 'default' : item.imported.name;
      assert.ok(target.exports.has(binding), `${name}: ${node.source.value} does not export ${binding}`);
    }
  }
});

test('assistant chest timers bind to the official readiness computations after minifier renaming', integratedOnly, async () => {
  const structure = JSON.parse(await readFile(new URL('../build/fullatlas-adapter/base/js_structure.json', import.meta.url), 'utf8'));
  const main = Object.entries(structure.files).find(([, meta]) => meta.snapshots.length === 1);
  assert.ok(main);
  const [name, meta] = main;
  const original = await readFile(new URL(`../build/fullatlas-adapter/base/readable_js/assets/${name}`, import.meta.url), 'utf8');
  const prepared = await readFile(new URL(`../build/frontend/assets/${name}`, import.meta.url), 'utf8');
  const escape = text => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const pattern = new RegExp('([\\w$]+) = ' + escape(meta.imports.computed) + '\\(\\(\\) => ([\\w$]+)\\.value \\|\\| !' + escape(meta.imports.globalStore) + '\\.isLoggedIn \\? !0 : ([\\w$]+)\\.value <= 0\\)', 'g');
  const ready = [...original.matchAll(pattern)];
  assert.equal(ready.length, 2);
  for (const [index, side] of ['left', 'right'].entries()) {
    assert.ok(prepared.includes(`${side}:{remaining:Math.max(0,${ready[index][3]}.value),ready:${ready[index][1]}.value}`), `${side}: assistant must read the same timer as the official readiness computation`);
  }
});

test('prepared appearance renderers preserve local Mod image URLs', integratedOnly, async () => {
  const structure = JSON.parse(await readFile(new URL('../build/fullatlas-adapter/base/js_structure.json', import.meta.url), 'utf8'));
  const source = await readFile(new URL(`../build/frontend/assets/${structure.shared}`, import.meta.url), 'utf8');
  assert.match(
    source,
    /customAssetUrl: \(reference, file\) => window\.__TAURI_INTERNALS__\.convertFileSrc\(file\)/,
    '注入器必须把本地图片解析方式交给共享控制器，否则 Mod 图片无处可寻',
  );
  assert.match(source, /if \(!paths \|\| !Object\.keys\(paths\)\.length\) return data\.catalog;/, '没有覆盖项时不应该克隆整份 catalog');
  const ast = parse(source, { sourceType: 'module', ecmaVersion: 'latest' });
  const functions = new Set(['resolveAppearanceAssetUrl', 'resolveAppearanceAssetDisplayUrl', 'resolveAppearanceAtlasUrl']);
  const declarations = ast.body.filter(node => node.type === 'FunctionDeclaration' && functions.has(node.id.name));
  assert.equal(declarations.length, 3);
  const url = 'asset://localhost/%2FUsers%2Fexample%2Fmod%2Fimage.png';
  const renderers = vm.runInNewContext(declarations.map(node => source.slice(node.start, node.end)).join('\n') + '\n({resolveAppearanceAssetUrl,resolveAppearanceAssetDisplayUrl,resolveAppearanceAtlasUrl})', {
    withAssetUrlCacheBust: value => value,
    resolveAppearanceMeta: () => ({ url, atlas: { url } }),
    getAppearanceAtlasById: () => ({ url, asset_type: 'emote_atlas' }),
    atlasTypeToAssetType: () => 'emote',
    loadedAssetsHost: 'https://official.invalid',
    buildAssetUrl() { throw new Error('Local Mod URL was sent to the CDN'); },
  });
  assert.equal(renderers.resolveAppearanceAssetUrl(url, 'https://official.invalid'), url);
  assert.equal(renderers.resolveAppearanceAssetDisplayUrl({ assetId: 1 }), url);
  assert.equal(renderers.resolveAppearanceAtlasUrl({ assetId: 1 }), url);
  assert.equal(renderers.resolveAppearanceAtlasUrl({}), url);
});
