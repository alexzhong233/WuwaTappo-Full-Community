<#
  把试注入成功的新版本正式应用到 app\（带回滚，失败自动还原）

  前置：tools\update-watch.ps1 已跑完，产生 build\updates\<版本>\base 与 stage_result.json

  流程：
    1. 确认程序已完全退出
    2. 备份当前主程序到 build\rollback\
    3. build.ps1 -Base <新基底> -NoDeploy（只构建，不碰 app\）
    4. deploy.ps1 -Target build\updates\<版本>\deploy（先装配到中转目录）
    5. 校验中转目录完整后，才 robocopy 覆盖 app\
    6. 更新版本号（build\local_config.json / app\local_config.json）
  任何一步失败：还原备份的主程序，app\ 保持可用

  用法： powershell -ExecutionPolicy Bypass -File tools\apply-update.ps1 -Version 1.0.4
#>
param(
  [Parameter(Mandatory = $true)][string]$Version,
  [string]$BaseDir,
  [switch]$Force
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
$App = Join-Path $Root 'app'
$workDir = Join-Path $Root "build\updates\$Version"
if (-not $BaseDir) { $BaseDir = Join-Path $workDir 'base' }
$backup = Join-Path $Root 'build\rollback'

function Say($m) { Write-Host $m }

# ---------------------------------------------------------------- 前置校验
if (-not (Test-Path -LiteralPath (Join-Path $workDir 'stage_result.json'))) {
  throw "缺少 build\updates\$Version\stage_result.json —— 先跑 tools\update-watch.ps1 完成试注入"
}
if (-not (Test-Path -LiteralPath (Join-Path $BaseDir 'WuwaTappo.official.exe'))) {
  throw "缺少新基底：$BaseDir\WuwaTappo.official.exe"
}
if (-not $Force) {
  $running = @(Get-Process -Name 'WuwaTappo.Local', 'LocalGalleryHost' -ErrorAction SilentlyContinue)
  if ($running.Count -gt 0) {
    throw "程序仍在运行（$($running.Name -join ', ')）。请先完全退出再更新。"
  }
}

# ---------------------------------------------------------------- 备份
Say "== 1/6 备份当前主程序 =="
New-Item -ItemType Directory -Path $backup -Force | Out-Null
$backupFiles = @()
foreach ($f in @('WuwaTappo.Local.exe', 'LocalGalleryHost.exe', 'package_config.json')) {
  $src = Join-Path $App $f
  if (Test-Path -LiteralPath $src) {
    Copy-Item -LiteralPath $src -Destination (Join-Path $backup $f) -Force
    $backupFiles += $f
  }
}
if (-not (Test-Path -LiteralPath (Join-Path $backup 'WuwaTappo.Local.exe'))) {
  throw "备份失败：app\WuwaTappo.Local.exe 不存在，无法保证可回滚，已中止"
}
Say "已备份: $($backupFiles -join ', ')"

function Restore-Backup {
  Say "!! 正在还原旧版主程序 ..."
  foreach ($f in $backupFiles) {
    $b = Join-Path $backup $f
    if (Test-Path -LiteralPath $b) {
      Copy-Item -LiteralPath $b -Destination (Join-Path $App $f) -Force -ErrorAction SilentlyContinue
    }
  }
  Say "!! 已还原。app\ 仍可用旧版本。"
}

try {
  # -------------------------------------------------------------- 版本号先行
  Say "== 2/6 写入新版本号 =="
  $cfgPath = Join-Path $Root 'build\local_config.json'
  $cfg = Get-Content -LiteralPath $cfgPath -Raw | ConvertFrom-Json
  $cfg.client_version = $Version
  $cfg | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $cfgPath -Encoding UTF8
  Say "build\local_config.json -> client_version = $Version"

  # -------------------------------------------------------------- 构建
  Say "== 3/6 重新构建（解包已复用，只重做注入与打包）=="
  & (Join-Path $PSScriptRoot 'build.ps1') -Base $BaseDir -NoDeploy
  if ($LASTEXITCODE -ne 0) { throw "build.ps1 失败（退出码 $LASTEXITCODE）" }

  # -------------------------------------------------------------- 装配到中转
  Say "== 4/6 装配到中转目录 =="
  $stage = Join-Path $workDir 'deploy'
  if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
  & (Join-Path $PSScriptRoot 'deploy.ps1') -Target $stage
  if ($LASTEXITCODE -ne 0) { throw "deploy.ps1 失败（退出码 $LASTEXITCODE）" }

  # -------------------------------------------------------------- 校验中转
  Say "== 5/6 校验中转产物 =="
  $must = @('WuwaTappo.Local.exe', 'LocalGalleryHost.exe', 'cache', 'runtime')
  foreach ($m in $must) {
    if (-not (Test-Path -LiteralPath (Join-Path $stage $m))) { throw "中转产物缺少 $m，已中止，未改动 app\" }
  }
  $exeSize = (Get-Item -LiteralPath (Join-Path $stage 'WuwaTappo.Local.exe')).Length
  if ($exeSize -lt 50MB) { throw "中转主程序仅 {0:N1} MB，明显异常，已中止" -f ($exeSize / 1MB) }
  Say "主程序 {0:N1} MB，产物完整" -f ($exeSize / 1MB)

  # -------------------------------------------------------------- 覆盖 app
  Say "== 6/6 覆盖 app\ =="
  robocopy $stage $App /E /NFL /NDL /NJH /NJS /R:1 /W:1 | Out-Null
  if ($LASTEXITCODE -ge 8) { throw "robocopy 覆盖失败（退出码 $LASTEXITCODE）" }

  # 运行时状态里的版本号同步（token 等其余字段保持不动）
  $appCfg = Join-Path $App 'local_config.json'
  if (Test-Path -LiteralPath $appCfg) {
    try {
      $ac = Get-Content -LiteralPath $appCfg -Raw | ConvertFrom-Json
      $ac.client_version = $Version
      $ac | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $appCfg -Encoding UTF8
    } catch { Say "警告：app\local_config.json 版本号未更新（$($_.Exception.Message)）" }
  }

  Say ""
  Say "更新完成：本地基底现在是 $Version"
  Say "回滚包保留在: $backup （确认新版本正常后可删）"
  Say "回滚方法：把 $backup 下的 WuwaTappo.Local.exe 复制回 app\ 覆盖即可"
  exit 0

} catch {
  $msg = $_.Exception.Message
  Say "失败：$msg"
  Restore-Backup
  exit 1
}
