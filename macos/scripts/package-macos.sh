#!/bin/bash
set -Eeuo pipefail

PHASE='启动'
failed() {
  local status=$?
  printf '打包失败：%s（第 %s 行，退出状态 %s）。\n' "$PHASE" "$1" "$status" >&2
  exit "$status"
}
trap 'failed "$LINENO"' ERR
phase() {
  PHASE="$1"
  printf '打包阶段：%s\n' "$PHASE"
}

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# ROOT 是 macos/（平台实现根），REPO_ROOT 是仓库根：产物统一落在仓库根的 dist/，与 Windows 侧共用。
REPO_ROOT="$(cd "$ROOT/.." && pwd)"
APP=''
OUTPUT="$REPO_ROOT/dist"

usage() {
  printf '%s\n' '用法：bash macos/scripts/package-macos.sh [--app 完整应用路径] [--output 输出目录]'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --app|--output)
      if [ "$#" -lt 2 ]; then usage >&2; exit 2; fi
      if [ "$1" = --app ]; then APP="$2"; else OUTPUT="$2"; fi
      shift 2
      ;;
    --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

PROFILE=$(node --input-type=module -e '
import { readFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
const root = process.argv[1];
const { getRegionProfile } = await import(pathToFileURL(path.join(root, "scripts/regions.mjs")));
const profile = getRegionProfile();
const base = JSON.parse(await readFile(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
const override = profile.tauriConfig ? JSON.parse(await readFile(path.join(root, profile.tauriConfig), "utf8")) : {};
const config = { ...base, ...override };
const minimum = override.bundle?.macOS?.minimumSystemVersion ?? base.bundle.macOS.minimumSystemVersion;
if (config.identifier !== profile.identifier || config.productName !== profile.productName) throw new Error("构建配置与服务区域不一致。");
console.log([profile.productName, profile.identifier, config.version, minimum, profile.region, profile.preparedDir, profile.artifactPrefix, profile.label].join("\t"));
' "$ROOT")
IFS=$'\t' read -r PRODUCT_NAME EXPECTED_IDENTIFIER EXPECTED_VERSION EXPECTED_MINIMUM EXPECTED_REGION PREPARED_DIR ARTIFACT_PREFIX REGION_LABEL <<< "$PROFILE"
APP="${APP:-$ROOT/src-tauri/target/release/bundle/macos/$PRODUCT_NAME.app}"

if [ "$(uname -s)" != Darwin ]; then
  printf '%s\n' '完整 macOS 安装包需要在 Mac 上生成。' >&2
  exit 1
fi
if [ ! -d "$APP/Contents" ] || [ ! -f "$APP/Contents/Info.plist" ]; then
  printf '%s\n' '未找到完整应用，请先运行 npm run build。' >&2
  exit 1
fi

phase '读取应用信息'
PLIST="$APP/Contents/Info.plist"
IDENTIFIER=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$PLIST")
EXECUTABLE=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$PLIST")
VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$PLIST")
MINIMUM=$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$PLIST")
DISPLAY_NAME=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$PLIST")
if [ "$IDENTIFIER" != "$EXPECTED_IDENTIFIER" ] || [ "$DISPLAY_NAME" != "$PRODUCT_NAME" ] || [ "$VERSION" != "$EXPECTED_VERSION" ] || [ "$MINIMUM" != "$EXPECTED_MINIMUM" ]; then
  printf '应用标识、名称、版本或最低系统要求与%s构建配置不一致。\n' "$REGION_LABEL" >&2
  exit 1
fi
case "$EXECUTABLE" in
  ''|*/*|.|..) printf '%s\n' '应用主程序名称无效。' >&2; exit 1 ;;
esac
BINARY="$APP/Contents/MacOS/$EXECUTABLE"
phase '检查架构与签名'
if [ ! -f "$BINARY" ] || [ "$(/usr/bin/lipo -archs "$BINARY")" != arm64 ]; then
  printf '%s\n' '安装包必须包含 Apple Silicon 原生主程序。' >&2
  exit 1
fi
/usr/bin/codesign --verify --deep --strict "$APP"

# 输入监控授权绑定到代码签名身份。临时签名（adhoc）每次构建的 cdhash 都不同，重装或升级后
# TCC 会留下失效记录：功能可能仍可用，但「系统设置 > 隐私与安全性 > 输入监控」里的条目会消失。
# 设置 APPLE_SIGNING_IDENTITY 后在这里改用固定证书重签，跨版本保留授权。
phase '检查签名身份'
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ] && [ "$APPLE_SIGNING_IDENTITY" != '-' ]; then
  /usr/bin/codesign --force --deep --options runtime --timestamp \
    --preserve-metadata=entitlements --sign "$APPLE_SIGNING_IDENTITY" "$APP"
  /usr/bin/codesign --verify --deep --strict "$APP"
  printf '签名身份：%s\n' "$APPLE_SIGNING_IDENTITY"
else
  SIGN_INFO=$(/usr/bin/codesign -dv --verbose=4 "$APP" 2>&1 || true)
  if printf '%s' "$SIGN_INFO" | grep -q 'Signature=adhoc'; then
    printf '%s\n' \
      '警告：本次为临时签名（adhoc）。输入监控授权与构建绑定，重装后可能需要在系统设置中重新授权，' \
      '「输入监控」列表也可能出现旧条目。正式发布请设置 APPLE_SIGNING_IDENTITY 使用固定开发者证书。' >&2
  else
    printf '签名身份：%s\n' "$(printf '%s' "$SIGN_INFO" | sed -n 's/^Authority=//p' | head -1)"
  fi
fi

# Tauri 将前端编译进主程序；检查打包输入及嵌入路径，而不是复制用户数据。
phase "检查${REGION_LABEL}内置资源"
node --input-type=module -e '
import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
const [root, binary, preparedDir, region, label] = process.argv.slice(1);
const source = path.join(root, preparedDir);
const index = await readFile(path.join(source, "index.html"), "utf8");
const entry = index.match(/src="\.\/([^\"]+\.js)"/)?.[1];
if (!entry) throw new Error(`缺少已准备的${label}前端入口。`);
const main = await readFile(path.join(source, entry), "utf8");
if (!main.includes(`const APP_REGION="${region}"`)) throw new Error(`安装包必须使用${label}前端。`);
const bytes = await readFile(binary);
for (const asset of ["/index.html", `/${entry}`, "/app-icon.png"]) {
  if (!bytes.includes(Buffer.from(asset))) throw new Error(`应用未嵌入必要资源：${asset}`);
}
const contents = await readdir(path.dirname(path.dirname(binary)));
for (const name of ["LICENSE", "THIRD_PARTY_NOTICES.md"]) {
  if (!(await readFile(path.join(path.dirname(path.dirname(binary)), "Resources", name))).length) throw new Error(`缺少发行说明文件：${name}`);
}
for (const name of ["Library", "Logs", "Caches", "Application Support"]) {
  if (contents.includes(name)) throw new Error(`应用中不应包含用户数据目录：${name}`);
}
' "$ROOT" "$BINARY" "$PREPARED_DIR" "$EXPECTED_REGION" "$REGION_LABEL"

phase '创建临时打包目录'
mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"
STAGE=$(mktemp -d "${TMPDIR:-/tmp}/wuwatappo-package.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
NAME="$ARTIFACT_PREFIX-$VERSION-macOS-arm64"
phase '复制应用并复查签名'
ditto --norsrc --noextattr --noqtn --noacl "$APP" "$STAGE/$PRODUCT_NAME.app"
/usr/bin/codesign --verify --deep --strict "$STAGE/$PRODUCT_NAME.app"
phase '生成 DMG'
mkdir "$STAGE/dmg"
mv "$STAGE/$PRODUCT_NAME.app" "$STAGE/dmg/"
ln -s /Applications "$STAGE/dmg/Applications"
IMAGE_SIZE_MIB=$(node "$ROOT/scripts/dmg-size.mjs" "$STAGE/dmg")
printf 'DMG 文件系统：HFS+；容量：%s MiB；主机可用空间：\n' "$IMAGE_SIZE_MIB"
df -h "$STAGE" "$OUTPUT"
hdiutil create -fs HFS+ -size "${IMAGE_SIZE_MIB}m" -volname "$PRODUCT_NAME" -srcfolder "$STAGE/dmg" -format UDZO "$STAGE/$NAME.dmg"
phase '校验 DMG'
hdiutil verify "$STAGE/$NAME.dmg"
phase '交付安装包'
mv "$STAGE/$NAME.dmg" "$OUTPUT/"
printf '完整%s安装包：\n%s\n' "$REGION_LABEL" "$OUTPUT/$NAME.dmg"
