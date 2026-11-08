import { cp, mkdir, readFile, readdir, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import beautify from 'js-beautify';
import { minify } from 'terser';
import { parse } from 'acorn';

// The upstream adapter uses parsed bindings and rejects unknown 1.0.2 layouts.
// Keep macOS patches first, then structurally integrate the original FullAtlas UI.
export function verifiedAdapterVersion(lock) {
  const reviewed = {
    '1.0.2': '152d8ac4847f1b4641a750c45b680091c5a7cf92cb6059cd6d97e803f6eb5487',
    '1.0.3': '29b6f4aaeb0ad4db5a8002e3686435692fa0ddb88ea9389e89b53fb93932136e',
    '1.0.4': 'eb4e6d2103c222297508837a5ca92eeaa058891c9f1e2e7bf034c23f37668e26',
  };
  if (lock.region !== 'CN' || !Object.hasOwn(reviewed, lock.version) || lock.installer?.sha256 !== reviewed[lock.version]) {
    throw new Error('FullAtlas: official version or installer digest has no verified Mac adapter; review upstream changes before building');
  }
  return lock.version;
}

export async function integrateFullAtlas(root, profile) {
  if (profile.id !== 'cn') return;
  const version = verifiedAdapterVersion(JSON.parse(await readFile(path.join(root, profile.upstreamLock), 'utf8')));
  const stage = path.join(root, 'build/fullatlas-adapter');
  const base = path.join(stage, 'base');
  await rm(stage, { recursive: true, force: true });
  await mkdir(path.join(base, 'readable_js/assets'), { recursive: true });
  await cp(path.join(root, profile.preparedDir), path.join(base, 'frontend'), { recursive: true });
  const source = path.join(base, 'frontend/assets');
  const files = (await readdir(source)).filter(name => name.endsWith('.js'));
  let shared;
  for (const name of files) {
    const raw = await readFile(path.join(source, name), 'utf8');
    const readable = beautify.js(raw, { indent_size: 2 });
    if (readable.includes('function applyCatalog(')) shared = name;
    await writeFile(path.join(base, 'readable_js/assets', name), readable);
  }
  if (!shared) throw new Error('FullAtlas: missing verified shared module');
  const run = (command, args, env = process.env) => {
    const result = spawnSync(command, args, { cwd: root, env, stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`FullAtlas adapter failed (${command}, ${result.status})`);
  };
  run(process.execPath, ['tools/fullatlas-inspect.cjs', base, shared]);
  await writeFile(path.join(stage, 'current_base.txt'), base);
  await writeFile(path.join(stage, 'local_config.json'), JSON.stringify({ client_version: version }));
  run(process.env.PYTHON || 'python3', ['tools/fullatlas-integration.py'], { ...process.env, WUWA_BUILD_ROOT: stage });
  const manifest = JSON.parse(await readFile(path.join(stage, 'build_manifest_v2.json'), 'utf8'));
  const changed = [manifest.shared, manifest.main, manifest.panel, ...files.filter(name => name.startsWith('switchDisplayMonitor-'))];
  for (const name of changed) {
    const content = await readFile(path.join(stage, 'frontend_v2/assets', name), 'utf8');
    parse(content, { sourceType: 'module', ecmaVersion: 'latest' });
    const result = await minify(content, { compress: false, mangle: false, format: { beautify: false, quote_style: 2 } });
    await writeFile(path.join(root, profile.preparedDir, 'assets', name), result.code);
  }
  console.log('Integrated FullAtlas 1.2 into the macOS CN frontend');
}
