import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { extractApplicationPayload } from './extract-assets.mjs';
import { getRegionProfile } from './regions.mjs';
import { findSevenZip, pinnedInstallerCache, validateLock, verifyInstaller } from './sync-upstream.mjs';

// Windows 侧的基线锁和 macOS 侧的锁指的是同一个官方安装包，只多两个字段：
// official_exe_sha256 / official_exe_size —— 安装包解出来的主程序本体。
// 那两个值是注入适配的指纹，以前只能由人在 Windows 上跑 fetch-official.ps1 得到，
// 于是每次官方发版都得等人工介入，两平台版本也就必然错开（alpha.1 就是这么来的）。
//
// 其实提取主程序的逻辑 extract-assets.mjs 本来就有（解内层 NSIS → 7-Zip 列条目 →
// 认出变量名背后的应用本体），它在 macOS runner 上跑得通。所以官方一更新，
// 同一台机器就能把 Windows 锁一起算出来，两平台基线天然同步。

const root = fileURLToPath(new URL('../', import.meta.url));

export function windowsLockPath(regionId) {
  if (regionId === 'cn') return 'windows/upstream-win.json';
  if (/^[a-z]+$/.test(String(regionId))) return `windows/upstream-win-${regionId}.json`;
  throw new Error(`未知服务区域：${regionId}`);
}

export function serializeLock(lock, eol = '\n') {
  // Windows 侧这两份锁历史上是 CRLF 的。整份重排会让 diff 变成"全文替换"，
  // 审核时看不出到底改了哪个字段，所以按原文件的行尾写回。
  return JSON.stringify(lock, null, 2).replace(/\n/g, eol) + eol;
}

export function windowsLockFromOfficial({ official, executable, current = null }) {
  // 只覆盖机器算得出来的字段；$comment 与 adapter_notes 是人写的，原样保留。
  // 键序跟着原文件走，diff 里才会只剩真正变化的行。
  const next = current ? { ...current } : {
    $comment: 'Windows 侧注入适配所锁定的官方客户端版本。由同步工作流自动跟随官方最新版；CI 与 fetch-official.ps1 按这里取包，保证可重现。',
  };
  next.client_version = official.version;
  next.official_exe_sha256 = createHash('sha256').update(executable).digest('hex');
  next.official_exe_size = executable.length;
  next.installer_url = official.installer.url;
  next.installer_sha256 = official.installer.sha256;
  next.installer_size = official.installer.size;
  return next;
}

export async function refreshWindowsLock({ projectRoot = root, profile = getRegionProfile(), sevenzip = null } = {}) {
  const official = validateLock(JSON.parse(await readFile(path.join(projectRoot, profile.upstreamLock), 'utf8')), profile);
  const installerPath = pinnedInstallerCache(official, projectRoot, profile).path;
  const installer = await readFile(installerPath);
  // 先确认这份安装包就是锁里那一份，再拿它去解主程序——否则算出来的哈希没有意义。
  verifyInstaller(installer, official, profile);
  const { filename, bytes } = await extractApplicationPayload(installer, sevenzip || findSevenZip());
  const target = path.join(projectRoot, windowsLockPath(profile.id));
  const existing = await readFile(target, 'utf8');
  const next = windowsLockFromOfficial({ official, executable: bytes, current: JSON.parse(existing) });
  await writeFile(target, serializeLock(next, existing.includes('\r\n') ? '\r\n' : '\n'));
  return {
    region: profile.id,
    file: windowsLockPath(profile.id),
    version: next.client_version,
    executable: filename,
    official_exe_sha256: next.official_exe_sha256,
    official_exe_size: next.official_exe_size,
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  refreshWindowsLock()
    .then(result => console.log(JSON.stringify(result)))
    .catch(error => { console.error(error.message); process.exitCode = 1; });
}
