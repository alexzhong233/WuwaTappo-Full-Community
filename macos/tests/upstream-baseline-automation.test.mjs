import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {
  assertConsistent, nextPrerelease, readSources, readVersions, rewriteSources,
  versionFiles,
} from '../../scripts/bump-version.mjs';
import { detectOutputs } from '../scripts/upstream-detect.mjs';
import { serializeLock, windowsLockFromOfficial, windowsLockPath } from '../scripts/upstream-windows-lock.mjs';

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

test('仓库里的八处版本号当前是齐的', async () => {
  const versions = readVersions(await readSources(repoRoot));
  assert.deepEqual(Object.keys(versions), versionFiles);
  // 少对齐一处，打包（CFBundleShortVersionString 校验）或 Cargo --locked 就会失败。
  assert.equal(new Set(Object.values(versions)).size, 1, JSON.stringify(versions, null, 2));
});

test('漂移的版本号必须在递增前被挡住', () => {
  const sources = Object.fromEntries(versionFiles.map(file => [file, '']));
  const versions = Object.fromEntries(versionFiles.map(file => [file, '1.3.0-alpha.1']));
  versions['macos/src-tauri/Cargo.lock'] = '1.3.0-alpha.0';
  assert.throws(() => assertConsistent(versions), /版本号已经漂移/);
  assert.equal(Object.keys(sources).length, versionFiles.length);
});

test('一次改写覆盖全部八处，且只动版本号本身', async () => {
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

test('Windows 基线锁只覆盖机器算得出的字段', () => {
  const official = {
    version: '1.0.5',
    installer: { url: 'https://example.test/WuwaTappo-cn_1.0.5_x64-setup.exe', sha256: 'a'.repeat(64), size: 201758432 },
  };
  const current = {
    $comment: '人工写的说明',
    client_version: '1.0.4',
    official_exe_sha256: 'b'.repeat(64),
    official_exe_size: 1,
    installer_url: 'https://example.test/old.exe',
    installer_sha256: 'c'.repeat(64),
    installer_size: 1,
    adapter_notes: '1.0.4 锚点全部命中',
  };
  const next = windowsLockFromOfficial({ official, executable: Buffer.from('official-exe'), current });
  assert.equal(next.client_version, '1.0.5');
  assert.equal(next.installer_url, official.installer.url);
  assert.equal(next.installer_sha256, official.installer.sha256);
  assert.equal(next.installer_size, official.installer.size);
  assert.equal(next.official_exe_size, Buffer.from('official-exe').length);
  assert.match(next.official_exe_sha256, /^[0-9a-f]{64}$/);
  // 人工写的字段原样保留，键序也不动 —— 否则 diff 会变成全文替换。
  assert.equal(next.$comment, current.$comment);
  assert.equal(next.adapter_notes, current.adapter_notes);
  assert.deepEqual(Object.keys(next), Object.keys(current));
});

test('Windows 锁按原行尾写回，避免整份文件被 diff 成替换', () => {
  const lock = { client_version: '1.0.5', official_exe_size: 3 };
  assert.equal(serializeLock(lock, '\r\n'), JSON.stringify(lock, null, 2).replace(/\n/g, '\r\n') + '\r\n');
  assert.equal(serializeLock(lock, '\n').endsWith('\n'), true);
  assert.equal(serializeLock(lock, '\n').includes('\r'), false);
  assert.equal(windowsLockPath('cn'), 'windows/upstream-win.json');
  assert.equal(windowsLockPath('global'), 'windows/upstream-win-global.json');
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

test('两平台锁指向同一个安装包', () => {
  for (const [macLock, winLock] of [
    ['macos/upstream-cn.json', 'windows/upstream-win.json'],
    ['macos/upstream-global.json', 'windows/upstream-win-global.json'],
  ]) {
    const mac = JSON.parse(readFileSync(path.join(repoRoot, macLock), 'utf8'));
    const win = JSON.parse(readFileSync(path.join(repoRoot, winLock), 'utf8'));
    assert.equal(win.client_version, mac.version, winLock);
    assert.equal(win.installer_sha256, mac.installer.sha256, winLock);
    assert.equal(win.installer_size, mac.installer.size, winLock);
  }
});
