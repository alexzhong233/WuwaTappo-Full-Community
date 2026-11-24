# WuwaTappo Full Community 1.4.1-beta.1

## 1.4.1-beta.1 修复与增强

- **修复装 Mod 后背包对应分页空白及图鉴 MOD 分栏展开不显示条目**：移除官方 `1.0.5` 已不存在的 `withAssetUrlCacheBust` 调用，并将 Mod 目录条目同步合并进官方运行期 `appearanceAssetCatalog` 与 `appearanceAtlasCatalog`，同时为背包 Mod 行补齐 `asset_code`。
- **修复本地预设（`preset_id < 0`）无法删除及应用串参**：`param_of` 改为按 AST 解析后的函数头区间切片提取首个形参，避免被同文件中更靠前的同名函数 `yn` / `an` 劫持；负数本地预设不再透传进官方预设接口。
- **修复 macOS 装/用 Mod 触发「网络繁忙（`code.1`）」**：为 `select-appearance`、`reorder-emotes`、联机房间表情同步 `syncEmotion`、面板收藏及 `beginSelect` 拒绝回退路径加上 `isModAsset` 无条件隔离，防止合成 Mod ID 或未持有条目打入官方接口；补齐 macOS 侧 `diagnostic/synth` 路由白名单。
- **修复 macOS 裸按数字键 `1~5` 误触发 `Alt+1~5` 表情**：原生输入监听改用当前事件自带的 `CGEventFlags` 判定左右修饰键并回退 `CGEventSourceKeyState(1, code)`，在 `KeyDown`/`FlagsChanged` 前自动清理已松开的残留修饰键；前端补充 `keyup` 与窗口 `blur` 清空 `heldAlt`。
- **修复按可用点数自动点击未达下限前提前触发**：为 `auto_click_balance` 引入初始阈值门控，未登录时或每次开启后未曾低于 `auto_click_below` 前保持暂停，跌破下限后再按迟滞区间运行直到高于 `auto_click_above`。
