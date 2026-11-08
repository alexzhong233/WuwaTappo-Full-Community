"""生成两个「只有挂饰 / 只有表情」的演示 Mod 包（纯标准库）。

用法：python make_accessory_emote_packs.py
产物：
  demo-accessories/          挂饰包源
  demo-emotes/               表情包源
  demo-accessories.zip       可直接导入「Mod 管理」
  demo-emotes.zip
  packs-preview.png          两类条目的逐项预览

角色包（demo-animated-pet.zip）里只有一个角色，看不出挂饰和表情长什么样；
这两个包各管一类，方便一次验证「背包里能选、桌宠上能戴」。
"""

from pathlib import Path
import json
import zipfile

from make_demo_pack import (Canvas, write_png, render_sheet,
                            OUTLINE, HEAD, EAR_INNER, CHEEK, EYE, WHITE, ACCENT)

# 与演示角色同一套坐标：头部圆心 (280, 236)、半径约 110。
CX, CY = 280, 236

PINK = (246, 178, 188, 255)
PINK_DARK = (214, 137, 156, 255)
GOLD = (247, 206, 106, 255)
GOLD_DARK = (206, 158, 54, 255)
SCARF = (127, 119, 221, 255)
SCARF_DARK = (95, 88, 184, 255)


def draw_cat_ears(canvas, cx, cy, scale=1.0):
    """两只猫耳，贴在头部圆的上缘。"""
    for sign in (-1, 1):
        ear = [(cx + sign * 34 * scale, cy - 46 * scale),
               (cx + sign * 62 * scale, cy - 144 * scale),
               (cx + sign * 128 * scale, cy - 58 * scale)]
        canvas.poly_outlined(ear, PINK, grow=5 * scale)
    for sign in (-1, 1):
        inner = [(cx + sign * 52 * scale, cy - 64 * scale),
                 (cx + sign * 66 * scale, cy - 114 * scale),
                 (cx + sign * 106 * scale, cy - 66 * scale)]
        canvas.polygon(inner, PINK_DARK)


def draw_bow(canvas, cx, cy, scale=1.0):
    """蝴蝶结：两片三角形 + 中间的结 + 两条垂带。"""
    left = [(cx, cy), (cx - 62 * scale, cy - 34 * scale), (cx - 62 * scale, cy + 34 * scale)]
    right = [(cx, cy), (cx + 62 * scale, cy - 34 * scale), (cx + 62 * scale, cy + 34 * scale)]
    canvas.poly_outlined(left, GOLD, grow=5 * scale)
    canvas.poly_outlined(right, GOLD, grow=5 * scale)
    canvas.capsule(cx - 40 * scale, cy + 6 * scale, cx - 58 * scale, cy + 52 * scale,
                   9 * scale, GOLD_DARK)
    canvas.capsule(cx + 40 * scale, cy + 6 * scale, cx + 58 * scale, cy + 52 * scale,
                   9 * scale, GOLD_DARK)
    canvas.ellipse(cx, cy, 17 * scale, 17 * scale, OUTLINE)
    canvas.ellipse(cx, cy, 12 * scale, 12 * scale, GOLD)


def draw_scarf(canvas, cx, cy, scale=1.0):
    """围巾：绕颈部一圈，加一条垂下来的尾巴。"""
    canvas.arc(cx, cy - 96 * scale, 86 * scale, 24, 156, 16 * scale, OUTLINE, steps=28)
    canvas.arc(cx, cy - 96 * scale, 86 * scale, 24, 156, 10 * scale, SCARF, steps=28)
    canvas.capsule(cx + 62 * scale, cy - 10 * scale, cx + 74 * scale, cy + 62 * scale,
                   14 * scale, OUTLINE)
    canvas.capsule(cx + 62 * scale, cy - 10 * scale, cx + 74 * scale, cy + 62 * scale,
                   9 * scale, SCARF_DARK)


def draw_emote(canvas, kind):
    """表情：一张圆脸，只换眉眼和嘴。"""
    canvas.ellipse(CX, CY + 44, 178, 172, OUTLINE)
    canvas.ellipse(CX, CY + 40, 170, 164, HEAD)
    canvas.ellipse(CX, CY - 20, 118, 62, (222, 247, 241, 120))
    canvas.ellipse(CX - 108, CY + 74, 26, 15, CHEEK)
    canvas.ellipse(CX + 108, CY + 74, 26, 15, CHEEK)

    if kind == 'happy':
        for sign in (-1, 1):
            canvas.arc(CX + sign * 56, CY + 20, 26, 200, 340, 7, EYE, steps=18)
        canvas.arc(CX, CY + 88, 30, 20, 160, 6, OUTLINE, steps=18)
    elif kind == 'wink':
        canvas.arc(CX - 56, CY + 20, 26, 200, 340, 7, EYE, steps=18)
        canvas.ellipse(CX + 56, CY + 16, 19, 22, EYE)
        canvas.ellipse(CX + 51, CY + 8, 6, 7, WHITE)
        canvas.arc(CX, CY + 84, 28, 20, 160, 6, OUTLINE, steps=18)
    else:  # angry
        for sign in (-1, 1):
            canvas.capsule(CX + sign * 34, CY - 16, CX + sign * 78, CY - 6, 7, OUTLINE)
            canvas.ellipse(CX + sign * 56, CY + 26, 16, 14, EYE)
        canvas.arc(CX, CY + 112, 24, 200, 340, 6, OUTLINE, steps=16)
        # 头顶的十字火气
        for sign in (-1, 1):
            canvas.capsule(CX + sign * 96, CY - 132, CX + sign * 128, CY - 168, 6, ACCENT)
            canvas.capsule(CX + sign * 128, CY - 132, CX + sign * 96, CY - 168, 6, ACCENT)


# (条目名, 文件名, 贴到桌宠上的画法, 缩略图里放大居中的画法, ASCII 预览标签)
ACCESSORIES = [
    ('猫耳', 'ears', lambda c: draw_cat_ears(c, CX, CY, 1.0),
     lambda c: draw_cat_ears(c, CX, 372, 1.9), 'EARS'),
    ('蝴蝶结', 'bow', lambda c: draw_bow(c, CX + 96, CY - 128, 1.0),
     lambda c: draw_bow(c, CX, 300, 2.3), 'BOW'),
    ('围巾', 'scarf', lambda c: draw_scarf(c, CX, CY, 1.0),
     lambda c: draw_scarf(c, CX, 430, 1.6), 'SCARF'),
]

# (条目名, 文件名, 表情分支, ASCII 预览标签)
EMOTES = [
    ('开心', 'happy', 'HAPPY'),
    ('眨眼', 'wink', 'WINK'),
    ('生气', 'angry', 'ANGRY'),
]


def render(draw):
    canvas = Canvas()
    draw(canvas)
    return canvas.downsample()


def build_pack(root, folder, name, items, preview, archive):
    """items: (名称, 文件名, 类型, 图片, 缩略图)。"""
    pack = root / folder
    (pack / 'assets').mkdir(parents=True, exist_ok=True)
    # Windows 的 write_text 会把 \n 翻成 CRLF；仓库统一 LF，直接写字节。
    (pack / 'manifest.json').write_bytes(
        (json.dumps({'schemaVersion': 1, 'name': name}, ensure_ascii=False, indent=2) + '\n').encode('utf-8'))
    documents, files = [], []
    for label, slug, kind, image, icon in items:
        files.append(('assets/%s.png' % slug, image))
        files.append(('assets/%s-icon.png' % slug, icon))
        documents.append({'name': label, 'type': kind,
                          'assets': {'image': 'assets/%s.png' % slug,
                                     'icon': 'assets/%s-icon.png' % slug}})
        # 预览用缩略图：挂饰在整张 560 画布里只占头顶一小块，直接拼版看不清。
        preview.append((label, icon))
    (pack / 'items.json').write_bytes(
        (json.dumps({'items': documents}, ensure_ascii=False, indent=2) + '\n').encode('utf-8'))
    for relative, rendered in files:
        write_png(pack / relative, *rendered)
    if archive.exists():
        archive.unlink()
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as bundle:
        bundle.write(pack / 'manifest.json', 'manifest.json')
        bundle.write(pack / 'items.json', 'items.json')
        for relative, _rendered in files:
            bundle.write(pack / relative, relative)
    print('wrote %-26s %5.1f KiB' % (archive.name, archive.stat().st_size / 1024))


def main():
    root = Path(__file__).resolve().parent

    accessory_items, preview = [], []
    for label, slug, on_pet, icon, tag in ACCESSORIES:
        accessory_items.append((label, 'accessory-' + slug, 'accessory',
                                render(on_pet), render(icon)))
    build_pack(root, 'demo-accessories', '演示 · 挂饰三件', accessory_items,
               preview, root / 'demo-accessories.zip')

    emote_items = []
    for label, slug, tag in EMOTES:
        image = render(lambda canvas, slug=slug: draw_emote(canvas, slug))
        emote_items.append((label, 'emote-' + slug, 'emote', image, image))
    build_pack(root, 'demo-emotes', '演示 · 表情三连', emote_items,
               preview, root / 'demo-emotes.zip')

    labels = [tag for *_rest, tag in ACCESSORIES] + [tag for *_rest, tag in EMOTES]
    write_png(root / 'packs-preview.png',
              *render_sheet([(tag, image) for tag, (_label, image) in zip(labels, preview)]))


if __name__ == '__main__':
    main()
