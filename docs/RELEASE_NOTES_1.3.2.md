# Release Notes — 1.3.2

本版是自 `1.3.1` 后的正式功能与架构大版本更新：重构双端自动弹表情与控制台体验、打通 macOS/Windows 应用内自动更新与统一版本展示、完成双端 Rust 核心服务与纯 Rust + Node 构建链统一（全仓零 Python 依赖），并继续基于官方国服 / 国际服 `1.0.4` 构建。

## 自动弹表情重构与控制台增强

- **不再抢占辅助功能权限**：移除 macOS 对系统的辅助功能（Accessibility）授权弹窗与输入框监听需求，双端统一改为跟随用户真实点击/按键次数触发随机/指定表情，并与自动点击（`F24`）自动避让，避免冲突触发。
- **灵活间隔与随机开关**：控制台新增触发间隔设置及「随机」开关——开启时按设定间隔 `±40%` 随机波动触发，关闭时按固定次数精确触发。
- **精简槽位选择器**：自动表情槽位按钮精简为直观的 `全部 / 1 / 2 / 3 / 4 / 5` 多选切换。

## 自动更新与双端版本展示统一

- **自动更新修复与防限流**：修复控制台「更新」页首次渲染报错与检查状态锁死问题；在 `shared/backend` 引入 15 分钟 `UpdateTracker` 内存缓存并拆分 `/update/status` 与 `/update/check`，避免轮询触发 GitHub API 限流。
- **macOS 原生应用内更新**：支持一键下载 DMG、校验代码签名与版本清单、原子替换 `/Applications` 中的应用并自动重启，失败时自动回滚（`mac_updater.rs`）；Windows 端补充原地升级各阶段的 `.updates/install-current.json` 实时进度落盘。
- **版本展示统一**：官方「设置 → 关于」与版本接口统一显示 `<整合版版本> · 基底 <官方版本>`（如 `1.3.2 · 基底 1.0.4`），Windows 安装与原地升级时自动刷新 `local_config.json` 公开配置并保留用户 `token` 与 `port`。

## 双端架构统一与纯 Rust + Node 工具链

- **共享 `FullAtlasService` 核心**：抽取 `shared/backend/src/service.rs`，统一 macOS（Tauri IPC）与 Windows（本机 HTTP）的图鉴同步、Mod 叠加、设置持久化和更新状态逻辑。
- **统一前端单一源与 macOS 离线缓存**：消除 `macos/fullatlas-source/` 与 `windows/src/src/` 的历史生成副本，右键菜单模块同步收敛至 `shared/frontend/`；macOS 移除构建期 `fullatlas-catalog.json` 打包，改为运行时统一校验缓存并通过 `mod_assets.rs` 提供离线官方 PNG 与 Mod 资源读取。
- **构建期与运行期零 Python 依赖**：将前端注入、Windows PE64 资源提取与重打包、`.lnk` 快捷方式二进制写入以及演示 Mod 包生成全部迁入 `shared/backend`（`assistant-core`），结合根目录 `scripts/` 跨平台 Node 脚本运行，移除全仓 `.py` 与 `.venv` 依赖。

## CI 与校验策略优化

- 移除 Windows 官方基底 EXE SHA-256 硬锁与冗余旁路文件，保留库洛官方 Authenticode EV 代码签名校验、Windows 原地升级包（`update.zip`）清单校验与发布 `SHA256SUMS.txt`；日常 CI 门禁仅运行快速单测与双端 Rust 测试。
- 国服与国际服官方客户端基线均确认保持 `1.0.4`。
