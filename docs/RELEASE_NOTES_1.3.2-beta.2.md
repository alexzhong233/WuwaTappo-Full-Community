# Release Notes — 1.3.2-beta.2

本版在前序 `1.3.2-beta.1` 的基础上完成了 macOS 与 Windows 的行为对齐及跨平台共享代码收敛，两平台继续统一基于官方国服 / 国际服 `1.0.4` 构建。

## 双端界面与注入行为对齐

- 原生面板导航在「联机」与「设置」之间新增统一的「控制台」内嵌页签，并统一 macOS 窗口/菜单的 `"console"` 路由与更新锁例外逻辑。
- 双端共用 `activity-config` 后台刷新不触发全屏遮罩规则，并将默认隐藏任务栏/程序坞图标（含历史设置首次迁移）与前端 `VERSION` 收敛至 `shared/` 单一源。
- 统一 `controller_init` 与 `settings_ui` 注入模板，精简两平台独立适配器实现。

## 跨平台共享层收敛

- 将 NSIS 静态解包、上游版本检查、镜像、缓存键、Windows 锁提取与发布产物清单等通用脚本统一迁至根目录 `scripts/`。
- `shared/backend` 统一提供随机令牌生成、`client_diagnostics.jsonl` 8 MiB 轮转、运行配置初始化、Mod 资源路径/MIME 安全解析、兼容数字/字符串 ID 的官方图鉴目录校验、增量图鉴同步进度、GitHub Release HTML 兜底以及官方 Windows 原包下载。
- 接口对账测试扩展至同时核验 Windows（`local_gallery_host.rs`）与 macOS（`fullatlas.rs`）两个 Rust 宿主。

## Windows 修复与构建精简

- 修复开机自启托管优先注册无控制台启动器 `WuwaTappoLauncher.exe`（旧部署缺少时回退 `launch.ps1`）。
- 修复便携 ZIP 与安装包 EXE 的双扩展名输出问题，安装包内升级包改由 `LocalGalleryHost.exe --make-update` 构建。
- 装配阶段为 `package_config.json` 剥离本机 `token` 并移除多余 `host/` 拷贝；发行文件收集改用 `shared/contracts/windows-update.json` 严格白名单。
- 移除已由 Rust 宿主承接的重复 Python/Tkinter 更新与缓存同步脚本，构建依赖精简为 `pefile` 与 `Brotli`。

## 已知事项与基线

- 预发布测试版本，完整端到端打包与业务功能将随 CI 构建及实机试用持续验证；macOS 侧编译与打包由 GitHub Actions `macos-15` runner 执行验证。
- 国服与国际服官方客户端基线均锁定为 `1.0.4`。
