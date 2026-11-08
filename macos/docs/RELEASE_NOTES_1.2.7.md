# Release Notes — 1.2.7

## Windows

- 适配官方 1.0.4 基底（11-05 发布，132 个资源锚点全部命中，无行为变更）
- 桌面助手适配器白名单放行 1.0.4；宝箱就绪/忙碌引用改为结构化解析，不再依赖官方压缩变量名（本次官方将 `Wn/Gn/Dn/kt` 漂移为 `Vn/Wn/On/_t`，此后版本漂移可自适配）
- `update-watch.ps1` staging 流程补齐 `build_integration.py` 依赖（`current_base.txt` / `local_config.json`，无 BOM 写入）与 `inspect_js` 步骤，试注入不再必挂

## 安装包

- 命名沿用 `WuwaTappo-Full-{CN|Global}-Community-*`，macOS 仅 DMG、Windows 仅 EXE（NSIS 安装器，双击快捷方式即用）
- Release 仍要求 mac + win 四个构建全部成功才发布

## 其他

- 修复本地部署后图鉴资源全部报「找不到资源」的路径问题（发行包不带 cache 属设计行为，v1.2.5 起 host 已带首次自动拉取逻辑）
