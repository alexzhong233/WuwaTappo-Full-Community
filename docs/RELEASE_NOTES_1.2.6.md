# WuwaTappo Full Community 1.2.6

本版本把官方版本检测、原包下载提示和维护环境更新入口接入 macOS 与 Windows，并同步上游图鉴与桌宠控制器改进。

## 更新内容

- macOS 设置面板在官方基底有新版本时提示用户，可打开本项目 Releases，或下载经过校验的官方 Windows 原包到 `Downloads`。
- Windows 发行包在官方有新版本时显示 Releases 和官方原包下载入口；只有维护环境具备完整适配脚本时才提供本地适配操作。
- 官方版本检测使用 HTTPS、区域、文件名和下载大小校验，下载失败不会覆盖已有文件，也不会自动执行官方安装器。
- macOS 与 Windows 的 FullAtlas 助手逻辑继续保持平台契约同步。
- macOS 原生全图鉴后端补充更新状态、下载安全和维护环境处理。

## 安装

- **macOS**：Apple Silicon / macOS 13 及以上，下载对应区服的 `.dmg`，拖入「应用程序」。
- **Windows**：Windows 10/11 x64，下载对应区服的 `.exe` 安装包。

本版本仍基于官方国服 1.0.3 适配基底；官方新版本需要维护者重新验证锚点、资源锁和构建结果后再发布新的整合版本。

## 发行产物

- `WuwaTappo-Full-CN-Community-1.2.6-macOS-arm64.dmg`
- `WuwaTappo-Full-Global-Community-1.2.6-macOS-arm64.dmg`
- `WuwaTappo-Full-CN-Community-Windows-x64-1.2.6.exe`
- `WuwaTappo-Full-Global-Community-Windows-x64-1.2.6.exe`
