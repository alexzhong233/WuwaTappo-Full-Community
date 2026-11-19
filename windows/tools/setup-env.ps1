<#
  创建 Windows 侧独立的 Python 构建环境（.venv）并安装依赖

  解释器查找顺序：
    1. 环境变量 WUWA_PYTHON 指定的路径
    2. 本机已知的托管 Python 3.13（开发者机器）
    3. PATH 上的 python3.13 / python3.12 / python

  用法： powershell -ExecutionPolicy Bypass -File tools\setup-env.ps1 [-Force]
#>
param([switch]$Force)

$ErrorActionPreference = 'Stop'
$Root = Split-Path $PSScriptRoot -Parent
$Venv = Join-Path $Root '.venv'

function Find-Python {
  if ($env:WUWA_PYTHON -and (Test-Path -LiteralPath $env:WUWA_PYTHON)) {
    return $env:WUWA_PYTHON
  }
  # 托管 3.13 优先：PATH 上常见的是 3.14（PyInstaller 6.16 对它有兼容问题）
  foreach ($p in @(
      'C:\Users\eatfo\.workbuddy\binaries\python\versions\3.13.12\python.exe',
      'C:\Users\eatfo\.workbuddy\binaries\python\versions\3.13.6\python.exe'
    )) {
    if (Test-Path -LiteralPath $p) { return $p }
  }
  foreach ($name in @('python3.13', 'python3.12', 'python3', 'python')) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if ($cmd -and $cmd.Source -notmatch 'WindowsApps') { return $cmd.Source }
  }
  throw '找不到 Python。请安装 3.13，或用 WUWA_PYTHON 指向解释器路径。'
}

$Python = Find-Python
$verText = (& $Python --version 2>&1) -join ' '
"Python: $Python"
"版本  : $verText"

if ($Force -and (Test-Path -LiteralPath $Venv)) {
  Remove-Item -LiteralPath $Venv -Recurse -Force
  '已删除旧 .venv（-Force）'
}

if (-not (Test-Path -LiteralPath (Join-Path $Venv 'Scripts\python.exe'))) {
  & $Python -m venv $Venv
  if ($LASTEXITCODE -ne 0) { throw '创建 venv 失败' }
}

$venvPy = Join-Path $Venv 'Scripts\python.exe'
& $venvPy -m pip install -q --disable-pip-version-check --upgrade pip
& $venvPy -m pip install -q --disable-pip-version-check -r (Join-Path $Root 'src\requirements-build.txt')
if ($LASTEXITCODE -ne 0) { throw '安装依赖失败' }

# 探针只输出 ASCII：CI 的 Python stdout 是 cp1252，打印中文会抛 UnicodeEncodeError，
# 看起来像依赖没装好，实际 import 早已成功。同时把退出码纳入判定，别让失败被吞掉。
& $venvPy -c "import pefile, brotli; print('deps ok: pefile ' + pefile.__version__)"
if ($LASTEXITCODE -ne 0) { throw '依赖导入失败，看上方 Python 报错' }
"环境就绪：$Venv"
