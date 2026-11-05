# 2026-11-05 上游同步记录

本次在 `upstream-sync-2026-11` 分支重新检查并同步两个上游：

- `Last-emo-boy/wuwa-tappo-mac`：`c4dd4bd9c5e1936ec920c09d9b00d902395eeccd`
- `Averionx/WuwaTappo-FullAtlas`：`2debed109b63824027b04d19fcedf74cca1d3a23`（1.3.6 系列）

Mac 上游同步内容包括：

- 使用 Tauri global shortcut 插件注册真正的全局快捷键，保留官方 Windows-VK 数据格式；设置快捷键时检查冲突，捕获期间释放注册，窗口失焦或页面重载后自动恢复。
- 增加 macOS 前台应用全屏检测，并把它和桌宠窗口自身的全屏状态分开传给前端。
- 修复官方 1.0.3 NSIS 安装包使用变量名 payload 时的静态提取，支持 `WuwaTappo.exe` 和 `$_数字_` 两种形式。
- 设置界面把 Windows 的 `Alt` / `Win` 显示为 macOS 用户看到的 `Option` / `Cmd`，底层快捷键协议保持兼容。
- 菜单栏增加“游戏模式”入口。

FullAtlas 只合入与本项目双平台架构兼容的改动：

- 房间历史支持“删除记录”。删除后后台重复回调不会重新出现，新访问同一房间时恢复记录。
- 房间历史按收藏优先、最近访问倒序排列；Mac Rust 与 Windows Python 两侧行为一致。
- Mac 的 Tauri 传输层和 Windows 本机服务都新增对应的 `new_visit` / `delete-room` 处理及测试。

没有原样覆盖 FullAtlas 1.3.6 中依赖 Windows 原生环境的 updater、portable 包、原生右键菜单和启动管理器；这些功能不能直接移植到当前 Mac Rust 传输层，也不会把 Windows 更新包当作 Mac 更新包使用。

验证结果：

- `npm test`：156 passed，1 skipped
- `npm run test:fullatlas`：6 passed
- `cargo check --manifest-path macos/src-tauri/Cargo.toml`：通过
- `cargo test --locked --manifest-path macos/src-tauri/Cargo.toml`：100 passed
- `python -m unittest discover windows/src/tests`：33 passed，1 skipped
- `npm run build`：macOS arm64 `.app` 构建通过
- `npm run package`：DMG 校验通过，产物写入根目录 `dist/`

官方资源仍锁定国服 1.0.3；本次同步没有改变官方版本锁，也没有绕过 FullAtlas 版本门禁。
