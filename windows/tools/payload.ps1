# app\ 里哪些文件该进发行包：以 shared/contracts/windows-update.json 为白名单单一源，
# zip、NSIS 安装包与原地更新共用同一边界；用户数据、token、缓存、Mod 和日志绝不带进发行包。
$script:PayloadContractPath = Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) 'shared\contracts\windows-update.json'

function Get-WuwaPayloadFiles {
  param(
    [Parameter(Mandatory)][string]$Source,
    [switch]$NoRuntime
  )

  $contractPath = $script:PayloadContractPath
  if (-not (Test-Path -LiteralPath $contractPath)) { throw "缺少共享更新契约：$contractPath" }
  $contract = Get-Content -LiteralPath $contractPath -Raw -Encoding UTF8 | ConvertFrom-Json

  $protected = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($item in $contract.protected) { [void]$protected.Add([string]$item) }

  $topAllowed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($item in $contract.topLevel) { [void]$topAllowed.Add([string]$item) }
  foreach ($extra in @('start.cmd', '启动.cmd')) { [void]$topAllowed.Add($extra) }

  $dirAllowed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($item in $contract.directories) { [void]$dirAllowed.Add([string]$item) }

  Get-ChildItem -LiteralPath $Source -Recurse -File -ErrorAction SilentlyContinue | Where-Object {
    $rel = $_.FullName.Substring($Source.Length).TrimStart('\')
    $parts = $rel -split '\\'
    if ($parts.Count -eq 1) {
      return (-not $protected.Contains($parts[0])) -and $topAllowed.Contains($parts[0])
    }
    $topDir = $parts[0]
    if ($topDir -ieq 'runtime') { return -not $NoRuntime }
    return (-not $protected.Contains($topDir)) -and $dirAllowed.Contains($topDir)
  }
}
