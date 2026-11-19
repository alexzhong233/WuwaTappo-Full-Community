import assert from 'node:assert/strict';
import test from 'node:test';
import { spawnSync } from 'node:child_process';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { getRegionProfile } from '../scripts/regions.mjs';
import { tauriArguments } from '../scripts/app.mjs';

test('CN retains its existing resource paths and application identity', () => {
  const cn = getRegionProfile('cn');
  assert.equal(cn.identifier, 'dev.w33d.wuwatappo.fullatlas.mac');
  assert.equal(cn.frontendDir, 'frontend');
  assert.equal(cn.preparedDir, 'build/frontend');
  assert.equal(cn.tauriConfig, null);
  assert.deepEqual(tauriArguments('build', cn), ['build', '--bundles', 'app']);
});

test('Global has separate inputs, prepared resources, icon output and application identity', () => {
  const cn = getRegionProfile('cn'), global = getRegionProfile('global');
  for (const key of ['identifier', 'productName', 'frontendDir', 'preparedDir', 'iconsDir', 'upstreamLock', 'configUrl', 'assetsOrigin', 'artifactPrefix']) {
    assert.notEqual(cn[key], global[key], key);
  }
  assert.deepEqual(tauriArguments('build', global, ['--', '--locked']), [
    'build', '--bundles', 'app', '--config', 'src-tauri/tauri.global.conf.json', '--', '--locked',
  ]);
  assert.deepEqual(tauriArguments('icons', global), ['icon', 'frontend-global/app-icon.png', '--output', 'src-tauri/icons-global']);
});

test('artifact naming is pinned per region and each platform ships exactly one format', async () => {
  const { readFile } = await import('node:fs/promises');
  const repo = fileURLToPath(new URL('../..', import.meta.url));
  assert.equal(getRegionProfile('cn').artifactPrefix, 'WuwaTappo-Full-CN-Community');
  assert.equal(getRegionProfile('global').artifactPrefix, 'WuwaTappo-Full-Global-Community');
  const mac = await readFile(path.join(repo, '.github/workflows/build.yml'), 'utf8');
  const win = await readFile(path.join(repo, '.github/workflows/build-windows.yml'), 'utf8');
  const release = await readFile(path.join(repo, '.github/workflows/release.yml'), 'utf8');
  const devRelease = await readFile(path.join(repo, '.github/workflows/dev-release.yml'), 'utf8');
  const releaseManifest = await readFile(path.join(repo, 'scripts/release-assets.mjs'), 'utf8');
  const packager = await readFile(path.join(repo, 'macos/scripts/package-macos.sh'), 'utf8');
  // mac 只出 dmg、win 只出 exe 安装包；zip 不再作为发行物
  assert.ok(mac.includes('dist/*.dmg'));
  assert.ok(!mac.includes('dist/*.zip'));
  assert.ok(win.includes('dist/*Windows-x64*.exe'));
  assert.ok(!win.includes('dist/*Windows-x64*.zip'));
  assert.match(packager, /release-assets\.mjs" name --kind=macos --region="\$EXPECTED_REGION" --version="\$VERSION"/);
  assert.ok(!packager.includes('NAME="$ARTIFACT_PREFIX-$VERSION-macOS-arm64"'));
  // 发版必须两个平台都构建成功，且六个产物齐全
  assert.match(release, /needs: \[build-macos, build-windows\]/);
  assert.match(releaseManifest, /shared\/contracts\/release-assets\.json/);
  assert.match(releaseManifest, /releaseAssetName/);
  assert.ok(!releaseManifest.includes('WuwaTappo-Full-CN-Community-'));
  assert.ok(!releaseManifest.includes('WuwaTappo-Full-Global-Community-'));
  assert.match(release, /writeReleaseChecksums/);
  assert.match(devRelease, /release-assets\.mjs prepare/);
});

test('unknown regions fail rather than silently selecting an account environment', () => {
  for (const region of ['CN', 'international', '', '__proto__', 'constructor']) {
    assert.throws(() => getRegionProfile(region), /未知服务区域/);
  }
  assert.throws(() => tauriArguments('install', getRegionProfile('cn')), /未知构建操作/);
});

test('Rust test entry point applies Global configuration without changing existing CN overrides', {
  // 用 PATH 里的假 `cargo`（#!/bin/sh 脚本）拦截调用，Windows 上拿不到可执行的 cargo，
  // 这条只在 macOS/Linux 上有意义。
  skip: process.platform === 'win32' ? '仅在 POSIX 上可验证（假 cargo 脚本）' : false,
}, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'wuwa-cargo-entry-'));
  try {
    await writeFile(path.join(directory, 'cargo'), '#!/bin/sh\nprintf \'%s\' "${TAURI_CONFIG:-null}"\n', { mode: 0o755 });
    const entry = fileURLToPath(new URL('../scripts/app.mjs', import.meta.url));
    const run = region => spawnSync(process.execPath, [entry, 'test:rust'], {
      encoding: 'utf8', env: { PATH: `${directory}:${process.env.PATH}`, WUWA_REGION: region, TAURI_CONFIG: '{"legacy":true}' },
    });
    const cn = run('cn'), global = run('global');
    assert.equal(cn.status, 0, cn.stderr);
    assert.equal(global.status, 0, global.stderr);
    assert.deepEqual(JSON.parse(cn.stdout), { legacy: true });
    assert.equal(JSON.parse(global.stdout).identifier, getRegionProfile('global').identifier);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
