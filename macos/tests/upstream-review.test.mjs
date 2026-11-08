import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { exactIssueNumber, inventoryChanges, openUpdatePullRequest, reportFailure, updatePullRequestArguments, writeInventoryReview } from '../scripts/upstream-review.mjs';
import { getRegionProfile } from '../scripts/regions.mjs';

const root = path.resolve(import.meta.dirname, '..');
const repoRoot = path.resolve(root, '..');
const inventory = () => ({
  sourceApplication: { region: 'CN', version: '1.0.1', entryModule: 'old.js' },
  runtimeConstants: { clickCacheEnvironment: 'cn-cn-prod' },
  commands: [{ name: 'load_key_press_count', response: { type: 'number' }, callSites: [{ file: 'old.js', charRange: [1, 2] }] }],
  events: [{ name: 'key-press', payload: { type: 'string' }, callSites: [{ file: 'old.js' }] }],
});

test('candidate summaries separate source churn from named command, payload and event changes', () => {
  const accepted = inventory(), observed = inventory();
  observed.sourceApplication.version = '1.0.2';
  observed.sourceApplication.entryModule = 'new.js';
  observed.commands[0].callSites[0] = { file: 'new.js', charRange: [100, 200] };
  observed.events[0].payload = { type: 'number' };
  observed.events.push({ name: 'gamepad-button-press', payload: { type: 'string' }, callSites: [] });
  const changes = inventoryChanges(accepted, observed);
  assert.equal(changes.acceptedVersion, '1.0.1');
  assert.equal(changes.observedVersion, '1.0.2');
  assert.deepEqual(changes.commands, { added: [], removed: [], changed: [] });
  assert.deepEqual(changes.events, { added: ['gamepad-button-press'], removed: [], changed: ['key-press'] });
  assert.equal(changes.runtimeConstants.changed, false);
  observed.commands = [];
  assert.deepEqual(inventoryChanges(accepted, observed).commands.removed, ['load_key_press_count']);
});

test('candidate inventory is review evidence and never overwrites the accepted fixtures or resource lock', async () => {
  const profile = getRegionProfile();
  const directory = await mkdtemp(path.join(tmpdir(), 'wuwa-upstream-review-'));
  try {
    await mkdir(path.join(directory, 'tests'));
    await mkdir(path.join(directory, profile.frontendDir));
    await symlink(path.join(root, profile.frontendDir, 'assets'), path.join(directory, profile.frontendDir, 'assets'));
    const filename = profile.id === 'cn' ? 'contracts.json' : 'contracts-global.json';
    const accepted = await readFile(path.join(root, 'tests', filename), 'utf8');
    const lock = JSON.stringify({ version: '9.9.9' });
    await writeFile(path.join(directory, 'tests', filename), accepted);
    await writeFile(path.join(directory, profile.upstreamLock), lock);
    await writeInventoryReview(directory, profile);
    const observed = JSON.parse(await readFile(path.join(directory, '.work/upstream-review/observed-inventory.json'), 'utf8'));
    const changes = JSON.parse(await readFile(path.join(directory, '.work/upstream-review/contract-changes.json'), 'utf8'));
    assert.equal(observed.sourceApplication.version, '9.9.9');
    assert.equal(observed.sourceApplication.region, profile.region);
    assert.equal(changes.observedVersion, '9.9.9');
    assert.equal(await readFile(path.join(directory, 'tests', filename), 'utf8'), accepted);
    assert.equal(await readFile(path.join(directory, profile.upstreamLock), 'utf8'), lock);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('an update is ready only when every required compatibility and packaging stage succeeded', () => {
  for (const region of ['cn', 'global']) {
    const profile = getRegionProfile(region);
    const outcomes = Object.fromEntries(['inventory', 'frontend', 'native', 'build', 'package'].map(stage => [stage, 'success']));
    const args = updatePullRequestArguments({ profile, version: '1.0.2', branch: 'codex/candidate', runUrl: 'https://github.com/owner/repo/actions/runs/1', outcomes });
    assert.ok(!args.includes('--draft'));
    assert.ok(args.includes(`同步${profile.label}官方资源 1.0.2`));
    assert.ok(args[args.indexOf('--body') + 1].includes(`official-${region}-review`));
    const mirrorFailed = updatePullRequestArguments({ profile, version: '1.0.2', branch: 'codex/candidate', runUrl: 'run', outcomes: { ...outcomes, mirror: 'failure' } });
    assert.ok(mirrorFailed[mirrorFailed.indexOf('--body') + 1].includes('mirror: failure'));
    for (const stage of Object.keys(outcomes)) for (const result of ['failure', 'skipped', 'cancelled', undefined]) {
      const draft = updatePullRequestArguments({ profile, version: '1.0.2', branch: 'codex/candidate', runUrl: 'run', outcomes: { ...outcomes, [stage]: result } });
      assert.ok(draft.includes('--draft'), `${region} ${stage} ${result}`);
      const body = draft[draft.indexOf('--body') + 1];
      assert.ok(body.includes(`${stage}: ${result || 'skipped'}`));
      assert.ok(body.includes('尚未通过兼容性验证'));
    }
  }
});

test('failure reports select only the exact regional issue title', () => {
  const issues = [{ number: 1, title: '国际服官方同步需要处理' }, { number: 2, title: '国服官方同步需要处理：旧问题' }, { number: 3, title: '国服官方同步需要处理' }];
  assert.equal(exactIssueNumber(issues, '国际服官方同步需要处理'), 1);
  assert.equal(exactIssueNumber(issues, '国服官方同步需要处理'), 3);
  assert.equal(exactIssueNumber(issues.slice(0, 2), '国服官方同步需要处理'), null);
});

test('update PR execution commits its regional lock plus the regenerated contract snapshot', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'wuwa-upstream-pr-'));
  const environment = { GITHUB_REPOSITORY: 'owner/repo', GITHUB_RUN_ID: '123', ...Object.fromEntries(['inventory', 'frontend', 'native', 'build', 'package'].map(stage => [`${stage.toUpperCase()}_OUTCOME`, 'success'])) };
  try {
    await mkdir(path.join(directory, '.work'));
    await writeFile(path.join(directory, '.work/upstream-check.json'), '{"latestVersion":"1.0.2"}');
    for (const region of ['cn', 'global']) for (const changed of [false, true]) for (const frontend of ['success', 'failure']) {
      const calls = [];
      const execute = (command, args, options) => {
        assert.equal(options.cwd, directory);
        calls.push([command, args]);
        return command === 'git' && args[0] === 'diff' ? (changed ? 'resource diff' : '') : '';
      };
      await openUpdatePullRequest({ projectRoot: directory, profile: getRegionProfile(region), environment: { ...environment, FRONTEND_OUTCOME: frontend }, execute });
      if (!changed) {
        assert.deepEqual(calls, [['git', ['diff', '--', `upstream-${region}.json`]]]);
        continue;
      }
      // 这个临时工程里没有官方前端，快照生成器无从下手，只应提交锁文件。
      assert.deepEqual(calls.find(([command, args]) => command === 'git' && args[0] === 'add'), ['git', ['add', '--', `upstream-${region}.json`]]);
      assert.deepEqual(calls.find(([command, args]) => command === 'git' && args[0] === 'push'), ['git', ['push', 'origin', `codex/official-${region}-1.0.2-123`]]);
      assert.equal(calls.at(-1)[0], 'gh');
      assert.equal(calls.at(-1)[1].includes('--draft'), frontend === 'failure');
    }
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('a stale contract snapshot is regenerated and staged without touching the hand-written network contract', async (t) => {
  // 只有跑过 npm run setup 的机器才有官方前端；CI 两个区域都会先跑 setup。
  const profile = getRegionProfile('global');
  const assets = path.join(root, profile.frontendDir, 'assets');
  if (!existsSync(assets)) return t.skip(`缺少 ${profile.frontendDir}，先跑 npm run setup`);
  const filename = 'contracts-global.json';
  const directory = await mkdtemp(path.join(tmpdir(), 'wuwa-upstream-snapshot-'));
  try {
    await mkdir(path.join(directory, '.work'));
    await mkdir(path.join(directory, 'tests'));
    await mkdir(path.join(directory, profile.frontendDir), { recursive: true });
    await symlink(assets, path.join(directory, profile.frontendDir, 'assets'), 'dir');
    await writeFile(path.join(directory, profile.upstreamLock), await readFile(path.join(root, profile.upstreamLock), 'utf8'));
    await writeFile(path.join(directory, '.work/upstream-check.json'), '{"latestVersion":"1.0.4"}');
    const accepted = await readFile(path.join(root, 'tests', filename), 'utf8');
    const stale = accepted.replace(/"version": "[^"]+"/, '"version": "0.0.1"');
    await writeFile(path.join(directory, 'tests', filename), stale);
    const networkContract = await readFile(path.join(root, 'tests/network-contract-global.json'), 'utf8');
    await writeFile(path.join(directory, 'tests/network-contract-global.json'), networkContract);
    const calls = [];
    const execute = (command, args, options) => {
      assert.equal(options.cwd, directory);
      calls.push([command, args]);
      return command === 'git' && args[0] === 'diff' ? 'resource diff' : '';
    };
    await openUpdatePullRequest({ projectRoot: directory, profile, environment: { GITHUB_REPOSITORY: 'owner/repo', GITHUB_RUN_ID: '9', FRONTEND_OUTCOME: 'failure' }, execute });
    const add = calls.find(([command, args]) => command === 'git' && args[0] === 'add');
    assert.deepEqual(add, ['git', ['add', '--', 'upstream-global.json', `tests/${filename}`]]);
    const regenerated = await readFile(path.join(directory, 'tests', filename), 'utf8');
    assert.notEqual(regenerated, stale);
    const lock = JSON.parse(await readFile(path.join(directory, profile.upstreamLock), 'utf8'));
    assert.equal(JSON.parse(regenerated).sourceApplication.version, lock.version);
    assert.equal(await readFile(path.join(directory, 'tests/network-contract-global.json'), 'utf8'), networkContract);
    const body = calls.at(-1)[1][calls.at(-1)[1].indexOf('--body') + 1];
    assert.ok(body.includes(`tests/${filename}`));
    assert.ok(body.includes('手写约束'));
    assert.ok(body.includes('旧契约快照'));
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('failure report execution creates CN issue instead of commenting on a fuzzy Global match', () => {
  for (const matching of [false, true]) {
    const calls = [];
    reportFailure({ profile: getRegionProfile('cn'), environment: { GITHUB_REPOSITORY: 'owner/repo', GITHUB_RUN_ID: '123' }, execute: (command, args) => {
      calls.push([command, args]);
      return JSON.stringify([{ number: 1, title: '国际服官方同步需要处理' }, ...(matching ? [{ number: 2, title: '国服官方同步需要处理' }] : [])]);
    } });
    assert.equal(calls.length, 2);
    assert.equal(calls.at(-1)[1][1], matching ? 'comment' : 'create');
    assert.ok(calls.at(-1)[1].includes(matching ? '2' : '国服官方同步需要处理'));
  }
});

// 语义从"验证通过就开 PR 等人合并"改成了"全绿自动发预发布、红了才开草稿 PR"：
// 版本号不再是准入条件，所以编译本身就是兼容性判决。这条测试守住新语义的两个要点——
// 发版必须等三个 stage 全绿，失败必须留得下可评审的现场。
test('sync workflow publishes only when every stage is green and otherwise leaves a reviewable draft', async () => {
  const workflow = await readFile(path.join(repoRoot, '.github/workflows/sync-upstream.yml'), 'utf8');
  assert.match(workflow, /needs\.lock\.result == 'success'/);
  assert.match(workflow, /needs\.build-macos\.result == 'success'/);
  assert.match(workflow, /needs\.build-windows\.result == 'success'/);
  assert.ok(workflow.includes('node macos/scripts/upstream-release.mjs publish'));
  assert.ok(workflow.includes('node macos/scripts/upstream-release.mjs report'));
  assert.match(workflow, /上报需要改适配\n\s+needs:[\s\S]{0,200}?if: failure\(\)/);
  // 失败时也要把分支推上去，否则没有地方可以改适配。
  assert.match(workflow, /Preserve partial baseline work when a step fails\n\s+if: failure\(\)/);
  // 只有镜像上传允许吞错：它影响的是"下次 CDN 抖动时的退路"，不是本次发版的必要条件。
  assert.equal((workflow.match(/continue-on-error: true/g) || []).length, 1);
  assert.ok(workflow.indexOf('continue-on-error: true') > workflow.indexOf('Publish verified installers to the build mirror'));
});
