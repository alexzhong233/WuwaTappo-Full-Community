# Release Notes — 1.4.1

本版基于 `1.4.0` 正式版进行稳定化维护，重点修复 Mod 背包/图鉴与预览兼容性、全局热键误触、多显示器切换与双端右键菜单同步，以及更新回滚与主窗口交互浮层穿透问题。

## 一、桌面交互与多屏增强

- **双端右键 / 托盘菜单同步并新增「切换屏幕」**：对齐 Windows 与 macOS 右键/托盘菜单项顺序与功能，新增多显示器「切换屏幕」快捷项（直接复用官方 `1.0.5` `switchDisplayMonitor` 能力并区分单屏跳过与实际切换失败提示），同时为 Windows 菜单补齐 `0%` 不透明度预设与「查看日志」（`/diagnostic/open-logs`）。
- **主窗口浮层点击命中修复**：为透明主窗口的更新提示条（`.wt-soft-update`）、强制更新遮罩（`.wt-gate`）与浮层控制台（`.wt-console`）接入 `[data-wuwa-interactive]` 原生命中区同步与鼠标接管，避免未与桌宠重叠时点击穿透到桌面。

## 二、Mod 与混合外观修复

- **修复装 Mod 后背包/图鉴显示与预览裂图**：移除官方 `1.0.5` 已不存在的 `withAssetUrlCacheBust` 调用，合并 Mod 目录进运行期 `appearanceAssetCatalog` / `appearanceAtlasCatalog`，并为背包 Mod 行补齐 `asset_code`。
- **修复背包预览裂图 `[?]` 与「MOD」角标遮挡**：同时按规范化类别下的 `asset_name` 与文件名主干建立 Mod 资源映射（支持 `.webp`、URL 编码文件名与 `_body_N` / `_head_N` 子帧，多 Mod 同名文件按当前已装备外观消歧），切回官方模式时清理全局映射；`.atlas-thumb--mod::after` 移至右下角避让官方左下角类型角标。
- **修复本地预设与混合模式接口隔离**：本地预设（`preset_id < 0`）不再误入官方接口，`applyPreset` 正确读取数值型 `meta.asset_id`；为选配、表情排序、房间表情同步与面板收藏补齐 `isModAsset` 与已持有状态分流，同步背包与图鉴间的 Mod 收藏状态，并修复通用挂饰（`role_id === '0'`）保留、5 槽满位表情替换、卸载 Mod 跨窗口清理与控制台浮翼尺寸计算。

## 三、后端、热键与原地更新可靠性

- **热键与自动任务门控**：修复 macOS 裸按数字键 `1~5` 误触发 `Alt+1~5` 表情的问题；为按余额自动点击增加初始阈值门控（需先低于下限或处于已启用的迟滞窗口内再触发）。
- **更新回滚与双端稳定性**：
  - 修复 Windows 更新失败触发回滚时 `taskkill` 未排除更新助手自身 PID 的问题，确保能完整恢复备份并记录结果；关闭旧实例前将 `flush()` 纳入 `try/finally`，并防范更新退出超时时重复拉起实例。
  - 修复 macOS 更新启动超时回滚时 `pkill` 后等待进程退出并兜底 `pkill -9`，统一 `official/update-check` 网络错误返回与请求体上限。
  - 共享后端统一默认端口 `43229` 与 `package_config.json` 端口回落、为日志轮转增加互斥锁、`/assistant/state` 只读查询免写盘，Windows 启动器校验 `/status` 的 `region` 防跨区服串端口。

## 四、下载与升级说明

- **已安装旧版（1.3.2 / 1.4.0 / 1.4.1-beta.*）**：进入控制台 **「系统与更新」 → 「检查更新」**，支持后台自动下载、校验并重启覆盖升级（保留账号、本地预设、Mod 资源包与设置）。
- **首次安装 / 手动升级**：根据地区与系统下载对应安装包并核对 `SHA256SUMS.txt`：
  - **Windows x64**：国服 `WuwaTappo-Full-CN-Community-Windows-x64-1.4.1.exe` / 国际服 `WuwaTappo-Full-Global-Community-Windows-x64-1.4.1.exe`。
  - **macOS (Apple Silicon, macOS 13+)**：国服 `WuwaTappo-Full-CN-Community-1.4.1-macOS-arm64.dmg` / 国际服 `WuwaTappo-Full-Global-Community-1.4.1-macOS-arm64.dmg`（因临时签名，首次安装或原地升级重启后若提示权限，请按引导在系统设置重新勾选一次「输入监控」）。
