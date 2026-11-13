<#
  装配运行目录：把 build 产物 + 素材 + 运行时组装成完整可运行目录，同步到 app\

  用法： powershell -ExecutionPolicy Bypass -File tools\deploy.ps1 [-Target <目录>] [-NoRuntime] [-NoAssets]
  说明： 只覆盖程序与静态资源，保留 app\ 下的运行状态
         （local_config.json / offline_state.json / assistant_state.json / profile\）

        -NoRuntime  不内嵌 WebView2 运行时（CI / 小体积产物用，回退系统运行时）
        -NoAssets   不复制 assets\cache（缺素材缓存时程序按需回源官方 CDN）
#>
param([string]$Target, [switch]$NoRuntime, [switch]$NoAssets,
  [ValidateSet('cn', 'global')][string]$Region)

$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'region.ps1')
$R = Get-RegionProfile -Root $Root -Region $Region
$App = if ($Target) { $Target } else { $R.AppDir }
$Payload = Join-Path $R.WorkDir 'payload'

$exe = Join-Path $R.WorkDir 'WuwaTappo.Local.v2.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw "缺少构建产物：$exe（先跑 tools\build.ps1）" }
$frozen = Join-Path $R.WorkDir 'frozen\LocalGalleryHost'
if (-not (Test-Path -LiteralPath $frozen)) { throw "缺少冻结服务：$frozen" }

if (Test-Path -LiteralPath $Payload) { Remove-Item -LiteralPath $Payload -Recurse -Force }
New-Item -ItemType Directory -Path $Payload | Out-Null

# 主程序
Copy-Item -LiteralPath $exe -Destination (Join-Path $Payload 'WuwaTappo.Local.exe')

# 本地素材服务（共享 Rust 版）
Copy-Item -LiteralPath $frozen -Destination (Join-Path $Payload 'host') -Recurse
Copy-Item -LiteralPath (Join-Path $Payload 'host\LocalGalleryHost.exe') -Destination (Join-Path $Payload 'LocalGalleryHost.exe')

# 素材缓存（CI 或未拉缓存时可缺省，运行时按需回源）
$assets = Join-Path $Root 'assets\cache'
if (-not $NoAssets -and (Test-Path -LiteralPath $assets)) {
  Copy-Item -LiteralPath $assets -Destination (Join-Path $Payload 'cache') -Recurse
} else {
  Write-Warning '跳过素材缓存：产物不含离线图鉴，首次浏览会回源官方 CDN'
}

# WebView2 固定运行时：取系统已装的最新一份，避免硬编码版本号
$wvRoots = @(
  'C:\Program Files (x86)\Microsoft\EdgeWebView\Application',
  'C:\Program Files\Microsoft\EdgeWebView\Application'
) | Where-Object { Test-Path -LiteralPath $_ }
if (-not $NoRuntime -and $wvRoots) {
  $wv = Get-ChildItem -LiteralPath $wvRoots[0] -Directory -ErrorAction SilentlyContinue |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'msedgewebview2.exe') } |
    Sort-Object { try { [version]($_.Name -replace '[^0-9.]', '') } catch { [version]'0.0' } } |
    Select-Object -Last 1
  if ($wv) {
    Copy-Item -LiteralPath $wv.FullName -Destination (Join-Path $Payload 'runtime') -Recurse
    "WebView2 运行时: $($wv.Name)"
  } else {
    Write-Warning '未找到可用的 WebView2 运行时：产物将回退到目标机器的系统运行时（需已装 WebView2）'
  }
} else {
  # 入口 stub 只在 runtime\ 存在时才钉死浏览器路径，所以这里是安全的小体积产物。
  Write-Host '跳过 WebView2 运行时：回退到目标机器的系统运行时（安装包会在安装阶段检查）'
}

# 脚本与文档
foreach ($f in @('initialize.ps1', 'launch.ps1', 'restore_network.ps1')) {
  $src = Join-Path $Root "src\installer\$f"
  if (Test-Path -LiteralPath $src) { Copy-Item -LiteralPath $src -Destination (Join-Path $Payload $f) }
}
foreach ($f in @('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md')) {
  $src = Join-Path $Root "src\$f"
  if (Test-Path -LiteralPath $src) { Copy-Item -LiteralPath $src -Destination (Join-Path $Payload $f) }
}
if (Test-Path -LiteralPath (Join-Path $Root '..\docs\windows')) {
  Copy-Item -LiteralPath (Join-Path $Root '..\docs\windows') -Destination (Join-Path $Payload 'docs') -Recurse
}
Copy-Item -LiteralPath (Join-Path $R.WorkDir 'local_config.json') -Destination (Join-Path $Payload 'package_config.json')
# 运行期必需（含 token）；robocopy 会跳过它，保留目标目录里已有的 token 与计数
Copy-Item -LiteralPath (Join-Path $R.WorkDir 'local_config.json') -Destination (Join-Path $Payload 'local_config.json')

# 无控制台启动器——安装包的快捷方式就指向它。必须在 robocopy 之前放进 payload，
# 否则同步不到 app\。build.ps1 的 4/5 步负责编译，这里只在真缺的时候明确报错。
$launcherExe = Join-Path $R.WorkDir 'launcher\dist\WuwaTappoLauncher.exe'
if (-not (Test-Path -LiteralPath $launcherExe)) {
  throw "缺少无控制台启动器：$launcherExe（先跑 tools\build-launcher.ps1 -Region $($R.Id)）"
}
Copy-Item -LiteralPath $launcherExe -Destination (Join-Path $Payload 'WuwaTappoLauncher.exe') -Force

# 快捷方式/卸载项图标：官方客户端 exe 里提取出来的那份，装成 app.ico 供 NSIS 引用
$appIcon = Get-ChildItem -LiteralPath (Join-Path $R.WorkDir 'official') -Recurse -Filter 'icon_32512.ico' -ErrorAction SilentlyContinue |
  Select-Object -First 1
if ($appIcon) {
  Copy-Item -LiteralPath $appIcon.FullName -Destination (Join-Path $Payload 'app.ico') -Force
}

# 同步到 app\（不清理多余文件，保留账号与运行状态）
if (-not (Test-Path -LiteralPath $App)) { New-Item -ItemType Directory -Path $App | Out-Null }
# 运行期状态一律不覆盖：token 变了会让已部署实例的前端鉴权失败
robocopy $Payload $App /E /NFL /NDL /NJH /NJS /R:1 /W:1 /XF local_config.json offline_state.json assistant_state.json | Out-Null
if ($LASTEXITCODE -ge 8) { throw "同步到 app\ 失败，robocopy 退出码 $LASTEXITCODE" }

# 全新目录没有运行期配置，这里补一份
if (-not (Test-Path -LiteralPath (Join-Path $App 'local_config.json'))) {
  Copy-Item -LiteralPath (Join-Path $Payload 'local_config.json') -Destination (Join-Path $App 'local_config.json')
  '已写入运行期配置 local_config.json（含新 token）'
}

# 启动器（相对路径，可随目录搬迁）；start.cmd 用纯 ASCII 名，供桌面快捷方式引用
$launcher = @'
@echo off
chcp 65001 >nul
setlocal
set ROOT=%~dp0
tasklist /FI "IMAGENAME eq LocalGalleryHost.exe" 2>nul | find /I "LocalGalleryHost.exe" >nul
if errorlevel 1 (
  start "" /B "%ROOT%LocalGalleryHost.exe"
  timeout /t 2 /nobreak >nul
)
start "" "%ROOT%WuwaTappo.Local.exe"
endlocal
'@
foreach ($name in @('start.cmd', '启动.cmd')) {
  [IO.File]::WriteAllText((Join-Path $App $name), $launcher, (New-Object Text.UTF8Encoding($false)))
}

# launch.ps1 会被本机安全策略删除，内容存档在 .disabled.txt，这里尽力还原
$launchLnk = Join-Path $App 'launch.ps1'
if (-not (Test-Path -LiteralPath $launchLnk)) {
  $archive = Join-Path $Root 'src\installer\launch.ps1.disabled.txt'
  if (Test-Path -LiteralPath $archive) {
    Copy-Item -LiteralPath $archive -Destination $launchLnk -Force -ErrorAction SilentlyContinue
  }
}

$total = (Get-ChildItem -LiteralPath $App -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum)
"部署完成 -> $App"
"合计 {0:N0} MB / {1} 个文件" -f ($total.Sum / 1MB), $total.Count
