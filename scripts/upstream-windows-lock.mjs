import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { extractApplicationPayload } from './extract-assets.mjs';
import { getRegionProfile } from './regions.mjs';
import { findSevenZip, pinnedInstallerCache, validateLock, verifyInstaller } from './sync-upstream.mjs';

// 一份 upstream lock 同时记录共同安装包和 Windows 解出的主程序指纹。
// Windows 适配只追加 platforms.windows，不再维护第二份版本/安装包锁。
const root = fileURLToPath(new URL('../', import.meta.url));

export function windowsLockPath(regionId) {
  return getRegionProfile(regionId).windowsLock;
}

export function serializeLock(lock, eol = '\n') {
  return JSON.stringify(lock, null, 2).replace(/\n/g, eol) + eol;
}

export function windowsLockFromOfficial({ official, executable, current = null }) {
  const next = current ? structuredClone(current) : {
    schemaVersion: 2,
    region: official.region,
    version: official.version,
    manifestUrl: official.manifestUrl,
    installer: official.installer,
    platforms: {},
  };
  next.schemaVersion = 2;
  next.region = official.region;
  next.version = official.version;
  next.manifestUrl = official.manifestUrl;
  next.installer = official.installer;
  const previous = next.platforms?.windows || {};
  next.platforms = {
    ...(next.platforms || {}),
    windows: {
      ...previous,
      executable: {
        sha256: createHash('sha256').update(executable).digest('hex'),
        size: executable.length,
      },
    },
  };
  return next;
}

export async function refreshWindowsLock({ projectRoot = root, profile = getRegionProfile(), sevenzip = null } = {}) {
  const target = path.join(projectRoot, windowsLockPath(profile.id));
  const existing = await readFile(target, 'utf8');
  const official = validateLock(JSON.parse(existing), profile);
  const directCache = pinnedInstallerCache(official, projectRoot, profile).path;
  const installerPath = existsSync(directCache)
    ? directCache
    : pinnedInstallerCache(official, path.join(projectRoot, 'macos'), profile).path;
  const installer = await readFile(installerPath);
  verifyInstaller(installer, official, profile);
  const { filename, bytes } = await extractApplicationPayload(installer, sevenzip || findSevenZip());
  const next = windowsLockFromOfficial({ official, executable: bytes, current: JSON.parse(existing) });
  await writeFile(target, serializeLock(next, existing.includes('\r\n') ? '\r\n' : '\n'));
  return {
    region: profile.id,
    file: windowsLockPath(profile.id),
    version: next.version,
    executable: filename,
    official_exe_sha256: next.platforms.windows.executable.sha256,
    official_exe_size: next.platforms.windows.executable.size,
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  refreshWindowsLock()
    .then(result => console.log(JSON.stringify(result)))
    .catch(error => { console.error(error.message); process.exitCode = 1; });
}
