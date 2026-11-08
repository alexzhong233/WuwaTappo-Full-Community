# 本地资源层（mod）设计评估与实施方案

> 状态：**提案，未实施**。基于 `main`（1.2.7 / 官方 1.0.4 适配）当前代码实测。
> 目标分支建议：从 `main` 开 `feature/mod-resource-layer`（现有 `feature/custom-local-resources` 与 main 零差异，可直接复用或废弃）。

---

## 一、结论先说

这个项目**已经具备本地资源层的全部零件，缺的只有一个「用户层」**：

- Windows 侧有一个本机 HTTP 服务（`127.0.0.1:43229`），**所有图片都从磁盘上的一个函数里读出来**。
- 官方前端解析图片地址时，**`http(s)://` 开头的绝对地址会被原样使用**，不做任何拼接。
- 所以：mod 不需要插件系统、不需要改前端 JS、不需要新协议栈。**只要在两处服务端的应答里，把被覆盖条目的 `url` 改写成指向本机的绝对地址**，前端自动生效。

反过来说，如果按「mod loader / 代码注入」去做，会同时踩三堵墙：3 个共享 JS 模块被 `platform-parity` 锁死在 ≤4 行差异、官方前端每次重建压缩名全漂、账号写回通道被 `restricted()` 硬拦。**不要走那条路。**

---

## 二、现状事实（代码依据）

### 2.1 两侧资源链路对照

| | Windows | macOS |
| --- | --- | --- |
| 图鉴数据 | `cache/offline_catalog.json`，经 `GET /offline-data` 下发 | `fullatlas.rs` 三級缓存（内存 → Application Support → 打包资源 → CDN），经 action `offline-data` 下发 |
| 图片地址 | 同步时被改写为 `assets/<sha256>.png`（相对） | **原样**指向官方 CDN |
| 图片读取 | `GET /assets/<name>`、`/character/*` 等 6 类前缀，**全部收口在 `Handler.cached_asset()`** | **无任何本地读取**，WebView 直连 CDN |
| 素材缓存 | `app/cache/`（实测 3098 张 / 106 MB） | **无** |
| 模式开关 | `app/offline_state.json` + `localStorage` | `fullatlas.json` + `localStorage` |
| 校验 | 缓存 manifest 的 size/sha256 | `validate_catalog()` 强校验每一行 atlas 必须映射到官方 catalog |
| 联机封锁 | fetch/XHR/WS/beacon + `restricted()` | 同左 + `checkInvoke()` |

关键位置：

- `windows/src/src/local_host.py:353-361` `cached_asset()` —— 唯一的图片出口，已带 `resolve().is_relative_to(ROOT)` 逃逸防护。**加 mod 就是在这一个函数前插一次查找。**
- `windows/src/src/local_host.py:431-439` `/assets/<name>`：拒绝 `/ \ ..`，未命中写 `client_diagnostics.jsonl`。
- `macos/build/frontend/assets/index-CM6qA69G.js` `resolveAppearanceAssetUrl()`：`if(/^https?:\/\//i.test(n))return n;` —— **绝对地址直通，官方代码，白送一个注入点**。
- `macos/tools/fullatlas-integration.py:85` `applyOfflineCatalog` 固定用 `offlineData.assets_host` 作为 host，而 `applyCatalog` 的守卫是 `t !== assetsHost 则 return`。**推论：catalog 顶层的 `assets_host` 不能动，只能逐条改 entry 的 `url`。**

### 2.2 macOS 侧三个既有缺口（做资源层时会被放大）

1. **零图片缓存 + 每次启动换 URL**：`ASSET_URL_CACHE_BUST=String(Date.now())` 是模块级常量，`withAssetUrlCacheBust()` 给每个图片地址追加 `?t=<启动时间戳>`。WKWebView 的 HTTP 缓存基本失效，**每次冷启都是全量回源 CDN**。
2. **`/sync` 只抓 JSON 不抓图**：`fullatlas.rs:770` 的 `sync` 只重新请求 3 个 JSON，返回 `atlas_count`；而 `local_atlas_controller.js:236-239` 里有 `while(status==='running')` 去请求 `/sync/status`，**Rust 的 action 表里没有这个 action**（只有 `sync`）。当前因为立即返回 `complete` 走不到，属于上了下载逻辑就会炸的定时炸弹。
3. `/status` 恒返回 `ready:true`，于是 `cacheReady` 永远为真、开关不被拦。这在「无缓存」时是合理的降级，但一旦真有本地资源，这个硬编码必须改成真实就绪判定。

### 2.3 不能碰的东西

`macos/tests/platform-parity.test.mjs:49-61` 锁住 `local_atlas_controller.js` / `desktop_assistant.js` / `assistant_dashboard.js` 两侧差异 ≤4 行，当前实际用到 2 行（`d.request` 两处 + 文案 + VERSION）。
→ **任何 mod 逻辑都不进这 3 个文件**，否则要么涨上限（丧失"防静默分叉"的意义），要么两侧同步改（Windows 侧是上游快照，会失去可比性）。

---

## 三、把 mod 定义清楚：两类，别混为一谈

社区对「mod」的期待其实混着两件事，成本差一个数量级：

| | L1 资源包（离线素材包） | L2 覆盖包（贴图 / 条目替换） |
| --- | --- | --- |
| 解决什么 | Windows 首启下 106 MB / 3098 张图；macOS 每次冷启全量回源 | 角色贴图、配件、表情的本地替换 |
| 形态 | 一堆按 hash 命名的 png + manifest | 少量 png + 条目映射 |
| 谁最需要 | **所有人**（装一次就够了） | 少数人 |
| 风险 | 极低（只读、内容与官方一致） | 低（只在本地显示） |
| 工作量 | 小（复用同一套目录与索引） | 中（要做条目映射与冲突） |

**建议先做 L1 的那套基础设施，L2 直接跑在上面。** 因为两者共用一个目录格式和一个挂载点，L1 是 L2 的免费副产品，反过来则不成立。

明确**不做**：
- 不做代码注入 / 脚本 mod（UE4SS 那类）。与「不注入系统输入」的既有设计声明、与账号安全模型都冲突。
- 不让 mod 在任何能写回账号的路径上可见（联机时不可见，沿用现有 `restricted()` 边界）。
- 不做联机共享 / 上传。mod 目录是本机的。

---

## 四、架构

### 4.1 四层叠加

```
        ┌─────────────────────────────────────────┐
  优先级 │ ③ mods/<id>/           用户层（新增）    │  ← 命中即返回，失败自动回落
  高 ↑  │ ② cache/assets/        本地缓存（Win 已有，Mac 待建）
        │ ① https://<cdn>/...    官方 CDN（默认）
        └─────────────────────────────────────────┘
                 ↑ 全部收口在一个"取图片字节"的函数里
```

本地模式关闭时，第 ③ 层完全不参与，链路退回现状 —— **这就是回滚方案：删目录即还原。**

### 4.2 目录布局

```
Windows: <app>\mods\
macOS:   ~/Library/Application Support/dev.w33d.wuwatappo.fullatlas.mac/mods/

mods/
├─ index.json              # 由宿主生成，不手写（记 mtime/sha/条目覆盖表）
└─ <mod-id>/
   ├─ mod.json
   ├─ 1105_body1.png
   └─ 1105_head1.png
```

### 4.3 mod.json

```json
{
  "schema": 1,
  "id": "demo-skin",
  "name": "示例皮肤包",
  "version": "1.0.0",
  "author": "",
  "adapter": "1.0.4",
  "order": 100,
  "kind": "override",
  "entries": [
    {
      "type": "character_skin",
      "gameRoleId": 1105,
      "layers": {
        "body1": "1105_body1.png",
        "head1": "1105_head1.png"
      }
    }
  ],
  "files": { "1105_body1.png": "<sha256>" }
}
```

两条硬规则：

1. **用 `gameRoleId` 定位，不用 `height_light_id`。** 图鉴的 `*_atlas` 行里 `height_light_id` 每次官方重建都会漂（这正是 1.0.4 适配踩过的坑），而 `character_skin` 里的 `game_role_id` 是业务 id，稳定。
2. `adapter` 不匹配就**只告警不加载**，不静默套用。官方改版后错位的贴图比不显示更难排查。

### 4.4 注入点：一条数据改写，两条传输

**统一约定：被覆盖条目的 `url` 改写为绝对地址，具体 scheme 由平台自己提供。**

| 平台 | 改写后的 URL | 由谁提供字节 |
| --- | --- | --- |
| Windows | `http://127.0.0.1:43229/mod/<id>/<file>` | `local_host.py` 新增 `/mod/` 路由 |
| macOS 方案 A（推荐） | `asset://localhost/<encoded 绝对路径>` | Tauri `assetProtocol`（scope 限定到 `mods/**`），Rust 侧只开权限不写服务 |
| macOS 方案 B（备选） | `http://127.0.0.1:<随机端口>/mod/...` | Rust 起一个 loopback 服务 |

方案 A 不新增端口、不受 ATS 限制、不碰 CSP（当前 `csp: null`），代价是需要在 `fullatlas-integration.py` 里给官方 5 处 URL 解析函数加「放行 `asset:`」的锚点（带 count 断言，结构定位）。方案 B 的代价是移动「本机不跑 localhost 服务」这条设计边界，且 macOS ATS 对 `http://127.0.0.1` 子资源未必放行，要先做 spike。

**建议 A，并把 B 作为 A 不通过时的降级。**

### 4.5 数据流（做完之后）

```
启动 → 宿主扫描 mods/ → 生成 index（mtime 未变则复用）
     → 用户开本地模式 → GET /offline-data
                          ├─ 读 cache/offline_catalog.json
                          ├─ 叠加 index 的覆盖表：把命中条目 url 换成绝对地址
                          └─ 返回（catalog 顶层 assets_host 保持官方值，不动）
     → 前端 applyOfflineCatalog → <img src="http://127.0.0.1:.../mod/...">
     → 命中 mods 返回字节；未命中 → 回落 cache；再未命中 → 302 CDN
```

---

## 五、分阶段实施

### P0 —— Windows 打穿（约 1 天，可独立验收）

改动全部在 Python 层，**前端零改动**：

| 文件 | 改动 |
| --- | --- |
| `windows/src/src/local_host.py` | 新增 `mods_index()`（扫描 + mtime 复用）、`mod_asset()`；`cached_asset()` 前插 mods 查找；`/assets/<name>` 与 `/character/*` 六类前缀同样先查 mods；新增 `GET /mod/<id>/<file>`（路径守卫复用 `is_relative_to`）、`GET /mods`（列表 + 状态）、`POST /mods/open`（打开目录）、`POST /mods/reload` |
| `windows/src/src/local_host.py` | `/offline-data` 应答前套一层 `apply_mod_overlay(catalog, index)` |
| `windows/build/build.ps1` | 统计 `mods_count` 写进 `local_config.json` |
| `windows/src/tools/build_integration.py` | 设置页插一行「本地资源包」：数量 + 打开目录 + 重新扫描（沿用现有 `setting-item` 结构） |
| `windows/src/tests/` | 新增 `test_mods.py`：路径逃逸（`..` / 反斜杠 / 绝对路径）、非 png 魔数拒绝、单文件 >8 MiB 拒绝、mod 命中优先于 cache、mod 损坏时回落 cache 而非 500 |

安全细则（必须全部实现）：

- 文件名白名单 `^[A-Za-z0-9._-]{1,128}$`，禁止 `/ \ : ..`；
- 只接受 PNG/JPEG/WebP，校验魔数而非扩展名；
- 单文件 ≤ 8 MiB，单个 mod ≤ 256 个文件；
- mods 目录只读挂载，**服务端不写、不删、不解压**（导入动作 P2 才做）；
- 只在 `ACTUAL_OFFLINE` 为真时应用 overlay，联机模式一律不参与。

**验收**：`app\mods\demo\` 放 `mod.json` + 两张 png → 开本地模式 → 角色贴图变化；关掉 → 恢复官方；把 `mod.json` 改坏 → 回落官方且只留一条诊断，不白屏。

### P1 —— macOS 对齐（约 3-5 天）

| 文件 | 改动 |
| --- | --- |
| `macos/src-tauri/src/mods.rs`（新） | 扫描 + manifest 解析 + 校验 + `index.json` 生成（纯 Rust，可单测） |
| `macos/src-tauri/src/fullatlas.rs` | action `offline-data` 应答前套 overlay；`status` 增加 `mods` 摘要 |
| `macos/src-tauri/src/lib.rs` | `manage(ModsState)`；`assetProtocol` scope 配置 |
| `macos/tools/fullatlas-integration.py` | 5 处 URL 解析函数放行 `asset:`（带 count 断言） |
| `macos/src-tauri/src/fullatlas.rs` | 顺手补 `sync/status` action（见 2.2 第 2 条） |
| `macos/tests/` | `mods.test`（Rust 内联）：逃逸、大小写、超限、坏 JSON、adapter 不匹配 |

**首个动作是 spike，不是写代码**：造一个 30 行的最小样例，验证 `asset://localhost/<path>` 的图片能在 `tauri://localhost` 页面的 `<img>` 里加载、且能通过 `resolveAppearanceAssetUrl` 的改写链路。这一步不过，直接切方案 B 或缩小到「macOS 只支持 L1 资源包」。

### P2 —— 按需（不做也算完整）

- **L1 资源包导入**：`POST /mods/import`，只收 zip，解压前逐条校验路径，逐文件校验魔数与 sha256，落盘到 `mods/<id>/`。Windows 上这一条能直接省掉首启 106 MB 下载。
- **自定义图鉴条目**（新增不存在的条目）：正规做法是 **base catalog 先过 `validate_catalog`，再在应答阶段 merge 独立的 `atlas-ext.json`**，绝不把自定义行写进 `fullatlas-catalog.json`（那会让缓存整体校验失败，进而回源失败）。
- **诊断面板**：冲突列表（两个 mod 覆盖同一条目时按 `order` 决胜并显示）、官方改版后 `gameRoleId` 失效告警、覆盖率。
- **模式提示条**：本地模式开启且存在 mod 时，在设置页显式标出「当前显示包含 N 个本地资源包」。

---

## 六、风险与必须同步处理的冲突点

| 风险 | 触发点 | 处置 |
| --- | --- | --- |
| 改了共享 JS，parity 测试红 | `platform-parity.test.mjs:49-61` | 本方案 JS 改动为 0；若最终必须加，涨上限前先说明是"新平台分化" |
| macOS 契约测试红 | `network-contract.test.mjs:50` 断言 `assets.publicHost === profile.assetsOrigin` | 该断言针对**契约快照**，不针对运行时 origin。overlay 不改 `assets_host`，可用 |
| 图鉴缓存整体失效 | 往 `fullatlas-catalog.json` 里塞自定义行 | 一律走应答期 overlay / `atlas-ext.json`，不动缓存文件 |
| 官方 1.0.5 重建后锚点漂 | `fullatlas-integration.py` 的 `replace()` 断言 | 所有锚点带 count 断言，失败即构建中断（现有约定，保持） |
| macOS ATS / 协议不通 | 方案 A spike | 不通过则降级方案 B 或只做 L1 |
| 未命中把磁盘写爆 | `client_diagnostics.jsonl` 无轮转 | 顺手加 8 MiB 轮转 + 同类去重 |
| 合规 | 本地贴图被看见 | 只在本地模式生效；联机模式第 ③ 层不参与 |

---

## 七、验收清单（可测）

- [ ] 关闭本地模式时，`/offline-data` 应答与当前**逐字节一致**（零影响证明）
- [ ] mod 覆盖生效后，被覆盖条目 URL 是绝对地址，未覆盖条目 URL 与原来一致
- [ ] 删除 mod 目录 + 重启 → 完全恢复官方显示
- [ ] `mod.json` 语法错误 / adapter 不匹配 / 文件缺失 → 单条诊断，界面不白屏
- [ ] 路径逃逸四例（`..`、`\`、绝对路径、`%2f`）全部 400
- [ ] 非图片文件（改名 png 的 exe）被魔数校验拒绝
- [ ] 平台差异测试仍为 ≤4 行；`npm test` / `test:fullatlas` / `test:rust` 全绿

---

## 八、我的取舍意见

如果只能做一件事：**做 P0 的 Windows 单侧 + 一个"打开 mod 目录"的按钮**。它用一天时间验证了整个假设（绝对 URL 注入是否真的无副作用），而且立刻能吃到 L1 资源包的红利。

macOS 那侧真正的价值不在 mod，在于**它现在连图片缓存都没有**；补本地资源服务是顺带解决一个更痛的体验问题。但如果不想动「macOS 不跑 localhost」这条设计边界，就老实走 `asset:` 协议那条路，别为了 mod 把架构承诺改掉。
