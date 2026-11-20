import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { syncFrontend } from './sync-frontend.mjs';

// 版本号在这个仓库里有十处副本，少改一处就会在打包那一步炸：
//   package.json / package-lock.json（顶层 + packages.""）/ 两个 tauri.conf /
//   macos 的 Cargo.toml + Cargo.lock / shared/backend 的 Cargo.toml + Cargo.lock /
//   共用前端的 desktop_assistant.js 的 VERSION 常量。
// macOS 的 dmg 名字取自 CFBundleShortVersionString（即 tauri.conf），
// release.yml 校验 tag 与 package.json 一致，Cargo 侧又因为 --locked 构建不允许
// Cargo.toml 与 Cargo.lock 打架；窗口端的本地服务来自 shared/backend，
// 那份 Cargo.toml/Cargo.lock 不同步同样会让 Windows 构建停在 --locked。
// 面板「全图鉴版本」读的就是 desktop_assistant.js 的 VERSION，漏改会让界面显示的
// 版本和实际发出去的包对不上。所以这里一次性全改，并在改之前先确认它们本来是齐的。
// VERSION 常量现在只存在于 shared/frontend/，改完必须重跑生成器把两侧副本刷出来。

const root = fileURLToPath(new URL('../', import.meta.url));

export const versionFiles = [
  'package.json',
  'package-lock.json',
  'macos/src-tauri/tauri.conf.json',
  'macos/src-tauri/tauri.global.conf.json',
  'macos/src-tauri/Cargo.toml',
  'macos/src-tauri/Cargo.lock',
  'shared/backend/Cargo.toml',
  'shared/backend/Cargo.lock',
  'shared/frontend/desktop_assistant.js',
];

// 所有模式都只把版本号本身放进第 1 个捕获组，前缀/后缀不参与捕获，
// 这样替换时可以直接用字符串替换，不会误伤同名的其他字段。
const jsonVersion = /"version":\s*"([^"]*)"/;
const cargoPackageVersion = /^version = "([^"]+)"/m;
const moduleVersion = /const VERSION='([^']+)'/;

const patterns = {
  'package.json': jsonVersion,
  'package-lock.json': jsonVersion,
  'macos/src-tauri/tauri.conf.json': jsonVersion,
  'macos/src-tauri/tauri.global.conf.json': jsonVersion,
  'macos/src-tauri/Cargo.toml': cargoPackageVersion,
  'shared/backend/Cargo.toml': cargoPackageVersion,
  'shared/frontend/desktop_assistant.js': moduleVersion,
};

// Cargo.lock 里本仓库自己的包都要跟着版本走。macOS 的锁除了 app 自己，
// 还带着 shared/backend 作为路径依赖的条目；只改前者会让 `--locked` 构建直接失败
// （1.3.2-beta.1 就栽在这里：cargo 拒绝在锁定模式下改锁文件）。
const ownLockPackages = /name = "(?:wuwatappo-mac|wuwatappo-assistant-core)"\r?\nversion = "([^"]+)"/g;

export function nextPrerelease(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z]+)\.(\d+))?$/.exec(String(version || ''));
  if (!match) throw new Error(`无法解析版本号：${JSON.stringify(version)}，期望形如 1.3.0 或 1.3.0-alpha.1`);
  const [, major, minor, patch, label, serial] = match;
  // 已经在预发布线上就顺着同一个标签往上数（alpha.1 → alpha.2），
  // 否则开一条新的 alpha 线并把 patch 加一，避免覆盖已经发出去的正式版本号。
  if (label && serial) return `${major}.${minor}.${patch}-${label}.${Number(serial) + 1}`;
  return `${major}.${minor}.${Number(patch) + 1}-alpha.1`;
}

function readLock(file, text) {
  const versions = [...text.matchAll(ownLockPackages)].map(match => match[1]);
  if (!versions.length) throw new Error(`${file} 里找不到本仓库的包版本号`);
  const unique = new Set(versions);
  if (unique.size !== 1) {
    throw new Error(`${file} 里本仓库的包版本号已经不一致：${versions.join(' / ')}`);
  }
  return versions[0];
}

function readOne(file, text) {
  if (file.endsWith('Cargo.lock')) return readLock(file, text);
  const match = patterns[file].exec(text);
  if (!match) throw new Error(`${file} 里找不到版本号`);
  return match[1];
}

export function readVersions(sources) {
  return Object.fromEntries(versionFiles.map(file => [file, readOne(file, sources[file])]));
}

export function assertConsistent(versions) {
  const unique = [...new Set(Object.values(versions))];
  if (unique.length !== 1) {
    const detail = Object.entries(versions).map(([file, value]) => `  ${file}: ${value}`).join('\n');
    throw new Error(`版本号已经漂移，先人工对齐再自动递增：\n${detail}`);
  }
  return unique[0];
}

function rewriteOne(text, pattern, previous, next, rounds, file) {
  let done = 0;
  const result = text.replace(pattern, (whole, value) => {
    if (done >= rounds || value !== previous) return whole;
    done += 1;
    return whole.replace(value, next);
  });
  if (done !== rounds) throw new Error(`${file} 期望改写 ${rounds} 处版本号，实际匹配 ${done} 处`);
  return result;
}

// package-lock.json 有两处需要改：顶层的 version 和 packages."" 的 version。
// 依赖条目也长成 "version": "x"，所以只改值等于旧版本号的前两处，并断言正好两处，
// 免得哪天某个依赖恰好跟主版本同号被误伤。
function rewriteLockfile(text, previous, next) {
  let done = 0;
  const result = text.replace(/"version":\s*"([^"]*)"/g, (whole, value) => {
    if (done >= 2 || value !== previous) return whole;
    done += 1;
    return whole.replace(value, next);
  });
  if (done !== 2) throw new Error(`package-lock.json 期望改写 2 处版本号，实际匹配 ${done} 处`);
  return result;
}

// Cargo.lock 可能同时含 app 与共享 crate 两条本仓库条目，两处都要改。
function rewriteCargoLock(text, previous, next, file) {
  let done = 0;
  const result = text.replace(ownLockPackages, (whole, value) => {
    if (value !== previous) return whole;
    done += 1;
    return whole.replace(value, next);
  });
  if (done === 0) throw new Error(`${file} 里没有找到本仓库的包版本号`);
  return result;
}

export function rewriteSources(sources, previous, next) {
  const updated = {};
  for (const file of versionFiles) {
    const text = sources[file];
    if (file === 'package-lock.json') updated[file] = rewriteLockfile(text, previous, next);
    else if (file.endsWith('Cargo.lock')) updated[file] = rewriteCargoLock(text, previous, next, file);
    else updated[file] = rewriteOne(text, patterns[file], previous, next, 1, file);
  }
  return updated;
}

export async function readSources(projectRoot = root) {
  const sources = {};
  for (const file of versionFiles) sources[file] = await readFile(path.join(projectRoot, file), 'utf8');
  return sources;
}

export async function currentVersion(projectRoot = root) {
  return assertConsistent(readVersions(await readSources(projectRoot)));
}

// 预发布线不一定总是 alpha：跨标签（alpha → beta → 正式）时传 target 直接指定，
// 否则按 nextPrerelease 在同一条标签上自增。
const RELEASE_VERSION = /^\d+\.\d+\.\d+(?:-(?:alpha|beta|rc)\.\d+)?$/;

export async function bumpVersion({ projectRoot = root, dryRun = false, target } = {}) {
  if (target !== undefined && !RELEASE_VERSION.test(String(target))) {
    throw new Error(`目标版本号不合法：${target}，期望形如 1.3.2 或 1.3.2-beta.1`);
  }
  const sources = await readSources(projectRoot);
  const previous = assertConsistent(readVersions(sources));
  const next = target === undefined ? nextPrerelease(previous) : String(target);
  if (next === previous) throw new Error(`目标版本号与当前一致（${previous}），没有可改的内容`);
  const updated = rewriteSources(sources, previous, next);
  // 改完再读一遍，确认全部副本都真的落到了新版本上（而不是替换到别的地方去了）。
  assertConsistent(readVersions(updated));
  if (!dryRun) {
    for (const file of versionFiles) await writeFile(path.join(projectRoot, file), updated[file]);
    // 确认共享前端唯一源渲染无误（不再生成平台目录副本）。
    await syncFrontend({ projectRoot, check: true });
  }
  return { previous, next, files: versionFiles };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const dryRun = process.argv.includes('--dry-run');
  // 用法：node scripts/bump-version.mjs [1.3.2-beta.1] [--dry-run]
  const target = process.argv.slice(2).find(arg => !arg.startsWith('--'));
  bumpVersion({ dryRun, target })
    .then(result => console.log(JSON.stringify({ ...result, dryRun })))
    .catch(error => { console.error(error.message); process.exitCode = 1; });
}
