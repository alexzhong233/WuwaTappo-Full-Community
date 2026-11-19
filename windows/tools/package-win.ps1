<#
  把装配好的 app\ 打成便携 zip（调试/绿色版用，发行走 package-installer.ps1 的 exe）

  产物： dist\WuwaTappo-Full-CN-Community-Windows-x64-<version>.zip
         dist\WuwaTappo-Full-CN-Community-Windows-x64-<version>.zip.sha256

  版本号取自仓库根的 package.json（与 macOS 产物保持一致）。

  用法：
    powershell -File windows\tools\package-win.ps1
    powershell -File windows\tools\package-win.ps1 -NoRuntime    # 小体积，回退系统 WebView2
#>
param(
  [string]$Source,
  [string]$OutDir,
  [switch]$NoRuntime,
  [switch]$KeepStaging,
  [ValidateSet('cn', 'global')][string]$Region
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
$RepoRoot = Split-Path $Root -Parent
. (Join-Path $PSScriptRoot 'region.ps1')
. (Join-Path $PSScriptRoot 'upstream-common.ps1')
$R = Get-RegionProfile -Root $Root -Region $Region

if (-not $Source) { $Source = $R.AppDir }
if (-not $OutDir) { $OutDir = Join-Path $RepoRoot 'dist' }

$version = (Get-Content -LiteralPath (Join-Path $RepoRoot 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json).version
$name = Get-ReleaseAssetName -Profile $R -Kind windowsPortable -Version $version
$zip = Join-Path $OutDir $name

if (-not (Test-Path -LiteralPath $Source)) { throw "找不到待打包目录：$Source（先跑 build.ps1 / deploy.ps1）" }
$mainExe = Join-Path $Source 'WuwaTappo.Local.exe'
if (-not (Test-Path -LiteralPath $mainExe)) { throw "目录里没有 WuwaTappo.Local.exe，可能是空部署：$Source" }

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }

# 运行时目录单独控制：CI 默认不内嵌，压缩体积从 ~700MB 降到 ~120MB
$staging = Join-Path $R.WorkDir 'package-staging'
if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
New-Item -ItemType Directory -Path $staging -Force | Out-Null

. (Join-Path $PSScriptRoot 'payload.ps1')

Write-Host '== 收集文件 =='
$files = Get-WuwaPayloadFiles -Source $Source -NoRuntime:$NoRuntime
Write-Host ("共 {0} 个文件" -f $files.Count)

$sevenZip = @(
  'C:\Program Files\7-Zip\7z.exe',
  'C:\Program Files (x86)\7-Zip\7z.exe'
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
# 与 fetch-official 一致：装在 PATH 上的 7z 也认，否则本地会静默退到 Compress-Archive
if (-not $sevenZip) {
  $cmd = Get-Command 7z -ErrorAction SilentlyContinue
  if ($cmd) { $sevenZip = $cmd.Source }
}

if ($sevenZip) {
  Write-Host "== 打包（7z）=="
  $listFile = Join-Path $staging 'files.txt'
  $lines = $files | ForEach-Object { $_.FullName.Substring($Source.Length).TrimStart('\') }
  [IO.File]::WriteAllLines($listFile, $lines, (New-Object Text.UTF8Encoding($false)))
  Push-Location $Source
  try {
    & $sevenZip a -tzip -mx=5 -bb0 $zip "@$listFile" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "7z 打包失败，退出码 $LASTEXITCODE" }
  } finally { Pop-Location }
} else {
  Write-Host "== 打包（Compress-Archive，未找到 7z）=="
  # Compress-Archive 需要实际目录结构，先复制到 staging 再压
  $skipped = 0
  foreach ($f in $files) {
    $rel = $f.FullName.Substring($Source.Length).TrimStart('\')
    $dest = Join-Path $staging $rel
    New-Item -ItemType Directory -Path (Split-Path $dest -Parent) -Force | Out-Null
    try {
      Copy-Item -LiteralPath $f.FullName -Destination $dest -ErrorAction Stop
    } catch {
      $skipped++
      Write-Warning "跳过无法读取的文件: $rel ($($_.Exception.GetType().Name))"
    }
  }
  if ($skipped -gt 0) { "已跳过 $skipped 个文件" }
  Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $zip -CompressionLevel Optimal
}

if (-not $KeepStaging) { Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue }

$zipSize = (Get-Item -LiteralPath $zip).Length
$sha = Get-Sha256Hex $zip

''
'打包完成'
"  文件: $zip"
"  大小: {0:N1} MB" -f ($zipSize / 1MB)
"  sha256: $sha"
