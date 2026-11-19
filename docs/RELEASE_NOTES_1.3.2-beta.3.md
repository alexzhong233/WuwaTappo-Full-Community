# Release Notes — 1.3.2-beta.3

本版基于 `1.3.2-beta.2` 修复双端自动更新卡死、版本显示不一致与打字自动弹表情问题，并改造 GitHub CI 门禁与 Windows 官方基底校验策略（双端继续基于官方国服 / 国际服 `1.0.4` 构建）。

## 自动更新修复

- 修复控制台「更新」页因 `fields` 未定义抛 `ReferenceError` 导致首次渲染中断、「检查更新」按钮触发 TDZ 锁死，以及切换「包含预发布版本」后未自动重新检查的问题。
- 在 `shared/backend` 新增 15 分钟 `UpdateTracker` 内存缓存（按当前版本与预发布开关分键）并拆分 `/update/status` 零联网读取与 `/update/check` 检查，防止全局 15 秒轮询触发 GitHub 匿名 60 次/小时限流；失败或无发布时不再将远端版本回写成 `"检测中…"` 或当前本机版本。
- macOS 宿主（`fullatlas.rs`）完整接入 `UpdateTracker` 更新检测与发布说明；Windows 宿主（`release_installer.rs`）补充原地升级下载、解压、应用替换、完成与失败阶段的 `.updates/install-current.json` 实时进度落盘。

## 双端版本显示统一

- 官方「设置 → 关于」与版本检测接口统一显示 `<整合包版本> · 基底 <官方版本>`（如 `1.3.2-beta.3 · 基底 1.0.4`），同时保持上传统计 `cachedAppVersion` 为纯官方版本号。
- 修复 `ensure_local_config` 在 Windows 安装/原地升级后不刷新 `local_config.json` 公开配置的问题，升级时自动同步 `community_version`、`client_version`、`mod_version` 等字段并保留用户 `token` 与 `port`。

## 打字自动弹表情修复

- macOS 端补充 `NSAccessibilityUsageDescription`、系统「辅助功能」授权弹窗引导与托盘/应用菜单「启用辅助功能（打字表情）…」入口；重构 `focused_text_context()` 排除自身窗口与密码框，为 Chromium/Electron 应用自动开启 `AXManualAccessibility`，并扩展 `AXEditableAncestor` / 可写属性判定。
- 修复嵌入式面板与主窗口跨窗口时 `emoteStats` 不同步的问题（通过 `/assistant/tick` / `/assistant/observe` 持久化至 `runtime.emote` 并在控制台设置页实时展示未授权引导按钮），并放宽 Windows UIA 首次焦点探测的 500ms 丢弃窗口以避免首轮触发被吞。

## CI 与门禁改造

- 移除 Windows 官方基底 EXE 的 SHA-256 硬锁与独立 `*.sha256` 旁路文件，保留库洛官方 EV 代码签名校验、Windows 原地升级包（`update.zip`）清单校验与发布 `SHA256SUMS.txt`。
- 清理脆弱的前端快照哈希文件名、硬编码数量与单字母压缩变量断言，拆分 `test:shared` / `test:mac` / `test:win`，日常 push/PR 仅跑跨平台快速单测与 Rust 测试，全量安装包打包集中在 `dev` 分支发布与正式发版工作流执行。
