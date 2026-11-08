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

## 2026-11-08 补检

两个上游在本轮之后又前进了两格，逐个核过：

- `Last-emo-boy/wuwa-tappo-mac`：HEAD 仍是 `c4dd4bd`（2026-11-02），与上次固定的提交一致，没有新内容。
- `Averionx/WuwaTappo-FullAtlas`：新增 `80eccd4`（适配官方 1.0.4）与 `98f5ded`（发行包体积优化）。

只合入与本项目双平台架构相关的一条：

- **裸 HTTP 403 不再当成凭证失效**。官方 1.0.4 的响应拦截器把 `403` 也算成登出信号（`clearAuthTokens` + `logoutAccount`），一次网关策略或版本门禁就能把人踹下线；上游收窄为只认业务码 `10002`/`10003` 与 HTTP `401`，两端注入脚本已跟上，并加 `macos/tests/fullatlas-adapter.test.mjs` 守住语义。

没有跟的部分及原因：

- 官方 1.0.4 适配：我们早已在锁里跟到 1.0.4（四份锁一致），四产物 CI 全绿。
- 原生控制台导航锚点改成结构化解析、`invoke$1` 版本号白名单：我们两端本来就按结构解析（`invoke_names` 动态探测），不需要这段。
- 1.3.7/1.3.8 的发行包体积优化、LZMA 压缩、`--check-uia` 自检、portable/updater 打包链：只服务 Windows 原生壳，与本项目「官方客户端打补丁」的分发方式不同，不适用。
