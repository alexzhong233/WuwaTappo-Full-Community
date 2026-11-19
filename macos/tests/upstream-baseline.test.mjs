import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { getRegionProfile } from '../scripts/regions.mjs';

// 一份 canonical upstream lock 同时供 macOS、Windows 和发布流水线读取，
// 只锁定官方版本、安装包地址与官方安装包校验信息。

const read = path => JSON.parse(readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8'));

const regions = [
  { label: '国服', id: 'cn', region: 'CN' },
  { label: '国际服', id: 'global', region: 'GLOBAL' },
];

for (const item of regions) {
  test(`${item.label}：canonical lock 同时满足两平台基线`, () => {
    const profile = getRegionProfile(item.id);
    const lock = read(profile.macLock);
    assert.equal(profile.macLock, profile.windowsLock);
    assert.equal(lock.schemaVersion, 2);
    assert.equal(lock.region, item.region);
    assert.match(lock.version, /^\d+\.\d+\.\d+/);
    const escaped = lock.version.replace(/\./g, '\\.');
    assert.match(lock.installer.url, new RegExp(`_${escaped}_x64-setup\\.exe$`));
    assert.match(lock.installer.sha256, /^[0-9a-f]{64}$/);
    assert.ok(lock.installer.size > 0);
  });
}
