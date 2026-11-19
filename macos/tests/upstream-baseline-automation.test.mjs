import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {
  assertConsistent, nextPrerelease, readSources, readVersions, rewriteSources,
  versionFiles,
} from '../../scripts/bump-version.mjs';
import { detectOutputs } from '../../scripts/upstream-detect.mjs';

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

test('canonical lock 包含官方安装包指纹且不包含 Windows 主程序硬锁', () => {
  for (const region of ['cn', 'global']) {
    const lock = JSON.parse(readFileSync(path.join(repoRoot, `shared/upstream/${region}.json`), 'utf8'));
    assert.equal(lock.schemaVersion, 2);
    assert.ok(lock.installer?.sha256);
    assert.equal(lock.platforms, undefined);
  }
});
