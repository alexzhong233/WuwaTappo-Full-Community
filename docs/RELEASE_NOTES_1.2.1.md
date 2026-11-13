# WuwaTappo Full Community 1.2.1 预览版

将 Wuwa Tappo Mac 原生适配与 FullAtlas v1.2 集成，当前使用官方国服 **1.0.2**。适用于 Apple Silicon、macOS 13 及以上。

- 在原有面板中加入本地全图鉴、桌面助手、统计与开箱记录。
- 本地图鉴不改变账号资产；开启时不能进入联机房间。
- 使用 Rust / Tauri 本机桥，替代 Windows 本地服务和 Windows 更新器。
- 图鉴目录可联网刷新，图像仍需官方 CDN。

下载 DMG 后拖入「应用程序」；ZIP 内的应用相同。首次使用需按系统提示授权输入监控。临时签名，未经过 Apple 公证。

当前验证包括 135 项 JavaScript 测试、4 项 FullAtlas 专项测试、开箱回归、81 项 Rust 测试以及本机构建、签名与 DMG 校验。该结果来自 1.0.2 构建，真实登录、系统授权、图鉴切换和联机仍需用户实机验收。

官方 1.0.3 在独立实验分支适配，不包含在此预览版中。Mac 程序暂不支持自动安装更新，请使用本项目的 Mac 安装包。

感谢 [Last-emo-boy/wuwa-tappo-mac](https://github.com/Last-emo-boy/wuwa-tappo-mac) 与 [Averionx/WuwaTappo-FullAtlas](https://github.com/Averionx/WuwaTappo-FullAtlas)。此项目为非官方社区整合，官方资源归原权利人所有。
