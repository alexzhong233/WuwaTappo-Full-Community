# Wuwa Tappo 官方版本跟进

检查日期：2026-10-30

> 本文记录最初检查时的 1.0.2 基底状态；后续 `official-1.0.3` 分支已完成候选适配和打包，当前结果见 [1.0.3 适配记录](macos/OFFICIAL_1.0.3.md)。
>
> 本文写作时项目名为 `WuwaTappo-FullAtlas-Mac`，后更名为 `WuwaTappo-Full-Community`。正文保留当时的名称与原貌，不作追改。

## 两个上游的机制

- `Last-emo-boy/wuwa-tappo-mac` 使用 GitHub Actions 每日检查国服和国际服官方活动配置、更新指针及安装清单。新版安装包先校验 URL、文件大小和 SHA-256，再静态提取资源、生成前端契约差异、运行测试和 macOS 打包。全部通过后开普通 PR；资源成功提取但接口/构建不兼容时只开草稿 PR，不自动发布未经验证的版本。
- `Averionx/WuwaTappo-FullAtlas` 的桌面整合代码有它自己的版本支持边界。上游构建工具校验已知官方二进制；缓存同步模块独立刷新图鉴 JSON 与图片。Windows 桌面助手在启动和每 30 分钟检查 FullAtlas 自己的 GitHub Release，并按 ZIP 哈希、备份、健康检查和回滚流程更新 FullAtlas 程序。它不会自动把新版官方 Wuwa Tappo 适配为 Mac。
- 官方客户端、公开图鉴数据、FullAtlas 整合代码是三条更新线。Mac 应用不能安装官方 Windows EXE 或 FullAtlas 的 Windows 更新包。官方图鉴数据可以在 Mac 端刷新；Mac 程序仍需本项目产出新的原生构建。

## 本次检查

官方国服已从锁定的 `1.0.2` 更新为 `1.0.3`。检测到的官方安装包为：

- URL：`https://mc-wuwatappo.kurogames-global.com/app/0ad48bb0-55bf-49f0-b57d-2b9f3edc0c73/WuwaTappo-cn_1.0.3_x64-setup.exe`
- 大小：201,697,504 bytes
- SHA-256：`29b6f4aaeb0ad4db5a8002e3686435692fa0ddb88ea9389e89b53fb93932136e`
- 与当前 1.0.2 比较：官方前端入口与 Tauri invoke 包装结构变化；新增 3 个快捷键命令、3 个事件，移除 3 个原生对话框命令。计数缓存环境及已提取主业务契约保持相同。

最初检查时，FullAtlas Mac 构建仍以官方 `1.0.2` 为适配基底；`1.0.3` 当时只完成下载、静态提取和契约盘点。之后候选锁文件与适配代码仅在 `official-1.0.3` 分支更新，`main` 保持 1.0.2；实验版已能构建，完整业务验收尚未完成。

## 自动化边界

源代码中可以自动发现上游更新、静态提取、生成差异、测试并制作审阅用候选构建。自动化不能安全地推断未知官方 UI 改版或 FullAtlas 语义变化；这些差异必须由维护者适配并验证。定时检查只有在仓库的 GitHub Actions 工作流实际启用后才持续运行；本地代码本身不提供后台监控服务。
