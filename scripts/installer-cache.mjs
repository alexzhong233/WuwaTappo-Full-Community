import { appendFile, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { getRegionProfile } from './regions.mjs';
import { pinnedInstallerCache, windowsOfficialCache } from './sync-upstream.mjs';

const args = process.argv.slice(2);
const platformArg = args.find(arg => arg.startsWith('--platform='))?.slice('--platform='.length);
const platform = (platformArg || process.env.WUWA_CACHE_PLATFORM || 'mac').toLowerCase();
if (!['mac', 'win'].includes(platform)) throw new Error(`Unsupported cache platform: ${platform}`);

const repoRoot = fileURLToPath(new URL('../', import.meta.url));
const macRoot = path.join(repoRoot, 'macos');
const profile = getRegionProfile();
const lock = JSON.parse(await readFile(path.join(repoRoot, platform === 'win' ? profile.windowsLock : profile.macLock), 'utf8'));

if (!process.env.GITHUB_OUTPUT) throw new Error('GITHUB_OUTPUT is required for installer cache metadata');

if (platform === 'win') {
  const cache = windowsOfficialCache(lock, repoRoot, profile);
  const lines = [
    `version=${cache.version}`,
    `urlhash=${cache.urlhash}`,
    `key=${cache.key}`,
    'path<<__WUWA_CACHE_PATH__',
    cache.path,
    '__WUWA_CACHE_PATH__',
  ];
  await appendFile(process.env.GITHUB_OUTPUT, `${lines.join('\n')}\n`);
  console.log(JSON.stringify(cache));
} else {
  const cache = pinnedInstallerCache(lock, macRoot, profile);
  await appendFile(process.env.GITHUB_OUTPUT, `key=${cache.key}\npath=${cache.path}\n`);
  console.log(JSON.stringify(cache));
}
