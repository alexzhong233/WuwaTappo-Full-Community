# WuwaTappo Full Community 1.2.3

基于官方 **1.0.3**（国服与国际服各锁一份）的实验版，两个平台、两个区服一起发。

## 这次的变化

- **安装包取代绿色包**。Windows 从 zip 换成 NSIS 安装包：按用户装到 `%LOCALAPPDATA%\Programs\`，不需要管理员权限，装完桌面和开始菜单各有一个快捷方式，双击即用；控制面板可卸载。macOS 只发 `.dmg`，拖进「应用程序」即可。
- **国际服两端都能构建**。此前只有 macOS 侧保留国际服代码、且不作为发行目标；现在国服 / 国际服在 macOS 与 Windows 上都有独立产物，基线与资源锁互不影响。
- **命名统一**，一眼能分清区服与平台：
  - `WuwaTappo-Full-CN-Community-<版本>-macOS-arm64.dmg`
  - `WuwaTappo-Full-Global-Community-<版本>-macOS-arm64.dmg`
  - `WuwaTappo-Full-CN-Community-Windows-x64-<版本>.exe`
  - `WuwaTappo-Full-Global-Community-Windows-x64-<版本>.exe`
- **发版门槛收紧**。两个平台、两个区服必须全部构建成功才创建 Release，缺一个就不发，不会出现只发了一半平台的版本。
- **每日官方检测覆盖两服**。此前只盯国服，官方发新版本时国际服不会有人提醒。

## 安装

- **macOS**：Apple Silicon / macOS 13 及以上，下载对应区服的 `.dmg`，拖入「应用程序」。应用使用 Ad hoc 签名，未通过 Apple 公证，首次打开如被拦截请在「系统设置 → 隐私与安全性」里放行。
- **Windows**：Windows 10/11 x64，下载对应区服的 `.exe` 运行安装。装完用桌面快捷方式启动。首次启动会请求管理员权限（本地素材服务需要）。

## 已知限制

**实验预发布，尚未完成完整业务验收。** 登录、输入监控授权后的计数与快捷键、多屏、助手任务以及联机仍需继续验证。

- 与 1.2.1 / 1.2.2 共用应用标识与数据目录，试用前请退出另一个副本并备份数据（macOS：`~/Library/Application Support/dev.w33d.wuwatappo.fullatlas.mac/`）。
- 程序内自动更新保持关闭：改过的客户端过不了官方签名校验，更新只能整包替换。
- 不支持把官方 Windows 更新包直接装到 macOS。
