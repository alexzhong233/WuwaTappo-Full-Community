# 自制角色、挂饰和表情 Mod

本功能在 `feature/custom-local-resources` 分支开发。macOS 国服和 Windows 的资源格式相同。

## 两种外观模式

| 模式 | 显示内容 |
| --- | --- |
| 官方 | 账号背包、账号已拥有的外观，恢复原有联机行为 |
| 全图鉴 + Mod | 原有完整本地图鉴，以及本机导入的角色、挂饰、表情 |

没有导入 Mod 时，「全图鉴 + Mod」就是原有全图鉴。删除最后一个 Mod 也不会关闭全图鉴。

进入「挂机与统计 → 设置 → 外观模式与 Mod」，点击「导入 Mod ZIP」，再切换到「全图鉴 + Mod」。导入本身不切换模式。在背包中选择新外观，与选择原有角色、挂饰和表情的方式相同。可以同时安装多个包，也可以单独移除。

切回「官方」会恢复账号装扮；Mod 保留在本机，重新开启全图鉴时可继续使用。全图鉴模式沿用现有联机与账号外观写入限制，Mod 不上传到账号。

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
| `character` 角色 | `body1` 普通身体、`head1` 普通头部 | `body2`、`head2` 点击状态；`body3` 额外身体状态；`icon` 图鉴缩略图；`preview` 角色预览 |
| `accessory` 挂饰 | `image` 挂饰图片 | `icon` 图鉴缩略图 |
| `emote` 表情 | `image` 表情图片 | `icon` 图鉴缩略图 |

建议使用透明 PNG，并让头部、身体使用尺寸一致的画布，在图片内对齐位置。第一版的位移值均为 0；暂不提供逐层坐标、动作脚本或骨骼动画配置。可选动作图未提供时，其显示行为沿用原版渲染器。

支持 PNG、JPEG、WebP 和 GIF 图片；这是一套图片外观包格式，不执行包内代码。缩略图未指定时，挂饰和表情使用 `image`，角色使用 `preview` 或 `body1`。

所有图片都放在 `assets/` 下，路径使用 `/`。可以使用中文文件名与空格，不能使用绝对路径、上级目录、网络地址、查询参数或符号链接。macOS 压缩生成的 `.DS_Store` 和 `__MACOSX/` 元数据会被忽略。

每包最多 5,000 个条目、20,000 个 ZIP 条目；压缩文件最大 512 MiB，解压后最大 1 GiB，单个 JSON 最大 16 MiB。导入前先校验，校验失败保留原包。

## 数据保存

macOS 的 Mod 单独存放在当前应用数据目录的 `custom-atlas/`；Windows 存放在本地服务目录的 `custom-atlas/`。官方图鉴缓存与 Mod 分开保存，叠加只发生在运行时。官方图鉴同步不会把 Mod 写进官方缓存。

重新启动应用会读取已安装的 Mod。移除正在使用的角色后会回退到可用角色，失效挂饰和表情会从当前显示中移除。Mod 列表中的“移除”只删除对应的本机资源包。

当前 macOS 国际服仍沿用仓库已有的原版前端，没有接入 FullAtlas 控制面板；本分支不承诺其 Mod 界面可用。Windows 的国服与国际服共用同一套注入入口。
