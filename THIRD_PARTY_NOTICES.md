# 第三方内容

## 社区项目来源

- macOS 壳层与构建工具来自 [Last-emo-boy/wuwa-tappo-mac](https://github.com/Last-emo-boy/wuwa-tappo-mac)，同步至提交 `c4dd4bd9c5e1936ec920c09d9b00d902395eeccd`，MIT 许可；原始版权声明保留在 `LICENSE`。
- FullAtlas 控制器、助手 UI 和集成工具来自 [Averionx/WuwaTappo-FullAtlas](https://github.com/Averionx/WuwaTappo-FullAtlas)，同步至提交 `2debed109b63824027b04d19fcedf74cca1d3a23`（1.3.6 系列），MIT 许可；原始版权与许可全文保留在 `licenses/FULLATLAS_LICENSE`。本项目将其本机服务传输适配为 Tauri/Rust，并移除 Windows 安装更新路径。
- Windows 侧（`windows/`）同样基于 Averionx/WuwaTappo-FullAtlas 的 v1.2 源码，保留其 Python 本机服务与注入工具，仅做官方 1.0.3 适配与 PE 重打包，MIT 许可；该目录内的 `src/LICENSE` 与 `src/THIRD_PARTY_NOTICES.md` 为上游原文。

## Wuwa Tappo 国服与国际服

本项目使用对应地区官方客户端的界面、角色、美术、音频、字体及其他内嵌资源，并通过该地区官方服务完成登录和业务请求。相关内容、名称和商标属于库洛及各自权利人，不适用本仓库的 MIT 许可。

- 国服下载入口：https://download-mc-wuwatappo.kurogames-global.com/download/
- 国际服下载入口：https://download-wutheringwaves-wuwatappo.kurogames-global.com/download/en/
- 构建所用版本、下载地址和完整性校验：[`upstream-cn.json`](upstream-cn.json)、[`upstream-global.json`](upstream-global.json)

安装包在构建时从官方客户端提取资源并完整打包；源代码仓库不存放个人账号数据。使用本项目仍需遵守官方服务条款。本项目由社区维护，与官方发行渠道独立。

## Preact

`tests/fixtures/kuro-sdk-preact.js` 中的渲染器测试代码使用 Preact，采用 MIT 许可；完整版权与许可声明保留在该文件头部。

## 构建依赖

Tauri、Rust crates、Node.js 包及 7-Zip 采用各自许可。依赖版本分别记录在 `src-tauri/Cargo.lock` 与 `package-lock.json`；7-Zip 仅用于构建时静态解包，不随应用分发。
