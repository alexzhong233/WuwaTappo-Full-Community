# 官方上游 HTTP、URL 校验与哈希工具。fetch/check/update/package 共享这一份。
function Get-Json([string]$url, [string]$UserAgent = 'WuwaTappo-Full-Community') {
  try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }
  $wc = New-Object System.Net.WebClient
  $wc.Proxy = $null
  $wc.Encoding = [Text.Encoding]::UTF8
  $wc.Headers.Add('User-Agent', $UserAgent)
  try { return ($wc.DownloadString($url) | ConvertFrom-Json) } finally { $wc.Dispose() }
}

function Assert-OfficialUrl([string]$url, [string]$expectedHost, [string]$expectedLeaf = '') {
  try { $u = [Uri]$url } catch { throw '官方资源地址不是有效 URL' }
  $hostName = ([Uri]$expectedHost).Host
  if ($u.Scheme -ne 'https' -or $u.Host -ne $hostName -or $u.Port -ne 443 -or $u.UserInfo -or $u.Fragment) {
    throw "官方资源地址不是当前区域的 HTTPS 官方地址: $url"
  }
  $leaf = [IO.Path]::GetFileName($u.AbsolutePath)
  if ($expectedLeaf -and $leaf -ne $expectedLeaf) { throw "官方原包文件名与版本或区域不匹配: $leaf" }
  return $u.AbsoluteUri
}

function Get-RemoteManifest([string]$AssetsHost, [string]$UpdaterHostFile, [string]$UserAgent = 'WuwaTappo-Full-Community') {
  $hostJson = Get-Json "$AssetsHost/app/json-host/$UpdaterHostFile" $UserAgent
  if (-not $hostJson.url) { throw '官方版本指针未返回 url 字段' }
  return Get-Json (Assert-OfficialUrl ([string]$hostJson.url) $AssetsHost) $UserAgent
}

function Get-Sha256Hex([string]$path) {
  $algorithm = [Security.Cryptography.SHA256]::Create()
  $stream = [IO.File]::OpenRead($path)
  try { $bytes = $algorithm.ComputeHash($stream) } finally { $stream.Dispose(); $algorithm.Dispose() }
  return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}
