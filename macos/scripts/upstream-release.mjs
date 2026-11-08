import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// 官方基线跟新的最后两公里：全绿就自动发预发布，任何一环红了就开草稿 PR 等人改适配。
// 之所以要判"红了"和"绿了"两条路，是因为这里的目标是「版本号不设限」——
// 官方发版就该自动编译，编不过才需要人介入，而不是反过来等人先批准。

const root = fileURLToPath(new URL('../', import.meta.url));

const stageLabels = {
  lock: '拉取官方资源并更新基线锁',
  'build-macos': 'macOS 编译与打包',
  'build-windows': 'Windows 编译与打包',
  publish: '合并基线、打 tag 与发布',
};

export function releaseAssetNames(version) {
  // 与 release.yml 的 expected 保持同一份清单：两服 × 两平台，格式各自唯一。
  return [
    `WuwaTappo-Full-CN-Community-${version}-macOS-arm64.dmg`,
    `WuwaTappo-Full-Global-Community-${version}-macOS-arm64.dmg`,
    `WuwaTappo-Full-CN-Community-Windows-x64-${version}.exe`,
    `WuwaTappo-Full-Global-Community-Windows-x64-${version}.exe`,
  ];
}

export function releaseNotes({ version, cn, global, winCn, winGlobal }) {
  return `# WuwaTappo Full Community ${version}

官方基线自动跟新版：国服 ${cn} / 国际服 ${global}。

- macOS：Apple Silicon / macOS 13 及以上，下载 \`.dmg\` 拖入「应用程序」
- Windows：下载 \`.exe\` 安装后双击快捷方式即可使用
- Windows 适配基底：国服 ${winCn} / 国际服 ${winGlobal}

本版由同步工作流在官方发布新版本后自动编译产出：注入器的结构断言与两平台测试全部通过才会走到这里。
`;
}

export function failureOutline({ version, outcomes, runUrl, branch }) {
  const rows = Object.entries(stageLabels)
    .map(([stage, label]) => `- ${label}（${stage}）：${outcomes[stage] || 'skipped'}`)
    .join('\n');
  return {
    title: `官方基线 ${version} 自动编译失败，需要改适配`,
    body: [
      `官方客户端更新到 ${version} 后自动编译没有通过。版本号本身不再是准入条件（白名单已经去掉），所以这里失败通常意味着注入锚点或前端结构真的变了。`,
      '各阶段结果：',
      rows,
      `分支 \`${branch}\` 已保留，里面是更新后的基线锁、版本号与失败现场。在这个分支上改适配，改完重跑即可。`,
      `日志：[Actions](${runUrl})`,
    ].join('\n\n'),
  };
}

function run(command, args, options = {}) {
  return execFileSync(command, args, { encoding: 'utf8', ...options });
}

function remoteBranchExists(branch) {
  const heads = run('git', ['ls-remote', '--heads', 'origin', branch]);
  return Boolean(heads.trim());
}

function releaseExists(tag, projectRoot, execute) {
  try {
    execute('gh', ['release', 'view', tag], { cwd: projectRoot, stdio: 'ignore' });
    return true;
  } catch {
    // gh 在 Release 不存在时以非零退出，不是异常状态。
    return false;
  }
}

export function publishUpstream({ version, branch, projectRoot = root, environment = process.env, execute = run }) {
  const names = releaseAssetNames(version);
  const missing = names.filter(name => !existsSync(path.join(projectRoot, 'dist', name)));
  if (missing.length) throw new Error(`缺少产物，不能发布：${missing.join(', ')}`);

  // 分支是在构建前从 main 切出来的。构建要跑一个多小时，期间 main 可能又前进了——
  // 这时候如果 rebase 一把再发，tag 指的源码就和刚刚编出来的产物不是同一份了。
  // 宁可这次不发，让失败路径去提示重跑。
  execute('git', ['fetch', 'origin', 'main'], { cwd: projectRoot });
  try {
    execute('git', ['merge-base', '--is-ancestor', 'origin/main', 'HEAD'], { cwd: projectRoot });
  } catch {
    throw new Error('构建期间 main 已经前进，产物与最新主干不再对应：重跑一次同步工作流即可');
  }
  execute('git', ['push', 'origin', 'HEAD:main'], { cwd: projectRoot, stdio: 'inherit' });

  const tag = `v${version}`;
  execute('git', ['tag', '-f', '-a', tag, '-m', `WuwaTappo Full Community ${version}`], { cwd: projectRoot });
  // 用 GITHUB_TOKEN 推的 tag 不会反过来触发 release.yml（GitHub 的既定行为），
  // 所以这里自己发，避免同一份产物被构建两次。
  execute('git', ['push', '-f', 'origin', `refs/tags/${tag}`], { cwd: projectRoot, stdio: 'inherit' });

  const cn = JSON.parse(readFileSync(path.join(projectRoot, 'macos/upstream-cn.json'), 'utf8')).version;
  const global = JSON.parse(readFileSync(path.join(projectRoot, 'macos/upstream-global.json'), 'utf8')).version;
  const winCn = JSON.parse(readFileSync(path.join(projectRoot, 'windows/upstream-win.json'), 'utf8')).client_version;
  const winGlobal = JSON.parse(readFileSync(path.join(projectRoot, 'windows/upstream-win-global.json'), 'utf8')).client_version;
  const notesFile = path.join(environment.RUNNER_TEMP || '/tmp', 'upstream-release-notes.md');
  writeFileSync(notesFile, releaseNotes({ version, cn, global, winCn, winGlobal }));

  const files = names.map(name => path.join('dist', name));
  const common = ['--title', `WuwaTappo Full Community ${tag}`, '--notes-file', notesFile];
  if (releaseExists(tag, projectRoot, execute)) {
    execute('gh', ['release', 'upload', tag, ...files, '--clobber'], { cwd: projectRoot, stdio: 'inherit' });
    execute('gh', ['release', 'edit', tag, ...common, '--draft=false', '--prerelease', '--latest=false'], { cwd: projectRoot, stdio: 'inherit' });
  } else {
    execute('gh', ['release', 'create', tag, ...files, '--verify-tag', ...common, '--prerelease', '--latest=false'], { cwd: projectRoot, stdio: 'inherit' });
  }
  return { tag, version, assets: names };
}

function openBaselinePr(branch, projectRoot, execute) {
  const raw = execute('gh', ['pr', 'list', '--head', branch, '--state', 'open', '--json', 'number'], { cwd: projectRoot });
  return JSON.parse(raw || '[]')[0]?.number ?? null;
}

function openIssue(title, projectRoot, execute) {
  const raw = execute('gh', ['issue', 'list', '--state', 'open', '--search', `${title} in:title`, '--limit', '100', '--json', 'number,title'], { cwd: projectRoot });
  return JSON.parse(raw || '[]').find(issue => issue.title === title)?.number ?? null;
}

export function reportUpstreamFailure({ version, branch, outcomes, projectRoot = root, environment = process.env, execute = run }) {
  const runUrl = `https://github.com/${environment.GITHUB_REPOSITORY}/actions/runs/${environment.GITHUB_RUN_ID}`;
  const { title, body } = failureOutline({ version, outcomes, runUrl, branch });
  // 失败会连续复发（同一个锚点没修好之前每天都跑），所以这里必须幂等：
  // 已经有同名草稿 PR / 问题单就追加一条运行记录，不再开新的。
  if (remoteBranchExists(branch)) {
    const existing = openBaselinePr(branch, projectRoot, execute);
    if (existing) {
      execute('gh', ['pr', 'comment', String(existing), '--body', `再次失败：${runUrl}`], { cwd: projectRoot, stdio: 'inherit' });
      return { reported: 'pull-request-comment', number: existing };
    }
    execute('gh', ['pr', 'create', '--base', 'main', '--head', branch, '--draft', '--title', title, '--body', body], { cwd: projectRoot, stdio: 'inherit' });
    return { reported: 'pull-request', title };
  }
  // 分支都没推上去（比如第一步就挂了），那就退回问题单，至少别让它静默。
  const existing = openIssue(title, projectRoot, execute);
  if (existing) {
    execute('gh', ['issue', 'comment', String(existing), '--body', `再次失败：${runUrl}`], { cwd: projectRoot, stdio: 'inherit' });
    return { reported: 'issue-comment', number: existing };
  }
  execute('gh', ['issue', 'create', '--title', title, '--body', body], { cwd: projectRoot, stdio: 'inherit' });
  return { reported: 'issue', title };
}

async function main(action) {
  const version = process.env.RELEASE_VERSION || '';
  const branch = process.env.BASELINE_BRANCH || '';
  const outcomes = JSON.parse(process.env.STAGE_OUTCOMES || '{}');
  if (action === 'publish') return publishUpstream({ version, branch });
  if (action === 'report') return reportUpstreamFailure({ version, branch, outcomes });
  throw new Error(`未知动作：${action}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv[2])
    .then(result => console.log(JSON.stringify(result)))
    .catch(error => { console.error(error.message); process.exitCode = 1; });
}
