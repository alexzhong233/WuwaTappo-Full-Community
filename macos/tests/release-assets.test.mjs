import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { releaseAssetName, releaseAssetNames, releaseFiles, writeReleaseChecksums } from '../../scripts/release-assets.mjs';

const version = '1.3.2-beta.1';

test('release manifest includes both platforms, both regions and Windows update packages', () => {
  const names = releaseAssetNames(version);
  assert.equal(names.length, 6);
  assert.equal(new Set(names).size, names.length);
  assert.equal(names.filter(name => name.endsWith('macOS-arm64.dmg')).length, 2);
  assert.equal(names.filter(name => name.includes('Windows-x64')).length, 2);
  assert.equal(names.filter(name => name.endsWith('win-x64-update.zip')).length, 2);
});

test('release names come from the shared contract and region profile', () => {
  assert.equal(releaseAssetName('macos', 'cn', version),
    `WuwaTappo-Full-CN-Community-${version}-macOS-arm64.dmg`);
  assert.equal(releaseAssetName('windowsInstaller', 'global', version),
    `WuwaTappo-Full-Global-Community-Windows-x64-${version}.exe`);
  assert.equal(releaseAssetName('windowsUpdate', 'cn', version),
    `WuwaTappo-Full-CN-Community-${version}-win-x64-update.zip`);
  assert.throws(() => releaseAssetName('unknown', 'cn', version), /未知发布产物类型/);
  assert.throws(() => releaseAssetName('macos', 'unknown', version), /未知发布区域/);
});

test('release checksums are generated from exactly the published files', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'wuwa-release-assets-'));
  try {
    await mkdir(path.join(root, 'dist'));
    for (const name of releaseAssetNames(version)) await writeFile(path.join(root, 'dist', name), name);
    const lines = writeReleaseChecksums(version, root);
    assert.equal(lines.length, 6);
    assert.equal((await readFile(path.join(root, 'dist/SHA256SUMS.txt'), 'utf8')).trim().split('\n').length, 6);
    assert.ok(releaseFiles(version, root).at(-1).endsWith('SHA256SUMS.txt'));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
