# Mod 功能实现 review

> 对象：`origin/feature/custom-local-resources` @ `a917043`（"支持导入 Mod ZIP 并新增「全图鉴 + Mod」外观模式"，2127 行）
> 方法：逐文件读码 + 在独立 worktree 里跑 `fullatlas-mod.test.mjs`（4 pass）、`platform-parity.test.mjs`（5 pass）、`windows/src/tests/test_custom*.py`（9 pass）。
> 本文只做评估，未改代码。

---

## 结论

方向对，落地也扎实：官方缓存只读、叠加在运行时、ZIP 校验覆盖到位、ID 稳定、双平台同一套行为测试。核心架构（`asset:` 协议 + 应答期叠加）与上一版方案推荐的路线一致，且规避了 ATS 和新增 localhost 服务。

问题集中在四类：**打包会把用户 Mod 打进发行包**、**Mod 层能拖垮基础功能**、**平台差异靠手改共享文件维持（在吃 parity 预算）**、**Windows 侧测试不在 CI**。

---

## 一、已经做对的（先说这个，免得像在刷问题）

| 项 | 证据 |
| --- | --- |
| 官方缓存只读，叠加只在运行时 | `custom_resources.rs:340` `merge_official` 克隆 official，测试断言 `original == before` |
| 删除/替换包能精确回滚 | `install_archive` 先 `.previous` 再 rename，失败还原；测试 `archive_import_replaces_..._rolls_back_invalid_input` |
| ID 稳定：换图、调序、重装都不换号 | 包名 hash + `kind\0name` hash + 稳定排序；两侧都断言了 `1_407_742_458_454_113` / `0xa430d84680aabd0b` |
| ZIP 校验确实严 | 路径逃逸、符号链接、Windows 保留名、重复路径（大小写折叠）、魔数、压缩比、512MiB/1GiB/20000 条上限 |
| 前端改动克制 | 两侧控制器差异 3 行（上限 4），共享行为由 `fullatlas-mod.test.mjs` 双平台各跑一遍 |
| 文案与文档同步 | `MODS.md` 的格式说明与 `parse_pack` 的校验逐条对得上 |

`inspect_bundle` / `parse_pack` 允许条目缺 `id`、由包名+类型+名称推导，这个设计对作者友好，且天然保证"同名即同一物品"。

---

## 二、按严重度排的问题

### 1. 发行包会把用户导入的 Mod 一起打进去

`windows/tools/payload.ps1` 的排除表里有 `profile\*`、`cache\*`、几个运行期 json、`*.log`，**没有 `custom-atlas\*`**。而 Mod 就存在 `app\custom-atlas\`（`local_host.py:47`），`app\` 正是打包源的根。

后果：维护机上一旦导入过 Mod 再执行 `npm run build:win -- --package`，zip / NSIS 里就带上别人（或自己）的整套图片。这个文件头注释自己写着"两边各写一遍迟早会漂移——比如运行期状态被漏排除，用户装完就带着别人的 token"，漏的正是同一类东西。

修复：`payload.ps1` 加 `if ($rel -like 'custom-atlas\*') { $skip = $true }`，并在 `windows/README.md` 的目录结构里注明该目录是运行期状态。

顺带确认过：`deploy.ps1` / `apply-update.ps1` 用的是 `robocopy /E`（无 `/MIR` `/PURGE`），所以更新不会删掉用户的 Mod —— 这点是对的，不用改。

### 2. Mod 层是硬耦合：merge 一失败，全图鉴整体不可用

`fullatlas.rs` 的 `catalog()` 现在无条件走 merge：

```rust
let official = self.official_catalog(app, refresh)?;
let value = crate::custom_resources::merge_official(&official, &self.dir.join("custom-atlas"), region)?;
```

而 `merge_official` 开头对 6 个 catalog kind 做硬校验，任一缺失就 `Err("官方图鉴缓存不完整，无法叠加 Mod")`。这个 `Err` 会一路冒到 `/offline-data` 和 `/mode`。

三个后果：

1. **一个 Mod 都没装的用户也承担这条失败路径**。今天不会触发（`validate_catalog` 恰好也要求同样 6 个 kind），但哪天官方加一个 kind、而 `validate_catalog` 放宽了，全图鉴会整体打不开，且原因显示成"无法叠加 Mod"——用户根本没装 Mod。
2. **新 kind 静默不叠加**。`merge_official` 只往固定的 6 个 kind 里 merge，官方新增类型时 Mod 无法出现在新分类，也没有任何提示。
3. **两侧失败模式不一致**：Python 版只在有 pack 时才校验 kind，无 pack 时完全不查；Rust 版无 pack 也校验。同一个"官方结构变了"在 Windows 上表现为"Mod 不显示"，在 macOS 上表现为"全图鉴打不开"。

建议：`merge_official` 入口先短路——目录不存在或没有有效包时直接返回 official 的克隆；merge 内部出错时降级返回 official 并记一条诊断，而不是让基础功能跟着挂。同时把 kind 列表抽成常量，与 `validate_catalog` 共用一处定义。

### 3. 平台差异靠"改共享文件"维持，正在吃 parity 预算

两侧控制器唯一的实质差异是这行：

```js
// macos
const convert = path => d.window.__TAURI_INTERNALS__.convertFileSrc(path);
// windows
const convert = null;
```

`__TAURI_INTERNALS__.convertFileSrc` 在所有 Tauri v2 应用里都存在（我在官方 `WuwaTappo.official.exe` 里找到了 init.js 原文：Windows 下返回 `http://asset.localhost/...`）。也就是说**这个分支不可能靠运行时特性嗅探自动分流**，只能靠两侧手写不同代码，而且要一直同步维护。

代价是可量化的：差异从 2 行涨到 3 行，上限 4 行。以后再加一条平台差异（比如某平台要支持 webp 转换、要不同的缓存策略）就会红。

更稳的写法：把"如何把包内相对路径变成可加载 URL"作为注入项，由两个 injector 各自提供：

```js
// macos/tools/fullatlas-integration.py
customAssetUrl: path => window.__TAURI_INTERNALS__.convertFileSrc(path),
// windows/src/tools/build_integration.py
customAssetUrl: path => 'http://127.0.0.1:43229/' + path.split('/').map(encodeURIComponent).join('/'),
```

共享文件里只写 `d.customAssetUrl(path)`。这样平台差异回到本来就应该分平台的地方（injector），差异行数回落，而且以后再分叉不占预算。

### 4. Windows 侧 9 个 Python 测试完全不在 CI 里

`.github/workflows/` 里没有任何 unittest/pytest 调用，`build-windows.yml` 只把 python 当构建工具用。`windows/src/tests/test_custom_resources.py`（8 个）和 `test_custom_resources_host.py`（1 个）我本地跑是 9 pass，但**没有任何门禁**——下次重构静默回归也不会有人知道。

mac 侧相反：`npm test` 用 `macos/tests/*.test.mjs` 通配，`fullatlas-mod.test.mjs` 会被自动带上。

建议：在 `build-windows.yml` 里加一步 `python -m unittest discover -s windows/src/tests -p "test_*.py"`（这些测试只依赖标准库，加进去成本几乎为零）。

### 5. 注入器用「到下一个 function 为止」定位，有可能越界改写

`macos/tools/fullatlas-integration.py`：

```python
pattern = r'function ' + name + r'\([^\n]*\) \{[\s\S]*?(?=\n(?:async )?function )'
```

非贪婪没错，但终止条件是"下一个 function 声明"。如果目标函数后面紧跟的是 `const x = () =>` 或对象字面量里的方法，匹配会一路吞到更远的 `function`，期间**所有** `^https?://` 都被改成 `(?:https?|asset)://`。assert 只保证"改动了"，不保证"没改多"。

建议加两条断言：捕获块长度上限（如 `{0,3000}?`）、块内不含 `\nfunction `。1.0.4 上目前没出问题，但这是下次同步官方前端时最容易静默变形的地方。

### 6. `game_role_id: 0` 被 `||` 当成"缺失"

自定义角色的 `game_role_id` 是 0（`custom_resources.rs:313`），控制器两处用 `||` 兜底：

```js
role_id: String(d.getAppearanceAssetById(character?.id)?.game_role_id || official?.role_id || '')
role_id: String(meta?.game_role_id || entry.role_id || '')
```

`0` 是 falsy，于是自定义角色选中后 `role_id` 回落到**账号真实角色**。影响：`select()` 里"配件/表情只能用于对应角色"的校验会用错角色比较。表现是偶发放行或误拦，不崩但难解释。

建议改 `??`，并显式约定 `game_role_id = 0` 表示"无归属、不做角色绑定校验"。

### 7. 每次 `/offline-data` 都全量解析所有包，没有索引缓存

`merge_official` 逐目录 `parse_pack`：读 `manifest.json` + `items.json`，然后对每条资源的每个图片 `is_file()`。`catalog()` 的内存 memo 只在 `custom/import|remove` 后被清空，但 `sync`（refresh=true）、首次进页面、每个 WebView 的 `prepare()` 都会触发。

上限是 5000 项/包，pack 一多，开本地模式的那一下会明显卡（而且是在 UI 线程等待的 IPC 超时区间内）。建议按 `(path, mtime, size)` 做一层解析结果缓存，或把合并结果落盘成 `custom-atlas/index.json`，由导入/移除时重写。

### 8. catalog 顶层永远带 `customAssetPaths`，空对象也会触发全量深拷贝

Rust 版无条件写 `merged["customAssetPaths"] = paths`，即便没有任何 Mod。前端第一步就是：

```js
if (!data.customAssetPaths) return data.catalog;
const catalog = JSON.parse(JSON.stringify(data.catalog));
```

空对象是 truthy，于是**每次模式切换/同步都深拷贝 651KB catalog 并遍历所有 url**。Python 版有 `if packs:` 保护，两边行为不一致。

建议两侧统一：没有覆盖项时不写该字段；前端也顺手判 `Object.keys(...).length`。

### 9. 未知文件一律拒绝，社区包里的 README.txt 会让整包导入失败

两侧都是：根目录除了 `manifest.json` / `items.json` / `assets/**` 之外任何文件都报错。作者往包里放一个说明文件是很常见的习惯，报错信息是"ZIP 包含未知文件：README.txt"——对作者不友好，也没在 `MODS.md` 里显式提醒。

建议：根目录 `*.md` / `*.txt` / `LICENSE*` 静默忽略（不写盘），其余未知文件仍拒绝。

### 10. `viewUser` 的过滤改变了官方配件的语义，未写进文档

```js
accessory: (() => { const item = ...; return d.getAppearanceAssetById(item?.id) ? item : null; })(),
emote: (s.emote || official?.emote || []).filter(item => d.getAppearanceAssetById(item.id)),
```

方向是对的（防止容器里查不到导致渲染失败），但它同时会**丢掉"官方有、本地目录没有"的配件**：本地图鉴缓存落后于账号时装的内容时会静默消失，且没有提示。建议只对 `custom_pack_id` 的条目做这层过滤，官方条目保留原语义（或在剔除时记一条诊断）。

---

## 三、与设计目标的两处偏差

1. **只支持"新增条目"，不支持"覆盖官方条目"。** 现在 Mod 产出的是一批 1e9 量级的新 id 和新的 atlas 行，官方条目完全不动。安全是安全，但社区最先想要的多半是"把某个官方皮肤换成我的图"。加入覆盖模式其实很便宜：`items.json` 里加 `"target": 官方 asset_id`，`materialize` 时直接往官方那个 id 上写 url 而不是新建 id——不用碰缓存文件，也不用动 `validate_catalog`。
2. **没有 L1 离线资源包。** Windows 首启仍要下 106MB，macOS 连图片缓存都没有（`ASSET_URL_CACHE_BUST` 每次启动都变，冷启必然全量回源）。这套 `custom-atlas/` 目录 + 挂载点其实已经能承载"社区分享的素材包"，成本主要是导入时别把它当"图鉴条目"解析。这可能是比自制皮肤更大的一块收益。

---

## 四、建议的下一步（按投入产出排）

| 顺序 | 动作 | 成本 | 收益 |
| --- | --- | --- | --- |
| 1 | `payload.ps1` 排除 `custom-atlas\*` | 1 行 | 避免发行包夹带用户资源 |
| 2 | `merge_official` 无包短路 + 失败降级 | ~10 行 | 去掉 Mod 层对基础功能的硬耦合 |
| 3 | Windows Python 测试进 CI | 1 步 | 现有 9 个测试变成门禁 |
| 4 | 图片 URL 生成改为 injector 注入 | ~15 行 ×2 | parity 预算回落，后续分叉不占额度 |
| 5 | `game_role_id` 的 `||` 改 `??` | 2 行 | 修角色绑定校验错位 |
| 6 | 空 `customAssetPaths` 不写 + 前端判空 | ~5 行 | 去掉每次切换的 651KB 深拷贝 |
| 7 | 根目录 README/LICENSE 放行 | ~5 行 | 少一个必然被踩的导入失败 |
| 8 | 解析结果按 mtime 缓存 | ~40 行 | 多包场景的开关卡顿 |

第 1–3 条加起来不到 20 行，建议先单独发一个 commit。

---

## 附：本轮处理结果（分支 `feature/mod-ui`）

| 问题 | 状态 |
| --- | --- |
| 1 打包夹带用户 Mod | 已修：`payload.ps1` 排除 `custom-atlas\*` |
| 2 Mod 层硬耦合 | 已修：无包时纯透传；叠加失败降级为官方图鉴，原因经 `custom/status.error` 回到面板 |
| 3 平台差异吃 parity 预算 | 已修：图片 URL 改由注入器提供 `customAssetUrl`，控制器回到 2 行差异；缺省实现不猜地址 |
| 4 Windows Python 测试不在 CI | 已修：`build-windows.yml` 增加 `test_custom*.py`（其余测试依赖 psutil / PowerShell 执行策略，未纳入门禁） |
| 5 注入器正则越界 | 未处理（下次同步官方前端时收窄） |
| 6 `game_role_id: 0` 被 `\|\|` 当缺失 | 未处理 |
| 7 每次请求全量解析 | 未处理（`/custom/status` 现在还会展开条目，量更大） |
| 8 空 `customAssetPaths` 深拷贝 | 已修：两侧都不再写空对象，前端也判空 |
| 9 未知文件整包失败 | 未处理 |
| 10 `viewUser` 静默丢官方配件 | 未处理 |

本轮另加：Mod 管理面板（独立 Tab）、Mod 条目 `MOD` 角标（仅本地模式挂载）。

## 必须先解决的前置问题

这条分支的基底是官方 **1.0.3**，`main` 已经是 **1.0.4**。本地执行 `npm run prepare:frontend` 会停在：

```text
Error: Original CN SDK reload function changed; reconcile the login reload patch before building.
```

即：分支在当前 1.0.4 前端上无法构建。上机验收前必须先 rebase 到 `main`，并逐条核对 `patch-frontend.mjs` 与两个注入器的锚点（本轮新增的 `customAssetUrl` 插入点也在其中）。

