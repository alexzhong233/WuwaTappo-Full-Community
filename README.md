# WuwaTappo Full Community

> 当前整合版本为 **1.3.1**，Windows / macOS 国服与国际服均锁定官方客户端 **1.0.4**。FullAtlas 1.3.4–1.3.8 中适用于本项目双平台架构的功能已同步；依赖上游 Windows 外壳的功能保留各平台原生实现。`1.2.8` 是上一条正式版本线。详见 [上游同步记录](macos/docs/UPSTREAM_SYNC_2026-11-05.md) 与 [1.0.4 适配记录](macos/docs/RELEASE_NOTES_1.2.7.md)。

让《鸣潮》桌宠在 macOS 和 Windows 上运行，并在原有面板中加入本地全图鉴和桌面助手。

基于 [wuwa-tappo-mac](https://github.com/Last-emo-boy/wuwa-tappo-mac) 的 Rust / Tauri macOS 后端，以及 [WuwaTappo-FullAtlas](https://github.com/Averionx/WuwaTappo-FullAtlas) 的图鉴和助手功能开发。macOS 侧无需 Windows、Wine 或虚拟机；Windows 侧直接给官方客户端打补丁，不需要重写。本项目为非官方社区整合版，与库洛官方及两个上游项目的发行渠道独立。最近一次上游同步记录见 [2026-11-05 上游同步](macos/docs/UPSTREAM_SYNC_2026-11-05.md)。

## 平台

本仓库包含两套平台实现，共用同一个上游（库洛官方国服客户端，安装包与 SHA-256 锁在 `macos/upstream-cn.json`）：

| | macOS | Windows |
| --- | --- | --- |
| 位置 | [`macos/`](macos/README.md)（`src-tauri/`、`scripts/`） | [`windows/`](windows/README.md) |
| 做法 | Rust / Tauri 原生壳重写 | 给官方客户端打补丁并重打包 PE |
| 为什么不同 | 官方没有 Mac 包 | 官方包本来能跑，重写没有收益 |
| 运行时 | 原生 IPC 绑定 | Python 本地服务 + 本机 HTTP |
| 产物 | `.app` / `.dmg`（约 70 MiB） | 绿色目录（约 1.1 GB） |
| 文档 | [macos/README.md](macos/README.md) | [windows/README.md](windows/README.md) |

两侧的 FullAtlas 前端模块同源，平台差异集中在 3 个文件的各 4 行里，收敛计划见 [windows/README.md](windows/README.md) 末尾。

## 目录结构

```text
.
├── macos/          macOS 实现：src-tauri/（Rust）、scripts/（构建与上游同步）、tests/、fullatlas-source/、docs/
├── windows/        Windows 实现：src/（上游快照）、tools/（PowerShell 构建层）
├── scripts/        只放跨平台分派器 build-platform.mjs
├── dist/           两平台共用的产物目录（不入库）
├── package.json    npm 入口与发版号来源，两侧都读它
└── licenses/       FullAtlas 许可原文
```

平台实现不放在仓库根：`macos/` 与 `windows/` 各自闭环，根目录只留两平台都要用的东西。
这条边界有测试守着（`macos/tests/repo-layout.test.mjs`），平台目录再长回根目录会直接失败。

## 构建

两个平台都无法交叉编译（macOS 侧要 Xcode，Windows 侧要官方 Windows 客户端 + PyInstaller），
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
| 打 tag `v*` | 两平台都构建成功才直接发布同一个 Release，四件产物缺一件就不发（`-alpha`/`-beta`/`-rc` 自动标 Pre-release） |

Windows 侧版本记录在 `windows/upstream-win.json`，由同步工作流跟随官方最新版自动更新；四份锁始终指向同一个官方安装包，发版前还会再校验一次两平台基线是否对齐。

## 支持情况

| 项目 | 当前状态 |
| --- | --- |
| 服务区域 | 国服与国际服都构建：两个平台锁定同一份官方 1.0.4 基底，产物名带 `Full-CN-Community` / `Full-Global-Community` |
| 系统 | macOS：Apple Silicon（M 系列），macOS 13 及以上；Windows：10/11 x64 |
| 整合版本 | 1.3.1，基于官方客户端 1.0.4 与 FullAtlas 1.3.4–1.3.8 兼容功能集；两个平台同一基底 |
| 分支 | `main` 为 1.0.4 基底，两个平台同仓共用；`dev` 用于开发版；`official-1.0.3` 保留为历史适配分支 |
| 验证 | 自动测试、原生构建、签名与 DMG / EXE 校验通过；角色和设置面板可显示，完整业务仍需实机验收 |
| 应用标识 | 沿用 `dev.w33d.wuwatappo.fullatlas.mac`，改名只动显示名与产物名，既有数据目录和钥匙串不受影响 |
| 签名 | Ad hoc 临时签名，未通过 Apple 公证 |

## 自定义 Mod

「官方 / 全图鉴 + Mod」外观模式：官方模式使用账号背包；全图鉴模式保留原有完整离线图鉴，并叠加本机导入的自制角色、挂饰和表情。没有 Mod 时仍可使用原有全图鉴，官方缓存独立保存。

在「挂机与统计 → Mod 管理」切换「官方 / 全图鉴 + Mod」，导入 ZIP 资源包后可同时安装多个包、查看条目缩略图或单独移除；官方模式下不会显示任何本机 Mod。包内无需填写 ID，应用自动管理编号，Mod 条目在背包里单独成组置顶、带 `MOD` 角标。格式与示例见 [Mod 制作说明](macos/docs/MODS.md)，想先试效果可直接导入 [演示包](macos/docs/mod-examples/demo-animated-pet.zip)：五个动作帧齐全，静置看眨眼、拖动看迈步；[逐帧预览](macos/docs/mod-examples/frames.png) 与[制作过程](macos/docs/mod-examples/README.md)（含生成脚本）在同目录。本功能在独立分支开发，尚未合入 1.2.8 主线；macOS 目前接入国服前端。

## 功能

- 原生透明桌宠、缩放、置顶、鼠标穿透、菜单栏控制和显示器定位。
- 在官方背包 / 图鉴 / 设置面板中切换「官方 / 全图鉴 + Mod」，可使用账号背包或完整本地图鉴与自制外观。
- 桌面助手：后台点击、按可用点数自动启停点击（默认低于 10,000 开启、高于 40,000 关闭）、自动开箱、开箱记录、每日统计、房间历史和显示器记忆。自动任务默认关闭。
- 后台点击通过桌宠内部事件工作；物理键鼠活动会暂停自动点击。不会向 macOS 系统注入键盘或鼠标事件。
- 账号登录、计数同步、宝箱冷却、消耗与奖励仍使用官方服务；本地图鉴只改变本地显示，不解锁账号资产。
- 本地图鉴开启时阻止联机房间，以及会把本地外观、收藏或合成写回账号的请求；关闭后恢复普通账号视图。
- 可联网同步公开图鉴目录。图片仍依赖官方素材 CDN，**不是完整离线资源包**。
- Windows 发行包不带素材缓存，首次启动会在后台自动拉取（约 100 MB / 3000+ 张），设置页能看到进度；补齐前游戏按联机模式正常运行。

## 安装与使用（macOS）

> Windows 侧的安装、运行与管理见 [windows/README.md](windows/README.md)。两侧产物不通用，不要混用安装包。

发布后在本仓库的 **Releases** 中下载对应的 `.dmg`（国服 `WuwaTappo-Full-CN-Community-<版本>-macOS-arm64.dmg`，国际服 `WuwaTappo-Full-Global-Community-<版本>-macOS-arm64.dmg`），将应用拖入「应用程序」。

1. 打开 `WuwaTappo Full Community.app`。如系统拦截，请在确认来源后到「系统设置 → 隐私与安全性」选择「仍要打开」，无需关闭系统安全保护。
2. 首次启动可选「仅本次会话」或「记住登录」。前者将登录信息保存在内存，后者使用 macOS 钥匙串。
3. 要响应全局键鼠输入，请在「隐私与安全性 → 输入监控」中授权应用，并按系统提示重启。
4. 在设置中切换「外观模式」（关闭为官方，开启为全图鉴 + Mod），通过「挂机与统计」打开助手面板并管理 Mod。首次使用请先验收普通登录和图鉴显示，再启用自动任务。

更新时退出旧版，再覆盖同名应用。若更新后输入失效，请检查输入监控条目并重新授权。此整合版使用独立应用标识，不会自动迁移原 Mac 项目的钥匙串和用户数据。

本项目的 1.2.1 与 1.2.2-alpha.1 使用相同应用标识和数据目录，试用前请退出另一个副本，并备份 `~/Library/Application Support/dev.w33d.wuwatappo.fullatlas.mac/`。实验版目前仅提供 Apple Silicon 构建。

应用内的官方更新检测会在发现官方基底高于当前适配版本时显示两个操作：打开本项目的 [GitHub Releases](https://github.com/alexzhong233/WuwaTappo-Full-Community/releases)，或把经过官方 HTTPS、区域和版本文件名校验的官方 Windows 原包下载到用户的 `Downloads`。原包只会保存，不会覆盖当前全图鉴版本；它也不能直接安装到 macOS。

Windows 发行包还会根据是否带有完整 `tools/` 适配脚本决定是否显示“本地适配并打补丁”。普通安装包没有源码、PowerShell 适配脚本和构建环境时，只提供原包下载与 Releases 入口。macOS 应用不会在用户机器上假装重编 Mac 包：从 Windows EXE 到 Mac 全图鉴 DMG 需要完整源码、Node、7-Zip、Rust、Xcode 和测试，交给维护者或 GitHub Actions 执行更安全。

## 从源码构建

需要 Xcode Command Line Tools、Node.js 24、[rustup](https://rustup.rs/) 和 7-Zip。Rust 版本固定在 `rust-toolchain.toml`（1.97.1），构建脚本还会使用 Python 3。

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

`setup` 下载 `upstream-cn.json` 锁定的官方安装包，核对大小和 SHA-256 后静态提取前端；整个过程不运行 Windows 程序。若已有对应官方原包，可用 `npm run assets:sync -- --installer /绝对路径/安装包.exe`，然后执行 `npm run icons` 和 `npm run prepare:frontend`。

构建应用位于 `macos/src-tauri/target/release/bundle/macos/`，ZIP / DMG 位于仓库根的 `dist/`（两平台共用这个交付目录）。官方解包资源、依赖、安装包和用户数据不提交到源码仓库。清洁检出需要先执行 `setup`，部分测试需要已准备的前端。

## 如何跟进官方更新

官方客户端程序、公开图鉴数据、FullAtlas 功能代码分别维护：

- `npm run upstream:check` 只检查官方客户端版本；`npm run upstream:update` 会下载候选版本并修改官方资源锁，需要在适配分支执行。
- `npm run official:update` 是本地一键编排入口：默认只检测；`npm run official:update -- --apply` 才会下载并校验官方安装包、静态提取、注入 FullAtlas、运行测试、构建和打包 ZIP / DMG。失败会恢复更新前的资源锁和前端；未知前端结构会让注入的结构断言直接失败，而不是靠版本号提前挡下。
- 应用内更新：Windows 版的 Release 会额外附带 `…-win-x64-update.zip` 与 `SHA256SUMS.txt`，控制台「更新」页发现新版本后可以一键下载、校验、备份并原地替换（失败自动回滚，账号、缓存、导入的 Mod 与设置都不动）。macOS 版受 app 包签名限制不做程序内替换，同一处按钮会提示「因系统原因」并跳转 GitHub 下载安装包。
- 应用内的「更新本地图鉴」刷新目录数据，不升级程序或接口适配器。
- `.github/workflows/sync-upstream.yml` 每 6 小时（北京时间 02:17 / 08:17 / 14:17 / 20:17）检查国服与国际服更新（也支持手动触发并指定区域）。发现新版就自动更新四份基线锁、递增整合版本号并编译两平台四产物：**全部通过后自动合并、打 tag 并发预发布**；任一环节失败会保留分支并开草稿 PR，写明是哪个阶段红的、日志在哪。
- FullAtlas 集成器不再用版本白名单做准入：兼容性由结构断言与注入结果判定，版本号只用于记录和产物命名。官方换了版本号但锚点没漂，就不需要改代码。
- 发版前会校验 Windows 锁与 macOS 锁是同一版本、同一个安装包，避免一个 Release 里两个平台的客户端不是同一代。
- `main` 和 `dev` 的 CI 按区域矩阵构建：macOS（`cn` / `global`）与 Windows（`cn` / `global`）各出一份产物。`dev` 的 CI 成功后会自动创建或更新 `dev` 滚动预发布版；版本标签触发的 Release 工作流只生成**草稿预发布**，需要维护者验收后手动发布.

在你的仓库启用 Actions 后这些工作流才会运行。定时同步需要在仓库设置中允许 Actions 创建 Pull Request，首次发布步骤见 [GitHub 发布指南](macos/docs/PUBLISHING.md)。[官方更新流水线](macos/docs/OFFICIAL_UPDATE_PIPELINE.md) 和 [来源与更新记录](macos/docs/official-update-status-2026-10-30.md) 保留实现边界与核验信息。

这条流水线不能在已安装的 Mac 应用内部运行：官方提供的是 Windows EXE，转换需要 Node、7-Zip、Rust、Xcode 工具链和完整源码。桌宠可以检测并提示新 Mac Release，但下载、构建和替换应用必须由维护者或 GitHub Actions 完成。

## 隐私与许可

不在发行包中加入登录凭据、钥匙串、账号缓存、输入正文或运行日志。全局输入用于计数与动作反馈；登录与在线业务会连接对应官方服务。

macOS 项目与 FullAtlas 的 MIT 声明均保留；固定来源提交见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)，FullAtlas 许可见 [licenses/FULLATLAS_LICENSE](licenses/FULLATLAS_LICENSE)。官方界面、角色、美术、音频、名称与商标保留各自权利，不因整合而转为 MIT 授权。使用官方服务仍需遵守相应规则。
