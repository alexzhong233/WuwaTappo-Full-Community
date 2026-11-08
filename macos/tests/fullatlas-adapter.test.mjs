import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { parse } from 'acorn';
import { test } from 'node:test';
import { adapterVersion } from '../scripts/fullatlas-frontend.mjs';
import { getRegionProfile } from '../scripts/regions.mjs';

const profile = getRegionProfile();
// 两个区域都接入 FullAtlas，产物目录按区域分开。
const assets = new URL(`../build/${profile.id === 'cn' ? 'frontend' : 'frontend-global'}/assets/`, import.meta.url);
const stage = new URL(`../build/fullatlas-adapter-${profile.id}/`, import.meta.url);

test('FullAtlas takes any official baseline as long as the installer is pinned', async () => {
  const lock = JSON.parse(await readFile(new URL('../upstream-cn.json', import.meta.url), 'utf8'));
  assert.equal(adapterVersion(lock), lock.version);
  // 版本号不再是准入条件：锚点是否还成立由注入器的结构断言回答，
  // 官方升版先编译，锚点漂移才需要人工改适配。
  assert.equal(adapterVersion({ ...lock, version: '1.9.9' }), '1.9.9');
  assert.throws(() => adapterVersion({ ...lock, version: '' }), /missing the official version or installer digest/);
  assert.throws(() => adapterVersion({ ...lock, installer: { ...lock.installer, sha256: '' } }), /missing the official version or installer digest/);
  // 国际服同样接入，锁也要能过。
  const global = JSON.parse(await readFile(new URL('../upstream-global.json', import.meta.url), 'utf8'));
  assert.equal(adapterVersion(global), global.version);
});

test('prepared FullAtlas factory stays inactive in login and other auxiliary routes', async () => {
  const { readdir } = await import('node:fs/promises');
  const directory = assets;
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

test('every prepared module imports exports that its actual dependency provides', async () => {
  const { readdir } = await import('node:fs/promises');
  const directory = assets;
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

test('assistant chest timers bind to the official readiness computations after minifier renaming', async () => {
  const structure = JSON.parse(await readFile(new URL('base/js_structure.json', stage), 'utf8'));
  const main = Object.entries(structure.files).find(([, meta]) => meta.snapshots.length === 1);
  assert.ok(main);
  const [name, meta] = main;
  const original = await readFile(new URL(`base/readable_js/assets/${name}`, stage), 'utf8');
  const prepared = await readFile(new URL(name, assets), 'utf8');
  const escape = text => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const pattern = new RegExp('([\\w$]+) = ' + escape(meta.imports.computed) + '\\(\\(\\) => ([\\w$]+)\\.value \\|\\| !' + escape(meta.imports.globalStore) + '\\.isLoggedIn \\? !0 : ([\\w$]+)\\.value <= 0\\)', 'g');
  const ready = [...original.matchAll(pattern)];
  assert.equal(ready.length, 2);
  for (const [index, side] of ['left', 'right'].entries()) {
    assert.ok(prepared.includes(`${side}:{remaining:Math.max(0,${ready[index][3]}.value),ready:${ready[index][1]}.value}`), `${side}: assistant must read the same timer as the official readiness computation`);
  }
});

test('prepared appearance renderers preserve local Mod image URLs', async () => {
  const structure = JSON.parse(await readFile(new URL('base/js_structure.json', stage), 'utf8'));
  const source = await readFile(new URL(structure.shared, assets), 'utf8');
  assert.match(
    source,
    // 1.0.4 的产物会把注入片段一起压缩，空格不再固定，这里只校验语义。
    /customAssetUrl:\s*\(reference,\s*file\)\s*=>\s*window\.__TAURI_INTERNALS__\.convertFileSrc\(file\)/,
    '注入器必须把本地图片解析方式交给共享控制器，否则 Mod 图片无处可寻',
  );
  assert.match(source, /if\s*\(!paths\s*\|\|\s*!Object\.keys\(paths\)\.length\)\s*return data\.catalog;/, '没有覆盖项时不应该克隆整份 catalog');
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
