<#
  获取官方 Windows 主程序基底（只解包，不执行安装器）

  用途：让 CI 与本地都能拿到 build\official\<版本>\base\WuwaTappo.official.exe，
        再交给 build.ps1 -Base。

  重要：官方安装包一律用 7-Zip 解包，**从不执行**。
        执行安装器会真的把客户端装进系统、启动进程、写 LOCALAPPDATA 缓存，
        这不是构建该做的事。

  版本来源优先级：
    1. -OfficialExe 直接指定已解出的 WuwaTappo.exe（最省事，推荐本地用）
    2. -Installer   指定本地安装包（用 7z 解）
    3. -Version     指定版本；与官方最新版一致时走官方清单
    4. 默认读 shared\upstream\<region>.json 的 version（-Latest 改为跟最新）

  用法：
    powershell -File windows\tools\fetch-official.ps1
    powershell -File windows\tools\fetch-official.ps1 -OfficialExe D:\path\WuwaTappo.exe
    powershell -File windows\tools\fetch-official.ps1 -Installer D:\path\setup.exe
    powershell -File windows\tools\fetch-official.ps1 -Latest
#>
param(
  [string]$Version,
  [ValidateSet('cn', 'global')]
  [string]$Region,
  [string]$Installer,
  [string]$OfficialExe,
  [switch]$Latest,
  [switch]$Force
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'region.ps1')
. (Join-Path $PSScriptRoot 'upstream-common.ps1')
$R = Get-RegionProfile -Root $Root -Region $Region
$LockFile = $R.LockFile

function Find-SevenZip {
  foreach ($p in @(
      'C:\Program Files\7-Zip\7z.exe',
      'C:\Program Files (x86)\7-Zip\7z.exe'
    )) {
    if (Test-Path -LiteralPath $p) { return $p }
  }
  $cmd = Get-Command 7z -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  return $null
}

function Get-OfficialExeFromInstaller([string]$installer, [string]$output, [string]$sevenZip) {
  # 与 macOS 共用同一套 PE/NSIS 解析；这里只负责把已选中的主程序落到 Windows 构建目录。
  $node = (Get-Command node -ErrorAction SilentlyContinue).Source
  if (-not $node) { throw '找不到 node：取官方基底需要共享提取器' }
  $extractor = Join-Path $Root '..\scripts\extract-assets.mjs'
  & $node $extractor --installer $installer --sevenzip $sevenZip --executable-output $output | Out-Host
  if ($LASTEXITCODE -ne 0) { throw "共享 PE/NSIS 提取器失败（退出码 $LASTEXITCODE）" }
  if (-not (Test-Path -LiteralPath $output)) { throw "提取器没有产出主程序：$output" }
  return Get-Item -LiteralPath $output
}
# 仓库里的 JSON 是无 BOM UTF-8，PS 5.1 默认按 GBK 读会把中文读坏，必须显式 -Encoding UTF8
$lock = if (Test-Path -LiteralPath $LockFile) { Get-Content -LiteralPath $LockFile -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
$lockedVersion = if ($lock) { [string]$lock.version } else { '' }

if ($OfficialExe) {
  if (-not (Test-Path -LiteralPath $OfficialExe)) { throw "指定的主程序不存在: $OfficialExe" }
  $target = if ($Version) { $Version } elseif ($lockedVersion) { $lockedVersion } else { 'custom' }
  $work = Join-Path $R.WorkDir "official\$target"
  New-Item -ItemType Directory -Path $work -Force | Out-Null
  "== 1/2 使用已有主程序 =="
  "来源: $((Resolve-Path -LiteralPath $OfficialExe).Path)"
  $sourceExe = (Resolve-Path -LiteralPath $OfficialExe).Path
} else {
  if (-not $Installer) {
    "== 1/4 解析目标版本 =="
    $manifest = $null
    if ($Latest -or $Version -or (-not $lockedVersion)) { $manifest = Get-RemoteManifest -AssetsHost $R.AssetsHost -UpdaterHostFile $R.UpdaterHostFile }

    if ($Version) { $target = $Version }
    elseif ($Latest) { $target = [string]$manifest.version }
    else { $target = $lockedVersion }

    "目标版本: $target"
    if ($lockedVersion) { "锁定版本: $lockedVersion" }

    $url = $null
    if ($manifest -and [string]$manifest.version -eq $target) {
      $expectedLeaf = "WuwaTappo-$($R.Id)_$target`_x64-setup.exe"
      $url = Assert-OfficialUrl ([string]$manifest.platforms.'windows-x86_64'.url) $R.AssetsHost $expectedLeaf
      '来源    : 官方清单（最新版）'
    }
    if (-not $url -and $lock -and [string]$lock.version -eq $target -and $lock.installer.url) {
      $url = Assert-OfficialUrl ([string]$lock.installer.url) $R.AssetsHost "WuwaTappo-$($R.Id)_$target`_x64-setup.exe"
      '来源    : canonical upstream lock 直链'
    }
    if (-not $url) {
      throw "找不到 $target 的下载地址。请用 -Installer 指定本地安装包，或 -Latest 取官方最新版。"
    }
    "地址    : $url"

    $work = Join-Path $R.WorkDir "official\$target"
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    $setup = Join-Path $work (Split-Path $url -Leaf)
    if ((Test-Path -LiteralPath $setup) -and -not $Force) {
      "已存在，跳过下载: $setup"
    } else {
      "== 2/4 下载安装包 =="
      # WebClient 的 100 秒上限对 200 MB 的官方包太紧，CDN 一慢就整条链失败
      # （CI 上已经因为 "operation has timed out" 挂过）。改用 HttpClient 并给 30 分钟，
      # 失败重试 3 次，每次清掉半截文件。
      Add-Type -AssemblyName System.Net.Http | Out-Null
      # 主源是官方 CDN。国服 CDN 从海外 runner 时通时不通（2026-11-08 发版就是被它
      # 连不上一路打死），失败后退到仓库自建的 upstream-resources 镜像：文件名带安装包
      # sha256，内容由下方的哈希校验兜底，镜像投毒改不动字节。
      $mirror = ''
      if ($env:WUWA_INSTALLER_MIRROR_BASE -and $lock -and [string]$lock.version -eq $target -and $lock.installer.sha256) {
        $mirror = "$($env:WUWA_INSTALLER_MIRROR_BASE.TrimEnd('/'))/$($lock.installer.sha256)-$(Split-Path $url -Leaf)"
      }
      $sources = @($url)
      if ($mirror) { $sources += $mirror }
      $lastError = $null
      foreach ($source in $sources) {
        for ($attempt = 1; $attempt -le 3; $attempt++) {
          $client = $null
          try {
            $client = New-Object System.Net.Http.HttpClient
            $client.Timeout = [TimeSpan]::FromMinutes(30)
            $client.DefaultRequestHeaders.Add('User-Agent', 'WuwaTappo-FullAtlas-Fetch/1.0')
            $response = $client.GetAsync($source, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
            $response.EnsureSuccessStatusCode() | Out-Null
            $stream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
            $file = [IO.File]::Create($setup)
            try { $stream.CopyTo($file) } finally { $file.Dispose(); $stream.Dispose(); $response.Dispose() }
            break
          } catch {
            $lastError = $_
            if (Test-Path -LiteralPath $setup) { Remove-Item -LiteralPath $setup -Force -ErrorAction SilentlyContinue }
            if ($attempt -lt 3) {
              "下载失败（第 $attempt 次）：$($_.Exception.Message)，10 秒后重试"
              Start-Sleep -Seconds 10
            }
          } finally {
            if ($client) { $client.Dispose() }
          }
        }
        if (Test-Path -LiteralPath $setup) {
          if ($source -ne $url) { "主源不可用，已改用镜像: $source" }
          break
        }
        if ($mirror -and $source -eq $url) { "主源 3 次均失败，改用镜像: $mirror" }
      }
      if (-not (Test-Path -LiteralPath $setup)) {
        throw "安装包下载失败（主源与镜像均不可用）：$url`n$($lastError.Exception.Message)"
      }
      "下载完成: {0:N1} MB" -f ((Get-Item -LiteralPath $setup).Length / 1MB)
    }
  } else {
    if (-not (Test-Path -LiteralPath $Installer)) { throw "安装包不存在: $Installer" }
    $target = if ($Version) { $Version } elseif ($lockedVersion) { $lockedVersion } else { 'custom' }
    $setup = (Resolve-Path -LiteralPath $Installer).Path
    $work = Join-Path $R.WorkDir "official\$target"
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    "使用指定安装包: $setup"
  }

  $sourceExe = $null
}

$base = Join-Path $work 'base'
$official = Join-Path $base 'WuwaTappo.official.exe'

if ((Test-Path -LiteralPath $official) -and -not $Force) {
  '已存在基底，跳过解包（要重解加 -Force）'
} else {
  if (-not $sourceExe) {
    "== 3/4 解包（共享 Node 提取器，不执行安装器）=="
    $sevenZip = Find-SevenZip
    if (-not $sevenZip) {
      throw @"
找不到 7-Zip。下载路径需要它，且本脚本不会执行官方安装器。
  安装： winget install 7zip.7zip
  或者（本地推荐，和上游 FullAtlas 的做法一致）：装好官方客户端后
        用 -OfficialExe 指向 %LOCALAPPDATA% 下的 WuwaTappo.exe，全程不下载、不解包
"@
    }
    "7-Zip: $sevenZip"
    $payloadPath = Join-Path $work 'WuwaTappo.official.exe'
    if (Test-Path -LiteralPath $payloadPath) { Remove-Item -LiteralPath $payloadPath -Force }
    $sourceExe = (Get-OfficialExeFromInstaller -installer $setup -output $payloadPath -sevenZip $sevenZip).FullName
  }
  if (Test-Path -LiteralPath $base) { Remove-Item -LiteralPath $base -Recurse -Force }
  New-Item -ItemType Directory -Path $base -Force | Out-Null
  Copy-Item -LiteralPath $sourceExe -Destination $official
}

"== 4/4 校验基底 =="
$sha = Get-Sha256Hex $official
$size = (Get-Item -LiteralPath $official).Length
"文件  : $official"
"大小  : {0:N1} MB" -f ($size / 1MB)
"sha256: $sha"

$expected = ''
$match = $null

# 签名由库洛私钥产生，是唯一独立于本仓库的官方发行方凭据，Windows 构建仅保留 EV 签名校验。
$authSignature = Get-AuthenticodeSignature -LiteralPath $official
if ($authSignature.Status -ne 'Valid') {
  throw "官方基底签名无效（$($authSignature.Status)）：$($authSignature.StatusMessage)"
}
$signerSubject = [string]$authSignature.SignerCertificate.Subject
$signerIssuer = [string]$authSignature.SignerCertificate.Issuer
if ($signerSubject -notmatch 'CN=广州库洛数界科技有限公司') {
  throw "官方基底签名主体不是库洛：$signerSubject"
}
if ($signerIssuer -notmatch 'DigiCert Trusted G4 Code Signing') {
  throw "官方基底签发 CA 不是官方 EV 证书：$signerIssuer"
}
'签名  : 官方发行方有效'

$info = [ordered]@{
  version      = $target
  base         = $base
  official     = $official
  sha256       = $sha
  size         = $size
  expected     = $expected
  hash_matched = $match
  fetched_at   = (Get-Date).ToString('s')
}
$outFile = Join-Path $R.WorkDir 'official\current.json'
New-Item -ItemType Directory -Path (Split-Path $outFile -Parent) -Force | Out-Null
# 不能带 BOM：Node 侧 JSON.parse 会把 \ufeff 当非法字符
[IO.File]::WriteAllText($outFile, ($info | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
"清单已写入: $outFile"
$info | ConvertTo-Json
