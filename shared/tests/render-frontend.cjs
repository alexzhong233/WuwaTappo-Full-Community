const fs = require('node:fs');
const path = require('node:path');

const REPO_ROOT = path.resolve(__dirname, '..', '..');
const SETTINGS_CONTRACT = JSON.parse(
  fs.readFileSync(path.join(REPO_ROOT, 'shared/contracts/assistant-settings.json'), 'utf8'),
);

const TARGETS = {
  mac: {
    brand: 'WUWA TAPPO FULLATLAS / MAC',
    platform: 'macOS',
  },
  win: {
    brand: 'WUWA TAPPO / 后台助手',
    platform: 'Windows',
  },
};

function renderFrontend(name, platform = 'mac') {
  const target = TARGETS[platform];
  if (!target) throw new Error(`Unknown frontend platform target: ${platform}`);
  let text = fs.readFileSync(path.join(REPO_ROOT, 'shared/frontend', name), 'utf8')
    .replaceAll('__WUWA_BRAND__', target.brand)
    .replaceAll('__WUWA_PLATFORM__', target.platform);
  if (name === 'desktop_assistant.js') {
    text = text.replaceAll('__WUWA_ASSISTANT_DEFAULTS__', JSON.stringify(SETTINGS_CONTRACT.defaults));
  }
  for (const placeholder of ['__WUWA_BRAND__', '__WUWA_PLATFORM__', '__WUWA_ASSISTANT_DEFAULTS__']) {
    if (text.includes(placeholder)) {
      throw new Error(`${name} still contains unresolved placeholder ${placeholder}`);
    }
  }
  return text;
}

module.exports = { TARGETS, renderFrontend };
