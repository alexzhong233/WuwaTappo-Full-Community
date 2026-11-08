#!/bin/bash
# 输入监控授权自检与修复。
#
# 只在显式加 --reset 时才改动系统授权：重置会撤销当前授权，需要重新在系统设置里勾选。
# 不带参数运行时只输出诊断信息，不做任何修改。
set -Eeuo pipefail

CANDIDATES=(
  "/Applications/WuwaTappo Full Community.app"
  "/Applications/WuwaTappo Global Community.app"
)
APP=''
RESET=0

usage() {
  cat <<'EOF'
用法：bash macos/scripts/repair-input-permission.sh [--app 应用路径] [--reset]

  --app 应用路径   指定要检查的 .app，默认在 /Applications 下查找
  --reset          重置该应用的输入监控授权，之后需要重新授权
  -h, --help       显示本说明
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --app)
      [ "$#" -ge 2 ] || { usage >&2; exit 2; }
      APP="$2"; shift 2 ;;
    --reset)
      RESET=1; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      usage >&2; exit 2 ;;
  esac
done

if [ "$(uname -s)" != Darwin ]; then
  printf '%s\n' '该脚本只能在 macOS 上运行。' >&2
  exit 1
fi

if [ -z "$APP" ]; then
  for candidate in "${CANDIDATES[@]}"; do
    if [ -d "$candidate" ]; then APP="$candidate"; break; fi
  done
fi
if [ -z "$APP" ] || [ ! -d "$APP" ]; then
  printf '%s\n' '未找到应用，请用 --app 指定 .app 的完整路径。' >&2
  exit 1
fi
APP="$(cd "$APP/.." && pwd)/$(basename "$APP")"
PLIST="$APP/Contents/Info.plist"
if [ ! -f "$PLIST" ]; then
  printf '不是完整的应用包：%s\n' "$APP" >&2
  exit 1
fi

IDENTIFIER=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$PLIST")
VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$PLIST" 2>/dev/null || printf '未知')

printf '应用：%s\n' "$APP"
printf '标识：%s\n版本：%s\n' "$IDENTIFIER" "$VERSION"

SIGN_INFO=$(/usr/bin/codesign -dv --verbose=4 "$APP" 2>&1 || true)
if printf '%s' "$SIGN_INFO" | grep -q 'Signature=adhoc'; then
  printf '%s\n' '签名：临时签名（adhoc）。重装或升级后授权可能失效，设置列表也可能出现旧条目。'
else
  printf '签名：%s\n' "$(printf '%s' "$SIGN_INFO" | sed -n 's/^Authority=//p' | head -1)"
fi

case "$APP" in
  /Applications/*)
    printf '%s\n' '位置：/Applications，正常。' ;;
  *)
    printf '%s\n' '位置：不在 /Applications。从 DMG 直接运行会被 Gatekeeper 放到随机路径，' \
      '授权无法跨启动保留，请先把应用拖入「应用程序」文件夹。' ;;
esac

printf '%s\n' '查看已记录的输入监控条目（终端需要有「完全磁盘访问」权限，否则读不到）：'
printf '  sqlite3 "$HOME/Library/Application Support/com.apple.TCC/TCC.db" "select client,auth_value from access where service='"'"'kTCCServiceListenEvent'"'"'"\n'

if [ "$RESET" -eq 0 ]; then
  printf '\n%s\n' '未做任何修改。确认要重建授权时，加 --reset 重新运行。'
  exit 0
fi

printf '\n重置 %s 的输入监控授权…\n' "$IDENTIFIER"
/usr/bin/tccutil reset ListenEvent "$IDENTIFIER" || {
  printf '%s\n' 'tccutil 重置失败。可在「系统设置 > 隐私与安全性 > 输入监控」里手动移除本应用后重试。' >&2
  exit 1
}
printf '%s\n' \
  '已重置。接下来：' \
  '  1. 打开应用，在主菜单 → 应用名 → 「启用键鼠监听…」重新授权；' \
  '  2. 在弹出的系统提示中允许；若没有提示，到「系统设置 > 隐私与安全性 > 输入监控」手动勾选。'
