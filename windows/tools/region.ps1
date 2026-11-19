# 区域配置来自 shared/config/regions.json；Windows 构建目录与文件名为平台派生值。
function Get-RegionProfile {
  param([string]$Root, [string]$Region)

  if (-not $Region) { $Region = if ($env:WUWA_REGION) { $env:WUWA_REGION } else { 'cn' } }
  $Region = $Region.ToLowerInvariant()
  $configPath = Join-Path $Root '..\shared\config\regions.json'
  if (-not (Test-Path -LiteralPath $configPath)) { throw "缺少共享区域配置：$configPath" }
  $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
  $entry = $config.PSObject.Properties[$Region]
  if (-not $entry) { throw "未知区域：$Region（可选 cn / global）" }
  $profile = $entry.Value
  $isGlobal = $Region -eq 'global'
  $repoRoot = Split-Path $Root -Parent
  $releasePath = Join-Path $repoRoot 'shared/contracts/release-assets.json'
  if (-not (Test-Path -LiteralPath $releasePath)) { throw "缺少共享发布契约：$releasePath" }
  $release = Get-Content -LiteralPath $releasePath -Raw -Encoding UTF8 | ConvertFrom-Json

  [pscustomobject]@{
    Id              = $Region
    Label           = [string]$profile.label
    IsGlobal        = $isGlobal
    WorkDir         = Join-Path $Root $(if ($isGlobal) { 'build-global' } else { 'build' })
    AppDir          = Join-Path $Root $(if ($isGlobal) { 'app-global' } else { 'app' })
    LockFile        = Join-Path $repoRoot ([string]$profile.windowsLock)
    MacLockFile     = Join-Path $repoRoot ([string]$profile.macLock)
    AssetsHost      = "https://$($profile.assetsHost)"
    ApiBase         = "https://$($profile.apiHost)"
    UpdaterHostFile = [string]$profile.updaterHostFile
    ArtifactPrefix  = [string]$profile.artifactPrefix
    ProductName     = [string]$profile.windowsProductName
    ReleaseFormats  = $release.formats
  }
}

function Get-ReleaseAssetName {
  param(
    [Parameter(Mandatory = $true)][object]$Profile,
    [Parameter(Mandatory = $true)][ValidateSet('macos', 'windowsInstaller', 'windowsPortable', 'windowsUpdate')][string]$Kind,
    [Parameter(Mandatory = $true)][string]$Version
  )
  $property = $Profile.ReleaseFormats.PSObject.Properties[$Kind]
  if (-not $property -or -not $property.Value) { throw "共享发布契约缺少产物类型：$Kind" }
  return ([string]$property.Value).Replace('{prefix}', [string]$Profile.ArtifactPrefix).Replace('{version}', $Version)
}
