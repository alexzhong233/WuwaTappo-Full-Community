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

## 2026-11-08 第二轮：FullAtlas 1.3.4 → 1.3.8 功能面补齐

这次把上游从 1.3.4 起新增、且不依赖它自己那套 Windows 外壳的功能全部搬了过来。

**本轮同步的功能**

| 上游 | 内容 | 落地方式 |
|---|---|---|
| 1.3.4 | 本地预设（保存/应用/删除/重启保留） | 控制器新增 `PRESET_KEY` 一套（最多 5 个），面板「我的预设」动作在本地模式下改走本机；登录守卫放行，未登录也能用 |
| 1.3.4 | 打字自动表情 | 真实物理按键 + 焦点判定。Windows 用只读 UIA 焦点探测（comtypes/UIAutomation），macOS 用 AX 系统级焦点元素（AXRole 白名单）；虚拟补点不触发 |
| 1.3.4 | 图鉴分类收集进度 | 图鉴顶部分类显示真实账号已收集比例；本地模式下整块隐藏（本地模式恒 100%，没有参考价值） |
| 1.3.4 | 退出释放后台服务 | Windows 宿主盯主程序存活，桌宠退出后自行收工；拿不到 psutil 时按「不确定」处理，不误杀 |
| 1.3.6 | 右键菜单加控制台入口 | mac 对应菜单栏：「控制台」直接打开自动化面板 |

**已经一致、不需要再动的**

- 1.3.5 启动时静默检查 Release：我们启动即检查、之后每 8 小时一次，只提示不拦截（比上游的强制更新更宽松）。
- 1.3.5 自启动先起服务：Windows 启动器本来先拉宿主、再开主程序。
- 1.3.5 独立置顶更新窗口：mac 的 `/update` 独立窗口本来就 `always_on_top`。
- 1.3.6 失去焦点置顶修复：Windows 窗口层级问题，mac 走 `always_on_top` + 全屏检测，没有对应缺陷。
- 1.3.7 适配官方 1.0.4 / 跟随 `latest_host.json` / 裸 403 不再登出：均已在前一轮完成。

**搬不动或需要拍板的（不是技术做不到，是会和现有架构冲突）**

| 上游项 | 结论 | 现在的做法 |
|---|---|---|
| 绿色版单 EXE | 不做 | 分发单位仍是 NSIS 安装包 + dmg；单文件化要把官方 exe 和运行时整体重打包，收益只是「少一个安装步骤」 |
| 程序内更新（下载 zip → 备份替换 → 自动重启） | **Windows 已做**，macOS 保持原样 | Windows 的 Release 现在多带一个 `…-win-x64-update.zip` + `SHA256SUMS.txt`：控制台「更新」里发现新版本即可一键下载、校验、备份、替换、重启，失败自动回滚。macOS 受签名与 app 包结构限制，仍跳 GitHub 下载安装包；两端弹窗都会写明「因系统原因跳转下载」 |
| LZMA 压缩、依赖裁剪 | 不做 | 只服务上游自己的打包链；我们的体积大头是官方客户端本体，换独立 WebView2 运行时反而变大（上游 400 MB 就是这么来的）。升级包用 deflate，够用且解压快 |
| 默认隐藏任务栏图标 | 不自己实现，改官方开关的默认值 | 桌宠原版设置里本来就有「显示任务栏图标」（默认勾选 = 显示）。整合版把默认值改成不勾选，并给存过设置的老用户做一次迁移；之后在官方设置里勾回来永久生效。macOS 侧同样走这个设置，程序坞图标由 `set_main_skip_taskbar` 一并处理 |
