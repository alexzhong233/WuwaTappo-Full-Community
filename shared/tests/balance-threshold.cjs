const assert = require('node:assert/strict');
const vm = require('node:vm');
const { renderFrontend } = require('./render-frontend.cjs');

for (const platform of ['mac', 'win']) {
  const source = renderFrontend('desktop_assistant.js', platform);
  const resolve = vm.runInNewContext(source + ';resolveBalanceAutoClick');
  const settings = { auto_click_balance: true, auto_click_below: 10000, auto_click_above: 40000 };
  assert.equal(resolve(settings, 9999), true, 'strictly below the lower bound starts clicking');
  assert.equal(resolve(settings, 10000), null, 'the lower bound preserves current state');
  assert.equal(resolve(settings, 10001), null, 'the middle band preserves current state');
  assert.equal(resolve(settings, 40000), null, 'the upper bound preserves current state');
  assert.equal(resolve(settings, 40001), false, 'strictly above the upper bound stops clicking');
  assert.equal(resolve({ ...settings, auto_click_balance: false }, 1), null, 'disabled policy makes no change');
  assert.equal(resolve(settings, NaN), null, 'invalid balance makes no change');
  console.log(`balance threshold contract passed (${platform})`);
}
