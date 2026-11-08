import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { syncFrontend } from './sync-frontend.mjs';

// 版本号在这个仓库里有八处副本，少改一处就会在打包那一步炸：
//   package.json / package-lock.json（顶层 + packages.""）/ 两个 tauri.conf /
//   Cargo.toml / Cargo.lock / 共用前端的 desktop_assistant.js 的 VERSION 常量。
// macOS 的 dmg 名字取自 CFBundleShortVersionString（即 tauri.conf），
// release.yml 校验 tag 与 package.json 一致，Cargo 侧又因为 --locked 构建不允许
// Cargo.toml 与 Cargo.lock 打架；面板「全图鉴版本」读的就是 desktop_assistant.js 的
// VERSION，漏改会让界面显示的版本和实际发出去的包对不上。
// 所以这里一次性全改，并在改之前先确认它们本来是齐的。
// VERSION 常量现在只存在于 shared/frontend/，改完必须重跑生成器把两侧副本刷出来。

const root = fileURLToPath(new URL('../', import.meta.url));

export const versionFiles = [
  'package.json',
  'package-lock.json',
  'macos/src-tauri/tauri.conf.json',
  'macos/src-tauri/tauri.global.conf.json',
  'macos/src-tauri/Cargo.toml',
  'macos/src-tauri/Cargo.lock',
  'shared/frontend/desktop_assistant.js',
];

// 所有模式都只把版本号本身放进第 1 个捕获组，前缀/后缀不参与捕获，
// 这样替换时可以直接用字符串替换，不会误伤同名的其他字段。
const jsonVersion = /"version":\s*"([^"]*)"/;
const cargoPackageVersion = /^version = "([^"]+)"/m;
const cargoLockVersion = /name = "wuwatappo-mac"\r?\nversion = "([^"]+)"/;
const moduleVersion = /const VERSION='([^']+)'/;

const patterns = {
  'package.json': jsonVersion,
  'package-lock.json': jsonVersion,
  'macos/src-tauri/tauri.conf.json': jsonVersion,
  'macos/src-tauri/tauri.global.conf.json': jsonVersion,
  'macos/src-tauri/Cargo.toml': cargoPackageVersion,
  'macos/src-tauri/Cargo.lock': cargoLockVersion,
  'shared/frontend/desktop_assistant.js': moduleVersion,
};

export function nextPrerelease(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z]+)\.(\d+))?$/.exec(String(version || ''));
  if (!match) throw new Error(`无法解析版本号：${JSON.stringify(version)}，期望形如 1.3.0 或 1.3.0-alpha.1`);
  const [, major, minor, patch, label, serial] = match;
  // 已经在预发布线上就顺着同一个标签往上数（alpha.1 → alpha.2），
  // 否则开一条新的 alpha 线并把 patch 加一，避免覆盖已经发出去的正式版本号。
  if (label && serial) return `${major}.${minor}.${patch}-${label}.${Number(serial) + 1}`;
  return `${major}.${minor}.${Number(patch) + 1}-alpha.1`;
}

function readOne(file, text) {
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

export function rewriteSources(sources, previous, next) {
  const updated = {};
  for (const file of versionFiles) {
    const text = sources[file];
    updated[file] = file === 'package-lock.json'
      ? rewriteLockfile(text, previous, next)
      : rewriteOne(text, patterns[file], previous, next, 1, file);
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

export async function bumpVersion({ projectRoot = root, dryRun = false } = {}) {
  const sources = await readSources(projectRoot);
  const previous = assertConsistent(readVersions(sources));
  const next = nextPrerelease(previous);
  const updated = rewriteSources(sources, previous, next);
  // 改完再读一遍，确认八处都真的落到了新版本上（而不是替换到别的地方去了）。
  assertConsistent(readVersions(updated));
  if (!dryRun) {
    for (const file of versionFiles) await writeFile(path.join(projectRoot, file), updated[file]);
    // 版本常量在共用源里，两侧副本不带这个数——刷一次，免得发出去的包版本号对不上。
    await syncFrontend({ projectRoot });
  }
  return { previous, next, files: versionFiles };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const dryRun = process.argv.includes('--dry-run');
  bumpVersion({ dryRun })
    .then(result => console.log(JSON.stringify({ ...result, dryRun })))
    .catch(error => { console.error(error.message); process.exitCode = 1; });
}
