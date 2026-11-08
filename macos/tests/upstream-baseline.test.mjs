import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// 锁定的官方基线不能各走各的：macOS 跟着 1.0.4 走、Windows 还停在 1.0.3 的话，
// Windows 打包出去的还是旧客户端，注入锚点也没有跟着新版本验证过，
// 而且官方 CDN 抖动时没法回退到按新版本命名的镜像。这里把它锁成硬约束。

const read = path => JSON.parse(readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8'));

const regions = [
  { label: '国服', mac: 'macos/upstream-cn.json', win: 'windows/upstream-win.json' },
  { label: '国际服', mac: 'macos/upstream-global.json', win: 'windows/upstream-win-global.json' },
];

for (const region of regions) {
  test(`${region.label}：Windows 官方基线必须与 macOS 同一版本`, () => {
    const mac = read(region.mac);
    const win = read(region.win);
    assert.equal(
      win.client_version,
      mac.version,
      `${region.win} 锁定 ${win.client_version}，但官方基线已经是 ${mac.version}：` +
        '请先用 fetch-official.ps1 取新版本，在 staging 上跑一遍注入确认锚点，再更新锁文件与 official_exe_sha256',
    );
  });

  test(`${region.label}：两边的安装包哈希必须指向同一个官方包`, () => {
    const mac = read(region.mac);
    const win = read(region.win);
    assert.equal(
      win.installer_sha256,
      mac.installer.sha256,
      'Windows 与 macOS 锁的安装包不是同一个：镜像回退与完整性校验会各对各错',
    );
    assert.equal(win.installer_size, mac.installer.size);
  });

  test(`${region.label}：Windows 锁文件自洽`, () => {
    const win = read(region.win);
    const escaped = win.client_version.replace(/\./g, '\\.');
    assert.match(
      win.installer_url,
      new RegExp(`_${escaped}_x64-setup\\.exe$`),
      '安装包地址里的版本号与 client_version 不一致',
    );
    assert.match(win.installer_sha256, /^[0-9a-f]{64}$/);
    assert.match(win.official_exe_sha256, /^[0-9a-f]{64}$/);
    assert.ok(win.installer_size > 0 && win.official_exe_size > 0);
  });
}
