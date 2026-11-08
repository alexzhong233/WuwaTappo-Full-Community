<#
  官方更新跟进流水线（默认只准备基底，不动 app\）

  流程：
    1. 查官方清单，拿到最新版本
    2. 调用 fetch-official.ps1 取基底（7-Zip 解包，绝不执行官方安装器）
    3. 在该基底上试跑注入（解包 + build_integration），只生成 staging，不碰 app\
    4. 输出结论文本：锚点是否全命中

  用法：
    powershell -ExecutionPolicy Bypass -File tools\update-watch.ps1              # 查版本 + 取基底 + 试注入
    powershell -ExecutionPolicy Bypass -File tools\update-watch.ps1 -SkipBuild   # 只查版本
    powershell -ExecutionPolicy Bypass -File tools\update-watch.ps1 -Installer D:\x\setup.exe

  注意：注入成功 ≠ 功能正常。staging 通过后仍需 tools\build.ps1 -Base <新基底> 全量重建并冒烟，
        再手工替换 app\（旧目录改名留作回滚）。
#>
param([switch]$SkipBuild, [string]$Installer, [switch]$Force)

$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
$Py = Join-Path $Root '.venv\Scripts\python.exe'
$AssetsHost = 'https://mc-wuwatappo.kurogames-global.com'

function Get-Json([string]$url) {
  # 兼容 Windows PowerShell 5.1 与 PowerShell 7，直连不走系统代理
  try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }
  $wc = New-Object System.Net.WebClient
  $wc.Proxy = $null
  $wc.Encoding = [Text.Encoding]::UTF8
  $wc.Headers.Add('User-Agent', 'WuwaTappo-FullAtlas-UpdateCheck/1.0')
  try {
    return ($wc.DownloadString($url) | ConvertFrom-Json)
  } finally { $wc.Dispose() }
}

function Get-LocalBaseVersion {
  $cfg = Join-Path $Root 'build\local_config.json'
  if (Test-Path -LiteralPath $cfg) { return [string](Get-Content -LiteralPath $cfg -Raw -Encoding UTF8 | ConvertFrom-Json).client_version }
  return ''
}

"== 1/4 查询官方版本 =="
$hostJson = Get-Json "$AssetsHost/app/json-host/latest_host.json"
$manifest = Get-Json ([string]$hostJson.url)
$remoteVer = [string]$manifest.version
$localVer = Get-LocalBaseVersion
"官方: $remoteVer   本地基底: $localVer"

if (-not $Force) {
  try {
    if ([version]($remoteVer -replace '[^0-9.].*$', '') -le [version]($localVer -replace '[^0-9.].*$', '')) {
      "官方版本不高于本地基底，无需处理（要强制继续加 -Force）。"
      return
    }
  } catch { "版本号无法比较，继续执行。" }
}
if ($SkipBuild) { return }

"== 2/4 获取新版基底（7-Zip 解包，不执行安装器）=="
$fetch = Join-Path $PSScriptRoot 'fetch-official.ps1'
$fetchArgs = @('-Version', $remoteVer)
if ($Installer) { $fetchArgs = @('-Installer', $Installer) }
if ($Force) { $fetchArgs += '-Force' }
& powershell -NoProfile -ExecutionPolicy Bypass -File $fetch @fetchArgs
if ($LASTEXITCODE -ne 0) { throw '获取基底失败' }

$currentFile = Join-Path $Root 'build\official\current.json'
if (-not (Test-Path -LiteralPath $currentFile)) { throw "fetch-official.ps1 未产出 $currentFile" }
$current = Get-Content -LiteralPath $currentFile -Raw -Encoding UTF8 | ConvertFrom-Json
$newBase = [string]$current.base
$officialExe = [string]$current.official
if (-not (Test-Path -LiteralPath $officialExe)) { throw "基底主程序不存在: $officialExe" }
"基底: $newBase"

"== 3/4 试跑注入（只生成 staging，不动 app\）=="
$workDir = Join-Path $Root "build\updates\$remoteVer"
New-Item -ItemType Directory -Path $workDir -Force | Out-Null
$stagingRoot = Join-Path $workDir 'staging'
if (Test-Path -LiteralPath $stagingRoot) { Remove-Item -LiteralPath $stagingRoot -Recurse -Force }
New-Item -ItemType Directory -Path $stagingRoot -Force | Out-Null
$env:WUWA_EXTRACT_ROOT = $newBase
& $Py (Join-Path $Root 'src\tools\extract_static.py') (Join-Path $newBase 'WuwaTappo.official.exe')
if ($LASTEXITCODE -ne 0) { throw '解包失败（extract_static.py）' }
# 与 build.py 相同：解包后必须跑 inspect_js.cjs 生成 js_structure.json
if (-not (Test-Path -LiteralPath (Join-Path $newBase 'js_structure.json'))) {
  $shared = Get-ChildItem -LiteralPath (Join-Path $newBase 'readable_js\assets') -Filter *.js |
    Where-Object { (Get-Content -LiteralPath $_.FullName -Raw -Encoding UTF8) -match 'function applyCatalog\(' } |
    Select-Object -First 1
  if (-not $shared) { throw 'readable_js 里找不到 applyCatalog 所在共享模块' }
  & node (Join-Path $Root '..\shared\tools\inspect_js.cjs') $newBase $shared.Name
  if ($LASTEXITCODE -ne 0) { throw 'js_structure.json 生成失败（inspect_js.cjs）' }
}

$env:WUWA_BUILD_ROOT = $stagingRoot
# build_integration.py 约定构建根目录内必须有 current_base.txt 与 local_config.json
# 无 BOM、无尾换行地写，Python 端按 utf-8 直接读，带 BOM 会污染路径/JSON
[System.IO.File]::WriteAllText((Join-Path $stagingRoot 'current_base.txt'), $newBase, (New-Object System.Text.UTF8Encoding($false)))
$cfgSrc = Join-Path $Root 'build\local_config.json'
if (Test-Path -LiteralPath $cfgSrc) {
  $cfg = Get-Content -LiteralPath $cfgSrc -Raw -Encoding UTF8 | ConvertFrom-Json
  $cfg.client_version = $remoteVer
  [System.IO.File]::WriteAllText((Join-Path $stagingRoot 'local_config.json'), ($cfg | ConvertTo-Json), (New-Object System.Text.UTF8Encoding($false)))
} else {
  throw "缺少 $cfgSrc —— 请先在旧基底上跑过一次 tools\build.ps1"
}
& $Py (Join-Path $Root 'src\tools\build_integration.py')
$code = $LASTEXITCODE

""
if ($code -ne 0) {
  "注入失败（退出码 $code）。大概率是锚点漂移，日志里会指出具体哪一处。"
  "app\ 未被改动，可继续用旧版。把上面的报错发我，通常几十行代码就能补上。"
  exit $code
}

"注入成功，staging 产物在: $stagingRoot"
@{
  version = $remoteVer
  built_at = (Get-Date).ToString('s')
  base = $newBase
  staging = $stagingRoot
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $workDir 'stage_result.json') -Encoding UTF8

"下一步（确认要升级时）："
"  1) powershell -File tools\build.ps1 -Base `"$newBase`""
"  2) 冒烟：双击 app\WuwaTappoLauncher.exe，看设置里图鉴是否正常"
"  3) 回滚：把 app\ 改名为 app.old，旧目录随时可用"



