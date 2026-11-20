//! Demo Mod pack generator in pure Rust.
//!
//! Replaces `mod-examples/make_demo_pack.py` and `mod-examples/make_accessory_emote_packs.py`,
//! producing identical 560x560 antialiasing demo character, accessory, and emote packs
//! plus preview contact sheets and importable `.zip` archives.

use crc32fast::Hasher as Crc32;
use flate2::{write::ZlibEncoder, Compression};
use serde_json::json;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const SIZE: usize = 560;
const SS: usize = 2;

type Color = [u8; 4];

const OUTLINE: Color = [52, 70, 82, 255];
const BODY: Color = [127, 211, 200, 255];
const BODY_DARK: Color = [94, 182, 172, 255];
const HEAD: Color = [166, 227, 216, 255];
const EAR_INNER: Color = [246, 178, 188, 255];
const CHEEK: Color = [244, 168, 178, 150];
const EYE: Color = [38, 52, 62, 255];
const WHITE: Color = [255, 255, 255, 210];
const ACCENT: Color = [127, 119, 221, 255];
const PINK: Color = [246, 178, 188, 255];
const PINK_DARK: Color = [214, 137, 156, 255];
const GOLD: Color = [247, 206, 106, 255];
const GOLD_DARK: Color = [206, 158, 54, 255];
const SCARF: Color = [127, 119, 221, 255];
const SCARF_DARK: Color = [95, 88, 184, 255];

fn glyph(ch: char) -> Option<[&'static str; 7]> {
    Some(match ch.to_ascii_uppercase() {
        'A' => [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
        'B' => ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."],
        'C' => [".####", "#....", "#....", "#....", "#....", "#....", ".####"],
        'D' => ["####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."],
        'E' => ["#####", "#....", "#....", "####.", "#....", "#....", "#####"],
        'F' => ["#####", "#....", "#....", "####.", "#....", "#....", "#...."],
        'G' => [".####", "#....", "#....", "#..##", "#...#", "#...#", ".###."],
        'H' => ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
        'I' => ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####"],
        'K' => ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"],
        'L' => ["#....", "#....", "#....", "#....", "#....", "#....", "#####"],
        'M' => ["#...#", "##.##", "#.#.#", "#...#", "#...#", "#...#", "#...#"],
        'N' => ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"],
        'O' => [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
        'P' => ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."],
        'R' => ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"],
        'S' => [".####", "#....", "#....", ".###.", "....#", "....#", "####."],
        'T' => ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."],
        'U' => ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
        'W' => ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#"],
        'Y' => ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."],
        '1' => ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."],
        '2' => [".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####"],
        '3' => ["####.", "....#", "....#", ".###.", "....#", "....#", "####."],
        _ => return None,
    })
}

struct Image {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

struct Canvas {
    w: usize,
    h: usize,
    scale: usize,
    buf: Vec<u8>,
}

impl Canvas {
    fn new(width: usize, height: usize, scale: usize) -> Self {
        let w = width * scale;
        let h = height * scale;
        Self {
            w,
            h,
            scale,
            buf: vec![0u8; w * h * 4],
        }
    }

    fn blend(&mut self, x: isize, y: isize, [sr, sg, sb, sa]: Color) {
        if x < 0 || y < 0 || x >= self.w as isize || y >= self.h as isize || sa == 0 {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        let da = self.buf[i + 3] as u32;
        if da == 0 {
            self.buf[i..i + 4].copy_from_slice(&[sr, sg, sb, sa]);
            return;
        }
        if sa == 255 && da == 255 {
            self.buf[i..i + 3].copy_from_slice(&[sr, sg, sb]);
            return;
        }
        let sa_u = sa as u32;
        let keep = (da * (255 - sa_u)) / 255;
        let out_a = sa_u + keep;
        self.buf[i] = ((sr as u32 * sa_u + self.buf[i] as u32 * keep) / out_a) as u8;
        self.buf[i + 1] = ((sg as u32 * sa_u + self.buf[i + 1] as u32 * keep) / out_a) as u8;
        self.buf[i + 2] = ((sb as u32 * sa_u + self.buf[i + 2] as u32 * keep) / out_a) as u8;
        self.buf[i + 3] = out_a as u8;
    }

    fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, color: Color) {
        let s = self.scale as f64;
        let (cx, cy, rx, ry) = (cx * s, cy * s, (rx * s).max(0.6), (ry * s).max(0.6));
        let y0 = ((cy - ry - 1.0).floor() as isize).max(0);
        let y1 = ((cy + ry + 1.0).ceil() as isize).min(self.h as isize);
        let (rx2, ry2) = (rx * rx, ry * ry);
        for y in y0..y1 {
            let dy = y as f64 + 0.5 - cy;
            let span = 1.0 - (dy * dy) / ry2;
            if span <= 0.0 {
                continue;
            }
            let half = rx * span.sqrt();
            let x0 = ((cx - half).floor() as isize).max(0);
            let x1 = ((cx + half + 1.0).ceil() as isize).min(self.w as isize);
            for x in x0..x1 {
                let dx = x as f64 + 0.5 - cx;
                if (dx * dx) / rx2 + (dy * dy) / ry2 <= 1.0 {
                    self.blend(x, y, color);
                }
            }
        }
    }

    fn capsule(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, radius: f64, color: Color) {
        let s = self.scale as f64;
        let (x1, y1, x2, y2, r) = (x1 * s, y1 * s, x2 * s, y2 * s, (radius * s).max(0.6));
        let x0 = (x1.min(x2) - r - 1.0).floor() as isize;
        let xe = (x1.max(x2) + r + 1.0).ceil() as isize;
        let y0 = (y1.min(y2) - r - 1.0).floor() as isize;
        let ye = (y1.max(y2) + r + 1.0).ceil() as isize;
        let (dx, dy) = (x2 - x1, y2 - y1);
        let len2 = {
            let v = dx * dx + dy * dy;
            if v == 0.0 { 1.0 } else { v }
        };
        for y in y0.max(0)..ye.min(self.h as isize) {
            for x in x0.max(0)..xe.min(self.w as isize) {
                let px = x as f64 + 0.5 - x1;
                let py = y as f64 + 0.5 - y1;
                let t = ((px * dx + py * dy) / len2).clamp(0.0, 1.0);
                let ex = px - dx * t;
                let ey = py - dy * t;
                if ex * ex + ey * ey <= r * r {
                    self.blend(x, y, color);
                }
            }
        }
    }

    fn polygon(&mut self, points: &[(f64, f64)], color: Color) {
        let s = self.scale as f64;
        let pts: Vec<(f64, f64)> = points.iter().map(|&(x, y)| (x * s, y * s)).collect();
        let min_y = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min).floor() as isize;
        let max_y = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max).ceil() as isize + 1;
        for y in min_y.max(0)..max_y.min(self.h as isize) {
            let sy = y as f64 + 0.5;
            let mut xs = Vec::new();
            for i in 0..pts.len() {
                let (ax, ay) = pts[i];
                let (bx, by) = pts[(i + 1) % pts.len()];
                if (ay <= sy && sy < by) || (by <= sy && sy < ay) {
                    xs.push(ax + (sy - ay) * (bx - ax) / (by - ay));
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for pair in xs.chunks_exact(2) {
                let x0 = (pair[0].floor() as isize).max(0);
                let x1 = ((pair[1].floor() as isize) + 1).min(self.w as isize);
                for x in x0..x1 {
                    self.blend(x, y, color);
                }
            }
        }
    }

    fn poly_outlined(&mut self, points: &[(f64, f64)], fill: Color, grow: f64) {
        let cx = points.iter().map(|p| p.0).sum::<f64>() / points.len() as f64;
        let cy = points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64;
        let outer: Vec<(f64, f64)> = points
            .iter()
            .map(|&(x, y)| {
                let (dx, dy) = (x - cx, y - cy);
                let dist = (dx * dx + dy * dy).sqrt().max(1.0);
                let factor = (dist + grow) / dist;
                (cx + dx * factor, cy + dy * factor)
            })
            .collect();
        self.polygon(&outer, OUTLINE);
        self.polygon(points, fill);
    }

    fn arc(
        &mut self,
        cx: f64,
        cy: f64,
        radius: f64,
        start_deg: f64,
        end_deg: f64,
        width: f64,
        color: Color,
        steps: usize,
    ) {
        let mut prev: Option<(f64, f64)> = None;
        for i in 0..=steps {
            let angle = (start_deg + (end_deg - start_deg) * (i as f64 / steps as f64)).to_radians();
            let pt = (cx + angle.cos() * radius, cy + angle.sin() * radius);
            if let Some(p) = prev {
                self.capsule(p.0, p.1, pt.0, pt.1, width, color);
            }
            prev = Some(pt);
        }
    }

    fn rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color) {
        let s = self.scale as f64;
        for y in (y0 * s).floor().max(0.0) as isize..=((y1 * s).floor() as isize).min(self.h as isize - 1) {
            for x in (x0 * s).floor().max(0.0) as isize..=((x1 * s).floor() as isize).min(self.w as isize - 1) {
                self.blend(x, y, color);
            }
        }
    }

    fn blit_half(&mut self, img: &Image, dx: isize, dy: isize) {
        for y in 0..img.height / 2 {
            for x in 0..img.width / 2 {
                let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
                for sy in 0..2 {
                    let base = ((y * 2 + sy) * img.width + x * 2) * 4;
                    for sx in 0..2 {
                        let i = base + sx * 4;
                        let alpha = img.data[i + 3] as u32;
                        r += img.data[i] as u32 * alpha;
                        g += img.data[i + 1] as u32 * alpha;
                        b += img.data[i + 2] as u32 * alpha;
                        a += alpha;
                    }
                }
                if a > 0 {
                    self.blend(
                        dx + x as isize,
                        dy + y as isize,
                        [(r / a) as u8, (g / a) as u8, (b / a) as u8, (a / 4) as u8],
                    );
                }
            }
        }
    }

    fn text(&mut self, x: f64, y: f64, value: &str, size: f64, color: Color) {
        let mut cursor = x;
        for ch in value.chars() {
            if let Some(rows) = glyph(ch) {
                for (row, line) in rows.iter().enumerate() {
                    for (col, cell) in line.bytes().enumerate() {
                        if cell == b'#' {
                            self.rect(
                                cursor + col as f64 * size,
                                y + row as f64 * size,
                                cursor + (col + 1) as f64 * size - 1.0,
                                y + (row + 1) as f64 * size - 1.0,
                                color,
                            );
                        }
                    }
                }
            }
            cursor += 6.0 * size;
        }
    }

    fn downsample(self) -> Image {
        let s = self.scale;
        let (w, h) = (self.w / s, self.h / s);
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
                for dy in 0..s {
                    let base = ((y * s + dy) * self.w + x * s) * 4;
                    for dx in 0..s {
                        let i = base + dx * 4;
                        let alpha = self.buf[i + 3] as u32;
                        r += self.buf[i] as u32 * alpha;
                        g += self.buf[i + 1] as u32 * alpha;
                        b += self.buf[i + 2] as u32 * alpha;
                        a += alpha;
                    }
                }
                let o = (y * w + x) * 4;
                if a > 0 {
                    out[o] = (r / a) as u8;
                    out[o + 1] = (g / a) as u8;
                    out[o + 2] = (b / a) as u8;
                    out[o + 3] = (a / (s * s) as u32) as u8;
                }
            }
        }
        Image {
            width: w,
            height: h,
            data: out,
        }
    }
}

fn png_chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(payload);
    let mut crc = Crc32::new();
    crc.update(tag);
    crc.update(payload);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
    out
}

fn write_png(path: &Path, img: &Image) -> Result<(), String> {
    let stride = img.width * 4;
    let mut raw = Vec::with_capacity(img.height * (stride + 1));
    for y in 0..img.height {
        raw.push(0);
        raw.extend_from_slice(&img.data[y * stride..(y + 1) * stride]);
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&raw).map_err(|e| e.to_string())?;
    let idat = encoder.finish().map_err(|e| e.to_string())?;

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(img.width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(img.height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    png.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&png_chunk(b"IDAT", &idat));
    png.extend_from_slice(&png_chunk(b"IEND", &[]));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, png).map_err(|e| e.to_string())
}

#[derive(Clone, Copy)]
enum Pose {
    Idle,
    Left,
    Right,
}

fn draw_body(c: &mut Canvas, pose: Pose) {
    let lean = match pose {
        Pose::Idle => 0.0,
        Pose::Left => -7.0,
        Pose::Right => 7.0,
    };
    let cx = 280.0 + lean;
    c.capsule(340.0, 448.0, 398.0, 408.0, 13.0, OUTLINE);
    c.capsule(340.0, 448.0, 398.0, 408.0, 8.0, BODY_DARK);

    let (legs, feet, arms, hands) = match pose {
        Pose::Idle => (
            [(254.0, 448.0, 250.0, 482.0), (306.0, 448.0, 310.0, 482.0)],
            [(246.0, 490.0, 30.0, 15.0), (314.0, 490.0, 30.0, 15.0)],
            [(208.0, 374.0, 190.0, 438.0), (352.0, 374.0, 370.0, 438.0)],
            [(188.0, 442.0, 11.0), (372.0, 442.0, 11.0)],
        ),
        Pose::Left => (
            [(252.0, 448.0, 218.0, 476.0), (306.0, 448.0, 336.0, 482.0)],
            [(206.0, 486.0, 33.0, 14.0), (344.0, 492.0, 27.0, 12.0)],
            [(208.0, 374.0, 176.0, 348.0), (352.0, 374.0, 364.0, 440.0)],
            [(172.0, 344.0, 11.0), (366.0, 444.0, 11.0)],
        ),
        Pose::Right => (
            [(254.0, 448.0, 224.0, 482.0), (308.0, 448.0, 342.0, 476.0)],
            [(216.0, 492.0, 27.0, 12.0), (354.0, 486.0, 33.0, 14.0)],
            [(208.0, 374.0, 196.0, 440.0), (352.0, 374.0, 384.0, 348.0)],
            [(194.0, 444.0, 11.0), (388.0, 344.0, 11.0)],
        ),
    };

    if !matches!(pose, Pose::Idle) {
        let sign = if matches!(pose, Pose::Left) { -1.0 } else { 1.0 };
        for (offset, length) in [(0.0, 26.0), (16.0, 18.0)] {
            c.capsule(
                280.0 + sign * (108.0 + offset),
                500.0 - offset,
                280.0 + sign * (108.0 + offset + length),
                500.0 - offset,
                3.0,
                [94, 182, 172, 120],
            );
        }
    }

    for (x1, y1, x2, y2) in legs {
        c.capsule(x1, y1, x2, y2, 22.0, OUTLINE);
    }
    for (x1, y1, x2, y2) in legs {
        c.capsule(x1, y1, x2, y2, 16.0, BODY);
    }
    for (fx, fy, rx, ry) in feet {
        c.ellipse(fx, fy, rx + 5.0, ry + 5.0, OUTLINE);
    }
    for (fx, fy, rx, ry) in feet {
        c.ellipse(fx, fy, rx, ry, BODY);
    }

    c.ellipse(cx, 404.0, 74.0, 66.0, OUTLINE);
    c.ellipse(cx, 404.0, 68.0, 60.0, BODY);
    c.ellipse(cx, 380.0, 50.0, 28.0, [215, 244, 238, 120]);
    c.polygon(
        &[(cx, 392.0), (cx - 13.0, 416.0), (cx + 13.0, 416.0)],
        ACCENT,
    );
    c.ellipse(cx, 436.0, 7.0, 7.0, ACCENT);

    for (x1, y1, x2, y2) in arms {
        c.capsule(x1, y1, x2, y2, 16.0, OUTLINE);
    }
    for (x1, y1, x2, y2) in arms {
        c.capsule(x1, y1, x2, y2, 10.0, BODY);
    }
    for (hx, hy, hr) in hands {
        c.ellipse(hx, hy, hr + 5.0, hr + 5.0, OUTLINE);
    }
    for (hx, hy, hr) in hands {
        c.ellipse(hx, hy, hr, hr, BODY);
    }
}

fn draw_head(c: &mut Canvas, blink: bool) {
    for sign in [-1.0, 1.0] {
        let ear = [
            (280.0 + sign * 34.0, 190.0),
            (280.0 + sign * 62.0, 92.0),
            (280.0 + sign * 128.0, 178.0),
        ];
        c.poly_outlined(&ear, HEAD, 5.0);
    }
    for sign in [-1.0, 1.0] {
        let inner = [
            (280.0 + sign * 52.0, 172.0),
            (280.0 + sign * 66.0, 122.0),
            (280.0 + sign * 106.0, 170.0),
        ];
        c.polygon(&inner, EAR_INNER);
    }
    c.ellipse(280.0, 236.0, 112.0, 106.0, OUTLINE);
    c.ellipse(280.0, 236.0, 106.0, 100.0, HEAD);
    c.ellipse(280.0, 200.0, 76.0, 42.0, [222, 247, 241, 130]);
    c.ellipse(204.0, 274.0, 17.0, 10.0, CHEEK);
    c.ellipse(356.0, 274.0, 17.0, 10.0, CHEEK);

    if blink {
        for sign in [-1.0, 1.0] {
            c.arc(280.0 + sign * 36.0, 240.0, 17.0, 200.0, 340.0, 4.0, EYE, 16);
        }
    } else {
        for sign in [-1.0, 1.0] {
            let ex = 280.0 + sign * 36.0;
            c.ellipse(ex, 232.0, 15.0, 17.0, EYE);
            c.ellipse(ex - 4.0, 225.0, 5.0, 6.0, WHITE);
        }
    }
    c.arc(272.0, 274.0, 9.0, 200.0, 340.0, 3.0, OUTLINE, 12);
    c.arc(288.0, 274.0, 9.0, 200.0, 340.0, 3.0, OUTLINE, 12);
}

fn draw_cat_ears(c: &mut Canvas, cx: f64, cy: f64, scale: f64) {
    for sign in [-1.0, 1.0] {
        c.poly_outlined(
            &[
                (cx + sign * 34.0 * scale, cy - 46.0 * scale),
                (cx + sign * 62.0 * scale, cy - 144.0 * scale),
                (cx + sign * 128.0 * scale, cy - 58.0 * scale),
            ],
            PINK,
            5.0 * scale,
        );
    }
    for sign in [-1.0, 1.0] {
        c.polygon(
            &[
                (cx + sign * 52.0 * scale, cy - 64.0 * scale),
                (cx + sign * 66.0 * scale, cy - 114.0 * scale),
                (cx + sign * 106.0 * scale, cy - 66.0 * scale),
            ],
            PINK_DARK,
        );
    }
}

fn draw_bow(c: &mut Canvas, cx: f64, cy: f64, scale: f64) {
    c.poly_outlined(
        &[
            (cx, cy),
            (cx - 62.0 * scale, cy - 34.0 * scale),
            (cx - 62.0 * scale, cy + 34.0 * scale),
        ],
        GOLD,
        5.0 * scale,
    );
    c.poly_outlined(
        &[
            (cx, cy),
            (cx + 62.0 * scale, cy - 34.0 * scale),
            (cx + 62.0 * scale, cy + 34.0 * scale),
        ],
        GOLD,
        5.0 * scale,
    );
    c.capsule(
        cx - 40.0 * scale,
        cy + 6.0 * scale,
        cx - 58.0 * scale,
        cy + 52.0 * scale,
        9.0 * scale,
        GOLD_DARK,
    );
    c.capsule(
        cx + 40.0 * scale,
        cy + 6.0 * scale,
        cx + 58.0 * scale,
        cy + 52.0 * scale,
        9.0 * scale,
        GOLD_DARK,
    );
    c.ellipse(cx, cy, 17.0 * scale, 17.0 * scale, OUTLINE);
    c.ellipse(cx, cy, 12.0 * scale, 12.0 * scale, GOLD);
}

fn draw_scarf(c: &mut Canvas, cx: f64, cy: f64, scale: f64) {
    c.arc(cx, cy - 96.0 * scale, 86.0 * scale, 24.0, 156.0, 16.0 * scale, OUTLINE, 28);
    c.arc(cx, cy - 96.0 * scale, 86.0 * scale, 24.0, 156.0, 10.0 * scale, SCARF, 28);
    c.capsule(
        cx + 62.0 * scale,
        cy - 10.0 * scale,
        cx + 74.0 * scale,
        cy + 62.0 * scale,
        14.0 * scale,
        OUTLINE,
    );
    c.capsule(
        cx + 62.0 * scale,
        cy - 10.0 * scale,
        cx + 74.0 * scale,
        cy + 62.0 * scale,
        9.0 * scale,
        SCARF_DARK,
    );
}

fn draw_emote(c: &mut Canvas, kind: &str) {
    let (cx, cy) = (280.0, 236.0);
    c.ellipse(cx, cy + 44.0, 178.0, 172.0, OUTLINE);
    c.ellipse(cx, cy + 40.0, 170.0, 164.0, HEAD);
    c.ellipse(cx, cy - 20.0, 118.0, 62.0, [222, 247, 241, 120]);
    c.ellipse(cx - 108.0, cy + 74.0, 26.0, 15.0, CHEEK);
    c.ellipse(cx + 108.0, cy + 74.0, 26.0, 15.0, CHEEK);
    match kind {
        "happy" => {
            for sign in [-1.0, 1.0] {
                c.arc(cx + sign * 56.0, cy + 20.0, 26.0, 200.0, 340.0, 7.0, EYE, 18);
            }
            c.arc(cx, cy + 88.0, 30.0, 20.0, 160.0, 6.0, OUTLINE, 18);
        }
        "wink" => {
            c.arc(cx - 56.0, cy + 20.0, 26.0, 200.0, 340.0, 7.0, EYE, 18);
            c.ellipse(cx + 56.0, cy + 16.0, 19.0, 22.0, EYE);
            c.ellipse(cx + 51.0, cy + 8.0, 6.0, 7.0, WHITE);
            c.arc(cx, cy + 84.0, 28.0, 20.0, 160.0, 6.0, OUTLINE, 18);
        }
        _ => {
            for sign in [-1.0, 1.0] {
                c.capsule(cx + sign * 34.0, cy - 16.0, cx + sign * 78.0, cy - 6.0, 7.0, OUTLINE);
                c.ellipse(cx + sign * 56.0, cy + 26.0, 16.0, 14.0, EYE);
            }
            c.arc(cx, cy + 112.0, 24.0, 200.0, 340.0, 6.0, OUTLINE, 16);
            for sign in [-1.0, 1.0] {
                c.capsule(cx + sign * 96.0, cy - 132.0, cx + sign * 128.0, cy - 168.0, 6.0, ACCENT);
                c.capsule(cx + sign * 128.0, cy - 132.0, cx + sign * 96.0, cy - 168.0, 6.0, ACCENT);
            }
        }
    }
}

fn render_with<F: FnOnce(&mut Canvas)>(draw: F) -> Image {
    let mut canvas = Canvas::new(SIZE, SIZE, SS);
    draw(&mut canvas);
    canvas.downsample()
}

fn render_sheet(entries: &[(&str, &Image)], columns: usize) -> Image {
    let cell = SIZE / 2;
    let label_height = 30;
    let rows = (entries.len() + columns - 1) / columns;
    let mut canvas = Canvas::new(cell * columns, (cell + label_height) * rows, 1);
    canvas.rect(
        0.0,
        0.0,
        (cell * columns) as f64,
        ((cell + label_height) * rows) as f64,
        [238, 240, 242, 255],
    );
    for (idx, (label, img)) in entries.iter().enumerate() {
        let ox = (idx % columns) * cell;
        let oy = (idx / columns) * (cell + label_height);
        canvas.blit_half(img, ox as isize, oy as isize);
        let text_width = label.len() * 6 * 2;
        canvas.text(
            (ox + (cell - text_width) / 2) as f64,
            (oy + cell + 6) as f64,
            label,
            2.0,
            OUTLINE,
        );
    }
    canvas.downsample()
}

fn write_zip_pack(archive: &Path, pack_dir: &Path, rel_files: &[String]) -> Result<(), String> {
    if archive.exists() {
        let _ = fs::remove_file(archive);
    }
    let file = File::create(archive).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for rel in ["manifest.json", "items.json"]
        .into_iter()
        .map(str::to_owned)
        .chain(rel_files.iter().cloned())
    {
        zip.start_file(&rel, opts).map_err(|e| e.to_string())?;
        let bytes = fs::read(pack_dir.join(&rel)).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn generate_all(out_dir: &Path) -> Result<(), String> {
    // 1. Demo character pack
    let pet_dir = out_dir.join("demo-pet");
    let pet_assets = pet_dir.join("assets/character");
    fs::create_dir_all(&pet_assets).map_err(|e| e.to_string())?;
    let manifest_pet = json!({"schemaVersion": 1, "name": "演示 · 会动的小家伙"});
    let items_pet = json!({
        "items": [{
            "name": "小家伙（演示）",
            "type": "character",
            "assets": {
                "icon": "assets/character/icon.png",
                "body1": "assets/character/body_1.png",
                "body2": "assets/character/body_2.png",
                "body3": "assets/character/body_3.png",
                "head1": "assets/character/head_1.png",
                "head2": "assets/character/head_2.png",
            }
        }]
    });
    fs::write(
        pet_dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest_pet).map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        pet_dir.join("items.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&items_pet).map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;

    let body1 = render_with(|c| draw_body(c, Pose::Idle));
    let body2 = render_with(|c| draw_body(c, Pose::Left));
    let body3 = render_with(|c| draw_body(c, Pose::Right));
    let head1 = render_with(|c| draw_head(c, false));
    let head2 = render_with(|c| draw_head(c, true));
    let icon = render_with(|c| {
        draw_body(c, Pose::Idle);
        draw_head(c, false);
    });
    let pet_images = [
        ("assets/character/body_1.png", &body1),
        ("assets/character/body_2.png", &body2),
        ("assets/character/body_3.png", &body3),
        ("assets/character/head_1.png", &head1),
        ("assets/character/head_2.png", &head2),
        ("assets/character/icon.png", &icon),
    ];
    for (rel, img) in &pet_images {
        write_png(&pet_dir.join(rel), img)?;
    }
    let sheet = render_sheet(
        &[
            ("BODY1 IDLE", &body1),
            ("BODY2 WALK L", &body2),
            ("BODY3 WALK R", &body3),
            ("HEAD1 OPEN", &head1),
            ("HEAD2 BLINK", &head2),
            ("ICON", &icon),
        ],
        3,
    );
    write_png(&out_dir.join("frames.png"), &sheet)?;
    let pet_rel: Vec<String> = pet_images.iter().map(|(r, _)| (*r).into()).collect();
    write_zip_pack(&out_dir.join("demo-animated-pet.zip"), &pet_dir, &pet_rel)?;

    // 2. Demo accessories pack
    let (cx, cy) = (280.0, 236.0);
    let acc_dir = out_dir.join("demo-accessories");
    fs::create_dir_all(acc_dir.join("assets")).map_err(|e| e.to_string())?;
    let acc_defs: [(&str, &str, &str, Image, Image); 3] = [
        (
            "猫耳",
            "ears",
            "EARS",
            render_with(|c| draw_cat_ears(c, cx, cy, 1.0)),
            render_with(|c| draw_cat_ears(c, cx, 372.0, 1.9)),
        ),
        (
            "蝴蝶结",
            "bow",
            "BOW",
            render_with(|c| draw_bow(c, cx + 96.0, cy - 128.0, 1.0)),
            render_with(|c| draw_bow(c, cx, 300.0, 2.3)),
        ),
        (
            "围巾",
            "scarf",
            "SCARF",
            render_with(|c| draw_scarf(c, cx, cy, 1.0)),
            render_with(|c| draw_scarf(c, cx, 430.0, 1.6)),
        ),
    ];
    let mut acc_docs = Vec::new();
    let mut acc_rel = Vec::new();
    for (label, slug, _, img, ico) in &acc_defs {
        let img_rel = format!("assets/accessory-{slug}.png");
        let ico_rel = format!("assets/accessory-{slug}-icon.png");
        write_png(&acc_dir.join(&img_rel), img)?;
        write_png(&acc_dir.join(&ico_rel), ico)?;
        acc_rel.push(img_rel.clone());
        acc_rel.push(ico_rel.clone());
        acc_docs.push(json!({"name": label, "type": "accessory", "assets": {"image": img_rel, "icon": ico_rel}}));
    }
    fs::write(
        acc_dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({"schemaVersion": 1, "name": "演示 · 挂饰三件"}))
                .map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        acc_dir.join("items.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({"items": acc_docs}))
                .map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;
    write_zip_pack(&out_dir.join("demo-accessories.zip"), &acc_dir, &acc_rel)?;

    // 3. Demo emotes pack
    let emo_dir = out_dir.join("demo-emotes");
    fs::create_dir_all(emo_dir.join("assets")).map_err(|e| e.to_string())?;
    let emo_defs = [
        ("开心", "happy", "HAPPY", render_with(|c| draw_emote(c, "happy"))),
        ("眨眼", "wink", "WINK", render_with(|c| draw_emote(c, "wink"))),
        ("生气", "angry", "ANGRY", render_with(|c| draw_emote(c, "angry"))),
    ];
    let mut emo_docs = Vec::new();
    let mut emo_rel = Vec::new();
    for (label, slug, _, img) in &emo_defs {
        let img_rel = format!("assets/emote-{slug}.png");
        let ico_rel = format!("assets/emote-{slug}-icon.png");
        write_png(&emo_dir.join(&img_rel), img)?;
        write_png(&emo_dir.join(&ico_rel), img)?;
        emo_rel.push(img_rel.clone());
        emo_rel.push(ico_rel.clone());
        emo_docs.push(json!({"name": label, "type": "emote", "assets": {"image": img_rel, "icon": ico_rel}}));
    }
    fs::write(
        emo_dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({"schemaVersion": 1, "name": "演示 · 表情三连"}))
                .map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        emo_dir.join("items.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({"items": emo_docs}))
                .map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;
    write_zip_pack(&out_dir.join("demo-emotes.zip"), &emo_dir, &emo_rel)?;

    let preview_entries: Vec<(&str, &Image)> = acc_defs
        .iter()
        .map(|(_, _, tag, _, ico)| (*tag, ico))
        .chain(emo_defs.iter().map(|(_, _, tag, img)| (*tag, img)))
        .collect();
    write_png(
        &out_dir.join("packs-preview.png"),
        &render_sheet(&preview_entries, 3),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_installable_demo_packs() {
        let dir = std::env::temp_dir().join(format!(
            "wuwa-demo-packs-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        generate_all(&dir).unwrap();

        let store = dir.join("custom-atlas");
        for archive in [
            "demo-animated-pet.zip",
            "demo-accessories.zip",
            "demo-emotes.zip",
        ] {
            let summary =
                crate::custom_resources::install_archive(&dir.join(archive), &store, "CN")
                    .unwrap();
            assert!(summary.item_count >= 1);
            assert!(summary.asset_count >= 6);
        }
        assert!(dir.join("frames.png").is_file());
        assert!(dir.join("packs-preview.png").is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}
