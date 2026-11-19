import { readFileSync } from 'node:fs';

const config = JSON.parse(readFileSync(new URL('../shared/config/regions.json', import.meta.url), 'utf8'));

function profileFromConfig(entry) {
  return Object.freeze({
    id: entry.id,
    region: entry.region,
    label: entry.label,
    configUrl: entry.configUrl,
    assetsOrigin: entry.assetsOrigin,
    updaterHostFile: entry.updaterHostFile,
    upstreamLock: `../${entry.macLock}`,
    frontendDir: entry.macFrontendDir,
    preparedDir: entry.macPreparedDir,
    iconsDir: entry.macIconsDir,
    tauriConfig: entry.macTauriConfig,
    productName: entry.macProductName,
    identifier: entry.macIdentifier,
    artifactPrefix: entry.artifactPrefix,
    macLock: entry.macLock,
    windowsLock: entry.windowsLock,
    petSite: entry.petSite,
  });
}

const profiles = Object.freeze(Object.fromEntries(
  Object.entries(config).map(([id, entry]) => [id, profileFromConfig(entry)]),
));

export function getRegionProfile(id = process.env.WUWA_REGION || 'cn') {
  if (!Object.hasOwn(profiles, id)) throw new Error(`未知服务区域：${id}，请选择 cn 或 global。`);
  return profiles[id];
}

export const regionConfig = config;
