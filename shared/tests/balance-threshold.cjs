const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const root = path.join(__dirname, '../..');
const sources = [
  path.join(root, 'macos/fullatlas-source/desktop_assistant.js'),
  path.join(root, 'windows/src/src/desktop_assistant.js'),
];

for (const file of sources) {
  const source = fs.readFileSync(file, 'utf8');
  const resolve = vm.runInNewContext(source + ';resolveBalanceAutoClick');
  const settings = { auto_click_balance: true, auto_click_below: 10000, auto_click_above: 40000 };
  assert.equal(resolve(settings, 9999), true, 'strictly below the lower bound starts clicking');
  assert.equal(resolve(settings, 10000), null, 'the lower bound preserves current state');
  assert.equal(resolve(settings, 10001), null, 'the middle band preserves current state');
  assert.equal(resolve(settings, 40000), null, 'the upper bound preserves current state');
  assert.equal(resolve(settings, 40001), false, 'strictly above the upper bound stops clicking');
  assert.equal(resolve({ ...settings, auto_click_balance: false }, 1), null, 'disabled policy makes no change');
  assert.equal(resolve(settings, NaN), null, 'invalid balance makes no change');
  console.log(`balance threshold contract passed (${path.relative(root, file)})`);
}
