#!/usr/bin/env python
"""產生 InkKeep 的圖示。需要 Pillow。

    python crates/inkkeep-app/icons/generate.py

圖案是白底上的一滴藍墨。底部是正圓，兩側用跟圓相切的三次貝茲曲線收到尖端，
相切讓接縫處沒有折角。

底框加一圈很淡的灰邊，讓白框在淺色工作列上仍看得出輪廓。

所有尺寸從同一份 8 倍超取樣的母圖縮下來，邊緣才不會有鋸齒。
"""

import math
import pathlib

from PIL import Image, ImageDraw

SIZES = [16, 24, 32, 48, 64, 128, 256]

SS = 8
MASTER = 256 * SS

BADGE_FILL = (255, 255, 255, 255)
BADGE_EDGE = (208, 214, 226, 255)
BADGE_EDGE_WIDTH = 0.02
BADGE_RADIUS = 0.22

INK_TOP = (64, 132, 250)
INK_BOTTOM = (22, 60, 158)

# 墨滴形狀。座標以底部那個圓為單位（半徑 1、圓心在原點、y 朝上）
TIP_Y = 2.05  # 尖端高度
JOIN_DEG = 25  # 曲線在水平線以上幾度接上圓
TIP_DEG = 18  # 尖端處曲線偏離垂直幾度：越小尖端越細、側邊越凹
PULL_JOIN = 0.55  # 貝茲控制點離接點多遠
PULL_TIP = 0.45  # 貝茲控制點離尖端多遠
HEIGHT = 0.70  # 墨滴佔圖高的比例
SLIM = 0.90  # 橫向收窄


def bezier(p0, p1, p2, p3, n=400):
    pts = []
    for i in range(n + 1):
        t = i / n
        u = 1 - t
        pts.append(
            (
                u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0],
                u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1],
            )
        )
    return pts


def drop_outline():
    a = math.radians(JOIN_DEG)
    b = math.radians(TIP_DEG)
    join = (math.cos(a), math.sin(a))
    tip = (0.0, TIP_Y)
    # 接點處的控制點沿著圓的切線方向，曲線才會跟圓相切
    c1 = (join[0] - PULL_JOIN * math.sin(a), join[1] + PULL_JOIN * math.cos(a))
    c2 = (tip[0] + PULL_TIP * math.sin(b), tip[1] - PULL_TIP * math.cos(b))

    right = bezier(join, c1, c2, tip)
    left = [(-x, y) for x, y in reversed(right)]
    # 從左邊接點沿著底部繞到右邊接點
    arc = []
    steps = 600
    for i in range(1, steps):
        ang = math.pi - a + (math.pi + 2 * a) * i / steps
        arc.append((math.cos(ang), math.sin(ang)))
    return right + left + arc


def to_canvas(pts):
    ys = [p[1] for p in pts]
    scale = HEIGHT * MASTER / (max(ys) - min(ys))
    mid = (max(ys) + min(ys)) / 2
    return [(MASTER / 2 + x * scale * SLIM, MASTER / 2 - (y - mid) * scale) for x, y in pts]


def vertical_gradient(size, top, bottom):
    grad = Image.new("RGB", (1, size))
    px = grad.load()
    for y in range(size):
        t = y / (size - 1)
        px[0, y] = tuple(round(p + (q - p) * t) for p, q in zip(top, bottom))
    return grad.resize((size, size), Image.NEAREST)


def draw_master():
    radius = int(MASTER * BADGE_RADIUS)
    box = [0, 0, MASTER - 1, MASTER - 1]

    art = Image.new("RGBA", (MASTER, MASTER), (0, 0, 0, 0))
    ImageDraw.Draw(art).rounded_rectangle(
        box,
        radius=radius,
        fill=BADGE_FILL,
        outline=BADGE_EDGE,
        width=int(MASTER * BADGE_EDGE_WIDTH),
    )
    corner = Image.new("L", (MASTER, MASTER), 0)
    ImageDraw.Draw(corner).rounded_rectangle(box, radius=radius, fill=255)
    art.putalpha(corner)

    drop = Image.new("L", (MASTER, MASTER), 0)
    ImageDraw.Draw(drop).polygon(to_canvas(drop_outline()), fill=255)
    ink = vertical_gradient(MASTER, INK_TOP, INK_BOTTOM).convert("RGBA")
    art.paste(ink, (0, 0), drop)
    return art


def main():
    here = pathlib.Path(__file__).parent
    icon = draw_master().resize((256, 256), Image.LANCZOS)
    icon.save(here / "icon.png")
    icon.save(here / "icon.ico", sizes=[(s, s) for s in SIZES])
    print("寫出 icon.ico（" + ", ".join(f"{s}x{s}" for s in SIZES) + "）與 icon.png")


if __name__ == "__main__":
    main()
