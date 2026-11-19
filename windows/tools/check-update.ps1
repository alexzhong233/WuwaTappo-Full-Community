<#
  检查官方是否发布新版本（不下载、不改动任何文件）

  官方更新接口（区域由 region.ps1 与 -Region 决定）：
    1) https://mc-wuwatappo.kurogames-global.com/app/json-host/latest_host.json
       -> {"url": ".../app/version-json/latest.json?t=..."}
    2) .../app/version-json/latest.json
       -> {version, pub_date, platforms["windows-x86_64"]:{url, signature}, min_ota_client_version}

  用法： powershell -ExecutionPolicy Bypass -File tools\check-update.ps1
#>
param([ValidateSet('cn', 'global')][string]$Region, [string]$AssetsHost)

$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'region.ps1')
. (Join-Path $PSScriptRoot 'upstream-common.ps1')
$R = Get-RegionProfile -Root $Root -Region $Region
if (-not $AssetsHost) { $AssetsHost = $R.AssetsHost }

"== 查询官方更新清单 =="
$hostJson = Get-Json "$AssetsHost/app/json-host/$($R.UpdaterHostFile)"
if (-not $hostJson.url) { throw "官方版本指针未返回 url 字段" }
"清单入口: $($hostJson.url)"

$manifest = Get-Json ([string]$hostJson.url)
$remoteVer = [string]$manifest.version
$platform = $manifest.platforms.'windows-x86_64'
$pubDate = [datetime]$manifest.pub_date

"官方版本 : $remoteVer"
"发布时间 : $($pubDate.ToLocalTime().ToString('yyyy-MM-dd HH:mm'))"
"安装包   : $($platform.url)"
"签名     : $(if ($platform.signature) { '有（minisign，官方私有密钥）' } else { '无' })"

"== 本地版本 =="
$localExe = Join-Path $R.AppDir 'WuwaTappo.Local.exe'
$localVer = if (Test-Path -LiteralPath $localExe) { (Get-Item -LiteralPath $localExe).VersionInfo.ProductVersion } else { '' }
$cfg = Join-Path $R.WorkDir 'local_config.json'
$baseVer = if (Test-Path -LiteralPath $cfg) { (Get-Content -LiteralPath $cfg -Raw | ConvertFrom-Json).client_version } else { '' }
"本地程序 : $localVer"
"适配基底 : $baseVer"

"== 结论 =="
try {
  $r = [version]($remoteVer -replace '[^0-9.].*$', '')
  $l = [version]($baseVer -replace '[^0-9.].*$', '')
  if ($r -gt $l) {
    "官方 $remoteVer 高于本地基底 $baseVer —— 可以先跑 tools\update-watch.ps1 试构建到 staging。"
    "提示：注入锚点可能变化，构建脚本会在找不到锚点时明确报错并保持 app\ 不动。"
  } elseif ($r -eq $l) {
    "已是最新（$baseVer），无需处理。"
  } else {
    "官方清单 $remoteVer 低于本地基底 $baseVer。"
    "该清单可能跟踪安装器/host 渠道而非应用本体，未必意味着新版本；如需确认可看安装包文件名里的版本号。"
  }
} catch {
  "版本号无法解析比较：remote='$remoteVer' base='$baseVer'"
}
