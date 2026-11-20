<#
  在桌面生成快捷方式：优先指向无控制台启动器 WuwaTappoLauncher.exe，
  没有启动器时回落到 app\start.cmd。

  用法： powershell -ExecutionPolicy Bypass -File tools\make-shortcut.ps1 [-Name "Wuwa Tappo 全图鉴版"]
  说明： 本机安全策略禁止 COM（WScript.Shell），因此由 shared/backend assistant-core 直接写 .lnk 二进制
#>
param([string]$Name = 'Wuwa Tappo 全图鉴版')
$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
$RepoRoot = Split-Path $Root -Parent
$CargoManifest = Join-Path $RepoRoot 'shared\backend\Cargo.toml'
$Cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $Cargo) { throw '找不到 cargo：请安装 Rust stable 工具链并将 cargo 加入 PATH' }
$App = Join-Path $Root 'app'

$launcher = Join-Path $App 'WuwaTappoLauncher.exe'
$cmdEntry = Join-Path $App 'start.cmd'
$out = Join-Path ([Environment]::GetFolderPath('Desktop')) "$Name.lnk"
$icon = Join-Path $App 'WuwaTappo.Local.exe'

if (Test-Path -LiteralPath $launcher) {
  $target = $launcher
  $extra = @('--window', 'normal')
  "入口: 无控制台启动器"
} elseif (Test-Path -LiteralPath $cmdEntry) {
  $target = $cmdEntry
  $extra = @('--window', 'minimized')
  "入口: start.cmd（建议跑 tools\build-launcher.ps1 换成无窗口启动器）"
} else {
  throw "app\ 里既没有 WuwaTappoLauncher.exe 也没有 start.cmd，先跑 tools\deploy.ps1"
}

& $Cargo run --locked --manifest-path $CargoManifest --bin assistant-core -- make-shortcut `
  --out $out `
  --target $target `
  --workdir $App `
  --icon $icon `
  --description 'WuwaTappo 全图鉴版' `
  @extra
if ($LASTEXITCODE -ne 0) { throw "生成快捷方式失败（退出码 $LASTEXITCODE）" }
"快捷方式: $out"
