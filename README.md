# WuwaTappo Full Community

> 当前整合版本为 **1.4.0**（与根目录 `package.json` 同源）。国服与国际服的官方客户端基线均锁定 **1.0.5**，分别记录在 `shared/upstream/cn.json` 与 `shared/upstream/global.json`。macOS（Apple Silicon）与 Windows（x64）共用同一套 `shared/frontend/`（4 个前端模块）与 `shared/backend/`（Rust 核心、注入器与本地服务），全仓零 Python 依赖。详见[架构说明](docs/ARCHITECTURE.md) 与 [开发说明](docs/DEVELOPMENT.md)。

让《鸣潮》桌宠在 macOS 和 Windows 上运行，并在官方面板中加入本地全图鉴、混合外观与桌面助手。

基于 [wuwa-tappo-mac](https://github.com/Last-emo-boy/wuwa-tappo-mac) 的 Rust / Tauri macOS 后端，以及 [WuwaTappo-FullAtlas](https://github.com/Averionx/WuwaTappo-FullAtlas) 的图鉴和助手功能开发。macOS 侧无需 Windows、Wine 或虚拟机；Windows 侧直接给官方客户端打补丁，不需要重写。本项目为非官方社区整合版，与库洛官方及两个上游项目的发行渠道独立。最近一次上游同步记录见 [2026-11-05 上游同步](docs/UPSTREAM_SYNC_2026-11-05.md)。

## 平台

本仓库包含两套平台实现，共用同一套领域源码与上游基线：国服与国际服的官方客户端安装包分别锁定在 `shared/upstream/cn.json` 和 `shared/upstream/global.json`。

| | macOS | Windows |
| --- | --- | --- |
| 位置 | [`macos/`](macos/README.md)（`src-tauri/`、`scripts/`） | [`windows/`](windows/README.md) |
| 做法 | Rust / Tauri 原生壳重写 | 给官方客户端打补丁并重打包 PE |
| 为什么不同 | 官方没有 Mac 包 | 官方包本来能跑，重写没有收益 |
| 运行时 | Rust / Tauri 原生 IPC + `shared/backend` 领域服务 | `shared/backend` Rust 本地服务 + 本机 HTTP |
| 产物 | `.app` / `.dmg` | 绿色目录、安装包与便携 ZIP |
| 文档 | [macOS 指南](macos/README.md) | [Windows 指南](windows/README.md) |

两端共用 `shared/frontend/` 的 4 个模块（`desktop_assistant.js`、`assistant_dashboard.js`、`desktop_context_menu.js`、`local_atlas_controller.js`）与 `shared/backend/` 的 Rust 核心状态、注入器、资源缓存和本地服务；平台目录只保留系统绑定、构建与打包。仓库内不含 Python 脚本或运行时。双端共用代码与适配边界见[架构说明](docs/ARCHITECTURE.md)，开发和测试命令见[开发说明](docs/DEVELOPMENT.md)。

## 目录结构

```text
.
├── macos/          macOS 适配器、原生实现、构建与平台专属文档
├── windows/        Windows 适配器、上游快照、构建与平台专属文档
├── shared/         双端共用前端、Rust 后端、区域/更新契约、注入核心与回归测试
├── scripts/        跨平台构建分派、前端生成器、官方解包/镜像与上游同步脚本
├── docs/           跨平台架构与开发文档
├── dist/           两平台共用的产物目录（不入库）
├── package.json    npm 入口与发版号来源，两侧都读它
└── licenses/       FullAtlas 许可原文
```

平台专属能力留在各自适配器里，共享功能源和跨平台文档集中在 `shared/` 与根目录 `docs/`。目录约定由 `macos/tests/repo-layout.test.mjs` 验证。

## 构建

两个平台都无法交叉编译（macOS 侧要 Xcode，Windows 侧要官方 Windows 客户端 + Rust 工具链），
所以统一入口按当前系统分派，真正的双平台并行发生在 CI。

```bash
npm run build:check      # 先看当前系统缺什么前置条件
npm run build:mac        # macOS：Tauri 构建
npm run build:win        # Windows：取官方基底 → 注入 → 冻结 → 装配
npm run build:all        # 构建当前系统能构建的那一平台，并提示另一平台该在哪构建

npm run build:win -- --package     # 构建完打 zip 到 dist/
npm run build:check -- --platform=win
```

| 场景 | 产出 |
| --- | --- |
| push 到 `main` / `dev` | CI 同时跑 `build.yml`（macos-15）与 `build-windows.yml`（windows-2022），各自上传 artifact |
| 打 tag `v*` | 两平台都构建成功才直接发布同一个 Release；六个产物（国服/国际服各一份 macOS DMG、Windows 安装包与程序内升级 ZIP）缺一件就不发（`-alpha`/`-beta`/`-rc` 自动标 Pre-release） |

国服 / 国际服各一份 canonical lock（`shared/upstream/cn.json`、`shared/upstream/global.json`），同时记录官方安装包和 Windows 主程序指纹；同步工作流一次更新两平台，发版前只校验 canonical lock 自洽。

## 支持情况

| 项目 | 当前状态 |
| --- | --- |
| 服务区域 | 国服与国际服都构建：两个平台都锁定官方 1.0.5 基底（国服、国际服各一份 canonical lock），产物名带 `Full-CN-Community` / `Full-Global-Community` |
| 系统 | macOS：Apple Silicon（M 系列），macOS 13 及以上；Windows：10/11 x64 |
| 整合版本 | 与仓库根 `package.json` 同源（当前 `1.4.0`），基于官方客户端 1.0.5 与 FullAtlas 兼容功能集；两个平台同一基底 |
| 代码共享 | 双端共用 `shared/frontend/`（4 个模块）与 `shared/backend/`（Rust 核心、注入器、本地服务）；零 Python 依赖 |
| 分支 | `main` 为 1.0.5 基底，两个平台同仓共用；`dev` 用于开发版；`official-1.0.3` 保留为历史适配分支 |
| 验证 | 自动测试、原生构建、签名与 DMG / EXE 校验通过；角色和设置面板可显示，完整业务仍需实机验收 |
| 应用标识 | 国服 macOS 沿用 `dev.w33d.wuwatappo.fullatlas.mac`，国际服为 `dev.w33d.wuwatappo.global.mac`；改名只动显示名与产物名，既有数据目录和钥匙串不受影响 |
| 签名 | Ad hoc 临时签名，未通过 Apple 公证 |

## 自定义 Mod 与混合外观

### 混合外观与完整联机

本地全图鉴以叠加层方式与官方账号并行工作：官方背包、登录、点击计数、宝箱、联机房间和线上外观写入都保持原样，本地资源只补充本机显示。图鉴与背包会实时显示「本地 / 官方」来源角标；同一件外观可以只在本机使用，也可以同时作为联机外观。

图鉴与背包各自内置独立的装备管理栏，可把条目拖到「本地」或「官方」槽位；本地槽位只改本机显示，官方槽位走官方外观请求，表情还能在栏内直接调整顺序。本地表情不做角色限制，可跨角色试装；放到线上时会先做角色兼容性检查。

预设会自动分流：本机预设使用负 ID，只保存在本机；全部素材都属于官方背包时沿用云端预设（正 ID）。本地预设可在本地列表删除，云端预设在官方预设列表中删除，两套并存、互不覆盖。

### 自定义 Mod 与挂饰微调

在控制台「Mod 与挂饰」页签点「导入 Mod ZIP」，可同时安装多个资源包、查看条目缩略图并单独移除；不再重复提供官方同步按钮。包内无需填写 ID，应用自动分配编号。

Mod 条目在控制台与官方界面统一带 `MOD` 角标。图鉴与背包的稀有度排序把 `MOD` 排在首位，同时保持官方原有顺序（epic → rare → uncommon → common）。

每个挂饰都可以单独调整水平 / 垂直位置、大小与角度（-180°～180°），只影响本机显示，不写回账号。格式说明见 [Mod 制作说明](docs/MODS.md)，演示 ZIP 和逐帧预览放在仓库根目录的 [`mod-examples/`](mod-examples/)。

## 功能

### 桌面助手（原生内嵌控制台）

控制台是两端统一的原生内嵌页签：支持面板页签的壳里位于「联机」与「设置」之间，否则以同一套界面的浮层打开。页签包含概览、开箱记录、每日统计、房间历史、Mod 与挂饰、更新和设置。

- 后台自动点击：使用桌宠原版计数；物理键鼠活动、打字和 Shift 会暂停自动点击。按可用点数自动启停采用滞回阈值（默认低于 10,000 恢复、高于 40,000 暂停），暂停时徽章显示「点数达上限 · 待机」，不会关掉后台点击总开关。
- 自动开箱与开箱记录：沿用官方登录、冷却、余额与服务器奖励；只记录服务器成功返回的奖励，最近保留 1,000 条。
- 打字自动表情：跟随真实打字与点击次数触发，可选 ±40% 随机波动或固定间隔，并限定 1～5 号表情槽位；Alt+1～5 可在窗口聚焦时直接发送对应槽位。
- 每日统计与房间历史：按官方累计点击增量统计最近 7 天；房间号、房主与收藏会记录，混合外观模式下同样生效。

自动任务默认关闭，首次使用请先验收普通登录与图鉴显示。

### 批量合成、OBS 绿幕与面板外观

- 背包批量合成：控制台提供「批量同阶合成（5 合 1）」与「批量升阶合成（10 合 1）」，只消耗官方背包中数量大于 1 的多余副本（每个条目保留 1 个本体），跳过本地与 Mod 条目；升阶合成会跳过史诗（epic）。
- OBS 桌宠绿幕：仅在桌宠 `.cat-wrap` 周围绘制绿色背景，方便 OBS 色键抠像；开关状态跨窗口同步，不提供透明模式。
- 面板外观：独立卡片支持上传自定义背景图（2 MB 以内）、「清除背景图」与面板透明度滑杆，实时作用于控制台与官方面板。

### 更新与上游跟进

- 启动后静默检查 GitHub Release，之后每 8 小时非强制检查一次；发现新版本只弹可关闭的提示，可点「忽略此版本」永久略过该版本。
- 「更新」页提供「包含预发布版本」开关，可在正式版与 alpha / beta 预发布通道之间切换；操作按钮顺序为「打开 GitHub Releases」→「打开官方桌宠下载页」→「检查更新」。
- 应用内更新：Windows 下载 `…-win-x64-update.zip`，校验 `SHA256SUMS.txt` 后原地替换，失败自动回滚；macOS 在已安装到「应用程序」的标准 `.app` 上支持下载 DMG、校验签名与版本清单、原子替换并自动重启，失败同样回滚。账号、图鉴缓存、导入的 Mod 与设置都会保留。
- 官方 1.0.5 已原生处理全屏游戏模式与显示器记忆，控制台和设置页不再重复提供这些助手开关。

### 基础能力

- 原生透明桌宠、缩放、置顶、鼠标穿透、菜单栏控制和显示器定位。
- 账号登录、计数同步、宝箱冷却、消耗与奖励仍使用官方服务；本地图鉴只改变本地显示，不解锁账号资产。
- 图鉴目录与图片会在校验后缓存到本机；首次同步仍需要联网，缓存完成前图片依赖官方素材 CDN。
- Windows 发行包不带素材缓存，首次启动会在后台自动拉取（约 100 MB / 3000+ 张），设置页能看到进度；补齐前游戏按联机模式正常运行。

## 安装与使用（macOS）

> Windows 侧的安装、运行与管理见 [windows/README.md](windows/README.md)。两侧产物不通用，不要混用安装包。

发布后在本仓库的 **Releases** 中下载对应的 `.dmg`（国服 `WuwaTappo-Full-CN-Community-<版本>-macOS-arm64.dmg`，国际服 `WuwaTappo-Full-Global-Community-<版本>-macOS-arm64.dmg`），将应用拖入「应用程序」。

1. 打开 `WuwaTappo Full Community.app`（国际服为 `WuwaTappo Global Community.app`）。如系统拦截，请在确认来源后到「系统设置 → 隐私与安全性」选择「仍要打开」，无需关闭系统安全保护。
2. 首次启动可选「仅本次会话」或「记住登录」。前者将登录信息保存在内存，后者使用 macOS 钥匙串。
3. 要响应全局键鼠输入，请在「隐私与安全性 → 输入监控」中授权应用，并按系统提示重启。
4. 本地全图鉴默认作为叠加层启用，官方背包与联机保持可用。在控制台「Mod 与挂饰」导入 ZIP 资源包、管理已安装的 Mod，并调节挂饰位置 / 大小 / 角度；「设置」页可开启后台点击、自动开箱、打字表情等自动任务。首次使用请先验收普通登录和图鉴显示，再启用自动任务。

更新时退出旧版，再覆盖同名应用。也支持在控制台「更新」页一键更新；若更新后输入失效，请检查输入监控条目并重新授权。此整合版使用独立应用标识，不会自动迁移原 Mac 项目的钥匙串和用户数据。

同一应用标识的旧实验版与当前版本使用相同数据目录，并行试用时请退出另一个副本，并备份 `~/Library/Application Support/dev.w33d.wuwatappo.fullatlas.mac/`。macOS 版目前仅提供 Apple Silicon 构建。

应用内的官方更新检测会在发现官方基底高于当前适配版本时显示两个操作：打开本项目的 [GitHub Releases](https://github.com/alexzhong233/WuwaTappo-Full-Community/releases)，或把经过官方 HTTPS、区域和版本文件名校验的官方 Windows 原包下载到用户的 `Downloads`。原包只会保存，不会覆盖当前全图鉴版本；它也不能直接安装到 macOS。

Windows 发行包还会根据是否带有完整 `tools/` 适配脚本决定是否显示“本地适配并打补丁”。普通安装包没有源码、PowerShell 适配脚本和构建环境时，只提供原包下载与 Releases 入口。macOS 应用不会在用户机器上假装重编 Mac 包：从 Windows EXE 到 Mac 全图鉴 DMG 需要完整源码、Node、7-Zip、Rust、Xcode 和测试，交给维护者或 GitHub Actions 执行更安全。

## 从源码构建

需要 Xcode Command Line Tools、Node.js 24、[rustup](https://rustup.rs/) 和 7-Zip。Rust 版本固定在 `macos/rust-toolchain.toml`（1.97.1），构建链使用 Node 与 Rust，不依赖 Python。

```bash
xcode-select --install
brew install node@24 sevenzip
# Homebrew 当前提供 rustup 管理器，不再提供 rustup-init。
brew install rustup
echo 'export PATH="$(brew --prefix rustup)/bin:$PATH"' >> ~/.zshrc
export PATH="$(brew --prefix rustup)/bin:$PATH"
rustup toolchain install 1.97.1 --profile minimal
rustup default 1.97.1

# 在源码根目录执行：
npm ci
npm run setup
npm test
npm run test:fullatlas
npm run test:rust
npm run build
npm run package
```

跨平台共享行为与生成副本检查由 `npm test` 覆盖（含 `npm run test:shared`、`npm run test:mac` 与 `npm run test:win` 的 Node 回归测试）；平台构建所需的官方前端资源需先执行 `npm run setup`。Mac 专属 Rust 测试使用 `npm run test:rust`。

`setup` 下载 `shared/upstream/cn.json`（国际服用 `WUWA_REGION=global` 读取 `shared/upstream/global.json`）锁定的官方安装包，核对大小和 SHA-256 后静态提取前端；整个过程不运行 Windows 程序。若已有对应官方原包，可用 `npm run assets:sync -- --installer /绝对路径/安装包.exe`，然后执行 `npm run icons` 和 `npm run prepare:frontend`。

构建应用位于 `macos/src-tauri/target/release/bundle/macos/`，ZIP / DMG 位于仓库根的 `dist/`（两平台共用这个交付目录）。官方解包资源、依赖、安装包和用户数据不提交到源码仓库。清洁检出需要先执行 `setup`，部分测试需要已准备的前端。

## 如何跟进官方更新

官方客户端程序、公开图鉴数据、FullAtlas 功能代码分别维护：

- `npm run upstream:check` 只检查官方客户端版本；`npm run upstream:update` 会下载候选版本并修改官方资源锁，需要在适配分支执行。
- `npm run official:update` 是本地一键编排入口：默认只检测；`npm run official:update -- --apply` 才会下载并校验官方安装包、静态提取、注入 FullAtlas、运行测试、构建和打包 ZIP / DMG。失败会恢复更新前的资源锁和前端；未知前端结构会让注入的结构断言直接失败，而不是靠版本号提前挡下。
- 应用内更新：Windows 版的 Release 会额外附带 `…-win-x64-update.zip` 与 `SHA256SUMS.txt`，控制台「更新」页发现新版本后可以一键下载、校验、备份并原地替换（失败自动回滚，账号、缓存、导入的 Mod 与设置都不动）。macOS 版在已安装到「应用程序」的标准 `.app` 上同样支持一键更新：下载 DMG、校验签名与版本清单、原子替换并自动重启，失败回滚；不满足条件时会提示改用 GitHub 安装包。
- 应用内的「更新本地图鉴」刷新目录数据，不升级程序或接口适配器。
- `.github/workflows/sync-upstream.yml` 每 6 小时（北京时间 02:17 / 08:17 / 14:17 / 20:17）检查国服与国际服更新（也支持手动触发并指定区域）。发现新版就自动更新国服 / 国际服两份 canonical lock（`shared/upstream/cn.json` 与 `shared/upstream/global.json`）、递增整合版本号并编译两平台六件产物：**全部通过后自动合并、打 tag 并发预发布**；任一环节失败会保留分支并开草稿 PR，写明是哪个阶段红的、日志在哪。
- FullAtlas 集成器不再用版本白名单做准入：兼容性由结构断言与注入结果判定，版本号只用于记录和产物命名。官方换了版本号但锚点没漂，就不需要改代码。
- 发版前会校验 Windows 锁与 macOS 锁是同一版本、同一个安装包，避免一个 Release 里两个平台的客户端不是同一代。
- `main` 和 `dev` 的 CI 按区域矩阵构建：macOS（`cn` / `global`）与 Windows（`cn` / `global`）各出一份产物。`dev` 的 CI 成功后会自动创建或更新 `dev` 滚动预发布版；打版本标签的 Release 工作流在两平台六件产物全部就绪后直接发布，预发布号自动标 Pre-release，正式号标 latest。

在你的仓库启用 Actions 后这些工作流才会运行。定时同步需要在仓库设置中允许 Actions 创建 Pull Request，首次发布步骤见 [GitHub 发布指南](docs/PUBLISHING.md)。[官方更新流水线](docs/OFFICIAL_UPDATE_PIPELINE.md) 和 [来源与更新记录](docs/official-update-status-2026-10-30.md) 保留实现边界与核验信息。

这条流水线不能在已安装的 Mac 应用内部运行：官方提供的是 Windows EXE，转换需要 Node、7-Zip、Rust、Xcode 工具链和完整源码。桌宠可以检测并提示新 Mac Release，也可以校验并下载 DMG 做应用内更新，但从 Windows EXE 到 Mac 全图鉴 DMG 的重建必须由维护者或 GitHub Actions 完成。

## 隐私与许可

不在发行包中加入登录凭据、钥匙串、账号缓存、输入正文或运行日志。全局输入用于计数与动作反馈；登录与在线业务会连接对应官方服务。

macOS 项目与 FullAtlas 的 MIT 声明均保留；固定来源提交见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)，FullAtlas 许可见 [licenses/FULLATLAS_LICENSE](licenses/FULLATLAS_LICENSE)。官方界面、角色、美术、音频、名称与商标保留各自权利，不因整合而转为 MIT 授权。使用官方服务仍需遵守相应规则。
