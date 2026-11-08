# WuwaTappo Full Community — Windows 侧

这个目录是本仓库的 Windows 平台实现。和 macOS 侧**不是同一套机制**，先看区别再动手：

| | macOS（[`macos/`](../macos/)） | Windows（本目录） |
| --- | --- | --- |
| 做法 | 用 Rust / Tauri 重写壳 | 给官方 Windows 客户端打补丁并重打包 |
| 为什么 | 官方没有 Mac 包 | 官方包本来就能跑，重写没有收益 |
| 运行时 | 原生 IPC 绑定 | Python 本地服务 + 本机 HTTP（端口 43229） |
| 产物 | `.app` / `.dmg`（约 70 MiB） | 绿色目录（约 1.1 GB，含 3098 张图） |

两侧共用同一条上游：库洛官方国服客户端（同一个安装包，SHA-256 在 [`macos/upstream-cn.json`](../macos/upstream-cn.json) 里锁着）。

## 产物形态

- 绿色版，不写注册表、不装服务，整个 `app\` 目录拷走就能用。
- 入口 `WuwaTappoLauncher.exe`：**GUI 子系统，没有控制台窗口**。启动器自身不提权，拉主程序时用 `ShellExecuteW("runas")`，由系统按主程序的清单（`requireAdministrator`）提权——所以双击只弹一次 UAC，而不是「启动器一次 + 主程序一次」。
- 官方自动更新**永久关闭**（`isAppUpdaterEnabled()` 返回 false）：改过的 exe 过不了官方 minisign 签名，走 OTA 会把补丁覆盖掉。

## 国服 / 国际服

两条构建链完全一致，只是数据源与目录不同。区域默认 `cn`，`--region=global` 切国际服：

| | 国服（默认） | 国际服 |
| --- | --- | --- |
| 锁定文件 | `upstream-win.json` | `upstream-win-global.json` |
| 素材 / 接口 | `mc-wuwatappo.kurogames-global.com` | `wutheringwaves-wuwatappo.kurogames-global.com` |
| 基底 | `build\official\` | `build-global\official\` |
| 构建 / 装配 | `build\`、`app\` | `build-global\`、`app-global\` |
| 产物 | `WuwaTappo-Full-CN-Community-Windows-x64-<ver>.exe` | `WuwaTappo-Full-Global-Community-Windows-x64-<ver>.exe` |

```powershell
node scripts/build-platform.mjs --platform=win --region=global --package --no-runtime
```

区域会写进 `local_config.json` 的 `region` / `cdn_host` / `api_base`，运行期拉素材和图鉴走本区域的官方源；老配置没有这几个字段时退回国服，既有部署不受影响。

两侧基底同版本（1.0.3）但哈希不同，注入脚本按候选兼容两处差异：国际服前端把 `listen` 压成 `listen$1`、应用标识是 `com.kuro.wuwatappo.global`（国服是 `com.kuro.wuwatappo.cn`）。不维护两套锚点。

## 从零构建

需要 Windows 10/11 x64、Python 3.13、PowerShell 5.1、Node 18+。

**推荐走仓库根的统一入口**（会自动建 `.venv`、取官方基底、注入、装配、打包）：

```powershell
cd <仓库根>
node scripts/build-platform.mjs --platform=win --package
# 等价于 npm run build:win -- --package
```

只想手动一步步来：

```powershell
cd windows

# 1. 建 .venv 并装依赖（只需一次；已存在会跳过）
powershell -File tools\setup-env.ps1

# 2. 取官方基底（按 upstream-win.json 锁定的 1.0.3 自动下载并解包）
powershell -File tools\fetch-official.ps1

#    本机已装官方客户端、只是不想再下 192 MB 安装包时（可选，不是必须）
powershell -File tools\fetch-official.ps1 -OfficialExe D:\path\WuwaTappo.exe

# 3. 全量重建：解包 → 注入全图鉴 → 冻结本地服务 → 装配
powershell -File tools\build.ps1 -Extract -Base build\official\1.0.3\base

# 4. 打安装包到 dist\（NSIS；首次先装：winget install -e --id NSIS.NSIS 或 choco install nsis -y）
powershell -File tools\package-installer.ps1 -NoRuntime

#    只要绿色版 zip 时用这个：
powershell -File tools\package-win.ps1 -NoRuntime
```

`build.ps1 -Extract` 会重新解包官方 EXE，`-Base <目录>` 可指向新版基底。`deploy.ps1 -Target <目录>` 可装配到别处，`-NoAssets` 跳过素材缓存。

`-NoRuntime` 不把系统那份 WebView2（约 870 MB）拷进 `runtime\`。入口 stub 探到 `runtime\` 不存在时不设 `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`，交给 WebView2 自己去找机器上的 Evergreen 运行时——和官方客户端的行为一致。代价是目标机器必须已经装了 WebView2（Win11 自带，Win10 未必）：安装包会在安装阶段检查并给出下载入口；已经装了却仍报「找不到 WebView2」，通常是 `runtime\` 里留了个坏目录，删掉即可，或用 `tools\fix-webview2.ps1` 重建。

### 官方安装包只解包，不执行

官方 1.0.3 的安装包是**两层包装**：外层是个自解压壳（把内层整个塞进 `.rdata`），里面才是真正的 NSIS-3 安装器。

7-Zip 只看得到外层，会把它当普通 PE 解析——解出来的是 `.text`、`.rdata`、`.pdata`、`CERTIFICATE` 这些节区，一个安装文件都没有。这跟 7-Zip 版本无关：23.01 / 24.09 / 25.01 三个版本表现一致，强制 `-t#` 也认不出。

所以 `fetch-official.ps1` 先自己剥壳：按 NSIS 签名（`EF BE AD DE` + `NullsoftInst`）定位内层安装器的头，再找一个「节区原始数据正好结束在该头起点」的 MZ/PE，切出 `[start, end)` 交给 7-Zip。判定与 macOS 侧 `macos/scripts/extract-assets.mjs` 的 `findEmbeddedNsis` 完全一致。

解出来的主程序在 NSIS 里叫 `$_21_` 而不是 `WuwaTappo.exe`（Tauri 的 NSIS 脚本用变量名保存），所以按 `upstream-win.json` 的 `official_exe_size` 认，再核对 SHA-256。

全程**不执行安装器**：执行会真的把客户端装进系统、启动进程、在 `%LOCALAPPDATA%` 写安装缓存，和本机已装版本混在一起。

需要 7-Zip（CI runner 自带；本地 `winget install 7zip.7zip`）。本地只想省掉这次下载时，才用 `-OfficialExe`——版本仍按 `upstream-win.json` 记为 1.0.3，不会退回旧版。

### 版本锁定

`upstream-win.json` 记录注入适配的官方客户端版本、主程序 SHA-256 和安装包直链。构建默认按它取包，**不会**因为官方发新版就悄悄换成未适配的版本。要跟新版：

```powershell
powershell -File tools\fetch-official.ps1 -Latest   # 取官方最新并校验哈希
```

哈希与锁定值不符时脚本会告警——这意味着锚点可能已漂移，需要重新定位后再改 `upstream-win.json`。

## 目录

```
windows\
├─ src\              注入用的 FullAtlas 源码（原上游仓库内容 + 1.0.3 适配改动）
│   ├─ src\          本地服务 local_host.py、助手后端、3 个前端模块
│   ├─ tools\        build_integration.py（注入）、extract_static.py（解包）、pe_packer.py（重打包）
│   ├─ docs\         上游原始文档与验证清单（构建记录、验证结果都看这里，不再另建副本）
│   └─ installer\    提权脚本存档（本机策略会删 .ps1，见「已知限制」）
├─ tools\
│   ├─ setup-env.ps1 / fetch-official.ps1          环境与官方基底
│   ├─ build.ps1 / deploy.ps1 / package-win.ps1    构建、装配、打包
│   ├─ check-update.ps1 / update-watch.ps1         官方版本检查与跟进
│   ├─ manager.py / build-manager.ps1              更新管理器 GUI
│   ├─ apply-update.ps1                            替换运行目录（带备份与回滚）
│   ├─ launcher.py / build-launcher.ps1            无控制台启动器
│   ├─ make_shortcut.py / make-shortcut.ps1        桌面快捷方式（本机禁 COM，手写 .lnk）
│   └─ patches\                                    1.0.3 适配差异存档
└─ upstream-win.json 注入适配锁定的官方版本与安装包直链
```

`app\`、`build\`、`.venv\`、`assets\` 都是产物或缓存，已在 `.gitignore` 里，不会进仓库。

## 和 macOS 侧怎么分工

`tools\`、`docs\` 两个名字两边都有，但不是重复劳动：

| | macOS（`macos/`） | Windows（本目录） | 能否共用 |
| --- | --- | --- | --- |
| 运行时 | `macos\src-tauri\src\*.rs`，Rust + Tauri IPC | `src\src\*.py`，Python 本机 HTTP 服务 | 不能。范式不同 |
| 构建层 | `macos\scripts\*.mjs`、`package-macos.sh`、dmg | `tools\*.ps1`、PyInstaller、PE 重打包 | 不能。工具链不同 |
| 注入器 | `macos\tools\fullatlas-integration.py`（改 Tauri 资源） | `src\tools\build_integration.py`（改 PE 资源段） | 不能。注入目标不同 |
| 前端模块 | `macos\fullatlas-source\*.js` | `src\src\*.js` | **共用**，见下 |

**唯一真正共用的是那 3 个前端模块**（`assistant_dashboard.js`、`desktop_assistant.js`、`local_atlas_controller.js`），各只差 4 行：品牌文案、更新监控卡、`VERSION` 常量、以及 `assetsHost` / `request` 的取值方式。

它们**没有**并成一份，原因是 `src\` 是上游仓库的完整快照，改动靠 `tools\patches\1.0.3-adaptation.diff` 存档；抽公共件会让「上游 v1.2 → v1.3」的同步失去可比对基线。代价由 `macos\tests\platform-parity.test.mjs` 兜住：两侧差异一旦超过 4 行（也就是只改了一边），CI 直接失败。

两份 `integrate_assistant.py`（`macos\tools\` 与本目录的 `src\tools\`）同理：前一份面向 1.0.3 并兼容双版本绑定，这份面向上游 1.0.2 布局、改 `src\` 下的模块，差异是有意的，不是漏同步。

## 官方更新怎么跟

游戏版本和 FullAtlas 前端是两条独立的轴：

- **游戏版本**：基线跟随官方最新版，**两个平台锁同一版本**。前端 JS 结构变化会让注入锚点失效，所以升级动作是「先验证、再改锁」：`check-update.ps1` 查版本，`update-watch.ps1`（或 `fetch-official.ps1 -Latest`）取新版基底并试注入到 staging（不碰 `app\`、不执行安装器），锚点断裂会直接报出是哪一处。验证通过后才改 `upstream-win*.json` 的 `client_version` / `official_exe_sha256` / `installer_*`。
- **版本漂移会被测试挡住**：`macos/tests/upstream-baseline.test.mjs` 断言 Windows 与 macOS 锁的官方版本、安装包哈希完全一致，`npm test` 里跑。一边升、另一边没升会当场红掉——2026-11-08 那次发版就是 Windows 停在 1.0.3、macOS 已经 1.0.4，Windows 产物里还是旧客户端。
- **FullAtlas 前端**（上游仓库 v1.2 → v1.3）：改动在 `src\src\` 那 3 个 JS 里，合并后要重跑一次构建验证。

发行版设置面板发现官方新版本时，会提供本项目 Releases 和官方原包下载入口。原包下载只保存到用户 `Downloads`，不会覆盖现有全图鉴程序；只有检测到完整源码与 `tools\update-watch.ps1` / `tools\apply-update.ps1` 的维护环境，才显示本地适配按钮。普通安装包无法安全地自行推断新版注入锚点。

注入锚点是压缩后的变量名，1.0.3 相对 1.0.2 断了 6 处（主锚点 `applyCatalog(e,t,n)` 三次都没变，辅助模块会漂移）；1.0.4 相对 1.0.3 没有新增断点，两侧锚点全部命中，行为不变。锚点不对时脚本直接报错并保持 `app\` 不动，不做「能启动但 UI 错位」的静默失败。

官方 CDN 从境外的 GitHub runner 会间歇性连不上（2026-11-08 国服两个平台都被它打断）。`fetch-official.ps1` 现在会在主源重试 3 次仍失败后，改用仓库 `upstream-resources` 发布里的镜像包（文件名带安装包 sha256），下载完照样按锁文件的 `installer_sha256` 校验字节。镜像地址由 CI 的 `WUWA_INSTALLER_MIRROR_BASE` 传入，与 macOS 侧同一套约定。

## 已知限制

1. **本机安全策略禁止 COM**（`WScript.Shell`、`Shell.Application` 一律拦截）。快捷方式改用 `pylnk3` 直接写 `.lnk` 二进制（`tools\make_shortcut.py`）。手写字节不行——Windows 要求 `LinkTargetIDList`，只有 `LinkInfo` 的链接会被 shell 拒绝。
2. **`launch.ps1` 会被安全策略自动删除**（自提权脚本 `Start-Process -Verb RunAs`）。内容存档在 `src\installer\launch.ps1.disabled.txt`，入口统一用 `WuwaTappoLauncher.exe`。
3. **`schtasks.exe` 被黑名单拦截**，注册定时检查要用 PowerShell 原生 `Register-ScheduledTask`（`tools\install-schedule.ps1` 已处理）。
4. **CI 已接入**（`.github/workflows/build-windows.yml`，`windows-2022` runner）。发行包统一不带素材缓存与内嵌 WebView2，体积约 80 MB；首次启动由本地素材服务自动补齐（约 100 MB / 3000+ 张），补齐前 `/status` 的 `ready` 为 `false`，前端按联机模式启动、设置页显示下载进度，不阻塞进入游戏（`tools\payload.ps1` 里排除了 `cache\`，本机跑过 `prepare_cache.py` 也不会让安装包翻倍）。没有内嵌运行时，所以产物依赖系统 WebView2；CI 里有一条断言盯着入口 stub 的回退分支（`build_report_v2.json` 的 `runtime_fallback`），防止「stub 又把浏览器路径钉死到空目录」这类回归。
5. **安装包用 NSIS + MUI2**（`tools\package-installer.ps1`）：按用户装到 `%LOCALAPPDATA%\Programs\`，安装器本身不需要管理员权限。界面是欢迎页 → 选目录 → 进度 → 完成页（可直接勾选启动），语言跟系统走（简中 / 英文）。桌面与开始菜单快捷方式指向 `WuwaTappoLauncher.exe`，图标用 `app.ico`（`deploy.ps1` 从官方客户端 exe 里提取的 `icon_32512.ico`），卸载项同样带图标。覆盖安装会先安静结束正在运行的进程，卸载时可选择保留素材缓存与登录状态。
   **入口必须是 GUI 启动器**：缺了 `WuwaTappoLauncher.exe` 时打包脚本直接报错，不再回退 `start.cmd`——那个回退会让安装包弹控制台黑框、快捷方式图标变成 cmd 的。
   绿色 zip 仍保留（`tools\package-win.ps1`），供调试与免安装场景。
6. **官方发新版不会自动适配**。`upstream-win.json` 锁住版本，构建不会静默换基底；跟版要手工重新定位锚点并实机验证。

## 与 macOS 侧的关系

前端那 3 个模块（`local_atlas_controller.js` / `assistant_dashboard.js` / `desktop_assistant.js`）两边同源，目前各存一份，差异只有 4 行/文件，都是平台绑定：

| 文件 | Windows 版 | macOS 版 |
| --- | --- | --- |
| `local_atlas_controller.js` | 直接 `rawFetch` 到本机 HTTP | 有 `d.request` 钩子时走原生 IPC；`assetsHost` 用 `offlineData.assets_host` |
| `assistant_dashboard.js` | 「官方基底监控」卡片，查 Kuro 版本 | 「Mac 版本更新」卡片，查 `/official/update-check` |
| `desktop_assistant.js` | `VERSION = '1.2'` | `VERSION = '1.2.6'` |

收敛方案：以 macOS 版已有的 `d.request` 作为传输层抽象，Windows 侧实现一个走本机 HTTP 的版本，两边共用一个文件。这一步还没做。
