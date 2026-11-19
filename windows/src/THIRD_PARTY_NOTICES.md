# 第三方内容说明

本项目名称和功能描述仅用于识别兼容的原程序，不表示官方认可或授权。

| 内容 | 说明 |
| --- | --- |
| WuwaTappo 原程序、原界面及其美术素材 | 权利归原权利人。仓库不包含原版二进制及完整提取前端。使用经过修改的官方客户端及从其公开素材接口拉取的离线缓存；不声称拥有这些内容，也不以 MIT 重新授权。 |
| Microsoft Edge WebView2 Fixed Version Runtime x64 | Microsoft 的第三方运行时，仅在离线完整构建内嵌，默认发行包回退系统 Evergreen 运行时，适用其独立许可条款。文档：https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution 。 |
| Rust crates（`shared/backend`） | Windows 本地服务 `LocalGalleryHost.exe` 与启动器 `WuwaTappoLauncher.exe` 由共享 Rust 工作区编译（`reqwest`、`serde`、`sha2`、`tiny_http`、`zip` 等），适用各自 MIT / Apache-2.0 等开源许可，版本锁定在 `shared/backend/Cargo.lock`。 |
| Acorn 8.15.0 | 构建时的 JavaScript 解析器，MIT License，许可见仓库根目录依赖锁及上游许可。 |
| Brotli、pefile | 仅为构建期解包/注入依赖（不随发行包分发），不改变它们各自的授权；安装依赖时获取其原有许可证。 |
| NSIS | 用于生成 Windows 用户级安装包的构建工具，保留其安装器自身标识；https://nsis.sourceforge.io/ 。 |

MIT 许可证只覆盖本项目新编写的修改逻辑、工具和文档，不构成对第三方内容的额外授权。
