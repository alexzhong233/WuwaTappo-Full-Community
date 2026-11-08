import assert from 'node:assert/strict';
import test from 'node:test';
import { patchBackgroundLoading } from '../scripts/patch-frontend.mjs';

const GATE = 't.loading&&useLoading().showLoading()';

/** 执行补丁后那一小段判断，观察它最终是否调用 showLoading。 */
function showsOverlay(patched, config) {
  const start = patched.indexOf('t.loading');
  const end = patched.lastIndexOf('showLoading()');
  assert.ok(start >= 0 && end > start, '补丁后应保留相同的遮罩入口');
  const snippet = patched.slice(start, end + 'showLoading()'.length);
  const calls = [];
  const useLoading = () => ({ showLoading: () => calls.push(1) });
  // eslint-disable-next-line no-new-func
  new Function('t', 'useLoading', snippet)(config, useLoading);
  return calls.length === 1;
}

test('活动配置的后台刷新不再挂全屏遮罩', () => {
  const patched = patchBackgroundLoading(`a${GATE}b`);
  assert.equal(showsOverlay(patched, { loading: true, url: 'api/common/activity-config' }), false);
  assert.equal(showsOverlay(patched, { loading: true, url: '/api/common/activity-config?x=1' }), false);
});

test('图鉴首屏与其它带 loading 的请求照旧显示遮罩', () => {
  const patched = patchBackgroundLoading(`a${GATE}b`);
  assert.equal(showsOverlay(patched, { loading: true, url: 'api/common/atlas' }), true);
  assert.equal(showsOverlay(patched, { loading: true, url: '/api/frontend/user/me' }), true);
  assert.equal(showsOverlay(patched, { loading: true, url: '' }), true);
  assert.equal(showsOverlay(patched, { url: 'api/common/activity-config' }), false);
});

test('入口改动后构建直接失败，而不是静默放过', () => {
  assert.throws(() => patchBackgroundLoading('console.log(1)'), /reconcile the background loading patch/);
});
