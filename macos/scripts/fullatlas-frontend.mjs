import { cp, mkdir, readFile, readdir, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import beautify from 'js-beautify';
import { minify } from 'terser';
import { parse } from 'acorn';

// The upstream adapter locates its anchors structurally, but the parsed layout it
// relies on has to come from a real official build. 版本号不再作为准入条件：
// 官方发新版就直接尝试注入，锚点漂移时下面的断言会明确报出是哪一处变了，
// 那时才需要人工改适配（对应「编译失败再提 PR」的流程）。
export function adapterVersion(lock) {
  const version = String(lock?.version || '').trim();
  const digest = String(lock?.installer?.sha256 || '').trim();
  if (!version || !/^[0-9a-f]{64}$/i.test(digest)) {
    throw new Error('FullAtlas: baseline lock is missing the official version or installer digest');
  }
  return version;
}

export async function integrateFullAtlas(root, profile) {
  const version = adapterVersion(JSON.parse(await readFile(path.join(root, profile.upstreamLock), 'utf8')));
  // 每个区域一套中间产物：国服与国际服的共享模块名不同，混用会让后续步骤读到别人的结构。
  const stage = path.join(root, `build/fullatlas-adapter-${profile.id}`);
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
  run(process.execPath, ['../shared/tools/inspect_js.cjs', base, shared]);
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
  console.log(`Integrated FullAtlas 1.2 into the macOS ${profile.region} frontend`);
}
