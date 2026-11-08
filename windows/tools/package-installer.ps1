<#
  把装配好的 app\ 打成 Windows 安装包（NSIS）

  产物： dist\WuwaTappo-Full-CN-Community-Windows-x64-<version>.exe
         dist\...-Windows-x64-<version>.exe.sha256

  按用户安装到 %LOCALAPPDATA%\Programs\<产品名>，不需要管理员权限；
  装完桌面与开始菜单各有快捷方式，双击即用，控制面板里可卸载。

  界面用 MUI2：欢迎页 → 选目录 → 安装进度 → 完成页（可直接勾选启动）。
  快捷方式的图标取自 app\app.ico（deploy.ps1 从官方客户端 exe 里提取）。

  版本号取自仓库根的 package.json（与 macOS 产物保持一致）。

  用法：
    powershell -File windows\tools\package-installer.ps1
    powershell -File windows\tools\package-installer.ps1 -Region global
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
. (Join-Path $PSScriptRoot 'payload.ps1')
$R = Get-RegionProfile -Root $Root -Region $Region

# 不依赖 Get-FileHash：从 pwsh 7 环境启动 PS 5.1 时该 cmdlet 可能加载不到
function Get-Sha256Hex([string]$path) {
  $algorithm = [Security.Cryptography.SHA256]::Create()
  $stream = [IO.File]::OpenRead($path)
  try { $bytes = $algorithm.ComputeHash($stream) } finally { $stream.Dispose(); $algorithm.Dispose() }
  return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}

function Find-Makensis {
  $candidates = @()
  $command = Get-Command makensis -ErrorAction SilentlyContinue
  if ($command) { $candidates += $command.Source }
  $programFilesX86 = ${env:ProgramFiles(x86)}
  if ($programFilesX86) { $candidates += (Join-Path $programFilesX86 'NSIS\makensis.exe') }
  if ($env:ProgramFiles) { $candidates += (Join-Path $env:ProgramFiles 'NSIS\makensis.exe') }
  foreach ($candidate in $candidates) {
    if ($candidate -and (Test-Path -LiteralPath $candidate)) { return $candidate }
  }
  return $null
}

if (-not $Source) { $Source = $R.AppDir }
if (-not $OutDir) { $OutDir = Join-Path $RepoRoot 'dist' }

$version = (Get-Content -LiteralPath (Join-Path $RepoRoot 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json).version
$name = "$($R.ArtifactPrefix)-Windows-x64-$version"
$exe = Join-Path $OutDir "$name.exe"

if (-not (Test-Path -LiteralPath $Source)) { throw "找不到待打包目录：$Source（先跑 build.ps1 / deploy.ps1）" }
if (-not (Test-Path -LiteralPath (Join-Path $Source 'WuwaTappo.Local.exe'))) { throw "目录里没有 WuwaTappo.Local.exe，可能是空部署：$Source" }

# 入口必须是 GUI 子系统的启动器：指向 start.cmd 会先弹一个控制台黑框，
# 快捷方式图标也会变成 cmd 的。缺了就报错，不要悄悄退回去——那是上一版的真实故障。
$entry = 'WuwaTappoLauncher.exe'
if (-not (Test-Path -LiteralPath (Join-Path $Source $entry))) {
  throw "缺少无控制台启动器 $entry：先跑 tools\build-launcher.ps1 -Region $($R.Id)。退用 start.cmd 会让安装包弹黑框、图标错误。"
}

$makensis = Find-Makensis
if (-not $makensis) {
  throw "找不到 makensis：先装 NSIS（choco install nsis -y），或把 makensis.exe 加进 PATH。"
}

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
if (Test-Path -LiteralPath $exe) { Remove-Item -LiteralPath $exe -Force }

$staging = Join-Path $R.WorkDir 'installer-staging'
if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
$payload = Join-Path $staging 'payload'
New-Item -ItemType Directory -Path $payload -Force | Out-Null

Write-Host "== 收集文件（入口：$entry）=="
$files = Get-WuwaPayloadFiles -Source $Source -NoRuntime:$NoRuntime
foreach ($f in $files) {
  $rel = $f.FullName.Substring($Source.Length).TrimStart('\')
  $dest = Join-Path $payload $rel
  New-Item -ItemType Directory -Path (Split-Path $dest -Parent) -Force | Out-Null
  Copy-Item -LiteralPath $f.FullName -Destination $dest -Force
}
Write-Host ("共 {0} 个文件，{1:N1} MB" -f $files.Count, (($files | Measure-Object -Property Length -Sum).Sum / 1MB))

# 图标：优先用装配时放进来的 app.ico，退回官方客户端里提取的原件
$icon = Join-Path $Source 'app.ico'
if (-not (Test-Path -LiteralPath $icon)) {
  $icon = Get-ChildItem -LiteralPath (Join-Path $R.WorkDir 'official') -Recurse -Filter 'icon_32512.ico' -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if ($icon) { $icon = $icon.FullName }
}
# MUI_ICON 必须在 !include MUI2.nsh 之前定义；没有图标就用 NSIS 默认
$muiIcon = if ($icon -and (Test-Path -LiteralPath $icon)) {
  '!define MUI_ICON "' + $icon + '"' + "`r`n" + '!define MUI_UNICON "' + $icon + '"'
} else { '; 未找到官方图标，使用 NSIS 默认图标' }
$iconFile = if ($icon -and (Test-Path -LiteralPath $icon)) { $icon } else { '' }

$nsi = Join-Path $staging 'installer.nsi'
$template = @'
Unicode true
SetCompressor /SOLID lzma

; MUI_ICON / MUI_UNICON 必须写在 include 之前
{{MUIICON}}

!include MUI2.nsh
!include LogicLib.nsh

!define PRODUCT_NAME "{{PRODUCT}}"
!define PRODUCT_VERSION "{{VERSION}}"
!define PRODUCT_ENTRY "{{ENTRY}}"

Name "{{PRODUCT}}"
OutFile "{{PKG}}"
InstallDir "$LOCALAPPDATA\Programs\{{PRODUCT}}"
InstallDirRegKey HKCU "Software\{{PRODUCT}}" "InstallDir"
RequestExecutionLevel user
ShowInstDetails show
ShowUninstDetails show
BrandingText "{{PRODUCT}} {{VERSION}}"

!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE_3LINES
!define MUI_WELCOMEPAGE_TITLE "欢迎安装 {{PRODUCT}}"
!define MUI_WELCOMEPAGE_TEXT "这是一个本地运行的社区整合版客户端。$\r$\n$\r$\n· 装到当前用户目录，不需要管理员权限$\r$\n· 桌面和开始菜单各建一个快捷方式$\r$\n· 依赖系统的 Microsoft Edge WebView2 运行时（Windows 11 自带）$\r$\n$\r$\n点「下一步」继续。"
!define MUI_DIRECTORYPAGE_TEXT_TOP "请选择安装位置。装好后这个目录里会有程序本体、本地素材服务和运行状态。"
!define MUI_FINISHPAGE_TITLE "{{PRODUCT}} 已就绪"
!define MUI_FINISHPAGE_TEXT "装好了。桌面上的快捷方式可以直接双击启动。$\r$\n$\r$\n首次启动会拉起本地素材服务，需要几秒才出窗口。"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${PRODUCT_ENTRY}"
!define MUI_FINISHPAGE_RUN_TEXT "立即启动 {{PRODUCT}}"
!define MUI_FINISHPAGE_NOREBOOTSUPPORT

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

; 界面语言跟着系统走：简体中文系统出中文，其它出英文
Function .onInit
  System::Call 'kernel32::GetUserDefaultUILanguage() i .r0'
  ${If} $0 == 2052
    StrCpy $LANGUAGE 2052
  ${Else}
    StrCpy $LANGUAGE 1033
  ${EndIf}
FunctionEnd

; 官方客户端不内嵌 WebView2，直接用系统运行时；缺失时只有双击那一刻才由 wry 报错。
; 这里把同一个判断提前到安装阶段：不下载、不静默安装，只给官方 Evergreen 入口。
; 内嵌了 runtime\ 的完整包（构建时没带 -NoRuntime）跳过检查。
Function CheckWebView2
  ${If} ${FileExists} "$INSTDIR\runtime\msedgewebview2.exe"
    DetailPrint "已内嵌 WebView2 运行时，跳过系统运行时检查"
    Return
  ${EndIf}
  StrCpy $0 ""
  SetRegView 64
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${If} $0 == ""
    SetRegView 32
    ReadRegStr $0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  SetRegView lastused
  ${If} $0 == ""
    ReadRegStr $0 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  ${If} $0 != ""
    DetailPrint "WebView2 运行时: $0"
    Return
  ${EndIf}
  DetailPrint "未检测到 Microsoft Edge WebView2 运行时"
  MessageBox MB_YESNO|MB_ICONEXCLAMATION \
    "未检测到 Microsoft Edge WebView2 运行时。$\r$\n$\r$\n程序靠它显示界面，缺失时双击会直接报错退出。$\r$\n$\r$\n是否现在打开官方下载页？装完再启动本程序。" \
    IDNO wv2_skip
  ExecShell "open" "https://go.microsoft.com/fwlink/p/?LinkId=2124703"
  wv2_skip:
FunctionEnd

; 覆盖安装时程序若在运行，文件会被占用。先安静地结束掉，避免出现「装到一半失败」。
Function StopRunningApp
  nsExec::Exec 'taskkill /F /IM {{ENTRY}}'
  Pop $0
  nsExec::Exec 'taskkill /F /IM WuwaTappo.Local.exe'
  Pop $0
  nsExec::Exec 'taskkill /F /IM LocalGalleryHost.exe'
  Pop $0
  Sleep 500
FunctionEnd

Section "Install"
  SetShellVarContext current
  SetOutPath "$INSTDIR"

  Call StopRunningApp
  File /r "{{PAYLOAD}}\*.*"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  Call CheckWebView2

  ; 快捷方式都带图标，否则会显示成启动器的默认小图标
  {{SHORTCUTICON}}
  CreateDirectory "$SMPROGRAMS\{{PRODUCT}}"
  CreateShortCut "$SMPROGRAMS\{{PRODUCT}}\{{PRODUCT}}.lnk" "$INSTDIR\${PRODUCT_ENTRY}" "" "{{ICONARG}}" 0
  CreateShortCut "$SMPROGRAMS\{{PRODUCT}}\卸载 {{PRODUCT}}.lnk" "$INSTDIR\uninstall.exe"
  CreateShortCut "$DESKTOP\{{PRODUCT}}.lnk" "$INSTDIR\${PRODUCT_ENTRY}" "" "{{ICONARG}}" 0

  WriteRegStr HKCU "Software\{{PRODUCT}}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{PRODUCT}}" "DisplayName" "{{PRODUCT}}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{PRODUCT}}" "DisplayVersion" "{{VERSION}}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{PRODUCT}}" "DisplayIcon" "{{DISPLAYICON}}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{PRODUCT}}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{PRODUCT}}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{PRODUCT}}" "Publisher" "WuwaTappo Full Community"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{PRODUCT}}" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{PRODUCT}}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  nsExec::Exec 'taskkill /F /IM {{ENTRY}}'
  Pop $0
  nsExec::Exec 'taskkill /F /IM WuwaTappo.Local.exe'
  Pop $0
  nsExec::Exec 'taskkill /F /IM LocalGalleryHost.exe'
  Pop $0
  Sleep 500

  Delete "$DESKTOP\{{PRODUCT}}.lnk"
  Delete "$SMPROGRAMS\{{PRODUCT}}\{{PRODUCT}}.lnk"
  Delete "$SMPROGRAMS\{{PRODUCT}}\卸载 {{PRODUCT}}.lnk"
  RMDir "$SMPROGRAMS\{{PRODUCT}}"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{PRODUCT}}"
  DeleteRegKey HKCU "Software\{{PRODUCT}}"

  ; 登录状态和素材缓存动辄上百 MB，重装就要重新同步；让用户自己决定留不留
  MessageBox MB_YESNO|MB_ICONQUESTION \
    "是否同时删除本地数据？$\r$\n$\r$\n· 是：素材缓存、登录状态一起清掉，重装后要重新同步$\r$\n· 否：保留这些数据，重装后可以直接接着用" \
    IDNO keep_data
  RMDir /r "$INSTDIR"
  Goto uninst_done
  keep_data:
    Delete "$INSTDIR\WuwaTappo.Local.exe"
    Delete "$INSTDIR\LocalGalleryHost.exe"
    Delete "$INSTDIR\{{ENTRY}}"
    Delete "$INSTDIR\app.ico"
    Delete "$INSTDIR\start.cmd"
    Delete "$INSTDIR\启动.cmd"
    Delete "$INSTDIR\launch.ps1"
    Delete "$INSTDIR\initialize.ps1"
    Delete "$INSTDIR\restore_network.ps1"
    Delete "$INSTDIR\README.md"
    Delete "$INSTDIR\LICENSE"
    Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
    Delete "$INSTDIR\package_config.json"
    Delete "$INSTDIR\uninstall.exe"
    RMDir /r "$INSTDIR\_internal"
    RMDir /r "$INSTDIR\host"
    RMDir /r "$INSTDIR\docs"
    RMDir "$INSTDIR"
  uninst_done:
SectionEnd
'@

$shortcutIcon = if ($iconFile) { '' } else { '; 无图标文件，快捷方式沿用可执行文件自带图标' }
$iconArg = if ($iconFile) { '$INSTDIR\app.ico' } else { '' }
$displayIcon = if ($iconFile) { '$INSTDIR\app.ico' } else { '$INSTDIR\' + $entry }

$script = $template
$script = $script.Replace('{{MUIICON}}', $muiIcon)
$script = $script.Replace('{{PRODUCT}}', $R.ProductName)
$script = $script.Replace('{{VERSION}}', $version)
$script = $script.Replace('{{ENTRY}}', $entry)
$script = $script.Replace('{{SHORTCUTICON}}', $shortcutIcon)
$script = $script.Replace('{{ICONARG}}', $iconArg)
$script = $script.Replace('{{DISPLAYICON}}', $displayIcon)
$script = $script.Replace('{{PKG}}', $exe)
$script = $script.Replace('{{PAYLOAD}}', $payload)
[IO.File]::WriteAllText($nsi, $script, (New-Object Text.UTF8Encoding($true)))

Write-Host "== 编译安装包（makensis）=="
& $makensis /V2 $nsi
if ($LASTEXITCODE -ne 0) { throw "makensis 失败，退出码 $LASTEXITCODE" }
if (-not (Test-Path -LiteralPath $exe)) { throw "没有生成安装包：$exe" }

if (-not $KeepStaging) { Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue }

# 程序内升级包：同一份 payload 再打一个带清单的 zip，供桌宠里的「一键更新」下载。
# 安装包本身仍然是 exe；zip 只走应用内更新，用户不会看到它。
Write-Host "== 生成程序内升级包 =="
$py = if ($env:WUWA_PYTHON) { $env:WUWA_PYTHON } else { Join-Path $Root '.venv\Scripts\python.exe' }
if (-not (Test-Path -LiteralPath $py)) {
  $command = Get-Command python -ErrorAction SilentlyContinue
  if (-not $command) { throw "找不到 python，无法生成程序内升级包（先跑 tools\setup-env.ps1）" }
  $py = $command.Source
}
& $py (Join-Path $Root 'src\tools\make_update.py') $R.Id
if ($LASTEXITCODE -ne 0) { throw "生成程序内升级包失败，退出码 $LASTEXITCODE" }
$zip = Join-Path $OutDir "$($R.ArtifactPrefix)-$version-win-x64-update.zip"
if (-not (Test-Path -LiteralPath $zip)) { throw "没有生成程序内升级包：$zip" }
$zipSha = Get-Sha256Hex $zip
"$zipSha  $(Split-Path $zip -Leaf)" | Set-Content -LiteralPath "$zip.sha256" -Encoding ASCII
Write-Host ("  升级包: {0}（{1:N1} MB）" -f $zip, ((Get-Item -LiteralPath $zip).Length / 1MB))

$exeSize = (Get-Item -LiteralPath $exe).Length
$sha = Get-Sha256Hex $exe
"$sha  $name.exe" | Set-Content -LiteralPath "$exe.sha256" -Encoding ASCII

''
'打包完成'
"  文件: $exe"
"  大小: {0:N1} MB" -f ($exeSize / 1MB)
"  sha256: $sha"
