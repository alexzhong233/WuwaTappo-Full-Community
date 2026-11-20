# WuwaTappo Full Community — Windows 侧

这个目录是本仓库的 Windows 平台实现。和 macOS 侧**不是同一套机制**，先看区别再动手：

| | macOS（[`macos/`](../macos/)） | Windows（本目录） |
| --- | --- | --- |
| 做法 | 用 Rust / Tauri 重写壳 | 给官方 Windows 客户端打补丁并重打包 |
| 为什么 | 官方没有 Mac 包 | 官方包本来就能跑，重写没有收益 |
| 运行时 | 原生 IPC 绑定 | Rust 本地服务 + 本机 HTTP（端口 43229），产物来自 `shared/backend` |
| 产物 | `.app` / `.dmg`（约 70 MiB） | 绿色目录（约 1.1 GB，含 3098 张图） |

两侧共用同一条上游：库洛官方国服客户端（同一个安装包，SHA-256 在 [`shared/upstream/cn.json`](../shared/upstream/cn.json) 里锁着）。

## 产物形态

- 绿色版，不写注册表、不装服务，整个 `app\` 目录拷走就能用。
- 入口 `WuwaTappoLauncher.exe`：**GUI 子系统，没有控制台窗口**。启动器自身不提权，拉主程序时用 `ShellExecuteW("runas")`，由系统按主程序的清单（`requireAdministrator`）提权——所以双击只弹一次 UAC，而不是「启动器一次 + 主程序一次」。
- 官方自动更新**永久关闭**（`isAppUpdaterEnabled()` 返回 false）：改过的 exe 过不了官方 minisign 签名，走 OTA 会把补丁覆盖掉。

## 国服 / 国际服

两条构建链完全一致，只是数据源与目录不同。区域默认 `cn`，`--region=global` 切国际服：

| | 国服（默认） | 国际服 |
| --- | --- | --- |
| 锁定文件 | `shared/upstream/cn.json` | `shared/upstream/global.json` |
| 素材 / 接口 | `mc-wuwatappo.kurogames-global.com` | `wutheringwaves-wuwatappo.kurogames-global.com` |
| 基底 | `build\official\` | `build-global\official\` |
| 构建 / 装配 | `build\`、`app\` | `build-global\`、`app-global\` |
| 产物 | `WuwaTappo-Full-CN-Community-Windows-x64-<ver>.exe` | `WuwaTappo-Full-Global-Community-Windows-x64-<ver>.exe` |

```powershell
node scripts/build-platform.mjs --platform=win --region=global --package --no-runtime
```

区域会写进 `local_config.json` 的 `region` / `cdn_host` / `api_base`，运行期拉素材和图鉴走本区域的官方源；老配置没有这几个字段时退回国服，既有部署不受影响。

两侧基底同版本（1.0.4）但哈希不同，注入脚本按候选兼容两处差异：国际服前端把 `listen` 压成 `listen$1`、应用标识是 `com.kuro.wuwatappo.global`（国服是 `com.kuro.wuwatappo.cn`）。不维护两套锚点。

## 从零构建

需要 Windows 10/11 x64、PowerShell 5.1、Node 24+，以及 Rust stable
（`cargo` + MSVC 链接器 + Windows SDK —— PE 解包元数据、注入打包、本地服务和启动器都是 Rust 产物，已零 Python 依赖）。

**推荐走仓库根的统一入口**（会自动取官方基底、注入、装配、打包）：

```powershell
cd <仓库根>
node scripts/build-platform.mjs --platform=win --package
# 等价于 npm run build:win -- --package
```

只想手动一步步来：

```powershell
# 1. 取官方基底（按 shared/upstream/cn.json 锁定版本自动下载并解包）
node scripts/fetch-official-win.mjs --region=cn

#    本机已装官方客户端、只是不想再下安装包时（可选）
node scripts/fetch-official-win.mjs --region=cn --official-exe D:\path\WuwaTappo.exe

# 2. 全量重建：解包 → 注入全图鉴 → 编译 Rust 本地服务与启动器 → 装配
cd windows
powershell -File tools\build.ps1 -Extract

# 3. 打安装包到 dist\（NSIS；首次先装：winget install -e --id NSIS.NSIS 或 choco install nsis -y）
powershell -File tools\package-installer.ps1 -NoRuntime

#    只要绿色版 zip 时用这个：
powershell -File tools\package-win.ps1 -NoRuntime
```

`build.ps1 -Extract` 会重新解包官方 EXE，`-Base <目录>` 可指向指定基底（默认读取 `build\official\current.json`）。`deploy.ps1 -Target <目录>` 可装配到别处，`-NoAssets` 跳过素材缓存。

`-NoRuntime` 不把系统那份 WebView2（约 870 MB）拷进 `runtime\`。入口 stub 探到 `runtime\` 不存在时不设 `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`，交给 WebView2 自己去找机器上的 Evergreen 运行时——和官方客户端的行为一致。代价是目标机器必须已经装了 WebView2（Win11 自带，Win10 未必）：安装包会在安装阶段检查并给出下载入口；已经装了却仍报「找不到 WebView2」，通常是 `runtime\` 里留了个坏目录，删掉重新装配即可。

### 官方安装包只解包，不执行

官方客户端安装包是**两层包装**：外层是个自解压壳（把内层整个塞进 `.rdata`），里面才是真正的 NSIS-3 安装器。

7-Zip 只看得到外层，会把它当普通 PE 解析——解出来的是 `.text`、`.rdata`、`.pdata`、`CERTIFICATE` 这些节区，一个安装文件都没有。这跟 7-Zip 版本无关：23.01 / 24.09 / 25.01 三个版本表现一致，强制 `-t#` 也认不出。

所以 `scripts/fetch-official-win.mjs` 调用统一的 `scripts/extract-assets.mjs`，用 `findEmbeddedNsis` 找到内层安装器，再把选中的主程序写到 Windows 构建目录。NSIS/PE 解析只有这一份实现。

共享提取器同时接受 `WuwaTappo.exe` 和 Tauri 的 `$_21_` 变量名，选中后写入 base；随后按 canonical lock 校验安装包 SHA-256 与官方 Authenticode EV 签名。

全程**不执行安装器**：执行会真的把客户端装进系统、启动进程、在 `%LOCALAPPDATA%` 写安装缓存，和本机已装版本混在一起。

需要 7-Zip（CI runner 自带；本地 `winget install 7zip.7zip`）。本地只想省掉这次下载时，才用 `--official-exe`——仍会校验主程序 Authenticode 签名。

### 版本锁定

`shared/upstream/cn.json` 记录的是**构建输入**：官方客户端版本、安装包直链与 SHA-256。它由 [同步工作流](../.github/workflows/sync-upstream.yml) 跟随官方最新版自动更新，不再需要人工改。

- 官方发新版后工作流自动更新国服 / 国际服两份 canonical lock（`shared/upstream/cn.json` 与 `shared/upstream/global.json`）并立刻编译；编过就直接发预发布，编不过才开草稿 PR 等人改适配。
- **版本号不是准入条件**：注入器不再维护"允许的版本清单"，兼容性由结构断言和注入结果判定。官方换了版本号但锚点没漂，就不需要改任何代码。
- 想在本机手动跟随：`node scripts/fetch-official-win.mjs --latest`，取官方最新并校验 EV 签名。

## 目录

```
windows\
├─ src\              Windows 安装脚本与测试（installer\、tests\）
├─ tools\
│   ├─ build.ps1 / deploy.ps1 / payload.ps1        构建（调用 ../scripts/extract-assets.mjs 与 shared/backend assistant-core）、装配、白名单过滤
│   ├─ package-win.ps1 / package-installer.ps1     绿色 zip 与 NSIS 安装包 + 原地升级 zip
│   ├─ build-launcher.ps1                          无控制台启动器（编译 shared/backend 的 Rust 产物）
│   ├─ make-shortcut.ps1                           桌面快捷方式（调用 assistant-core make-shortcut 写 .lnk 二进制）
│   └─ region.ps1                                  从 shared/config/regions.json 读取区域配置
```

`app\`、`build\`、`assets\` 都是产物或缓存，已在 `.gitignore` 里，不会进仓库。

## 和 macOS 侧怎么分工

平台源代码目录不再存放文档；通用文档统一维护在仓库根 `docs/`，平台专属资料才分别放在 `docs/windows/` 或 `docs/macos/`：

| | macOS（`macos/`） | Windows（本目录） | 能否共用 |
| --- | --- | --- | --- |
| 运行时 | `macos\src-tauri\src\*.rs` + `shared\backend` `FullAtlasService` | `shared\backend` Rust 本机 HTTP 服务 + 原生适配 | 共用域逻辑与 `FullAtlasService`，平台壳层各自实现 |
| 构建层 | `macos\scripts\*.mjs`、`package-macos.sh`、dmg | `scripts/fetch-official-win.mjs`、`tools\*.ps1`、Cargo、NSIS | 解包/上游同步完全共用 `scripts/`；封装按平台实现 |
| 注入与 PE 重打包 | `shared\backend\src\injector.rs`（输出前端目录） | `shared\backend\src\injector.rs` + `pe_repack.rs`（写入 PE 节区） | **共用**同一套注入器，只按 `--platform` 选择输出目标 |
| 前端模块 | 直接由 `injector.rs` 渲染 `shared/frontend/` | 直接由 `injector.rs` 渲染 `shared/frontend/` | **共用**唯一源，无生成副本 |

助手主流程、FullAtlas 服务与注入器全部收敛到 `shared/`（`shared/frontend/` 与 `shared/backend/`）；不再留存平台生成副本或 Python 脚本。跨平台文档、共享源和维护命令见根目录 [`docs/`](../docs/ARCHITECTURE.md)。

## 官方更新怎么跟

游戏版本和 FullAtlas 前端是两条独立的轴：

- **游戏版本**：基线跟随官方最新版，**两个平台锁同一版本**。前端 JS 结构变化会让注入锚点失效，所以升级动作是「先验证、再改锁」：跑 `node scripts/fetch-official-win.mjs --latest` 与 `powershell -File windows\tools\build.ps1 -Extract -NoDeploy` 在 `build\` 验证注入锚点（不碰 `app\`、不执行安装器），锚点断裂会直接报出是哪一处。
- **版本漂移会被测试挡住**：`macos/tests/upstream-baseline.test.mjs` 断言 Windows 与 macOS 锁的官方版本、安装包哈希完全一致，`npm test` 里跑。
- **FullAtlas 前端**：只改 `shared/frontend/` 里的共享 JS，运行 `npm test` 验证双端渲染与契约测试。

发行版设置面板发现官方新版本时，会提示新版本并提供下载入口（或原地更新入口），用户可选择更新并校验文件完整性。

注入锚点是压缩后的变量名；锚点不对时 Rust 注入器直接报错并保持 `app\` 不动，不做「能启动但 UI 错位」的静默失败。

官方 CDN 从境外的 GitHub runner 会间歇性连不上（2026-11-08 国服两个平台都被它打断）。`scripts/fetch-official-win.mjs` 会在主源重试后，改用仓库 `upstream-resources` 发布里的镜像包（文件名带安装包 sha256），下载完照样按 canonical lock 的 `installer.sha256` 与 Authenticode EV 签名校验字节。镜像地址由 CI 的 `WUWA_INSTALLER_MIRROR_BASE` 传入，与 macOS 侧同一套约定。

## 已知限制

1. **本机安全策略禁止 COM**（`WScript.Shell`、`Shell.Application` 一律拦截）。快捷方式由 `shared/backend` 的 `assistant-core make-shortcut` 模块直接写入带 `LinkTargetIDList` 和 `LinkInfo` 的 `.lnk` 二进制。
2. **`launch.ps1` 会被安全策略自动删除**（自提权脚本 `Start-Process -Verb RunAs`）。内容存档在 `src\installer\launch.ps1.disabled.txt`，入口统一用 `WuwaTappoLauncher.exe`。
3. **`schtasks.exe` 被黑名单拦截**，开机自启托管由 `shared/backend/src/windows_startup.rs` 用 PowerShell 原生 `Register-ScheduledTask` 注册 `WuwaTappoLauncher.exe`（缺失时兼容 `launch.ps1`）。
4. **CI 已接入**（`.github/workflows/build-windows.yml`，`windows-2022` runner）。发行包统一不带素材缓存与内嵌 WebView2，体积约 80 MB；首次启动由本地素材服务自动补齐（约 100 MB / 3000+ 张），补齐前 `/status` 的 `ready` 为 `false`，前端按联机模式启动、设置页显示下载进度，不阻塞进入游戏（`tools\payload.ps1` 直接读取 `shared/contracts/windows-update.json` 白名单过滤文件，本机跑过 `LocalGalleryHost.exe --prepare-cache` 也不会让安装包翻倍）。没有内嵌运行时，所以产物依赖系统 WebView2；CI 里有一条断言盯着入口 stub 的回退分支（`build_report_v2.json` 的 `runtime_fallback`），防止「stub 又把浏览器路径钉死到空目录」这类回归。
5. **安装包用 NSIS + MUI2**（`tools\package-installer.ps1`）：按用户装到 `%LOCALAPPDATA%\Programs\`，安装器本身不需要管理员权限。界面是欢迎页 → 选目录 → 进度 → 完成页（可直接勾选启动），语言跟系统走（简中 / 英文）。桌面与开始菜单快捷方式指向 `WuwaTappoLauncher.exe`，图标用 `app.ico`（`deploy.ps1` 从官方客户端 exe 里提取的 `icon_32512.ico`），卸载项同样带图标。覆盖安装会先安静结束正在运行的进程，卸载时可选择保留素材缓存与登录状态；同时由 `LocalGalleryHost.exe --make-update` 生成带 SHA-256 清单的原地升级包 `*-win-x64-update.zip`。
   **入口必须是 GUI 启动器**：缺了 `WuwaTappoLauncher.exe` 时打包脚本直接报错，不再回退 `start.cmd`——那个回退会让安装包弹控制台黑框、快捷方式图标变成 cmd 的。
   绿色 zip 仍保留（`tools\package-win.ps1`），供调试与免安装场景。
6. **官方发新版不会自动适配**。`shared/upstream/*.json` 锁住版本，构建不会静默换基底；跟版要手工重新定位锚点并实机验证。

## 与 macOS 侧的关系

助手前端 4 个模块（`local_atlas_controller.js` / `assistant_dashboard.js` / `desktop_assistant.js` / `desktop_context_menu.js`）由 `shared/frontend/` 单一源维护，构建时由 `shared/backend/src/injector.rs` 按目标平台占位符直接渲染，不再保留平台目录下的生成副本。
