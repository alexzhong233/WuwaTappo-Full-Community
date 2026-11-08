#!/usr/bin/env node
/**
 * 前端三件套的单一源生成器
 *
 * `shared/frontend/` 是唯一源，`macos/fullatlas-source/` 与 `windows/src/src/`
 * 下的同名文件都是它的产物。以前这两份是手工同步的，靠 `platform-parity.test.mjs`
 * 锁「差异不超过 4 行」来兜底；但每加一个功能都要两边各改一遍，改漏了就是静默分叉，
 * 所以改成生成：谁改功能只改唯一源，副本由这里产出，`frontend-sync.test.mjs` 卡住手改副本。
 *
 * 生成时只做三件事，其余逐字节相同：
 *   1. `__WUWA_BRAND__`  → 各平台标题栏文案
 *   2. `__WUWA_PLATFORM__` → 各平台名（只出现在首行注释里）
 *   3. `const VERSION=` → Windows 侧沿用上游 FullAtlas 的 1.2，Mac 侧跟随本仓库发版号
 *
 * 构建链没变：两个注入器仍然读各自目录下的副本，不需要认识 shared/。
 */
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));

export const MODULES = ['assistant_dashboard.js', 'desktop_assistant.js', 'local_atlas_controller.js'];

export const TARGETS = {
  mac: {
    dir: 'macos/fullatlas-source',
    brand: 'WUWA TAPPO FULLATLAS / MAC',
    platform: 'macOS',
    // 跟随 package.json 的发版号，也就是唯一源里写的那个值，这里不覆盖。
    version: null,
  },
  win: {
    dir: 'windows/src/src',
    brand: 'WUWA TAPPO / 后台助手',
    platform: 'Windows',
    // 上游 FullAtlas 的 MOD 版本号：Windows 侧 VERSION 用于 /update/healthy，
    // 语义和 Mac 的发版号不同，历史上一直是 1.2，保持不动。
    version: '1.2',
  },
};

/** 版本号常量只在这一个模块里，别处没有 `const VERSION=`。 */
export const VERSION_MODULE = 'desktop_assistant.js';

/** 把唯一源渲染成某个平台的副本。 */
export function render(name, source, target) {
  let text = source
    .replaceAll('__WUWA_BRAND__', target.brand)
    .replaceAll('__WUWA_PLATFORM__', target.platform);
  if (target.version && name === VERSION_MODULE) {
    const pattern = /const VERSION='[^']*'/;
    if (!pattern.test(text)) throw new Error(`${name} 里找不到 const VERSION='...'，无法覆盖版本号`);
    text = text.replace(pattern, `const VERSION='${target.version}'`);
  }
  // 只查这两个占位符：代码里还有 __WUWA_OPEN_PANEL_TAB__ 这类真实全局名，不能用宽泛前缀匹配。
  for (const placeholder of ['__WUWA_BRAND__', '__WUWA_PLATFORM__']) {
    if (text.includes(placeholder)) throw new Error(`${name} 渲染后仍有未替换的占位符 ${placeholder}`);
  }
  return text;
}

/**
 * 生成两侧副本。check=true 只比对不落盘，用于测试与 CI。
 * 返回 { changed, drifted }，drifted 只在 check 模式下非空。
 */
export async function syncFrontend({ projectRoot = root, check = false } = {}) {
  const changed = [];
  const drifted = [];
  for (const name of MODULES) {
    const source = await readFile(path.join(projectRoot, 'shared/frontend', name), 'utf8');
    for (const target of Object.values(TARGETS)) {
      const expected = render(name, source, target);
      const relative = `${target.dir}/${name}`;
      const file = path.join(projectRoot, target.dir, name);
      let actual = null;
      try {
        actual = await readFile(file, 'utf8');
      } catch {
        // 副本不存在按漂移处理，check 模式会报出来。
      }
      if (actual === expected) continue;
      if (check) drifted.push(relative);
      else {
        await writeFile(file, expected);
        changed.push(relative);
      }
    }
  }
  if (check && drifted.length) {
    throw new Error(
      `${drifted.length} 个前端副本与唯一源不一致（改功能请只改 shared/frontend/，然后跑 npm run sync:frontend）：\n  ${drifted.join('\n  ')}`,
    );
  }
  return { changed, drifted };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const check = process.argv.includes('--check');
  syncFrontend({ check })
    .then(result => {
      if (check) console.log(`前端副本与唯一源一致（${MODULES.length} 个模块 × ${Object.keys(TARGETS).length} 个平台）`);
      else if (!result.changed.length) console.log('前端副本已是最新，无需改写');
      else console.log(`已生成 ${result.changed.length} 个副本：\n  ${result.changed.join('\n  ')}`);
    })
    .catch(error => {
      console.error(error.message);
      process.exitCode = 1;
    });
}
