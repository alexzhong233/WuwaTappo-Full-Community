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
    4. 默认读 windows\upstream-win.json 的 client_version（-Latest 改为跟最新）

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
$R = Get-RegionProfile -Root $Root -Region $Region
$LockFile = $R.LockFile

function Get-Json([string]$url) {
  try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }
  $wc = New-Object System.Net.WebClient
  $wc.Proxy = $null
  $wc.Encoding = [Text.Encoding]::UTF8
  $wc.Headers.Add('User-Agent', 'WuwaTappo-FullAtlas-Fetch/1.0')
  try { return ($wc.DownloadString($url) | ConvertFrom-Json) } finally { $wc.Dispose() }
}

function Assert-OfficialUrl([string]$url, [string]$expectedLeaf = '') {
  try { $u = [Uri]$url } catch { throw '官方资源地址不是有效 URL' }
  $expectedHost = ([Uri]$R.AssetsHost).Host
  if ($u.Scheme -ne 'https' -or $u.Host -ne $expectedHost -or $u.Port -ne 443 -or $u.UserInfo -or $u.Fragment) {
    throw "官方资源地址不是当前区域的 HTTPS 官方地址: $url"
  }
  $leaf = [IO.Path]::GetFileName($u.AbsolutePath)
  if ($expectedLeaf -and $leaf -ne $expectedLeaf) { throw "官方原包文件名与版本或区域不匹配: $leaf" }
  return $u.AbsoluteUri
}

function Get-RemoteManifest {
  $hostJson = Get-Json "$($R.AssetsHost)/app/json-host/$($R.UpdaterHostFile)"
  if (-not $hostJson.url) { throw 'latest_host.json 未返回 url 字段' }
  # 必须用清单返回的带 ?t= 的地址，直连 version-json 会命中 CDN 缓存拿到旧版本
  return Get-Json (Assert-OfficialUrl ([string]$hostJson.url))
}

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

<#
  官方安装包是两层包装：外层是个自解压壳（PE，内层整个塞进 .rdata），
  真正的 NSIS-3 安装器从壳内部某个偏移开始。

  7-Zip 只能看到外层，会把它当普通 PE 解析 —— 于是解出来的是 .text/.rdata/
  .pdata/CERTIFICATE 这些节区，一个安装文件都没有。这不是 7-Zip 版本问题：
  23.01 / 24.09 / 25.01 三个版本都这样，强制 -t# 也不行。

  判定与 macOS 侧 macos/scripts/extract-assets.mjs 的 findEmbeddedNsis 一致：
    1. 扫 NSIS 签名 EF BE AD DE + 'NullsoftInst'，读 +24 处的长度字段得到头的结束位置
    2. 找一个 MZ/PE，其节区原始数据正好结束在该 NSIS 头处 —— 那就是内层安装器
  切片用 NSIS 头给出的 end，而不是文件尾，避免把外壳的签名尾巴带进去。
#>
function Read-PeRawEnd([byte[]]$bytes, [long]$start) {
  if ($start + 0x40 -gt $bytes.Length) { return $null }
  if ($bytes[$start] -ne 0x4D -or $bytes[$start + 1] -ne 0x5A) { return $null }
  $peOff = [BitConverter]::ToUInt32($bytes, $start + 0x3c)
  if ($peOff -lt 0x40 -or $peOff -gt 0x400) { return $null }
  $pe = $start + $peOff
  if ($pe + 24 -gt $bytes.Length) { return $null }
  if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x00004550) { return $null }
  $sections = [BitConverter]::ToUInt16($bytes, $pe + 6)
  $optSize = [BitConverter]::ToUInt16($bytes, $pe + 20)
  $tableStart = $pe + 24 + $optSize
  $rawEnd = [long]0
  for ($i = 0; $i -lt $sections; $i++) {
    $entry = $tableStart + $i * 40
    if ($entry + 40 -gt $bytes.Length) { return $null }
    $rawSize = [BitConverter]::ToUInt32($bytes, $entry + 16)
    $rawPtr = [BitConverter]::ToUInt32($bytes, $entry + 20)
    $end = $start + [long]$rawPtr + [long]$rawSize
    if ($end -gt $rawEnd) { $rawEnd = $end }
  }
  return $rawEnd
}

function Get-NsisPayloadBounds([string]$path) {
  $bytes = [IO.File]::ReadAllBytes($path)
  $latin = [Text.Encoding]::GetEncoding(28591)
  $text = $latin.GetString($bytes)
  $signature = $latin.GetString([byte[]](0xEF, 0xBE, 0xAD, 0xDE) + [Text.Encoding]::ASCII.GetBytes('NullsoftInst'))

  $headers = @{}
  $pos = 0
  while (($pos = $text.IndexOf($signature, $pos, [StringComparison]::Ordinal)) -ge 0) {
    # 签名之前还有 4 字节（长度字段所在位置），与 mac 侧 header = offset - 4 对齐
    $header = $pos - 4
    if ($header -ge 0 -and $header + 28 -le $bytes.Length) {
      $following = [BitConverter]::ToUInt32($bytes, $header + 24)
      $end = [long]$header + [long]$following
      # 键是头的起点：PE 的节区数据正好结束在这里，才说明它紧跟的就是内层安装器
      if ($following -ge 28 -and $end -le $bytes.Length) { $headers["$header"] = $end }
    }
    $pos++
  }
  if ($headers.Count -eq 0) { return $null }

  $mz = 0
  while (($mz = $text.IndexOf('MZ', $mz, [StringComparison]::Ordinal)) -ge 0) {
    $rawEnd = Read-PeRawEnd $bytes $mz
    if ($rawEnd -and $headers.ContainsKey("$rawEnd")) {
      return [pscustomobject]@{ Start = [long]$mz; End = [long]$headers["$rawEnd"]; Header = [long]$rawEnd }
    }
    $mz += 2
  }
  return $null
}

# 主程序在 NSIS 里不一定叫 WuwaTappo.exe：Tauri 的 NSIS 脚本会把它改成 $_21_ 之类的变量名。
# 所以按锁定大小认；大小对不上再退回「体积最大的候选」。
# 排除 setup.exe / uninstall.exe / *.dll：它们同样是 PE，但不是应用本体。
function Select-OfficialExe([string]$dir, $expectedSize) {
  $candidates = @()
  foreach ($file in @(Get-ChildItem -LiteralPath $dir -Recurse -File -ErrorAction SilentlyContinue)) {
    if ($file.Length -lt 20MB) { continue }
    if ($file.Name -match '^(setup|uninstall)\.exe$' -or $file.Extension -eq '.dll') { continue }
    $stream = [IO.File]::OpenRead($file.FullName)
    try {
      $head = New-Object byte[] 2
      $read = $stream.Read($head, 0, 2)
    } finally { $stream.Dispose() }
    if ($read -ne 2 -or $head[0] -ne 0x4D -or $head[1] -ne 0x5A) { continue }
    $candidates += $file
  }
  if ($candidates.Count -eq 0) { return $null }
  if ($expectedSize) {
    $exact = @($candidates | Where-Object { $_.Length -eq [long]$expectedSize })
    if ($exact.Count -ge 1) { return $exact[0] }
  }
  return ($candidates | Sort-Object Length -Descending | Select-Object -First 1)
}

# 不依赖 Get-FileHash：从 pwsh 7 环境启动 PS 5.1 时该 cmdlet 可能加载不到
function Get-Sha256Hex([string]$path) {
  $algorithm = [Security.Cryptography.SHA256]::Create()
  $stream = [IO.File]::OpenRead($path)
  try { $bytes = $algorithm.ComputeHash($stream) } finally { $stream.Dispose(); $algorithm.Dispose() }
  return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}

# 仓库里的 JSON 是无 BOM UTF-8，PS 5.1 默认按 GBK 读会把中文读坏，必须显式 -Encoding UTF8
$lock = if (Test-Path -LiteralPath $LockFile) { Get-Content -LiteralPath $LockFile -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
$lockedVersion = if ($lock) { [string]$lock.client_version } else { '' }

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
    if ($Latest -or $Version -or (-not $lockedVersion)) { $manifest = Get-RemoteManifest }

    if ($Version) { $target = $Version }
    elseif ($Latest) { $target = [string]$manifest.version }
    else { $target = $lockedVersion }

    "目标版本: $target"
    if ($lockedVersion) { "锁定版本: $lockedVersion" }

    $url = $null
    if ($manifest -and [string]$manifest.version -eq $target) {
      $expectedLeaf = "WuwaTappo-$($R.Id)_$target`_x64-setup.exe"
      $url = Assert-OfficialUrl ([string]$manifest.platforms.'windows-x86_64'.url) $expectedLeaf
      '来源    : 官方清单（最新版）'
    }
    if (-not $url -and $lock -and [string]$lock.client_version -eq $target -and $lock.installer_url) {
      $url = Assert-OfficialUrl ([string]$lock.installer_url) "WuwaTappo-$($R.Id)_$target`_x64-setup.exe"
      '来源    : upstream-win.json 锁定直链'
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
      if ($env:WUWA_INSTALLER_MIRROR_BASE -and $lock -and [string]$lock.client_version -eq $target -and $lock.installer_sha256) {
        $mirror = "$($env:WUWA_INSTALLER_MIRROR_BASE.TrimEnd('/'))/$($lock.installer_sha256)-$(Split-Path $url -Leaf)"
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

  # 安装包完整性：锁定版本的包哈希必须对得上，否则说明下载被截断或被替换，或锁文件写错。
  # 只对锁定版本校验 —— 用 -Latest 或指定别的版本时没有可比对的基准。
  if ($lock -and [string]$lock.client_version -eq $target -and $lock.installer_sha256) {
    $actualInstallerSha = Get-Sha256Hex $setup
    if ($actualInstallerSha -ne [string]$lock.installer_sha256) {
      throw "安装包 sha256 与锁文件不一致：$setup`n实际 $actualInstallerSha`n期望 $($lock.installer_sha256)"
    }
    '安装包 sha256 校验通过'
  }
  $sourceExe = $null
}

$base = Join-Path $work 'base'
$official = Join-Path $base 'WuwaTappo.official.exe'

if ((Test-Path -LiteralPath $official) -and -not $Force) {
  '已存在基底，跳过解包（要重解加 -Force）'
} else {
  if (-not $sourceExe) {
    "== 3/4 解包（7-Zip，不执行安装器）=="
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
    $extractDir = Join-Path $work 'app'
    if (Test-Path -LiteralPath $extractDir) { Remove-Item -LiteralPath $extractDir -Recurse -Force }
    New-Item -ItemType Directory -Path $extractDir -Force | Out-Null

    $bounds = Get-NsisPayloadBounds $setup
    $archive = $setup
    $payloadFile = $null
    if ($bounds) {
      $payloadFile = Join-Path $work '_nsis_payload.exe'
      $length = $bounds.End - $bounds.Start
      $reader = [IO.File]::OpenRead($setup)
      try {
        $writer = [IO.File]::Create($payloadFile)
        try {
          $reader.Position = $bounds.Start
          $buffer = New-Object byte[] (1MB)
          $left = $length
          while ($left -gt 0) {
            $read = $reader.Read($buffer, 0, [int][Math]::Min($buffer.Length, $left))
            if ($read -le 0) { throw "读取内层安装包时提前结束（还差 $left 字节）" }
            $writer.Write($buffer, 0, $read)
            $left -= $read
          }
        } finally { $writer.Dispose() }
      } finally { $reader.Dispose() }
      $archive = $payloadFile
      "外层壳: 0x{0:x}-0x{1:x} 是内层 NSIS 安装包（{2:N1} MB）" -f $bounds.Start, $bounds.End, ($length / 1MB)
    }

    & $sevenZip x $archive "-o$extractDir" -y -bso0 -bsp0
    if ($LASTEXITCODE -ne 0) { throw "7z 解包失败，退出码 $LASTEXITCODE（包: $archive）" }
    if ($payloadFile) { Remove-Item -LiteralPath $payloadFile -Force -ErrorAction SilentlyContinue }

    $expectedSize = if ($lock -and $lock.official_exe_size) { [long]$lock.official_exe_size } else { $null }
    $inner = Select-OfficialExe -dir $extractDir -expectedSize $expectedSize
    if (-not $inner) {
      throw "解包后未找到主程序。目录内容：$((Get-ChildItem $extractDir -Recurse -File | Select-Object -First 10 -ExpandProperty Name) -join ', ')"
    }
    "内层主程序: $($inner.Name)  ({0:N1} MB)" -f ($inner.Length / 1MB)
    $sourceExe = $inner.FullName
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

$expected = if ($lock -and [string]$lock.client_version -eq $target) { [string]$lock.official_exe_sha256 } else { '' }
if ($expected -and $expected -ne $sha) {
  # 锁是构建输入：字节对不上就说明拿到的不是锁里那一份，不能继续编译。
  throw "基底哈希与 upstream-win.json 不符（期望 $expected，实际 $sha）。请先更新基线锁，不要用未核对的原包构建。"
}
if ($expected) {
  '哈希  : 与锁定值一致'
  $match = $true
} else {
  '哈希  : 无锁定值可比对（非适配版本）'
  $match = $null
}

# 哈希只证明「和锁里记的是同一份」，而锁本身也可能被改。
# 签名由库洛私钥产生，是唯一独立于本仓库的凭据，所以两条都要过。
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
