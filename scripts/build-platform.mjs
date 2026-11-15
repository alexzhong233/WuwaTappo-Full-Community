#!/usr/bin/env node
/**
 * 统一构建入口：按平台分派到对应的构建链。
 *
 * 为什么不能真正「一条命令出两个平台」：
 *   macOS 侧   Tauri + Rust，必须 macOS + Xcode，产物是 .app/.dmg
 *   Windows 侧 给官方 Windows 客户端打补丁并重打包 PE，必须 Windows + Rust 工具链 + 官方安装包
 * 两者都无法交叉编译。所以这里的 "all" 是「构建当前系统能构建的那一平台，并明确告知另一平台
 * 该在哪里构建」，而不是假装在本机产出两个平台。真正的双平台并行发生在 CI（两个 runner）。
 *
 * 用法：
 *   node scripts/build-platform.mjs                 # 自动识别当前平台并构建
 *   node scripts/build-platform.mjs --check         # 只检查前置条件
 *   node scripts/build-platform.mjs --platform=mac
 *   node scripts/build-platform.mjs --platform=win
 *   node scripts/build-platform.mjs --platform=all
 *   node scripts/build-platform.mjs --package       # 构建完顺手打包
 *   node scripts/build-platform.mjs --latest        # Windows 侧取官方最新版（默认锁定适配版）
 */

import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const WINDOWS = join(repoRoot, 'windows');
// macOS 平台实现整体在 macos/ 下；跨平台分派器（本文件）留在仓库根的 scripts/。
const MACOS = join(repoRoot, 'macos');

const argv = process.argv.slice(2);
const has = name => argv.includes(`--${name}`);
const value = name => {
  const hit = argv.find(a => a.startsWith(`--${name}=`));
  return hit ? hit.slice(name.length + 3) : undefined;
};
const passthrough = argv.includes('--') ? argv.slice(argv.indexOf('--') + 1) : [];

const hostPlatform = process.platform === 'darwin' ? 'mac' : process.platform === 'win32' ? 'win' : 'other';
const requested = value('platform') ?? 'auto';
const target = requested === 'auto' ? hostPlatform : requested;
const checkOnly = has('check');
const wantPackage = has('package');
const latest = has('latest');
const noRuntime = has('no-runtime');
// 区域：国服 cn / 国际服 global。Windows 侧靠 -Region 分派到两套锁与产物目录，
// macOS 侧沿用 WUWA_REGION 环境变量，所以这里统一写回 env，platform=all 时两边一致。
const region = String(value('region') ?? process.env.WUWA_REGION ?? 'cn').toLowerCase();
if (!['cn', 'global'].includes(region)) throw new Error(`未知区域：${region}（可选 cn / global）`);
process.env.WUWA_REGION = region;
const winBuildDir = region === 'global' ? 'build-global' : 'build';

const log = (...args) => console.log(...args);
const section = title => console.log(`\n=== ${title} ===`);

function which(command) {
  const probe = process.platform === 'win32'
    ? spawnSync('where', [command], { encoding: 'utf8' })
    : spawnSync('sh', ['-c', `command -v ${command}`], { encoding: 'utf8' });
  return probe.status === 0 && probe.stdout.trim() ? probe.stdout.trim().split(/\r?\n/)[0] : null;
}

function readJson(path) {
  // 兼容 Windows 侧脚本可能带 BOM 写出的 JSON
  return JSON.parse(readFileSync(path, 'utf8').replace(/^\uFEFF/, ''));
}

function run(command, args, options = {}) {
  log(`$ ${[command, ...args].join(' ')}`);
  const env = { ...process.env, ...(options.env ?? {}) };
  // PowerShell 5.1 在继承 pwsh 7 的 PSModulePath 时加载不到自己的 Utility 模块，
  // Get-FileHash 之类的 cmdlet 会消失。删掉让它用自身默认路径。
  if (/powershell(\.exe)?$/i.test(command)) delete env.PSModulePath;
  const result = spawnSync(command, args, {
    stdio: 'inherit',
    cwd: options.cwd ?? repoRoot,
    env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    const error = new Error(`${command} 退出码 ${result.status}`);
    error.exitCode = result.status;
    throw error;
  }
}

/* ---------------------------------- 前置检查 --------------------------------- */

function checkMac() {
  const rows = [];
  rows.push(['平台', process.platform === 'darwin' ? `darwin/${process.arch}` : `当前是 ${process.platform}`, process.platform === 'darwin']);
  rows.push(['Node', process.version, true]);
  const rustup = which('rustup');
  rows.push(['rustup', rustup ?? '未找到', Boolean(rustup)]);
  rows.push(['src-tauri', existsSync(join(MACOS, 'src-tauri', 'Cargo.toml')) ? '存在' : '缺失', existsSync(join(MACOS, 'src-tauri', 'Cargo.toml'))]);
  rows.push(['node_modules', existsSync(join(repoRoot, 'node_modules')) ? '已装' : '未装（先 npm ci）', existsSync(join(repoRoot, 'node_modules'))]);
  const upstream = readJson(join(MACOS, 'upstream-cn.json'));
  rows.push(['官方基底（锁）', upstream.version, true]);
  return { rows, ok: rows.every(([, , ok]) => ok) };
}

function checkWin() {
  const rows = [];
  const py = process.env.WUWA_PYTHON ?? which('python3.13') ?? which('python3.12') ?? which('python');
  rows.push(['平台', process.platform === 'win32' ? `win32/${process.arch}` : `当前是 ${process.platform}`, process.platform === 'win32']);
  rows.push(['Node', process.version, true]);
  // 本地服务与启动器都是 shared/backend 的 Rust 产物；缺 cargo 时 build.ps1 会停在
  // 第 3 步，所以这里必须挡住，别让前置检查报「可以构建」。
  const cargo = which('cargo');
  rows.push(['cargo', cargo ?? '未找到（装 Rust stable）', Boolean(cargo)]);
  rows.push(['Python', py ?? '未找到', Boolean(py)]);
  const venv = join(WINDOWS, '.venv', 'Scripts', 'python.exe');
  rows.push(['windows\\.venv', existsSync(venv) ? '已就绪' : '未创建（setup-env.ps1 会自动建）', true]);
  const lockName = region === 'global' ? 'upstream-win-global.json' : 'upstream-win.json';
  const lockPath = join(WINDOWS, lockName);
  rows.push(['区域', region === 'global' ? '国际服（global）' : '国服（cn）', true]);
  if (existsSync(lockPath)) {
    const lock = readJson(lockPath);
    rows.push(['适配基底（锁）', lock.client_version, true]);
    rows.push(['安装包直链', lock.installer_url ? '已配置' : '缺失', Boolean(lock.installer_url)]);
  } else {
    rows.push([lockName, '缺失', false]);
  }
  // 构建产物依赖：官方基底是否已解出（没有也可以由 fetch-official.ps1 现取）
  const base = join(WINDOWS, winBuildDir, 'official', 'current.json');
  if (existsSync(base)) {
    const info = readJson(base);
    rows.push(['已解基底', `${info.version}（${Math.round(info.size / 1048576)} MB）`, true]);
  } else {
    rows.push(['已解基底', '尚未获取（构建时会自动下载）', true]);
  }
  return { rows, ok: rows.every(([, , ok]) => ok) };
}

function printTable(title, rows) {
  section(title);
  const width = Math.max(...rows.map(([k]) => displayWidth(k)));
  for (const [key, val, ok] of rows) {
    log(`  ${ok ? 'OK  ' : 'FAIL'} ${key.padEnd(width + 2 - (displayWidth(key) - key.length))}${val}`);
  }
}

// 中文按两列宽计算，保证表格不歪
function displayWidth(text) {
  return [...text].reduce((sum, ch) => sum + (/[\u3000-\u9fff\uff00-\uffef]/.test(ch) ? 2 : 1), 0);
}

/* ---------------------------------- 各平台构建 -------------------------------- */

function buildMac({ withPackage }) {
  section('macOS 构建');
  const args = [join('macos', 'scripts', 'app.mjs'), 'build'];
  if (passthrough.length) args.push('--', ...passthrough);
  run(process.execPath, args);

  if (!withPackage) {
    log('\n已跳过打包（加 --package 产出 dmg/zip）');
    return;
  }
  section('macOS 打包');
  if (process.platform !== 'darwin') throw new Error('打包只能在 macOS 上执行');
  run('bash', [join('macos', 'scripts', 'package-macos.sh')]);
}

function buildWin({ withPackage }) {
  section('Windows 构建');
  const ps = process.platform === 'win32' ? 'powershell.exe' : null;
  if (!ps) throw new Error('Windows 构建只能在 Windows 上执行');

  const call = (script, args = []) =>
    run(ps, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(WINDOWS, 'tools', script), '-Region', region, ...args]);
  log(`区域：${region === 'global' ? '国际服' : '国服'}`);

  if (!existsSync(join(WINDOWS, '.venv', 'Scripts', 'python.exe'))) {
    section('准备 Python 环境');
    call('setup-env.ps1');
  }

  section('获取官方基底');
  call('fetch-official.ps1', latest ? ['-Latest'] : []);

  const infoPath = join(WINDOWS, winBuildDir, 'official', 'current.json');
  if (!existsSync(infoPath)) throw new Error('fetch-official.ps1 未产出 build\\official\\current.json');
  const info = readJson(infoPath);
  if (info.hash_matched === false) {
    log(`\n警告：基底 ${info.version} 的哈希与锁定值不一致，注入锚点可能已漂移。`);
  }

  section('注入并装配');
  call('build.ps1', ['-Extract', '-Base', info.base, ...(noRuntime ? ['-NoRuntime'] : [])]);

  if (!withPackage) {
    log('\n已跳过打包（加 --package 产出 exe 安装包）');
    return;
  }
  section('Windows 打包');
  call('package-installer.ps1', noRuntime ? ['-NoRuntime'] : []);
}

const otherPlatformHint = platform => {
  if (platform === 'mac') {
    return [
      '另一平台 macOS 无法在当前系统构建（Tauri 需要 macOS + Xcode）。',
      '  · CI：push 到 main 后 .github/workflows/build.yml 会在 macos-15 runner 上产出 dmg',
      '  · 本机：在 macOS 上执行 npm run build:mac -- --package',
    ].join('\n');
  }
  return [
    '另一平台 Windows 无法在当前系统构建（需要 Windows + 官方 Windows 客户端）。',
    '  · CI：push 到 main 后 .github/workflows/build-windows.yml 会在 windows-2022 runner 上产出 exe 安装包',
    '  · 本机：在 Windows 上执行 npm run build:win -- --package',
  ].join('\n');
};

/* ----------------------------------- 主流程 ---------------------------------- */

function main() {
  log(`仓库：${repoRoot}`);
  log(`主机：${process.platform}/${process.arch}   目标：${target}`);

  if (checkOnly) {
    const mac = checkMac();
    const win = checkWin();
    printTable('macOS 前置条件', mac.rows);
    printTable('Windows 前置条件', win.rows);
    const relevant = target === 'mac' ? mac : target === 'win' ? win : (hostPlatform === 'mac' ? mac : win);
    section('结论');
    log(relevant.ok ? `当前平台（${hostPlatform}）可以构建。` : `当前平台（${hostPlatform}）还缺前置条件，见上面的 FAIL 行。`);
    return relevant.ok ? 0 : 1;
  }

  if (target === 'all') {
    const local = hostPlatform;
    if (local === 'other') throw new Error('当前系统既不是 macOS 也不是 Windows，无法构建任一平台');
    log(`\n[1/2] 先构建本平台：${local}`);
    if (local === 'mac') buildMac({ withPackage: wantPackage });
    else buildWin({ withPackage: wantPackage });
    log('\n[2/2] 另一平台');
    log(otherPlatformHint(local === 'mac' ? 'win' : 'mac'));
    return 0;
  }

  if (target === 'mac') {
    if (process.platform !== 'darwin') {
      log(`\n无法在当前系统构建 macOS 版。\n${otherPlatformHint('mac')}`);
      return 1;
    }
    buildMac({ withPackage: wantPackage });
    return 0;
  }

  if (target === 'win') {
    if (process.platform !== 'win32') {
      log(`\n无法在当前系统构建 Windows 版。\n${otherPlatformHint('win')}`);
      return 1;
    }
    buildWin({ withPackage: wantPackage });
    return 0;
  }

  throw new Error(`未知平台：${target}（可选 auto|mac|win|all）`);
}

try {
  process.exitCode = main() ?? 0;
} catch (error) {
  console.error(`\n构建失败：${error.message}`);
  if (error.exitCode) process.exitCode = error.exitCode;
  else process.exitCode = 1;
}
