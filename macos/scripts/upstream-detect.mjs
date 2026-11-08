import { readFile, appendFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { getRegionProfile } from './regions.mjs';
import { compareUpstream, discoverUpstream, validateLock } from './sync-upstream.mjs';

// 探测官方是否发新版。这一步刻意不下载安装包（200 MB × 2），
// 只读清单，让"每天看一眼"的成本可以忽略。

const root = fileURLToPath(new URL('../', import.meta.url));

export async function detectUpstream({ regions = ['cn', 'global'], projectRoot = root, fetcher = fetch } = {}) {
  const results = [];
  for (const region of regions) {
    const profile = getRegionProfile(region);
    const lock = validateLock(JSON.parse(await readFile(path.join(projectRoot, profile.upstreamLock), 'utf8')), profile);
    results.push(compareUpstream(lock, await discoverUpstream(fetcher, profile)));
  }
  return results;
}

export function detectOutputs(results) {
  const byRegion = Object.fromEntries(results.map(item => [item.regionId, item]));
  const outputs = { need_update: results.some(item => item.changed) ? 'true' : 'false' };
  for (const id of ['cn', 'global']) {
    const item = byRegion[id];
    outputs[`${id}_changed`] = item?.changed ? 'true' : 'false';
    outputs[`${id}_version`] = item?.latestVersion || '';
  }
  return outputs;
}

export async function main({ target = process.env.TARGET || 'both', projectRoot = root } = {}) {
  const regions = target === 'both' ? ['cn', 'global'] : [target];
  const results = await detectUpstream({ regions, projectRoot });
  const outputs = detectOutputs(results);
  if (process.env.GITHUB_OUTPUT) {
    await appendFile(process.env.GITHUB_OUTPUT, Object.entries(outputs).map(([key, value]) => `${key}=${value}\n`).join(''));
  }
  return { ...outputs, regions: results };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main()
    .then(result => console.log(JSON.stringify(result)))
    .catch(error => { console.error(error.message); process.exitCode = 1; });
}
