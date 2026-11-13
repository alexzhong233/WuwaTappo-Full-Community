# 自制角色、挂饰和表情 Mod

本功能在独立分支开发（已同步官方 1.0.4 基底），尚未合入主线。macOS 国服和 Windows 的资源格式相同。

## 两种外观模式

| 模式 | 显示内容 |
| --- | --- |
| 官方 | 账号背包、账号已拥有的外观，恢复原有联机行为 |
| 全图鉴 + Mod | 原有完整本地图鉴，以及本机导入的角色、挂饰、表情 |

没有导入 Mod 时，「全图鉴 + Mod」就是原有全图鉴。删除最后一个 Mod 也不会关闭全图鉴。

进入官方设置里的「外观模式 → Mod 管理」（或控制台底部「桌宠助手 → 打开控制台 → Mod 管理」），点击「导入 Mod ZIP」，再切换到「全图鉴 + Mod」。导入本身不切换模式。可以同时安装多个包，也可以单独移除；每个包会展开条目缩略图，导入后即可确认资源是否配对正确。

切换回「官方」后，背包只显示账号已拥有的物品：Mod 条目、Mod 缩略图样式都不再出现，控制台右上角的模式按钮同样能切换。在「全图鉴 + Mod」下，Mod 条目在背包/图鉴里的缩略图带虚线描边与左下角的 `MOD` 角标，与官方条目一眼可分。排序和稀有度筛选都保持官方原样：Mod 条目的稀有度是 `MOD`，官方下拉框里没有这一项，所以选中其它稀有度时它们会被筛掉，选「全部」时按官方规则排在最后。条目名称直接取包里的 `name`（官方字典里查不到，回退到条目名），不会再显示成空或占位符；背包「我的预设」左侧的角色/挂饰预览也按名字落到本机图，不再是一个「？」。

切回「官方」会恢复账号装扮；Mod 保留在本机，重新开启全图鉴时可继续使用。全图鉴模式沿用现有联机与账号外观写入限制，Mod 不上传到账号。如果 Mod 叠加失败（例如官方图鉴结构变化），界面会退回官方图鉴并在 Mod 管理里显示原因，不会让整个本地图鉴不可用。

## ZIP 格式

将以下内容直接放在 ZIP 根目录，不要再套一层文件夹。JSON 使用 UTF-8。

```text
我的外观包.zip
├── manifest.json
├── items.json
└── assets/
    ├── character/
    │   ├── icon.png
    │   ├── body.png
    │   └── head.png
    ├── accessory/
    │   └── ears.png
    └── emote/
        └── happy.png
```

`manifest.json`：

```json
{
  "schemaVersion": 1,
  "name": "我的外观包"
}
```

`items.json`：

```json
{
  "items": [
    {
      "name": "自制角色",
      "type": "character",
      "assets": {
        "icon": "assets/character/icon.png",
        "body1": "assets/character/body.png",
        "head1": "assets/character/head.png"
      }
    },
    {
      "name": "猫耳挂饰",
      "type": "accessory",
      "assets": {
        "image": "assets/accessory/ears.png"
      }
    },
    {
      "name": "开心表情",
      "type": "emote",
      "assets": {
        "image": "assets/emote/happy.png"
      }
    }
  ]
}
```

包和条目均无需填写 ID，应用会根据包名以及条目的类型、名称生成内部 ID 和数值外观编号。同一个包内，同类型条目的名称不能重复；不同类型可以同名。名称两端的空格会被去除。

包名、条目名称及类型保持一致时，修改图片或调整条目顺序不会改变编号。同名包再次导入会替换该包；要同时保留两个包，请使用不同包名。改名会被视为新包或新条目。

可选的 `manifest.region` 为 `ANY`（默认）、`CN` 或 `GLOBAL`；与当前客户端区域不符的包会被拒绝。

## 图片字段

| 类型 | 必需字段 | 可选字段 |
| --- | --- | --- |
| `character` 角色 | `body1` 待机身体、`head1` 常规头部 | `body2` 向左移动、`body3` 向右移动、`head2` 眨眼；`icon` 图鉴缩略图；`preview` 角色预览 |
| `accessory` 挂饰 | `image` 挂饰图片 | `icon` 图鉴缩略图 |
| `emote` 表情 | `image` 表情图片 | `icon` 图鉴缩略图 |

建议使用透明 PNG，并让头部、身体使用尺寸一致的画布，在图片内对齐位置。画布建议 560×560，与官方角色分层图一致。第一版的位移值均为 0；暂不提供逐层坐标、动作脚本或骨骼动画配置。

## 动作帧怎么用

官方渲染器把角色拆成身体和头部两层，各自按“动作编号”取图。编号不需要在包里配置，渲染器自己切换，你只要把对应的图放对字段：

| 字段 | 什么时候显示 | 时长 |
| --- | --- | --- |
| `body1` | 待机，以及没有 `body2`/`body3` 时 | 常驻 |
| `body2` | 桌宠向左走动时 | 每次约 0.2 秒 |
| `body3` | 桌宠向右走动时 | 每次约 0.2 秒 |
| `head1` | 常规状态 | 常驻 |
| `head2` | 眨眼 | 每 5 秒约 0.25 秒 |

所以“两张图做出动作变化”最小的做法就是 `head1` + `head2`：睁眼与闭眼。想要走动时也有变化，再加 `body2`、`body3` 两种迈步姿势，共 5 张图。

没提供的动作帧会由基础帧自动补齐（`body2`/`body3` 用 `body1`，`head2` 用 `head1`），效果是“做动作时画面保持静止”。这一点必须补齐而不是留空：渲染器缺帧时不会回退，那一层会闪一下。

想直接看效果，按类型各导一个演示包：

| 包 | 内容 |
| --- | --- |
| [demo-animated-pet.zip](../mod-examples/demo-animated-pet.zip) | 一个完整角色：五帧动作 + 缩略图，静置看眨眼、拖动看迈步 |
| [demo-accessories.zip](../mod-examples/demo-accessories.zip) | 三个挂饰：猫耳、蝴蝶结、围巾 |
| [demo-emotes.zip](../mod-examples/demo-emotes.zip) | 三个表情：开心、眨眼、生气 |

[逐帧预览](../mod-examples/frames.png) 和 [制作过程](../mod-examples/README.md) 在同一目录；[make_demo_pack.py](../mod-examples/make_demo_pack.py) 画角色，[make_accessory_emote_packs.py](../mod-examples/make_accessory_emote_packs.py) 画挂饰与表情，改完重跑即可重新打包。


支持 PNG、JPEG、WebP 和 GIF 图片；这是一套图片外观包格式，不执行包内代码。缩略图未指定时，挂饰和表情使用 `image`，角色使用 `preview` 或 `body1`。

所有图片都放在 `assets/` 下，路径使用 `/`。可以使用中文文件名与空格，不能使用绝对路径、上级目录、网络地址、查询参数或符号链接。macOS 压缩生成的 `.DS_Store` 和 `__MACOSX/` 元数据会被忽略。

每包最多 5,000 个条目、20,000 个 ZIP 条目；压缩文件最大 512 MiB，解压后最大 1 GiB，单个 JSON 最大 16 MiB。导入前先校验，校验失败保留原包。

## 数据保存

macOS 的 Mod 单独存放在当前应用数据目录的 `custom-atlas/`；Windows 存放在本地服务目录的 `custom-atlas/`。官方图鉴缓存与 Mod 分开保存，叠加只发生在运行时。官方图鉴同步不会把 Mod 写进官方缓存。

重新启动应用会读取已安装的 Mod。移除正在使用的角色后会回退到可用角色，失效挂饰和表情会从当前显示中移除。Mod 列表中的“移除”只删除对应的本机资源包。

macOS 国服与国际服都接入 FullAtlas 控制面板，Mod 界面两服一致；Windows 的国服与国际服共用同一套注入入口。

## 本机取图路由

背包、图鉴和 Mod 管理面板里的缩略图都从宿主的本机 HTTP 服务取，不用 `asset://`：官方渲染器只把 `http(s)` / `asset` 原样交给 `<img>`，而 Mac 的 WebView 从 `tauri://` 页面里取 `asset://` 子资源拿不到图（表现为 404）。

- macOS：`src-tauri/src/mod_assets.rs` 起一个只监听 `127.0.0.1` 的只读服务，端口由系统分配，路径前缀是每次启动随机生成的 token，只服务 `custom-assets/<包>/<文件>` 这类校验过的相对键。
- Windows：沿用本地服务的 `/custom-assets/` 路由，语义相同。
- 前端通过 `/asset-base` 拿前缀，拼接 `custom-assets/...` 相对键；拿不到前缀时退回注入器给的转换函数，宁可这张图不显示也不拼出必然 404 的地址。
