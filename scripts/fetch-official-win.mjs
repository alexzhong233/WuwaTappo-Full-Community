#!/usr/bin/env node
/**
 * 获取官方 Windows 主程序基底（只用 7-Zip 解包或读取现成官方 exe，从不执行安装器）。
 *
 * 替代原 windows/tools/fetch-official.ps1 + upstream-common.ps1：
 * 复用 scripts/sync-upstream.mjs 的官方清单发现、CDN 镜像回退、SHA-256 锁定校验、
 * Authenticode 签名主体校验，以及 scripts/extract-assets.mjs 的 NSIS/PE 解包逻辑。
 */

import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { copyFile, mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { extractApplicationPayload } from './extract-assets.mjs';
import { getRegionProfile } from './regions.mjs';
import {
  discoverUpstream,
  downloadInstaller,
  findSevenZip,
  validateLock,
  verifyInstaller,
  verifyPublisher,
} from './sync-upstream.mjs';

const repoRoot = fileURLToPath(new URL('../', import.meta.url));

export function parseOptions(args) {
  const options = { latest: false, force: false };
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--latest') options.latest = true;
    else if (arg === '--force') options.force = true;
    else if (arg.startsWith('--region=')) options.region = arg.slice('--region='.length);
    else if (arg.startsWith('--version=')) options.version = arg.slice('--version='.length);
    else if (arg.startsWith('--installer=')) options.installer = arg.slice('--installer='.length);
    else if (arg.startsWith('--official-exe=')) options.officialExe = arg.slice('--official-exe='.length);
    else if (arg.startsWith('--sevenzip=')) options.sevenzip = arg.slice('--sevenzip='.length);
    else if (['--region', '--version', '--installer', '--official-exe', '--sevenzip'].includes(arg)) {
      const val = args[++index];
      if (!val || val.startsWith('--')) throw new Error(`${arg} 需要参数值。`);
      const key = arg.slice(2).replace(/-([a-z])/g, (_, ch) => ch.toUpperCase());
      options[key] = val;
    } else {
      throw new Error(`未知参数：${arg}`);
    }
  }
  if (options.installer && options.officialExe) {
    throw new Error('--installer 与 --official-exe 不能同时指定。');
  }
  if (options.latest && (options.installer || options.officialExe)) {
    throw new Error('--latest 不能与本地 --installer / --official-exe 同时使用。');
  }
  return options;
}

export async function fetchOfficialWindows(options = {}, projectRoot = repoRoot) {
  const regionId = (options.region || process.env.WUWA_REGION || 'cn').toLowerCase();
  const profile = getRegionProfile(regionId);
  const buildDir = profile.id === 'global' ? 'build-global' : 'build';
  const workRoot = path.join(projectRoot, 'windows', buildDir, 'official');
  const lockPath = path.join(projectRoot, profile.windowsLock);
  const lock = existsSync(lockPath)
    ? validateLock(JSON.parse((await readFile(lockPath, 'utf8')).replace(/^\uFEFF/, '')), profile)
    : null;
  const lockedVersion = lock?.version || '';

  let target;
  let setupBytes = null;
  let setupPath = null;
  let sourceExe = null;

  if (options.officialExe) {
    const resolved = path.resolve(options.officialExe);
    if (!existsSync(resolved)) throw new Error(`指定的主程序不存在：${options.officialExe}`);
    target = options.version || lockedVersion || 'custom';
    sourceExe = resolved;
  } else if (options.installer) {
    const resolved = path.resolve(options.installer);
    if (!existsSync(resolved)) throw new Error(`指定的安装包不存在：${options.installer}`);
    target = options.version || lockedVersion || 'custom';
    setupPath = resolved;
  } else {
    let manifest = null;
    if (options.latest || (options.version && options.version !== lockedVersion) || !lockedVersion) {
      manifest = await discoverUpstream(fetch, profile);
    }
    target = options.version || (options.latest ? manifest.version : lockedVersion);
    if (!target) throw new Error('未能解析目标版本。');

    let descriptor = null;
    let isLocked = false;
    if (lock && lock.version === target && lock.installer?.url && !options.latest) {
      descriptor = lock;
      isLocked = true;
    } else if (manifest && manifest.version === target) {
      descriptor = manifest;
      isLocked = Boolean(lock && lock.version === target && lock.installer?.url === manifest.installer.url && lock.installer?.sha256);
      if (isLocked) descriptor = lock;
    } else if (lock && lock.version === target && lock.installer?.url) {
      descriptor = lock;
      isLocked = true;
    } else {
      throw new Error(`找不到 ${target} 的下载地址。请用 --installer 指定本地安装包，或 --latest 取官方最新版。`);
    }

    const work = path.join(workRoot, target);
    const leaf = path.posix.basename(new URL(descriptor.installer.url).pathname);
    setupPath = path.join(work, leaf);

    const officialCandidate = path.join(work, 'base', 'WuwaTappo.official.exe');
    if (!existsSync(officialCandidate) || options.force) {
      if (existsSync(setupPath) && !options.force) {
        setupBytes = await readFile(setupPath);
        if (isLocked) verifyInstaller(setupBytes, descriptor, profile);
      } else {
        setupBytes = await downloadInstaller(descriptor, { profile, updateLock: !isLocked });
        await mkdir(work, { recursive: true });
        await writeFile(`${setupPath}.download`, setupBytes);
        await rename(`${setupPath}.download`, setupPath);
      }
    }
  }

  const work = path.join(workRoot, target);
  const base = path.join(work, 'base');
  const official = path.join(base, 'WuwaTappo.official.exe');

  if (!existsSync(official) || options.force) {
    if (sourceExe) {
      await rm(base, { recursive: true, force: true });
      await mkdir(base, { recursive: true });
      await copyFile(sourceExe, official);
    } else {
      if (!setupBytes) setupBytes = await readFile(setupPath);
      verifyPublisher(setupBytes);
      const sevenzip = findSevenZip(options.sevenzip);
      const { bytes } = await extractApplicationPayload(setupBytes, sevenzip);
      await rm(base, { recursive: true, force: true });
      await mkdir(base, { recursive: true });
      await writeFile(`${official}.download`, bytes);
      await rename(`${official}.download`, official);
    }
  }

  const officialBytes = await readFile(official);
  verifyPublisher(officialBytes);
  const sha256 = createHash('sha256').update(officialBytes).digest('hex');
  const info = {
    version: target,
    base,
    official,
    sha256,
    size: officialBytes.length,
    expected: '',
    hash_matched: null,
    fetched_at: new Date().toISOString().replace(/\.\d{3}Z$/, ''),
  };
  await mkdir(workRoot, { recursive: true });
  await writeFile(path.join(workRoot, 'current.json'), `${JSON.stringify(info, null, 2)}\n`);
  return info;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  Promise.resolve()
    .then(() => fetchOfficialWindows(parseOptions(process.argv.slice(2))))
    .then(info => console.log(JSON.stringify(info, null, 2)))
    .catch(error => {
      console.error(error.message);
      process.exitCode = 1;
    });
}
