const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');

if (process.platform !== 'win32') {
  console.log('skip: initialize.ps1 integration test only runs on Windows');
  process.exit(0);
}

const repoRoot = resolve(__dirname, '..', '..', '..');
const source = join(repoRoot, 'windows', 'src', 'installer', 'initialize.ps1');
const dir = mkdtempSync(join(tmpdir(), 'wuwa-init-test-'));

try {
  mkdirSync(join(dir, 'cache'), { recursive: true });
  writeFileSync(join(dir, 'initialize.ps1'), readFileSync(source));
  writeFileSync(
    join(dir, 'package_config.json'),
    JSON.stringify({ mod_version: '1.1', client_version: '1.0.2', asset_count: 899, atlas_count: 899, port: 43229 }),
  );
  writeFileSync(
    join(dir, 'local_config.json'),
    JSON.stringify({ token: 'test-only', mod_version: '1.0', asset_count: 2, atlas_count: 2 }),
  );
  writeFileSync(join(dir, 'cache', 'asset_manifest.json'), JSON.stringify({ errors: [], records: [1, 2, 3] }));
  writeFileSync(join(dir, 'cache', 'atlas.json'), JSON.stringify({ data: [1, 2] }));

  const env = { ...process.env };
  delete env.PSModulePath;
  const powershell = join(
    process.env.SystemRoot || 'C:\\Windows',
    'System32',
    'WindowsPowerShell',
    'v1.0',
    'powershell.exe',
  );
  execFileSync(powershell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(dir, 'initialize.ps1')], {
    env,
    stdio: 'pipe',
  });

  const config = JSON.parse(readFileSync(join(dir, 'local_config.json'), 'utf8').replace(/^\uFEFF/, ''));
  assert.equal(config.token, 'test-only');
  assert.equal(config.mod_version, '1.1');
  assert.equal(config.asset_count, 3);
  assert.equal(config.atlas_count, 2);
  console.log('ok: initialize.ps1 preserves token and syncs catalog counts');
} finally {
  rmSync(dir, { recursive: true, force: true });
}
