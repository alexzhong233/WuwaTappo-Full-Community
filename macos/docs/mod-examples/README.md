# 演示 Mod 包

一份能立刻试出「动作帧」效果的角色资源包，以及它是怎么画出来的。

| 文件 | 用途 |
| --- | --- |
| `demo-animated-pet.zip` | 直接丢进「Mod 管理 → 导入 Mod ZIP」 |
| `demo-pet/` | 解压后的包源，`manifest.json` / `items.json` / `assets/` 都在，改完重打即可 |
| `frames.png` | 逐帧预览：每一帧长什么样、标签对应哪个字段 |
| `make_demo_pack.py` | 生成脚本，纯标准库，不需要 Pillow |

包名「演示 · 会动的小家伙」，1 个角色、5 个动作帧 + 1 张图标，画布 560×560、透明 PNG。

## 五帧就是全部

渲染器给角色准备的图层只有这五个（`normalizeAssetEntry` 只认这几个字段），演示包**一帧不落全给了**，不依赖「缺帧回退」：

| 字段 | 文件 | 什么时候出现 | 时长 |
| --- | --- | --- | --- |
| `body1` | `body_1.png` | 待机 | 常驻 |
| `body2` | `body_2.png` | 桌宠向左走 | 每次约 0.2 秒 |
| `body3` | `body_3.png` | 桌宠向右走 | 每次约 0.2 秒 |
| `head1` | `head_1.png` | 常规 | 常驻 |
| `head2` | `head_2.png` | 眨眼 | 每 5 秒约 0.25 秒 |
| `icon` | `icon.png` | 背包/图鉴缩略图 | — |

`icon` 不是动作帧，是把头部和待机身体合成一张，当作缩略图。可选的 `preview` 同理不是帧，演示包没给——没给时缩略图回退到 `icon`。

试效果时看两处就够：桌宠静置时**每 5 秒会眨一次眼**（`head2`），用鼠标拖它走动时会**换迈步姿势**（`body2` / `body3`）。

## 画布与对齐

- 560×560，和官方角色分层图同尺寸，导入后大小、站位与正常角色一致。
- 头部、身体是两张独立图片，**画在同尺寸画布上、靠图内位置对齐**；格式里的偏移量目前恒为 0，不能靠坐标调。
- 身体画在下半（躯干 y≈330~470，脚到 y≈500），头部画在上半（圆 y≈130~340，耳朵到 y≈90），头部下缘压住躯干顶部约 10px——这样两层叠起来时不会在脖子处露出背景缝。
- 走动帧的速度线画在身体层里，只在 `body2`/`body3` 出现，所以站立时不会有残留。

## 怎么画出来的

不用任何美术资源、不装 Pillow，纯标准库：

1. **写 PNG**：`zlib` + `struct` 手工拼 IHDR/IDAT/IEND，RGBA8、每行 filter 0。
2. **画布**：RGBA8 `bytearray`，按 `alpha over` 合成；所有图元先画在 2 倍尺寸上，最后 2×2 均值下采样，边缘就是抗锯齿的。
3. **图元**只有四个，够画完整个角色：
   - `ellipse`：头、躯干、脚、腮红、眼睛；
   - `capsule`（粗线段）：四肢、尾巴、闭眼与嘴的弧线；
   - `polygon`：耳朵、肚子上的紫色标记；
   - `arc`：沿圆弧密集描点，本质是连续 capsule。
4. **描边不靠 stroke**：先按重心把同一形状外扩 5px 画深色，再填内色，得到一圈粗细均匀的描边（`poly_outlined` / `ellipse+5`）。
5. **三帧身体共用一套五官之外的骨架**，只有 `legs` / `feet` / `arms` / `hands` 四张坐标表不同：待机双脚并拢、双手下垂；向左走左脚前伸、右手上抬；向右走左右镜像。**两帧头只有眼睛分支不同**：`head1` 画眼珠加高光，`head2` 换成两段下弯的弧线。
6. **拼版**用同一个画布：每帧按 2×2 缩小贴进格子，标签用内置的 5×7 点阵字画（`FONT` 表里只收了标注需要的字母数字，避免引入字体依赖）。

## 想改成自己的样子

都在 `make_demo_pack.py` 里，改完跑 `python make_demo_pack.py` 重生成全部产物：

- **配色**：文件顶部常量（`BODY` 身体、`HEAD` 头部、`OUTLINE` 描边、`EAR_INNER` 耳内、`CHEEK` 腮红、`ACCENT` 标记）。
- **体型**：`draw_body` 里躯干的 `ellipse(cx, 404, 74, 66, ...)`、`draw_head` 里头的 `ellipse(280, 236, 112, 106, ...)`。
- **姿势**：`draw_body` 里 `if pose == 'idle' / 'left' / else` 三组坐标。
- **眨眼方式**：`draw_head(canvas, blink)` 的 `if blink:` 分支。
- **包名 / 条目名**：文件末尾 `MANIFEST` 与 `ITEMS`。改名会被当成新包或新条目（ID 由名字推导），想保留编号就别动名字。

注意两点：一张图都别少给（缺 `body2`/`body3`/`head2` 会被基础帧自动补齐，效果是动作时画面静止）；头、身两张图必须同画布尺寸，否则叠起来会错位。

## 校验

```bash
cd macos/docs/mod-examples && python make_demo_pack.py     # 重新生成包与预览
cd windows/src && python -m unittest discover -s tests -p "test_custom*.py"
```

导入时的 ZIP 校验（路径逃逸、符号链接、Windows 保留名、图片魔数、压缩比、大小上限）在 `windows/src/src/custom_resources.py`，两侧行为一致。包的格式规范见 [Mod 制作说明](../MODS.md)。
