import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { getRegionProfile } from './regions.mjs';

const PLACEHOLDER_PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGNgAAIAAAUAAXpeqz8AAAAASUVORK5CYII=', 'base64');
const PLACEHOLDER_ICNS = Buffer.from('aWNucwAAAFRpY3A0AAAATIlQTkcNChoKAAAADUlIRFIAAAABAAAAAQgGAAAAHxXEiQAAAAtJREFUeJxjYAACAAAFAAF6Xqs/AAAAAElFTkSuQmCC', 'base64');

async function ensureRustTestInputs() {
  for (const dir of ['build/frontend', 'build/frontend-global']) {
    const target = path.join(root, dir);
    if (!existsSync(target)) {
      await mkdir(target, { recursive: true });
      await writeFile(path.join(target, 'index.html'), '<!doctype html><html><body></body></html>');
    }
  }
  const catalog = path.join(root, 'build/fullatlas-catalog.json');
  if (!existsSync(catalog)) {
    await mkdir(path.dirname(catalog), { recursive: true });
    await writeFile(catalog, '{}');
  }
  for (const iconDir of ['src-tauri/icons', 'src-tauri/icons-global']) {
    const dir = path.join(root, iconDir);
    await mkdir(dir, { recursive: true });
    for (const [name, content] of [
      ['32x32.png', PLACEHOLDER_PNG],
      ['128x128.png', PLACEHOLDER_PNG],
      ['128x128@2x.png', PLACEHOLDER_PNG],
      ['icon.icns', PLACEHOLDER_ICNS],
    ]) {
      const file = path.join(dir, name);
      if (!existsSync(file)) await writeFile(file, content);
    }
  }
}

// 平台实现都在 macos/ 下，所以这里的一切都相对 macos/ 解析；
// 仓库根只放跨平台入口与共享产物（dist/、node_modules/），单独算一个 repoRoot。
const root = fileURLToPath(new URL('../', import.meta.url));
const repoRoot = path.dirname(root.replace(/[\\/]$/, ''));

/** Tauri CLI 可以装在仓库根（npm ci 惯例）也可以装在 macos/ 下，两处都认。 */
function resolveTauriCli() {
  const candidates = [
    path.join(repoRoot, 'node_modules/@tauri-apps/cli/tauri.js'),
    path.join(root, 'node_modules/@tauri-apps/cli/tauri.js'),
  ];
  const cli = candidates.find(candidate => existsSync(candidate));
  if (!cli) throw new Error(`未找到 Tauri CLI，请先在仓库根执行 npm ci。已查找：${candidates.join('、')}`);
  return cli;
}

export function tauriArguments(action, profile, extra = []) {
  if (action === 'icons') return ['icon', `${profile.frontendDir}/app-icon.png`, '--output', profile.iconsDir, ...extra];
  if (!['dev', 'build'].includes(action)) throw new Error(`未知构建操作：${action}`);
  return [action, ...(action === 'build' ? ['--bundles', 'app'] : []),
    ...(profile.tauriConfig ? ['--config', profile.tauriConfig] : []), ...extra];
}

function run(command, args, environment = process.env) {
  const result = spawnSync(command, args, { cwd: root, env: environment, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

async function main([action, ...extra]) {
  const profile = getRegionProfile();
  const cli = action === 'test:rust' ? null : resolveTauriCli();
  if (action === 'test:rust') {
    await ensureRustTestInputs();
    const environment = { ...process.env };
    if (profile.tauriConfig) environment.TAURI_CONFIG = await readFile(path.join(root, profile.tauriConfig), 'utf8');
    run('cargo', ['test', '--locked', '--manifest-path', 'src-tauri/Cargo.toml', ...extra], environment);
    return;
  }
  if (action === 'dev' || action === 'build') {
    run(process.execPath, [cli, ...tauriArguments('icons', profile)]);
    run(process.execPath, [path.join(root, 'scripts/prepare-frontend.mjs')]);
  }
  run(process.execPath, [cli, ...tauriArguments(action, profile, extra)]);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
}
