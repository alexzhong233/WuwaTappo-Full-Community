# Windows 构建

构建期与发行包均不再包含或依赖 Python：解包元数据提取、前端注入、PE 重打包、快捷方式生成、本地服务 `LocalGalleryHost.exe`（含 `--prepare-cache`、`--make-update`、`--apply-update`）以及无控制台启动器 `WuwaTappoLauncher.exe` 全部基于 `shared/backend` 的 Rust 产物与仓库根 `scripts/` Node 工具链。

## 前置

| 依赖 | 用途 |
| --- | --- |
| Windows x64 | 只能在 Windows 上构建（要跑官方 Windows 客户端与校验 Authenticode） |
| Node.js 24+ | 提取官方安装包资源、解析 JS 结构、分派构建与获取官方基底 |
| 7-Zip | 解包官方安装器外层（与 NSIS 提取器配合，从不执行 setup.exe） |
| Rust stable（`cargo` + MSVC 链接器 + Windows SDK） | 编译 `assistant-core`（注入/PE 重打包/快捷方式）、本地服务与启动器 |

Rust 部分需要 MSVC 的 `link.exe` 和 Windows SDK 的 `.lib`。装了 Visual Studio 的话，
最省事的是在「Developer PowerShell for VS」里构建；只装了 rustup 没装 C++ 生成工具时，
`cargo build` 会以找不到链接器或 `kernel32.lib` 失败。

## 官方基底

基底版本与安装包直链锁定在 `shared/upstream/cn.json`（国际服是 `shared/upstream/global.json`），
不需要手动准备：

```powershell
node scripts/fetch-official-win.mjs --region=cn   # 按锁定版本取包，校验 sha256 与 EV 签名后解出主程序
```

官方主程序会经过官方发行方 Authenticode EV 签名校验；异常即终止。

## 构建

在仓库根执行（推荐，会自动分派并带上区域）：

```powershell
npm run build:win
npm run build:win -- --package        # 顺带产出安装包
npm run build:win -- --no-runtime     # 不内嵌 WebView2 私有运行时（体积小，回退系统运行时）
```

单独跑也等价（进入 `windows` 目录）：

```powershell
powershell -ExecutionPolicy Bypass -File tools\build.ps1 [-Extract] [-NoDeploy] [-NoRuntime] [-Base <目录>]
```

流程是：解包官方 EXE（`scripts/extract-assets.mjs` + `assistant-core extract-pe`）→ 注入图鉴与助手并重打包 PE（`assistant-core inject --pack-pe`）→ 编译 Rust 本地服务 → 编译启动器 → 装配到 `app\`。

- `-Extract` 重新解包官方包并重算 `js_structure.json`（改了注入锚点或换了基底才需要）
- `-NoDeploy` 只出 `build\` 产物，不动 `app\`
- 国服产物在 `windows\build\`，国际服在 `windows\build-global\`

装配会清掉旧版 PyInstaller 宿主留下的 `app\_internal\` 与 `app\host\`，再整目录铺一遍；
账号目录、`local_config.json`、`offline_state.json`、`assistant_state.json` 不会被覆盖。

## 验证

```sh
npm test                                          # 前端/注入器接口对账（双端）+ 共享控制器与结构守卫测试
cargo test --locked --manifest-path shared/backend/Cargo.toml
```

构建后可以直接起宿主验接口（`local_config.json` 里有端口与 token）：

```powershell
cd app; .\LocalGalleryHost.exe
# 另开一个窗口
curl http://127.0.0.1:43229/status
```

`/status` 的 `ready` 只有在 `cache\asset_manifest.json` 与素材齐全时才为 `true`；
没有内嵌缓存时首次启动会按需回源官方 CDN。

发布前必须装到含空格或中文的新目录实测：本地图鉴、联机房间可见、更新入口、
开机自启开关（`/startup`）和桌宠右键菜单（`/assistant/context-menu`）都要点一遍。

## 打包

```powershell
powershell -ExecutionPolicy Bypass -File tools\package-installer.ps1
```

产出三样东西：NSIS 安装包 `dist\WuwaTappo-Full-{CN|Global}-Community-Windows-x64-<版本>.exe`、
与原地升级 ZIP `dist\...-win-x64-update.zip`。

升级 ZIP 只含程序本体：`WuwaTappo.Local.exe`、`LocalGalleryHost.exe`、`WuwaTappoLauncher.exe`、
`package_config.json`（已剥掉本机 token）、图标与许可文件；不含 `local_config.json`、账号目录、
缓存、导入的 Mod 和设置。客户端只接受本仓库 Release、匹配版本清单、精确文件列表和逐文件 SHA256。

`build\`、`dist\`、`inputs\`、`app\`、`assets\` 都是产物或缓存，不进 Git。
