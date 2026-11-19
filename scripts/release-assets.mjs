#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = fileURLToPath(new URL('../', import.meta.url));
const regions = JSON.parse(readFileSync(new URL('../shared/config/regions.json', import.meta.url), 'utf8'));
const contract = JSON.parse(readFileSync(new URL('../shared/contracts/release-assets.json', import.meta.url), 'utf8'));
export const CHECKSUM_FILE = contract.checksumFile;

export function releaseAssetName(kind, region, version) {
  const template = contract.formats[kind];
  const profile = regions[String(region).toLowerCase()];
  if (!template) throw new Error(`未知发布产物类型：${kind}`);
  if (!profile) throw new Error(`未知发布区域：${region}`);
  if (!version) throw new Error('发布版本不能为空');
  return template
    .replaceAll('{prefix}', profile.artifactPrefix)
    .replaceAll('{version}', version);
}

export function releaseAssetNames(version) {
  return ['cn', 'global'].flatMap(region => [
    releaseAssetName('macos', region, version),
    releaseAssetName('windowsInstaller', region, version),
    releaseAssetName('windowsUpdate', region, version),
  ]);
}

export function releaseAssetPaths(version, root = projectRoot) {
  return releaseAssetNames(version).map(name => path.join(root, 'dist', name));
}

export function releaseFiles(version, root = projectRoot) {
  return [...releaseAssetPaths(version, root), path.join(root, 'dist', CHECKSUM_FILE)];
}

export function assertReleaseAssets(version, root = projectRoot) {
  const missing = releaseAssetPaths(version, root).filter(file => !existsSync(file));
  if (missing.length) {
    throw new Error(`Release 缺少产物，两个平台都必须构建成功：${missing.map(file => path.relative(root, file)).join(', ')}`);
  }
}

export function writeReleaseChecksums(version, root = projectRoot) {
  assertReleaseAssets(version, root);
  const lines = releaseAssetNames(version).map(name => {
    const bytes = readFileSync(path.join(root, 'dist', name));
    return `${createHash('sha256').update(bytes).digest('hex')}  ${name}`;
  });
  writeFileSync(path.join(root, 'dist', CHECKSUM_FILE), `${lines.join('\n')}\n`);
  return lines;
}

function parseArgs(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--version' && args[index + 1]) options.version = args[++index];
    else if (arg === '--kind' && args[index + 1]) options.kind = args[++index];
    else if (arg === '--region' && args[index + 1]) options.region = args[++index];
    else if (arg.startsWith('--version=')) options.version = arg.slice('--version='.length);
    else if (arg.startsWith('--kind=')) options.kind = arg.slice('--kind='.length);
    else if (arg.startsWith('--region=')) options.region = arg.slice('--region='.length);
    else throw new Error(`未知参数：${arg}`);
  }
  if (!options.version) throw new Error('缺少 --version');
  return options;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, ...rest] = process.argv.slice(2);
  try {
    const { version, kind, region } = parseArgs(rest);
    if (command === 'list') console.log(releaseAssetNames(version).join('\n'));
    else if (command === 'files') console.log(releaseFiles(version).join('\n'));
    else if (command === 'name') console.log(releaseAssetName(kind, region, version));
    else if (command === 'prepare') writeReleaseChecksums(version);
    else throw new Error(`未知命令：${command || '(empty)'}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
