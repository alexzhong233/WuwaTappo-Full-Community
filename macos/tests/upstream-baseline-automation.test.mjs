import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {
  assertConsistent, nextPrerelease, readSources, readVersions, rewriteSources,
  versionFiles,
} from '../../scripts/bump-version.mjs';
import { detectOutputs } from '../../scripts/upstream-detect.mjs';
import { serializeLock, windowsLockFromOfficial, windowsLockPath } from '../../scripts/upstream-windows-lock.mjs';

const repoRoot = path.resolve(import.meta.dirname, '../..');

test('预发布号顺着同一条标签递增，正式版才另开 alpha 线', () => {
  assert.equal(nextPrerelease('1.3.0-alpha.1'), '1.3.0-alpha.2');
  assert.equal(nextPrerelease('1.3.0-alpha.9'), '1.3.0-alpha.10');
  assert.equal(nextPrerelease('1.3.0-beta.2'), '1.3.0-beta.3');
  // 正式版已经发出去了，不能覆盖同一个版本号，只能抬 patch。
  assert.equal(nextPrerelease('1.3.0'), '1.3.1-alpha.1');
  assert.equal(nextPrerelease('1.2.8'), '1.2.9-alpha.1');
  for (const invalid of ['', 'v1.3.0', '1.3', '1.3.0.1', 'alpha.1']) {
    assert.throws(() => nextPrerelease(invalid), /无法解析版本号/);
  }
});

test('仓库里的十处版本号当前是齐的', async () => {
  const versions = readVersions(await readSources(repoRoot));
  assert.deepEqual(Object.keys(versions), versionFiles);
  // 少对齐一处，打包（CFBundleShortVersionString 校验）或 Cargo --locked 就会失败。
  assert.equal(new Set(Object.values(versions)).size, 1, JSON.stringify(versions, null, 2));
});

test('两个 Cargo.lock 里本仓库的包版本都与 package.json 一致', () => {
  // macOS 的锁除了 app 自己，还带 shared/backend（路径依赖）的条目；只改一个，
  // `--locked` 构建就会报「cannot update the lock file」直接失败。
  const version = JSON.parse(readFileSync(path.join(repoRoot, 'package.json'), 'utf8')).version;
  const own = /name = "(?:wuwatappo-mac|wuwatappo-assistant-core)"\r?\nversion = "([^"]+)"/g;
  for (const file of ['macos/src-tauri/Cargo.lock', 'shared/backend/Cargo.lock']) {
    const text = readFileSync(path.join(repoRoot, file), 'utf8');
    const found = [...text.matchAll(own)].map(match => match[1]);
    assert.ok(found.length > 0, `${file} 里没有本仓库的包条目`);
    for (const value of found) assert.equal(value, version, `${file} 里的包版本是 ${value}`);
  }
});

test('漂移的版本号必须在递增前被挡住', () => {
  const sources = Object.fromEntries(versionFiles.map(file => [file, '']));
  const versions = Object.fromEntries(versionFiles.map(file => [file, '1.3.0-alpha.1']));
  versions['macos/src-tauri/Cargo.lock'] = '1.3.0-alpha.0';
  assert.throws(() => assertConsistent(versions), /版本号已经漂移/);
  assert.equal(Object.keys(sources).length, versionFiles.length);
});

test('一次改写覆盖全部十处，且只动版本号本身', async () => {
  const sources = await readSources(repoRoot);
  const previous = assertConsistent(readVersions(sources));
  const next = nextPrerelease(previous);
  const updated = rewriteSources(sources, previous, next);
  assert.deepEqual(readVersions(updated), Object.fromEntries(versionFiles.map(file => [file, next])));
  // package-lock 只应该改顶层与 packages."" 两处，依赖条目的版本不能被顺手改掉。
  assert.equal((updated['package-lock.json'].match(new RegExp(`"version": "${next.replace(/\./g, '\\.')}"`, 'g')) || []).length, 2);
  // 除版本号外内容不变：把新版本号换回旧值应当逐字节还原。
  for (const file of versionFiles) {
    assert.equal(updated[file].split(next).join(previous), sources[file], file);
  }
});

test('Windows 指纹只覆盖机器算得出的 platforms.windows 字段', () => {
  const official = {
    schemaVersion: 2,
    region: 'CN',
    version: '1.0.5',
    manifestUrl: 'https://example.test/latest.json',
    installer: { url: 'https://example.test/WuwaTappo-cn_1.0.5_x64-setup.exe', sha256: 'a'.repeat(64), size: 201758432 },
  };
  const current = {
    schemaVersion: 2,
    region: 'CN',
    version: '1.0.4',
    manifestUrl: 'https://example.test/old.json',
    installer: { url: 'https://example.test/old.exe', sha256: 'c'.repeat(64), size: 1 },
    platforms: {
      windows: {
        executable: { sha256: 'b'.repeat(64), size: 1 },
        adapterNotes: '1.0.4 锚点全部命中',
        comment: '人工写的说明',
      },
    },
  };
  const next = windowsLockFromOfficial({ official, executable: Buffer.from('official-exe'), current });
  assert.equal(next.version, '1.0.5');
  assert.equal(next.installer.url, official.installer.url);
  assert.equal(next.installer.sha256, official.installer.sha256);
  assert.equal(next.installer.size, official.installer.size);
  assert.equal(next.platforms.windows.executable.size, Buffer.from('official-exe').length);
  assert.match(next.platforms.windows.executable.sha256, /^[0-9a-f]{64}$/);
  assert.equal(next.platforms.windows.adapterNotes, current.platforms.windows.adapterNotes);
  assert.equal(next.platforms.windows.comment, current.platforms.windows.comment);
});

test('canonical upstream lock keeps its newline style', () => {
  const lock = { version: '1.0.5', platforms: { windows: { executable: { size: 3 } } } };
  assert.equal(serializeLock(lock, '\r\n'), JSON.stringify(lock, null, 2).replace(/\n/g, '\r\n') + '\r\n');
  assert.equal(serializeLock(lock, '\n').endsWith('\n'), true);
  assert.equal(serializeLock(lock, '\n').includes('\r'), false);
  assert.equal(windowsLockPath('cn'), 'shared/upstream/cn.json');
  assert.equal(windowsLockPath('global'), 'shared/upstream/global.json');
  assert.throws(() => windowsLockPath('../etc'), /未知服务区域/);
});
test('官方版本探测只在任一区域变化时请求编译', () => {
  const unchanged = detectOutputs([
    { regionId: 'cn', changed: false, latestVersion: '1.0.4' },
    { regionId: 'global', changed: false, latestVersion: '1.0.4' },
  ]);
  assert.equal(unchanged.need_update, 'false');
  const cnOnly = detectOutputs([
    { regionId: 'cn', changed: true, latestVersion: '1.0.5' },
    { regionId: 'global', changed: false, latestVersion: '1.0.4' },
  ]);
  assert.equal(cnOnly.need_update, 'true');
  assert.equal(cnOnly.cn_changed, 'true');
  assert.equal(cnOnly.global_changed, 'false');
  assert.equal(cnOnly.cn_version, '1.0.5');
  assert.equal(cnOnly.global_version, '1.0.4');
});

test('canonical lock 同时包含共同安装包和 Windows 主程序指纹', () => {
  for (const region of ['cn', 'global']) {
    const lock = JSON.parse(readFileSync(path.join(repoRoot, `shared/upstream/${region}.json`), 'utf8'));
    assert.equal(lock.schemaVersion, 2);
    assert.ok(lock.installer?.sha256);
    assert.ok(lock.platforms?.windows?.executable?.sha256);
  }
});
