"""生成一个用于试效果的演示 Mod 包（纯标准库，无需 Pillow）。

用法：python make_demo_pack.py
产物：
  demo-pet/            解压后的包源（manifest.json / items.json / assets/）
  demo-animated-pet.zip 可直接导入「Mod 管理」的包
"""

from pathlib import Path
import json
import struct
import zlib

# 官方角色分层图是 560×560 的全画布，头/身各一张、靠画布对齐。
# 这里沿用同样尺寸，导入后与官方角色的大小、站位一致。
SIZE = 560
SS = 2  # 超采样倍数，下采样后得到抗锯齿边缘

OUTLINE = (52, 70, 82, 255)
BODY = (127, 211, 200, 255)
BODY_DARK = (94, 182, 172, 255)
BODY_LIGHT = (198, 240, 232, 255)
HEAD = (166, 227, 216, 255)
HEAD_DARK = (127, 202, 191, 255)
EAR_INNER = (246, 178, 188, 255)
CHEEK = (244, 168, 178, 150)
EYE = (38, 52, 62, 255)
WHITE = (255, 255, 255, 210)
BELLY = (255, 250, 226, 255)
ACCENT = (127, 119, 221, 255)


# 5×7 点阵字，只收拼版标注需要的字母与数字，避免引入字体依赖。
FONT = {
    'A': ['.###.', '#...#', '#...#', '#####', '#...#', '#...#', '#...#'],
    'B': ['####.', '#...#', '#...#', '####.', '#...#', '#...#', '####.'],
    'C': ['.####', '#....', '#....', '#....', '#....', '#....', '.####'],
    'D': ['####.', '#...#', '#...#', '#...#', '#...#', '#...#', '####.'],
    'E': ['#####', '#....', '#....', '####.', '#....', '#....', '#####'],
    'G': ['.####', '#....', '#....', '#..##', '#...#', '#...#', '.###.'],
    'H': ['#...#', '#...#', '#...#', '#####', '#...#', '#...#', '#...#'],
    'I': ['#####', '..#..', '..#..', '..#..', '..#..', '..#..', '#####'],
    'K': ['#...#', '#..#.', '#.#..', '##...', '#.#..', '#..#.', '#...#'],
    'L': ['#....', '#....', '#....', '#....', '#....', '#....', '#####'],
    'M': ['#...#', '##.##', '#.#.#', '#...#', '#...#', '#...#', '#...#'],
    'N': ['#...#', '##..#', '#.#.#', '#..##', '#...#', '#...#', '#...#'],
    'O': ['.###.', '#...#', '#...#', '#...#', '#...#', '#...#', '.###.'],
    'P': ['####.', '#...#', '#...#', '####.', '#....', '#....', '#....'],
    'R': ['####.', '#...#', '#...#', '####.', '#.#..', '#..#.', '#...#'],
    'S': ['.####', '#....', '#....', '.###.', '....#', '....#', '####.'],
    'T': ['#####', '..#..', '..#..', '..#..', '..#..', '..#..', '..#..'],
    'U': ['#...#', '#...#', '#...#', '#...#', '#...#', '#...#', '.###.'],
    'W': ['#...#', '#...#', '#...#', '#.#.#', '#.#.#', '##.##', '#...#'],
    'Y': ['#...#', '#...#', '.#.#.', '..#..', '..#..', '..#..', '..#..'],
    '1': ['..#..', '.##..', '..#..', '..#..', '..#..', '..#..', '.###.'],
    '2': ['.###.', '#...#', '....#', '...#.', '..#..', '.#...', '#####'],
    '3': ['####.', '....#', '....#', '.###.', '....#', '....#', '####.'],
}

class Canvas:
    """RGBA8 画布，内部按超采样尺寸绘制。"""

    def __init__(self, width=SIZE, height=SIZE, scale=SS):
        self.w = width * scale
        self.h = height * scale
        self.scale = scale
        self.buf = bytearray(self.w * self.h * 4)

    def _blend(self, x, y, color):
        if x < 0 or y < 0 or x >= self.w or y >= self.h:
            return
        sr, sg, sb, sa = color
        if sa <= 0:
            return
        i = (y * self.w + x) * 4
        buf = self.buf
        da = buf[i + 3]
        if da == 0:
            buf[i] = sr
            buf[i + 1] = sg
            buf[i + 2] = sb
            buf[i + 3] = sa
            return
        if sa == 255 and da == 255:
            buf[i] = sr
            buf[i + 1] = sg
            buf[i + 2] = sb
            return
        keep = (da * (255 - sa)) // 255
        out_a = sa + keep
        buf[i] = (sr * sa + buf[i] * keep) // out_a
        buf[i + 1] = (sg * sa + buf[i + 1] * keep) // out_a
        buf[i + 2] = (sb * sa + buf[i + 2] * keep) // out_a
        buf[i + 3] = out_a

    def ellipse(self, cx, cy, rx, ry, color):
        s = self.scale
        cx, cy, rx, ry = cx * s, cy * s, max(rx * s, 0.6), max(ry * s, 0.6)
        x0, x1 = int(cx - rx - 1), int(cx + rx + 1)
        y0, y1 = int(cy - ry - 1), int(cy + ry + 1)
        rx2, ry2 = rx * rx, ry * ry
        for y in range(max(y0, 0), min(y1, self.h)):
            dy = (y + 0.5 - cy)
            span = 1 - (dy * dy) / ry2
            if span <= 0:
                continue
            half = rx * span ** 0.5
            for x in range(max(int(cx - half), 0), min(int(cx + half) + 1, self.w)):
                dx = x + 0.5 - cx
                if dx * dx / rx2 + dy * dy / ry2 <= 1:
                    self._blend(x, y, color)

    def capsule(self, x1, y1, x2, y2, radius, color):
        """粗线段（圆头），用于四肢和描边弧线。"""
        s = self.scale
        x1, y1, x2, y2, r = x1 * s, y1 * s, x2 * s, y2 * s, max(radius * s, 0.6)
        x0, xe = int(min(x1, x2) - r - 1), int(max(x1, x2) + r + 1)
        y0, ye = int(min(y1, y2) - r - 1), int(max(y1, y2) + r + 1)
        dx, dy = x2 - x1, y2 - y1
        length2 = dx * dx + dy * dy or 1.0
        for y in range(max(y0, 0), min(ye, self.h)):
            for x in range(max(x0, 0), min(xe, self.w)):
                px, py = x + 0.5 - x1, y + 0.5 - y1
                t = max(0.0, min(1.0, (px * dx + py * dy) / length2))
                ex, ey = px - dx * t, py - dy * t
                if ex * ex + ey * ey <= r * r:
                    self._blend(x, y, color)

    def polygon(self, points, color):
        s = self.scale
        pts = [(x * s, y * s) for x, y in points]
        ys = [p[1] for p in pts]
        y0, y1 = int(min(ys)), int(max(ys)) + 1
        for y in range(max(y0, 0), min(y1, self.h)):
            sy = y + 0.5
            xs = []
            for i in range(len(pts)):
                ax, ay = pts[i]
                bx, by = pts[(i + 1) % len(pts)]
                if (ay <= sy < by) or (by <= sy < ay):
                    xs.append(ax + (sy - ay) * (bx - ax) / (by - ay))
            xs.sort()
            for i in range(0, len(xs) - 1, 2):
                for x in range(max(int(xs[i]), 0), min(int(xs[i + 1]) + 1, self.w)):
                    self._blend(x, y, color)

    def poly_outlined(self, points, fill, grow=5):
        """先用外扩后的同形状画描边，再填内色，得到一圈均匀描边。"""
        center_x = sum(p[0] for p in points) / len(points)
        center_y = sum(p[1] for p in points) / len(points)
        outer = []
        for x, y in points:
            dx, dy = x - center_x, y - center_y
            distance = max((dx * dx + dy * dy) ** 0.5, 1.0)
            scale = (distance + grow) / distance
            outer.append((center_x + dx * scale, center_y + dy * scale))
        self.polygon(outer, OUTLINE)
        self.polygon(points, fill)

    def arc(self, cx, cy, radius, start, end, width, color, steps=40):
        import math
        prev = None
        for i in range(steps + 1):
            angle = math.radians(start + (end - start) * i / steps)
            point = (cx + math.cos(angle) * radius, cy + math.sin(angle) * radius)
            if prev:
                self.capsule(prev[0], prev[1], point[0], point[1], width, color)
            prev = point

    def rect(self, x0, y0, x1, y1, color):
        s = self.scale
        for y in range(max(int(y0 * s), 0), min(int(y1 * s) + 1, self.h)):
            for x in range(max(int(x0 * s), 0), min(int(x1 * s) + 1, self.w)):
                self._blend(x, y, color)

    def blit_half(self, image, dx, dy):
        """把 560 的帧按 2×2 均值缩到一半贴进来（预览拼版用）。"""
        width, height, data = image
        for y in range(height // 2):
            for x in range(width // 2):
                r = g = b = a = 0
                for sub_y in range(2):
                    base = ((y * 2 + sub_y) * width + x * 2) * 4
                    for sub_x in range(2):
                        i = base + sub_x * 4
                        alpha = data[i + 3]
                        r += data[i] * alpha
                        g += data[i + 1] * alpha
                        b += data[i + 2] * alpha
                        a += alpha
                if not a:
                    continue
                self._blend(int(dx) + x, int(dy) + y,
                            (r // a, g // a, b // a, a // 4))

    def text(self, x, y, value, size, color):
        """5×7 点阵字，size 是一个点的边长（逻辑坐标）。"""
        cursor = x
        for character in value.upper():
            glyph = FONT.get(character)
            if glyph:
                for row, line in enumerate(glyph):
                    for column, cell in enumerate(line):
                        if cell == '#':
                            self.rect(cursor + column * size, y + row * size,
                                      cursor + (column + 1) * size - 1,
                                      y + (row + 1) * size - 1, color)
            cursor += 6 * size

    def downsample(self):
        """超采样均值下采样，按 alpha 加权避免边缘发黑。"""
        s = self.scale
        w, h = self.w // s, self.h // s
        out = bytearray(w * h * 4)
        buf = self.buf
        for y in range(h):
            for x in range(w):
                r = g = b = a = 0
                for dy in range(s):
                    base = ((y * s + dy) * self.w + x * s) * 4
                    for dx in range(s):
                        i = base + dx * 4
                        alpha = buf[i + 3]
                        r += buf[i] * alpha
                        g += buf[i + 1] * alpha
                        b += buf[i + 2] * alpha
                        a += alpha
                o = (y * w + x) * 4
                if a:
                    out[o] = r // a
                    out[o + 1] = g // a
                    out[o + 2] = b // a
                    out[o + 3] = a // (s * s)
        return w, h, out


def write_png(path, width, height, rgba):
    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)
        raw += rgba[y * stride:(y + 1) * stride]

    def chunk(tag, payload):
        return (struct.pack('>I', len(payload)) + tag + payload
                + struct.pack('>I', zlib.crc32(tag + payload) & 0xFFFFFFFF))

    png = (b'\x89PNG\r\n\x1a\n'
           + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
           + chunk(b'IDAT', zlib.compress(bytes(raw), 9))
           + chunk(b'IEND', b''))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)


def draw_body(canvas, pose):
    """身体层：待机 / 向左走 / 向右走。

    三帧只有四肢摆位和重心偏移不同 —— 这正是渲染器在移动时切换的
    body2（朝左）/ body3（朝右），待机用的是 body1。
    """
    lean = {'idle': 0, 'left': -7, 'right': 7}[pose]
    cx = 280 + lean

    # 尾巴（先画，压在身体后面）
    canvas.capsule(340, 448, 398, 408, 13, OUTLINE)
    canvas.capsule(340, 448, 398, 408, 8, BODY_DARK)

    if pose == 'idle':
        legs = [(254, 448, 250, 482), (306, 448, 310, 482)]
        feet = [(246, 490, 30, 15), (314, 490, 30, 15)]
        arms = [(208, 374, 190, 438), (352, 374, 370, 438)]
        hands = [(188, 442, 11), (372, 442, 11)]
    elif pose == 'left':
        # 重心左移，左脚迈出、右脚蹬地，右臂前摆
        legs = [(252, 448, 218, 476), (306, 448, 336, 482)]
        feet = [(206, 486, 33, 14), (344, 492, 27, 12)]
        arms = [(208, 374, 176, 348), (352, 374, 364, 440)]
        hands = [(172, 344, 11), (366, 444, 11)]
    else:
        # 与「向左走」左右镜像
        legs = [(254, 448, 224, 482), (308, 448, 342, 476)]
        feet = [(216, 492, 27, 12), (354, 486, 33, 14)]
        arms = [(208, 374, 196, 440), (352, 374, 384, 348)]
        hands = [(194, 444, 11), (388, 344, 11)]

    # 走动时在身后留两道速度线，让"在动"一眼可见
    if pose != 'idle':
        sign = -1 if pose == 'left' else 1
        for offset, length in ((0, 26), (16, 18)):
            canvas.capsule(280 + sign * (108 + offset), 500 - offset,
                           280 + sign * (108 + offset + length), 500 - offset,
                           3, (94, 182, 172, 120))

    for x1, y1, x2, y2 in legs:
        canvas.capsule(x1, y1, x2, y2, 22, OUTLINE)
    for x1, y1, x2, y2 in legs:
        canvas.capsule(x1, y1, x2, y2, 16, BODY)
    for fx, fy, rx, ry in feet:
        canvas.ellipse(fx, fy, rx + 5, ry + 5, OUTLINE)
    for fx, fy, rx, ry in feet:
        canvas.ellipse(fx, fy, rx, ry, BODY)

    # 躯干
    canvas.ellipse(cx, 404, 74, 66, OUTLINE)
    canvas.ellipse(cx, 404, 68, 60, BODY)
    canvas.ellipse(cx, 380, 50, 28, (215, 244, 238, 120))

    # 肚皮上的演示标记，用来确认"背包里这张图确实是我导入的"
    canvas.polygon([(cx, 392), (cx - 13, 416), (cx + 13, 416)], ACCENT)
    canvas.ellipse(cx, 436, 7, 7, ACCENT)

    for x1, y1, x2, y2 in arms:
        canvas.capsule(x1, y1, x2, y2, 16, OUTLINE)
    for x1, y1, x2, y2 in arms:
        canvas.capsule(x1, y1, x2, y2, 10, BODY)
    for hx, hy, hr in hands:
        canvas.ellipse(hx, hy, hr + 5, hr + 5, OUTLINE)
    for hx, hy, hr in hands:
        canvas.ellipse(hx, hy, hr, hr, BODY)


def draw_head(canvas, blink):
    """头部层：睁眼与闭眼。

    渲染器每 5 秒把 head 的 action 切成 2 约 250ms —— 也就是眨眼，
    所以 head1/head2 就是"两张图做出动作变化"的最小例子。
    """
    # 耳朵先画，头部圆压在上面，看起来就是"耳后脑前"
    for sign in (-1, 1):
        ear = [(280 + sign * 34, 190), (280 + sign * 62, 92), (280 + sign * 128, 178)]
        canvas.poly_outlined(ear, HEAD, grow=5)
    for sign in (-1, 1):
        inner = [(280 + sign * 52, 172), (280 + sign * 66, 122), (280 + sign * 106, 170)]
        canvas.polygon(inner, EAR_INNER)

    # 头
    canvas.ellipse(280, 236, 112, 106, OUTLINE)
    canvas.ellipse(280, 236, 106, 100, HEAD)
    canvas.ellipse(280, 200, 76, 42, (222, 247, 241, 130))

    # 腮红
    canvas.ellipse(204, 274, 17, 10, CHEEK)
    canvas.ellipse(356, 274, 17, 10, CHEEK)

    if blink:
        for sign in (-1, 1):
            canvas.arc(280 + sign * 36, 240, 17, 200, 340, 4, EYE, steps=16)
    else:
        for sign in (-1, 1):
            eye_x = 280 + sign * 36
            canvas.ellipse(eye_x, 232, 15, 17, EYE)
            canvas.ellipse(eye_x - 4, 225, 5, 6, WHITE)

    # 嘴：一个小小的 ω
    canvas.arc(272, 274, 9, 200, 340, 3, OUTLINE, steps=12)
    canvas.arc(288, 274, 9, 200, 340, 3, OUTLINE, steps=12)


def render(body_pose, blink):
    canvas = Canvas()
    draw_body(canvas, body_pose)
    draw_head(canvas, blink)
    return canvas.downsample()


def render_body_only(pose):
    canvas = Canvas()
    draw_body(canvas, pose)
    return canvas.downsample()


def render_head_only(blink):
    canvas = Canvas()
    draw_head(canvas, blink)
    return canvas.downsample()


def render_sheet(entries, columns=3):
    """把全部帧拼成带标注的一张图：导入前就能确认每一帧是什么样。"""
    cell = SIZE // 2
    label_height = 30
    rows = (len(entries) + columns - 1) // columns
    canvas = Canvas(cell * columns, (cell + label_height) * rows, scale=1)
    canvas.rect(0, 0, cell * columns, (cell + label_height) * rows, (238, 240, 242, 255))
    for index, (label, image) in enumerate(entries):
        ox = (index % columns) * cell
        oy = (index // columns) * (cell + label_height)
        canvas.blit_half(image, ox, oy)
        text_width = len(label) * 6 * 2
        canvas.text(ox + (cell - text_width) // 2, oy + cell + 6, label, 2, (52, 70, 82, 255))
    return canvas.downsample()


MANIFEST = {'schemaVersion': 1, 'name': '演示 · 会动的小家伙'}

ITEMS = {
    'items': [
        {
            'name': '小家伙（演示）',
            'type': 'character',
            'assets': {
                'icon': 'assets/character/icon.png',
                'body1': 'assets/character/body_1.png',
                'body2': 'assets/character/body_2.png',
                'body3': 'assets/character/body_3.png',
                'head1': 'assets/character/head_1.png',
                'head2': 'assets/character/head_2.png',
            },
        }
    ]
}


def main():
    root = Path(__file__).resolve().parent
    pack = root / 'demo-pet'
    assets = pack / 'assets' / 'character'
    assets.mkdir(parents=True, exist_ok=True)
    (pack / 'manifest.json').write_text(
        json.dumps(MANIFEST, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    (pack / 'items.json').write_text(
        json.dumps(ITEMS, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')

    outputs = {
        'body_1.png': render_body_only('idle'),
        'body_2.png': render_body_only('left'),
        'body_3.png': render_body_only('right'),
        'head_1.png': render_head_only(False),
        'head_2.png': render_head_only(True),
        'icon.png': render('idle', False),
    }
    for name, (width, height, data) in outputs.items():
        write_png(assets / name, width, height, data)
        print('wrote %-12s %dx%d' % (name, width, height))

    sheet = root / 'frames.png'
    write_png(sheet, *render_sheet([
        ('BODY1 IDLE', outputs['body_1.png']),
        ('BODY2 WALK L', outputs['body_2.png']),
        ('BODY3 WALK R', outputs['body_3.png']),
        ('HEAD1 OPEN', outputs['head_1.png']),
        ('HEAD2 BLINK', outputs['head_2.png']),
        ('ICON', outputs['icon.png']),
    ]))
    print('wrote %s' % sheet.name)

    archive = root / 'demo-animated-pet.zip'
    import zipfile
    if archive.exists():
        archive.unlink()
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as bundle:
        bundle.write(pack / 'manifest.json', 'manifest.json')
        bundle.write(pack / 'items.json', 'items.json')
        for name in outputs:
            bundle.write(assets / name, 'assets/character/' + name)
    print('wrote %s (%.1f KiB)' % (archive.name, archive.stat().st_size / 1024))


if __name__ == '__main__':
    main()
