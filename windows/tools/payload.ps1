# app\ 里哪些文件该进发行包：zip 与 NSIS 安装包共用这一份判断。
# 两边各写一遍迟早会漂移——比如运行期状态被漏排除，用户装完就带着别人的 token。
function Get-WuwaPayloadFiles {
  param(
    [Parameter(Mandatory)][string]$Source,
    [switch]$NoRuntime
  )

  $excludeDirs = @()
  if ($NoRuntime) { $excludeDirs += 'runtime' }

  Get-ChildItem -LiteralPath $Source -Recurse -File -ErrorAction SilentlyContinue | Where-Object {
    $rel = $_.FullName.Substring($Source.Length).TrimStart('\')
    $skip = $false
    foreach ($d in $excludeDirs) { if ($rel -eq $d -or $rel.StartsWith("$d\")) { $skip = $true } }
    if ($rel -like 'profile\*') { $skip = $true }
    # 离线素材缓存：几十到上百 MB，且与本机同步进度绑定。发行包走「按需回源官方 CDN」，
    # 和 CI 产物保持一致；否则本机打过一次图鉴，安装包体积就翻倍。
    if ($rel -like 'cache\*') { $skip = $true }
    # 用户导入的 Mod：维护机一旦导入过，发行包里就带着别人的角色图。与 cache\ 同理，
    # 属于运行期状态而非程序本体，发行包必须从空目录开始。
    if ($rel -like 'custom-atlas\*') { $skip = $true }
    # 只排除真正的运行期状态；local_config.json 是部署必需（含本机 HTTP 的 token），必须带上
    if ($rel -in @('offline_state.json', 'assistant_state.json', 'update_hint.json', 'update_state.json')) { $skip = $true }
    # 自提权脚本：部分机器的安全策略禁止读取/分发，且产物用 WuwaTappoLauncher.exe 即进入口
    if ($rel -eq 'launch.ps1') { $skip = $true }
    # 运行期缓存轮换残留与本机日志同样不该进发行包：日志里有本机路径，GitHub 上人人可下
    if ($rel -like 'cache_previous_*') { $skip = $true }
    if ($rel -like '*.log') { $skip = $true }
    -not $skip
  }
}
