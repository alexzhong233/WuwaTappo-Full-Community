# macOS 输入监控授权

应用用 `CGEventTapCreate`（HID 层、只监听）读取键鼠事件。该能力由「系统设置 > 隐私与安全性 > 输入监控」授权，
授权记录写在 `~/Library/Application Support/com.apple.TCC/TCC.db`，并由代码签名身份（cdhash / csreq）绑定。

## 症状：功能正常，但设置列表里没有本应用

重装后常见：桌宠能响应键鼠，但「输入监控」列表里找不到本应用，也就无法查看或撤销授权。

三个原因会叠加出现：

1. **授权请求被跳过。** 旧版本的授权请求只在 `CGPreflightListenEventAccess()` 返回 false 时发起。
   重装后残留的 TCC 记录仍让 preflight 返回 true，于是应用从不重新请求，系统设置里的那一行
   也就不会被重建——功能依赖旧记录继续工作。
2. **临时签名（adhoc）。** `tauri.conf.json` 的 `signingIdentity: "-"` 意味着每次构建都是新的
   cdhash，TCC 记录随即失效，但旧记录不会被删除，列表里会留下对不上号的条目。
3. **从 DMG 直接运行。** Gatekeeper 会把应用放到随机的 `AppTranslocation` 路径，授权记录指向
   这个临时路径，下次启动必然对不上。

## 应用侧行为

- 启动时若输入监控已授权，会静默调用 `CGRequestListenEventAccess()` 与
  `IOHIDRequestAccess(kIOHIDRequestTypeListenEvent)` 重新登记条目。已授权时这两个调用不弹窗、
  不改变现有授权，只是让系统把本应用重新写进列表。
- 主菜单 →「启用键鼠监听…」会无条件发起一次授权请求：未授权时弹系统提示，已授权时仅补登记，
  即修复「列表里没有本应用」的情况。
- 启动日志会打印一行 `Input access: input-monitoring=… hid=… accessibility=… translocated=… hint=…`，
  只包含权限状态与路径，不含任何输入内容。日志可在菜单「打开日志目录」中查看。

## 手动修复

只做诊断（不改动任何授权）：

```bash
bash macos/scripts/repair-input-permission.sh
```

清掉失效记录后重新授权（会撤销当前授权，需要重新勾选）：

```bash
bash macos/scripts/repair-input-permission.sh --reset
```

重置后打开应用，走一次「启用键鼠监听…」，允许即可。设置面板若仍无条目，手动把应用从列表移除再重新添加。

## 发布侧：使用固定签名

临时签名是上述问题的根因。发布前设置固定的开发者证书，让授权跨版本保留：

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
```

`macos/scripts/package-macos.sh` 检测到该变量时会用它重新签名（`--options runtime --timestamp`，
保留原有 entitlements）。未设置时若仍是 adhoc 签名，构建会打印警告但不阻断。

分发时必须在说明里要求：**把应用拖入「应用程序」后再运行**，不要直接在 DMG 里双击。

## 验收

1. 冷启动应用，允许输入监控；
2. 「系统设置 > 隐私与安全性 > 输入监控」中能看到本应用且开关为开；
3. 退出并重新安装同一安装包，列表条目仍在、开关仍为开，桌宠能响应键鼠与快捷键；
4. 日志中 `hint=ok`。
