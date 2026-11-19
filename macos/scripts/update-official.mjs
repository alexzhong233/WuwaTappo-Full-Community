import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { access, copyFile, cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { getRegionProfile } from './regions.mjs';
import { compareUpstream, discoverUpstream, syncUpstream, validateLock } from '../../scripts/sync-upstream.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
// 仓库根：npm 脚本入口（package.json）与两平台共用的产物目录 dist/ 都在这里。
const repoRoot = path.dirname(root.replace(/[\\/]$/, ''));

function parseOptions(args) {
  const options = { apply: false, force: false, package: true };
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--check') options.check = true;
    else if (arg === '--apply') options.apply = true;
    else if (arg === '--force') options.force = true;
    else if (arg === '--no-package') options.package = false;
    else if (arg === '--installer') {
      const value = args[++index];
      if (!value || value.startsWith('--')) throw new Error('--installer 需要本地官方 EXE 路径。');
      options.installer = value;
    } else throw new Error(`未知参数：${arg}`);
  }
  if (options.installer && !options.apply) throw new Error('--installer 必须和 --apply 一起使用。');
  if (options.check && (options.apply || options.force || options.installer)) {
    throw new Error('--check 只检测版本，不能和 --apply、--force 或 --installer 同时使用。');
  }
  return options;
}

function run(command, args, label) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: repoRoot, stdio: 'inherit', env: process.env });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() : reject(new Error(`${label}失败（退出码 ${code ?? 'unknown'}）`)));
  });
}

const npmCommand = process.env.npm_execpath
  ? [process.execPath, process.env.npm_execpath]
  : ['npm'];
function runNpm(args, label) {
  return run(npmCommand[0], [...npmCommand.slice(1), ...args], label);
}

async function sha256(filename) {
  const bytes = await readFile(filename);
  return createHash('sha256').update(bytes).digest('hex');
}

async function exists(filename) {
  try { await access(filename); return true; }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}

async function snapshot(projectRoot, profile) {
  const lockPath = path.join(projectRoot, profile.upstreamLock);
  const backup = await readFile(lockPath);
  const frontend = path.join(projectRoot, profile.frontendDir);
  const frontendBackup = await path.join(projectRoot, '.work', `frontend-before-update-${Date.now()}`);
  await mkdir(path.dirname(frontendBackup), { recursive: true });
  if (await exists(frontend)) await cp(frontend, frontendBackup, { recursive: true });
  return { lockPath, backup, frontend, frontendBackup };
}

async function restore(state) {
  await writeFile(state.lockPath, state.backup);
  if (await exists(state.frontendBackup)) {
    await rm(state.frontend, { recursive: true, force: true });
    await cp(state.frontendBackup, state.frontend, { recursive: true });
  }
  await rm(state.frontendBackup, { recursive: true, force: true });
}

async function cleanupSnapshot(state) {
  await rm(state.frontendBackup, { recursive: true, force: true });
}

async function main(args) {
  const options = parseOptions(args);
  const profile = getRegionProfile();
  const lockPath = path.join(root, profile.upstreamLock);
  const current = validateLock(JSON.parse(await readFile(lockPath)), profile);
  const latest = await discoverUpstream(fetch, profile);
  const comparison = compareUpstream(current, latest);
  const checkResult = {
    checkedAt: new Date().toISOString(),
    region: profile.region,
    currentVersion: comparison.currentVersion,
    latestVersion: comparison.latestVersion,
    changed: comparison.changed,
    installerUrl: comparison.installerUrl,
  };
  await mkdir(path.join(root, '.work'), { recursive: true });
  await writeFile(path.join(root, '.work', 'official-update-check.json'), `${JSON.stringify(checkResult, null, 2)}\n`);
  console.log(JSON.stringify(checkResult));

  if (options.check || (!options.apply && !options.force)) {
    if (comparison.changed) console.log(`检测到官方 ${profile.region} ${comparison.latestVersion}，使用 --apply 开始下载和适配。`);
    else console.log(`官方 ${profile.region} 仍为 ${comparison.currentVersion}，无需更新。`);
    return;
  }
  if (!comparison.changed && !options.force) {
    console.log('没有新的官方版本；如需重建当前版本，请加 --force。');
    return;
  }

  const state = await snapshot(root, profile);
  try {
    console.log(`开始下载并校验官方 ${latest.version}，随后提取并注入 FullAtlas…`);
    await syncUpstream({ updateLock: true, installer: options.installer }, root);
    await runNpm(['run', 'icons'], '图标准备');
    await runNpm(['run', 'prepare:frontend'], '官方前端适配');
    await runNpm(['test'], 'JavaScript 契约测试');
    await runNpm(['run', 'test:fullatlas'], 'FullAtlas 测试');
    await runNpm(['run', 'test:rust'], 'Rust 测试');
    await runNpm(['run', 'build', '--', '--', '--locked'], 'Mac 原生构建');
    if (options.package) await runNpm(['run', 'package'], 'ZIP / DMG 打包');
    const lock = JSON.parse(await readFile(lockPath));
    const result = {
      ...checkResult,
      appliedVersion: lock.version,
      package: options.package,
      artifacts: [],
    };
    if (options.package) {
      const dist = path.join(repoRoot, 'dist');
      for (const extension of ['zip', 'dmg']) {
        const filename = path.join(dist, `${profile.artifactPrefix}-${JSON.parse(await readFile(path.join(repoRoot, 'package.json'))).version}-arm64.${extension}`);
        if (await exists(filename)) result.artifacts.push({ file: path.relative(repoRoot, filename), sha256: await sha256(filename) });
      }
    }
    await writeFile(path.join(root, '.work', 'official-update-result.json'), `${JSON.stringify(result, null, 2)}\n`);
    await cleanupSnapshot(state);
    console.log(`官方 ${lock.version} 已完成 Mac + FullAtlas 构建。请检查 .work/official-update-result.json 后再提交或发布。`);
  } catch (error) {
    console.error(`官方更新未通过适配或构建门禁：${error.message}`);
    await restore(state);
    console.error('已恢复更新前的官方资源锁和前端。');
    process.exitCode = 1;
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
}

export { parseOptions };
