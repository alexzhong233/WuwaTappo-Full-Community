import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, truncateSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { measureImage } from '../scripts/dmg-size.mjs';
import { getRegionProfile } from '../scripts/regions.mjs';

const script = path.resolve(import.meta.dirname, '../scripts/package-macos.sh');
const root = path.resolve(import.meta.dirname, '..');
const repoRoot = path.resolve(root, '..');
const MIB = 1024 * 1024;
const packagingTest = process.platform === 'darwin' ? test : test.skip;

packagingTest('packaging exposes explicit application and output paths', () => {
  const result = spawnSync('bash', [script, '--help'], { encoding: 'utf8' });
  assert.equal(result.status, 0);
  assert.match(result.stdout, /--app/);
  assert.match(result.stdout, /--output/);
});

packagingTest('packaging rejects incomplete arguments and missing application bundles', () => {
  const incomplete = spawnSync('bash', [script, '--app'], { encoding: 'utf8' });
  assert.equal(incomplete.status, 2);
  const missing = spawnSync('bash', [script, '--app', '/nonexistent/Wuwa Tappo CN Mac.app'], { encoding: 'utf8' });
  assert.equal(missing.status, 1);
  assert.match(missing.stderr, /未找到完整应用|需要在 Mac/);
});

packagingTest('packaging reports the failing stage and preserves the tool exit status', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'wuwa-package-error-'));
  try {
    const app = path.join(directory, 'Broken.app');
    mkdirSync(path.join(app, 'Contents'), { recursive: true });
    writeFileSync(path.join(app, 'Contents/Info.plist'), '<?xml version="1.0"?><plist version="1.0"><dict/></plist>');
    const result = spawnSync('bash', [script, '--app', app], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    if (process.platform === 'darwin') {
      assert.match(result.stdout, /打包阶段：读取应用信息/);
      assert.match(result.stderr, /打包失败：读取应用信息（第 \d+ 行，退出状态 1）/);
    } else {
      assert.match(result.stderr, /需要在 Mac/);
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

packagingTest('packaging rejects unknown regions instead of silently choosing CN', () => {
  const result = spawnSync('bash', [script], { encoding: 'utf8', env: { ...process.env, WUWA_REGION: 'unknown' } });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /未知服务区域/);
});

packagingTest('both regions reject an application bundle from the other region before reading its binary', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'wuwa-package-region-'));
  try {
    const base = JSON.parse(readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
    for (const region of ['cn', 'global']) {
      const profile = getRegionProfile(region);
      const other = getRegionProfile(region === 'cn' ? 'global' : 'cn');
      const override = profile.tauriConfig ? JSON.parse(readFileSync(path.join(root, profile.tauriConfig), 'utf8')) : {};
      const app = path.join(directory, `${region}.app`);
      mkdirSync(path.join(app, 'Contents'), { recursive: true });
      writeFileSync(path.join(app, 'Contents/Info.plist'), `<?xml version="1.0"?><plist version="1.0"><dict>
        <key>CFBundleIdentifier</key><string>${other.identifier}</string>
        <key>CFBundleName</key><string>${other.productName}</string>
        <key>CFBundleExecutable</key><string>wuwatappo-mac</string>
        <key>CFBundleShortVersionString</key><string>${override.version ?? base.version}</string>
        <key>LSMinimumSystemVersion</key><string>13.0</string>
      </dict></plist>`);
      const result = spawnSync('bash', [script, '--app', app], { encoding: 'utf8', env: { ...process.env, WUWA_REGION: region } });
      assert.equal(result.status, 1);
      if (process.platform === 'darwin') {
        assert.ok(result.stderr.includes(`与${profile.label}构建配置不一致`));
        assert.ok(!result.stdout.includes('检查架构与签名'));
      } else {
        assert.match(result.stderr, /需要在 Mac/);
      }
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test('package resource validation rejects mixed-region frontend and embedded entry points', () => {
  const source = readFileSync(script, 'utf8');
  const validation = source.match(/node --input-type=module -e '\n(import \{ readFile, readdir \}[\s\S]*?)\n' "\$ROOT" "\$BINARY"/)?.[1];
  assert.ok(validation, 'the package resource validation must be exercised');
  const directory = mkdtempSync(path.join(tmpdir(), 'wuwa-package-embedded-'));
  try {
    const prepared = path.join(directory, 'build/frontend-global/assets');
    const contents = path.join(directory, 'Global.app/Contents');
    mkdirSync(prepared, { recursive: true });
    mkdirSync(path.join(contents, 'MacOS'), { recursive: true });
    mkdirSync(path.join(contents, 'Resources'), { recursive: true });
    writeFileSync(path.join(prepared, '../index.html'), '<script src="./assets/global.js"></script>');
    writeFileSync(path.join(prepared, 'global.js'), 'const APP_REGION="GLOBAL";');
    for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) writeFileSync(path.join(contents, 'Resources', name), 'notice');
    const binary = path.join(contents, 'MacOS/wuwatappo-mac');
    writeFileSync(binary, '/index.html\0/assets/cn.js\0/app-icon.png');
    const run = () => spawnSync(process.execPath, ['--input-type=module', '-e', validation, directory, binary, 'build/frontend-global', 'GLOBAL', '国际服'], { encoding: 'utf8' });
    const mixed = run();
    assert.equal(mixed.status, 1);
    assert.match(mixed.stderr, /未嵌入必要资源：\/assets\/global.js/);
    writeFileSync(binary, '/index.html\0/assets/global.js\0/app-icon.png');
    assert.equal(run().status, 0);
    writeFileSync(path.join(prepared, 'global.js'), 'const APP_REGION="CN";');
    const wrongFrontend = run();
    assert.equal(wrongFrontend.status, 1);
    assert.match(wrongFrontend.stderr, /必须使用国际服前端/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test('DMG capacity uses sparse-file logical size rather than allocated disk blocks', async () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'wuwa-dmg-sparse-'));
  try {
    const filename = path.join(directory, 'payload');
    writeFileSync(filename, '');
    truncateSync(filename, 512 * MIB);
    assert.deepEqual(await measureImage(directory), { logicalBytes: 512 * MIB, sizeMiB: 640 });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test('DMG capacity reserves filesystem space for small payloads', async () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'wuwa-dmg-small-'));
  try {
    assert.deepEqual(await measureImage(directory), { logicalBytes: 0, sizeMiB: 64 });
    const filename = path.join(directory, 'payload');
    writeFileSync(filename, '');
    truncateSync(filename, 64 * MIB);
    assert.deepEqual(await measureImage(directory), { logicalBytes: 64 * MIB, sizeMiB: 96 });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test('DMG sizing counts symlinks without traversing outside the packaging directory', async () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'wuwa-dmg-link-'));
  try {
    const content = path.join(directory, 'content');
    const external = path.join(directory, 'external');
    mkdirSync(content);
    mkdirSync(external);
    writeFileSync(path.join(external, 'large-file'), '');
    truncateSync(path.join(external, 'large-file'), 512 * MIB);
    symlinkSync(external, path.join(content, 'Applications'));
    symlinkSync(content, path.join(content, 'cycle'));
    const result = await measureImage(content);
    assert.equal(result.logicalBytes, Buffer.byteLength(external) + Buffer.byteLength(content));
    assert.equal(result.sizeMiB, 64);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

/*
 * 搬目录时最容易漏的是 bundle.resources：本机构建看不出来，只有 macOS CI 打包或运行期取资源时才炸。
 * 这里把「配置里写的每个资源都真实存在」变成一次本地可跑的断言，路径改动越界会立刻失败。
 * ../build/ 下的条目是 npm run setup 生成的产物，未构建时本来就不存在，跳过。
 */
test('tauri 配置引用的打包资源都真实存在', () => {
  const GENERATED = /^\.\.\/build\//;
  let checked = 0;
  for (const config of ['src-tauri/tauri.conf.json', 'src-tauri/tauri.global.conf.json']) {
    const file = path.join(root, config);
    if (!existsSync(file)) continue;
    const { bundle = {} } = JSON.parse(readFileSync(file, 'utf8'));
    for (const [source, target] of Object.entries(bundle.resources ?? {})) {
      assert.ok(target, `${config} 的资源缺少目标名：${source}`);
      if (GENERATED.test(source)) continue;
      const resolved = path.resolve(path.dirname(file), source);
      assert.ok(existsSync(resolved), `${config} 的 bundle.resources 指向不存在的文件：${source}（解析为 ${resolved}）`);
      checked += 1;
    }
  }
  assert.ok(checked >= 3, `应至少核对 3 个已入库资源（LICENSE、THIRD_PARTY_NOTICES、FULLATLAS_LICENSE），实际 ${checked} 个`);
});

test('打包产物默认落在仓库根的 dist/，与 Windows 侧共用同一个交付目录', () => {
  const source = readFileSync(script, 'utf8');
  assert.match(source, /^REPO_ROOT="\$\(cd "\$ROOT\/\.\." && pwd\)"$/m);
  assert.match(source, /^OUTPUT="\$REPO_ROOT\/dist"$/m);
  assert.ok(existsSync(path.join(repoRoot, 'package.json')), '仓库根应保留 package.json（npm 入口与发版号来源）');
});
