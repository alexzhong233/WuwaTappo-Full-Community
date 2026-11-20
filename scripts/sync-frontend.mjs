#!/usr/bin/env node
/**
 * 共享前端唯一源校验与内存渲染器
 *
 * `shared/frontend/` 是双端唯一源（含 Windows 右键菜单模块）；构建期由 Rust 注入器
 * `shared/backend/src/injector.rs` 直接按平台渲染并写入产物，不再在仓库里保留
 * `macos/fullatlas-source/` 与 `windows/src/src/` 的生成副本。
 *
 * 渲染时只替换平台占位符，其余逐字节相同：
 *   1. `__WUWA_BRAND__`              → 各平台标题栏文案
 *   2. `__WUWA_PLATFORM__`           → 各平台名（首行注释）
 *   3. `__WUWA_ASSISTANT_DEFAULTS__` → shared/contracts/assistant-settings.json defaults
 */
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));

export const MODULES = [
  'assistant_dashboard.js',
  'desktop_assistant.js',
  'desktop_context_menu.js',
  'local_atlas_controller.js',
];

export const LEGACY_COPY_DIRS = ['macos/fullatlas-source', 'windows/src/src'];

export const TARGETS = {
  mac: {
    brand: 'WUWA TAPPO FULLATLAS / MAC',
    platform: 'macOS',
    version: null,
  },
  win: {
    brand: 'WUWA TAPPO / 后台助手',
    platform: 'Windows',
    version: null,
  },
};

/** 版本号常量只在这一个模块里，别处没有 `const VERSION=`。 */
export const VERSION_MODULE = 'desktop_assistant.js';
const SETTINGS_CONTRACT = JSON.parse(await readFile(new URL('../shared/contracts/assistant-settings.json', import.meta.url), 'utf8'));

/** 把唯一源渲染成某个平台的源码字符串。 */
export function render(name, source, target, settingsDefaults = SETTINGS_CONTRACT.defaults) {
  let text = source
    .replaceAll('__WUWA_BRAND__', target.brand)
    .replaceAll('__WUWA_PLATFORM__', target.platform);
  if (name === VERSION_MODULE) {
    text = text.replaceAll('__WUWA_ASSISTANT_DEFAULTS__', JSON.stringify(settingsDefaults));
  }
  if (target.version && name === VERSION_MODULE) {
    const pattern = /const VERSION='[^']*'/;
    if (!pattern.test(text)) throw new Error(`${name} 里找不到 const VERSION='...'，无法覆盖版本号`);
    text = text.replace(pattern, `const VERSION='${target.version}'`);
  }
  for (const placeholder of ['__WUWA_BRAND__', '__WUWA_PLATFORM__', '__WUWA_ASSISTANT_DEFAULTS__']) {
    if (text.includes(placeholder)) throw new Error(`${name} 渲染后仍有未替换的占位符 ${placeholder}`);
  }
  return text;
}

/** 从 shared/frontend 读取并渲染指定平台模块。 */
export async function renderPlatformModule(name, platformKey, projectRoot = root) {
  const target = TARGETS[platformKey];
  if (!target) throw new Error(`未知平台：${platformKey}`);
  const source = await readFile(path.join(projectRoot, 'shared/frontend', name), 'utf8');
  const contract = projectRoot === root
    ? SETTINGS_CONTRACT
    : JSON.parse(await readFile(path.join(projectRoot, 'shared/contracts/assistant-settings.json'), 'utf8'));
  return render(name, source, target, contract.defaults);
}

/**
 * 校验唯一源完整性与无残余生成副本。
 * 返回 { checked, drifted }，check=true 时发现问题抛错。
 */
export async function syncFrontend({ projectRoot = root, check = false } = {}) {
  const drifted = [];
  const contract = projectRoot === root
    ? SETTINGS_CONTRACT
    : JSON.parse(await readFile(path.join(projectRoot, 'shared/contracts/assistant-settings.json'), 'utf8'));

  for (const dir of LEGACY_COPY_DIRS) {
    if (existsSync(path.join(projectRoot, dir))) {
      drifted.push(`发现残留的生成副本目录：${dir}`);
    }
  }

  for (const name of MODULES) {
    const source = await readFile(path.join(projectRoot, 'shared/frontend', name), 'utf8');
    for (const [key, target] of Object.entries(TARGETS)) {
      try {
        render(name, source, target, contract.defaults);
      } catch (error) {
        drifted.push(`${name} (${key}): ${error.message}`);
      }
    }
  }

  if (check && drifted.length) {
    throw new Error(`共享前端检查失败：\n  ${drifted.join('\n  ')}`);
  }
  return { changed: [], drifted };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const check = process.argv.includes('--check');
  syncFrontend({ check })
    .then(() => {
      console.log(`共享前端唯一源校验通过（${MODULES.length} 个模块 × ${Object.keys(TARGETS).length} 个平台，无重复副本）`);
    })
    .catch(error => {
      console.error(error.message);
      process.exitCode = 1;
    });
}
