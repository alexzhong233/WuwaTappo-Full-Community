import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const injectCore = await readFile(new URL('../../shared/tools/inject_core.py', import.meta.url), 'utf8');
const guardLine = injectCore.match(/if \(t && typeof t\.url === "string" && t\.url\.indexOf\("api\/common\/activity-config"\) !== -1\) t\.loading = !1;/)?.[0];
assert.ok(guardLine, 'shared/tools/inject_core.py 必须包含共用的 activity-config 遮罩抑制规则');

function showsOverlay(config) {
  const calls = [];
  const useLoading = () => ({ showLoading: () => calls.push(1) });
  // eslint-disable-next-line no-new-func
  new Function('t', 'useLoading', `${guardLine}\nif (t.loading) useLoading().showLoading();`)(config, useLoading);
  return calls.length === 1;
}

test('活动配置的后台刷新不再挂全屏遮罩（双端共用拦截器）', () => {
  assert.equal(showsOverlay({ loading: true, url: 'api/common/activity-config' }), false);
  assert.equal(showsOverlay({ loading: true, url: '/api/common/activity-config?x=1' }), false);
});

test('图鉴首屏与其它带 loading 的请求照旧显示遮罩', () => {
  assert.equal(showsOverlay({ loading: true, url: 'api/common/atlas' }), true);
  assert.equal(showsOverlay({ loading: true, url: '/api/frontend/user/me' }), true);
  assert.equal(showsOverlay({ loading: true, url: '' }), true);
  assert.equal(showsOverlay({ url: 'api/common/activity-config' }), false);
});
