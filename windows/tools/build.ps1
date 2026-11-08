<#
  全量重建：解包官方 EXE -> 注入全图鉴 -> 冻结本地服务 -> 编译启动器 -> 装配到 app\

  前置：
    1. tools\setup-env.ps1 已执行（.venv 就绪）
    2. tools\fetch-official.ps1 已取好官方基底（按 upstream-win*.json 锁定的最新版本）
    3. assets\cache 已由 prepare_cache.py 拉好

  用法： powershell -ExecutionPolicy Bypass -File tools\build.ps1 [-Extract] [-NoDeploy] [-NoRuntime] [-Base <目录>]
        -Extract   先重新解包官方 EXE（覆盖基底目录里的前端产物）
        -NoDeploy  只构建到 build\，不自动同步到 app\（更新流程用，先装配到中转目录再整体覆盖）
        -NoRuntime 装配时不内嵌 WebView2 运行时（CI / 小体积产物用）
        -Base      指定官方主程序所在基底目录（默认取 build\official\current.json，未取包时退回 build\base-103）
#>
param([switch]$Extract, [switch]$NoDeploy, [switch]$NoRuntime, [string]$Base,
  [ValidateSet('cn', 'global')][string]$Region)
$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'region.ps1')
$R = Get-RegionProfile -Root $Root -Region $Region
$env:WUWA_REGION = $R.Id
$Py = Join-Path $Root '.venv\Scripts\python.exe'
if (-not $Base) {
  # 默认用 fetch-official.ps1 最近一次取到的基底——跟着锁定的官方版本走，
  # 不再写死 base-103（否则换了基线还得靠人记得传 -Base，实际就会一直用旧客户端）。
  $current = Join-Path $R.WorkDir 'official\current.json'
  if (Test-Path -LiteralPath $current) {
    $Base = [string]((Get-Content -LiteralPath $current -Raw -Encoding UTF8 | ConvertFrom-Json).base)
  } else {
    $Base = Join-Path $R.WorkDir 'base-103'
  }
}
$Official = Join-Path $Base 'WuwaTappo.official.exe'
$Patched = Join-Path $Root 'src'

if (-not (Test-Path -LiteralPath $Py)) { throw "缺少 .venv，先跑 tools\setup-env.ps1" }
if (-not (Test-Path -LiteralPath $Official)) { throw "缺少官方主程序：$Official" }


if ($Extract) {
  "== 1/5 解包官方 EXE =="
  $env:WUWA_EXTRACT_ROOT = $Base
  & $Py (Join-Path $Patched 'tools\extract_static.py') $Official
  if ($LASTEXITCODE -ne 0) { throw "解包失败" }

  # 注入脚本依赖 js_structure.json（共享 JS 里各模块的位置与压缩变量名），
  # 这个文件由 inspect_js.cjs 从解包产物里分析得出，extract_static.py 不产出。
  "== 1.5/4 定位 JS 结构 =="
  $readable = Join-Path $Base 'readable_js\assets'
  if (-not (Test-Path -LiteralPath $readable)) { throw "解包后缺少 readable_js\assets：$readable" }
  $shared = Get-ChildItem -LiteralPath $readable -Filter *.js |
    Where-Object { Select-String -LiteralPath $_.FullName -Pattern 'function applyCatalog(' -SimpleMatch -Quiet } |
    Select-Object -First 1
  if (-not $shared) { throw '未找到含 applyCatalog 的共享 JS，无法定位注入锚点' }
  "共享 JS: $($shared.Name)"
  $nodeExe = (Get-Command node -ErrorAction SilentlyContinue).Source
  if (-not $nodeExe) { throw '需要 Node 生成 js_structure.json，请确认 node 在 PATH 上' }
  & $nodeExe (Join-Path $Root '..\shared\tools\inspect_js.cjs') $Base $shared.Name
  if ($LASTEXITCODE -ne 0) { throw '生成 js_structure.json 失败' }
}

"== 2/5 注入全图鉴控制器（$($R.Label)） =="
$env:WUWA_BUILD_ROOT = $R.WorkDir
$env:WUWA_EXTRACT_ROOT = $Base

# build_integration.py 与运行期都要读 build\local_config.json，之前它靠上游 build.py 生成，
# 现在这一步由本脚本负责。素材缓存缺失时计数为 0，程序仍可启动，图鉴按需回源官方 CDN。
$cacheDir = Join-Path $Root 'assets\cache'
$assetCount = 0
$atlasCount = 0
$manifestPath = Join-Path $cacheDir 'asset_manifest.json'
if (Test-Path -LiteralPath $manifestPath) {
  $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
  $assetCount = @($manifest.records).Count
}
$atlasPath = Join-Path $cacheDir 'atlas.json'
if (Test-Path -LiteralPath $atlasPath) {
  $atlas = Get-Content -LiteralPath $atlasPath -Raw -Encoding UTF8 | ConvertFrom-Json
  $atlasCount = @($atlas.data).Count
}
if ($assetCount -eq 0) { Write-Warning '未找到素材缓存（assets\cache），产物不含离线图鉴' }

$clientVersion = (Get-Item -LiteralPath $Official).VersionInfo.ProductVersion
if (-not $clientVersion) { $clientVersion = '0.0.0' }
# 整合版本沿用注入源码里声明的值，避免面板显示与实际补丁版本不一致
$modVersion = '1.2'
$buildPy = Join-Path $Patched 'build.py'
if (Test-Path -LiteralPath $buildPy) {
  $hit = Select-String -LiteralPath $buildPy -Pattern "MOD_VERSION\s*=\s*'([^']+)'" | Select-Object -First 1
  if ($hit) { $modVersion = $hit.Matches[0].Groups[1].Value }
}
# 面板「全图鉴版本」跟本仓库 package.json 走，和 GitHub Release 的 tag 同源
$communityVersion = ''
$packageJson = Join-Path (Split-Path $Root -Parent) 'package.json'
if (Test-Path -LiteralPath $packageJson) {
  try {
    $communityVersion = [string]((Get-Content -LiteralPath $packageJson -Raw -Encoding UTF8 | ConvertFrom-Json).version)
  } catch { $communityVersion = '' }
}
if (-not $communityVersion) { Write-Warning '未读到 package.json 版本，面板全图鉴版本会显示为空' }

# 前端用 token 给本机 HTTP 请求签名，必须存在。已生成的沿用，避免每次构建换掉已部署实例的凭据
$configPath = Join-Path $R.WorkDir 'local_config.json'
$token = ''
if (Test-Path -LiteralPath $configPath) {
  $previous = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
  if ($previous.token) { $token = [string]$previous.token }
}
if (-not $token) {
  $random = New-Object byte[] 32
  $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
  try { $generator.GetBytes($random) } finally { $generator.Dispose() }
  $token = [Convert]::ToBase64String($random)
}

$config = [ordered]@{
  token          = $token
  port           = 43229
  region         = $R.Id
  label          = $R.Label
  # 运行期拉素材与图鉴走本区域的官方源；换区域不能沿用国服域名
  cdn_host       = $R.AssetsHost
  api_base       = $R.ApiBase
  asset_count    = $assetCount
  atlas_count    = $atlasCount
  client_version = $clientVersion
  mod_version    = $modVersion
  # 面板「全图鉴版本」要显示本仓库的发布号，不能和上游 MOD_VERSION 混用
  community_version = $communityVersion
}
[IO.File]::WriteAllText($configPath,
  ($config | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
"基底版本 $clientVersion / 整合 $modVersion / 素材 $assetCount 张 / 图鉴 $atlasCount 条"

# build_integration.py 从 build\current_base.txt 读基底路径，必须随 -Base 同步，
# 否则换了基底仍会指向旧目录（无 BOM，避免 read_text 读出 \ufeff）
[IO.File]::WriteAllText((Join-Path $R.WorkDir 'current_base.txt'), $Base,
  (New-Object Text.UTF8Encoding($false)))
if (Test-Path -LiteralPath (Join-Path $R.WorkDir 'frontend_v2')) {
  Remove-Item -LiteralPath (Join-Path $R.WorkDir 'frontend_v2') -Recurse -Force
}
& $Py (Join-Path $Patched 'tools\build_integration.py')
if ($LASTEXITCODE -ne 0) { throw "注入失败" }

"== 3/5 冻结本地素材服务 =="
& $Py -m PyInstaller --noconfirm --clean --onedir --console --name LocalGalleryHost `
  --distpath (Join-Path $R.WorkDir 'frozen') `
  --workpath (Join-Path $R.WorkDir 'pyinstaller') `
  --specpath $R.WorkDir `
  --paths (Join-Path $Patched 'src') `
  (Join-Path $Patched 'src\local_host.py')
if ($LASTEXITCODE -ne 0) { throw "冻结失败" }

"== 4/5 编译无控制台启动器 =="
# 安装包的快捷方式指向它；缺失就只能退到 start.cmd（弹黑框 + cmd 图标）。
& (Join-Path $PSScriptRoot 'build-launcher.ps1') -Region $R.Id

if ($NoDeploy) {
  "== 5/5 跳过装配（-NoDeploy）=="
  "构建产物: build\WuwaTappo.Local.v2.exe 与 build\frozen\LocalGalleryHost"
  exit 0
}

"== 5/5 装配并同步到 app\ =="
if ($NoRuntime) { & (Join-Path $PSScriptRoot 'deploy.ps1') -Region $R.Id -NoRuntime }
else { & (Join-Path $PSScriptRoot 'deploy.ps1') -Region $R.Id }
