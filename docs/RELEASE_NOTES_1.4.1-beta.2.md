# WuwaTappo Full Community 1.4.1-beta.2

## 1.4.1-beta.2 修复与增强

- **修复装 Mod 后背包预览裂图 `[?]` 与「MOD」角标遮挡**：`registerModNames` 与 `__WUWA_MOD_ASSET_PATH__` 改为同时索引规范化类别下的 `asset_name` 与文件名主干（支持 `.webp` / URL 编码文件名与 `_body_N` / `_head_N` 子帧，并在多 Mod 同名文件时按当前已装备外观消歧），切回官方模式时清理全局映射；`.atlas-thumb--mod::after` 移至卡片右下角，避免被左下角官方类型角标遮挡。
- **双端右键 / 托盘菜单同步并新增「切换屏幕」**：Windows 与 macOS 菜单项顺序与功能对齐，新增多显示器「切换屏幕」快捷项（直接复用官方 `1.0.5` `switchDisplayMonitor` 逻辑并在单屏不可切换时给出提示），同时为 Windows 右键菜单补齐 `0%` 不透明度与「查看日志」（`/diagnostic/open-logs`）。
- **修复双端后端、更新流程与注入器一致性缺陷**：
  - 共享/Windows 端对齐默认端口 `43229` 与 `package_config.json` 端口回落、移除过时 `asset_count`/`atlas_count` 强制覆盖、只读 `/assistant/state` 查询免写盘、为日志轮转增加互斥锁、补齐非数组房间状态容错、`/assets/*` 启用不可变缓存及 `path_of` URL 解码；Windows 启动器校验 `/status` 的 `region` 防跨区服串端口，更新回滚前结束残留进程并在旧主程序仍在运行时避免重复拉起，`desktop_assistant` 退出前将 `flush()` 纳入 `try/finally` 确保执行 `quit()`。
  - macOS 端清理重复共享动作白名单并统一请求体上限至 `65536`，修复 `official/update-check` 网络失败返回与消息分支判定、更新超时未生成 `healthy` 标记时触发 `pkill` 与回滚恢复、`WindowState` 鼠标穿透状态同步及主窗口浮层弹窗（`.wt-soft-update` / `.wt-gate`）点击放行。
  - 修复混合模式下已拥有官方外观收藏、无限制表情 5 槽满位替换、通用挂饰（`role_id === '0'`）切换角色保留、`applyPreset` ID 读取、卸载 Mod 跨窗口清理选中项、设备栏浮翼尺寸与同窗网格重排、控制台标签页切换防重建，并清理无引用的遗留 `windows/src/installer/setup.iss`。
