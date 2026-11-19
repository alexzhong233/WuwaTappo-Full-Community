# WuwaTappo Full Community — macOS 侧

这个目录是本仓库的 macOS 平台实现。对侧是 [`windows/`](../windows/README.md)，两边机制不同：

| | macOS（本目录） | Windows（[`windows/`](../windows/README.md)） |
| --- | --- | --- |
| 做法 | Rust / Tauri 原生壳重写 | 给官方客户端打补丁并重打包 PE |
| 为什么 | 官方没有 Mac 包 | 官方包本来能跑，重写没有收益 |
| 运行时 | 原生 IPC 绑定 | Rust 本地服务 + 本机 HTTP（端口 43229） |
| 工具链 | Node + Rust + Xcode CLT + 7-Zip | PowerShell + Python + Rust（本地服务与启动器都是 Rust 产物） |
| 产物 | `.dmg`（约 130 MiB，拖入「应用程序」） | `.exe` 安装包（NSIS，装完双击快捷方式即用） |

两侧共用同一份 canonical upstream lock：`shared/upstream/cn.json` 与 `shared/upstream/global.json`，同时记录安装包直链、SHA-256 和 Windows 主程序指纹。服务区域由 `shared/config/regions.json` 单点定义，构建脚本一律通过 `WUWA_REGION` 取。

## 目录

```text
macos/
├─ src-tauri/            Rust 后端与 Tauri 配置（tauri.conf.json、tauri.global.conf.json）
├─ scripts/              Mac 专属构建脚本：前端准备、注入、图标、签名与 DMG 打包（通用上游/NSIS 提取在 ../scripts/）
├─ tests/                Mac 适配、注入、打包与平台集成测试
├─ tools/                FullAtlas 集成器适配器（调用 ../shared/tools/）
├─ fullatlas-source/     ../shared/frontend/ 生成的 Mac 构建输入
└─ rust-toolchain.toml   Rust 版本固定（1.97.1，CI 与本地共用）
```

`build/`、`frontend/`、`frontend-global/`、`src-tauri/target/`、`src-tauri/icons*/`、`.work/` 都是本目录下的产物或缓存，已在仓库根的 `.gitignore` 里。

## 命令入口

**npm 入口在仓库根**（`package.json` 只有一份，发版号也是它），脚本内部再指到本目录：

```bash
npm ci                 # 仓库根；依赖装在仓库根 node_modules
npm run setup          # 同步官方基底 + 生成图标 + 准备前端
npm test               # 双端共享与平台集成 Node 测试
npm run test:fullatlas # 3 个共用前端模块的适配测试
npm run test:rust      # cargo test（仅 macOS）
npm run build          # Tauri 构建 → macos/src-tauri/target/release/bundle/macos/
npm run package        # 校验签名与内嵌资源，产出 ZIP / DMG 到仓库根 dist/
npm run build:mac -- --package   # 上面两步合一，走跨平台分派器
```

跨平台共享代码边界、文档归属与测试入口见仓库根的 [架构说明](../docs/ARCHITECTURE.md) 和 [开发说明](../docs/DEVELOPMENT.md)。

产物统一落在**仓库根的 `dist/`**，与 Windows 侧同一个交付目录；临时日志与中间状态落在本目录的 `.work/`，不进仓库。

## 国际服

同一套脚本，换个区域标识即可；CI 的 `build / cn`、`build / global` 两个 job 就是这么跑的：

```bash
npm run setup:global   # = WUWA_REGION=global npm run setup
npm run build:global
npm run package:global # 产物名 WuwaTappo-Full-Global-Community-*-macOS-arm64.dmg
```

区域差异全部由 `shared/config/regions.json` 决定，`scripts/regions.mjs` 只把它转换成 Mac profile：`frontend`/`frontend-global`、`build/frontend`/`build/frontend-global`、`icons`/`icons-global`、`tauri.conf.json`/`tauri.global.conf.json`、以及产物前缀。两者**不能交叉**：`prepare-frontend.mjs` 会校验入口模块里是否写着对应区域的 `APP_REGION`，`network-smoke.mjs` 还会拒绝另一服的源码，串了会直接报错而不是静默构建出错的包。

FullAtlas 控制面板两个区域都注入（`integrateFullAtlas` 对国服/国际服各跑一遍），中间产物也按区域分开放在 `build/fullatlas-adapter-<cn|global>/`，所以同一工作区里先后构建两服不会互相覆盖结构快照。

每日官方更新检测（`.github/workflows/sync-upstream.yml`，北京时间 10:17）国服、国际服各跑一遍，手动触发时可以用 `region` 入参只挑一个。两个区域互不加锁，各自的锁文件、契约快照、候选包和 PR 都是分开的。

契约快照同样分两份（`tests/contracts.json`、`tests/contracts-global.json`），官方版本一变就要重新生成：

```bash
WUWA_REGION=global node macos/scripts/inventory-contract.mjs   # 覆盖 tests/contracts-global.json
```

`tests/network-contract-global.json` 是手写约束，不会自动生成，但里面的 `application` 版本号和 `source` 四个入口文件名要跟着改，否则测试会以 ENOENT 或版本不一致失败。

## 与 Windows 侧怎么分工

- **共用**：三个助手前端模块唯一源位于仓库根 `shared/frontend/`；助手主流程、FullAtlas 注入核心与「控制台」页签、状态/图鉴/Mod/更新/日志/配置等 Rust 领域逻辑均在 `shared/`；NSIS 解包与上游同步脚本位于根目录 `scripts/`。平台目录内前端副本由 `npm run sync:frontend` 生成并由测试严格校验。
- **平台专属**：Tauri IPC、辅助功能/输入监控/窗口、DMG 打包，以及 Windows 的 PE 重打包、Win32 菜单/自启和本机 HTTP 传输。共享层不写平台分支。

## 应用内官方版本检测

设置面板会查询对应区域的官方版本指针。发现版本高于当前 Mac 适配基底时，用户可以打开本项目 Releases，或下载官方 Windows 原包到 `Downloads` 供维护者取用。下载链路只接受当前区域的 HTTPS 官方资源、匹配版本的安装包文件名，并限制最大体积；不会执行安装器或覆盖 Mac 应用。

Mac 应用不在本地做“Windows EXE → Mac + FullAtlas DMG”重编。这个过程需要仓库源码、Node、7-Zip、Rust、Xcode 和完整测试，应该由维护者或 GitHub Actions 运行 `npm run official:update -- --apply` 后发布新的 Mac Release。

## 输入监控授权

键鼠监听走 HID 层只监听的 event tap，需要用户在「系统设置 > 隐私与安全性 > 输入监控」授权。授权绑定代码签名身份，重装后可能出现「功能可用但设置列表里没有本应用」：旧 TCC 记录仍在生效，而旧版授权请求在 preflight 为真时会跳过，列表那一行就再也没被重建。现在启动时会在已授权前提下静默重新登记，主菜单「启用键鼠监听…」也会无条件补登记；诊断与手动重置见 [输入监控授权说明](../docs/macos/INPUT_PERMISSION.md)。发布时设置 `APPLE_SIGNING_IDENTITY` 用固定证书签名，可让授权跨版本保留。

## 三条刻意不动的约定

1. **Bundle identifier 保持 `dev.w33d.wuwatappo.fullatlas.mac`**。改了等于换一个应用：已有用户的 `~/Library/Application Support/` 与钥匙串全部作废。改名只动显示名与产物名。
2. **`fullatlas-source/` 与 `tools/`、`fullatlas-*` 模块名保持上游命名**，那是上游项目和它自己的模块结构，动了就断链。
3. **`src-tauri/src/*.rs` 里发往官方 CDN 的 User-Agent 常量保留旧名**：纯网络标识，与数据目录无关，改它没有收益。

## 搬目录时最容易踩的坑

`src-tauri/tauri.conf.json` 的 `bundle.resources` 会往父目录取文件（`../../LICENSE`、`../../THIRD_PARTY_NOTICES.md`、`../../licenses/FULLATLAS_LICENSE`、`../build/fullatlas-catalog.json`）。这类路径写错，本地构建不报错，只有 macOS 打包或运行期取资源时才炸——所以 `tests/package-macos.test.mjs` 会逐个核对已入库资源是否真实存在。

同理，`regions.mjs` 里的 `frontendDir`、`preparedDir`、`iconsDir`、`tauriConfig` 都是相对本目录的路径；`package-macos.sh` 还会强校验 `tauri.conf.json` 的 `productName` 与 `regions.mjs` 完全一致（国际服配置漏改会直接断构建）。
