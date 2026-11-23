#!/usr/bin/env python3
# Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
"""Crew 的视觉资产生成器：锚（crew-mark）+ 25 面国际信号旗 A–Y（crew-hand-01…25）。

全部是单色模板 SVG（viewBox 0 0 100 100，黑色填充）：运行时按 hand 自己的颜色着色，
所以旗面用 **不透明度** 表达原旗的色阶 —— 蓝/黑 1.0、红 0.62、黄 0.3、白 镂空；
每面旗外面一圈 4 单位的细框，镂空多的旗（Q、S、X…）也能看出旗形。
用法：python3 scripts/gen_crew_flags.py   （直接覆写 Resources/Assets.xcassets 里的 26 个 svg）
"""
import math, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
ASSETS = ROOT / "Bottega/Resources/Assets.xcassets"
BLUE, RED, YEL = 1.0, 0.62, 0.3          # 色阶 → 不透明度
L, R = 8.0, 92.0                          # 旗面边界
FRAME = 4.0

def fmt(v):
    return f"{v:.2f}".rstrip("0").rstrip(".")

def poly(points):
    return "M" + " L".join(f"{fmt(x)} {fmt(y)}" for x, y in points) + " Z"

def rect(x0, y0, x1, y1):
    return poly([(x0, y0), (x1, y0), (x1, y1), (x0, y1)])

def path(d, opacity=1.0, rule="nonzero"):
    op = "" if opacity >= 1 else f' fill-opacity="{opacity}"'
    fr = "" if rule == "nonzero" else f' fill-rule="{rule}"'
    return f'<path fill="#000000"{op}{fr} d="{d}"/>'

def svg(body):
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="100" height="100">'
            + "".join(body) + "</svg>\n")

# ── 裁剪：把多边形裁进旗面（Sutherland–Hodgman），给对角条纹用 ──
def clip(points, x0=L, y0=L, x1=R, y1=R):
    def inside(p, edge):
        x, y = p
        return {"l": x >= x0, "r": x <= x1, "t": y >= y0, "b": y <= y1}[edge]
    def intersect(p, q, edge):
        (px, py), (qx, qy) = p, q
        if edge in "lr":
            x = x0 if edge == "l" else x1
            t = (x - px) / (qx - px); return (x, py + t * (qy - py))
        y = y0 if edge == "t" else y1
        t = (y - py) / (qy - py); return (px + t * (qx - px), y)
    out = list(points)
    for edge in "lrtb":
        src, out = out, []
        if not src: break
        prev = src[-1]
        for cur in src:
            if inside(cur, edge):
                if not inside(prev, edge): out.append(intersect(prev, cur, edge))
                out.append(cur)
            elif inside(prev, edge):
                out.append(intersect(prev, cur, edge))
            prev = cur
    return out

def frame(swallow=False):
    if swallow:
        outer = [(L, L), (R, L), (R - 18, 50), (R, R), (L, R)]
        inner = [(L + FRAME, L + FRAME), (R - 5.5, L + FRAME), (R - 18 - 5.5, 50), (R - 5.5, R - FRAME), (L + FRAME, R - FRAME)]
    else:
        outer = [(L, L), (R, L), (R, R), (L, R)]
        inner = [(L + FRAME, L + FRAME), (R - FRAME, L + FRAME), (R - FRAME, R - FRAME), (L + FRAME, R - FRAME)]
    return path(poly(outer) + " " + poly(list(reversed(inner))), rule="evenodd")

def notch(d_points):
    """燕尾：把右缘的三角缺口从多边形里去掉。"""
    return d_points

W = R - L
def hband(k, n):          # 第 k 条横带（共 n 条）
    return rect(L, L + W * k / n, R, L + W * (k + 1) / n)
def vband(k, n):
    return rect(L + W * k / n, L, L + W * (k + 1) / n, R)
def cell(r, c, n):
    s = W / n; return rect(L + c * s, L + r * s, L + (c + 1) * s, L + (r + 1) * s)
def plus(half=8):         # 十字
    c = 50
    return poly([(c - half, L), (c + half, L), (c + half, c - half), (R, c - half), (R, c + half), (c + half, c + half),
                 (c + half, R), (c - half, R), (c - half, c + half), (L, c + half), (L, c - half), (c - half, c - half)])
def saltire(half=7):      # 斜十字：两条斜带各自裁进旗面，同一 path 里 nonzero 并集
    d = half * math.sqrt(2)
    a = clip([(L - d, L + d), (L + d, L - d), (R + d, R - d), (R - d, R + d)])
    b = clip([(R - d, L - d), (R + d, L + d), (L + d, R + d), (L - d, R - d)])
    return poly(a) + " " + poly(b)
def saltire_holes(half=7):   # 蓝底白斜十字：四个三角
    d = half * math.sqrt(2)
    return " ".join(poly(t) for t in [
        [(L + d, L), (R - d, L), (50, 50 - d)], [(L, L + d), (L, R - d), (50 - d, 50)],
        [(L + d, R), (R - d, R), (50, 50 + d)], [(R, L + d), (R, R - d), (50 + d, 50)]])
def diagonals(n=10):
    """左上→右下方向的对角条纹，k 偶 = 第一色。"""
    out = {0: [], 1: []}
    lo, hi = 2 * L, 2 * R
    step = (hi - lo) / n
    for k in range(n):
        a, b = lo + k * step, lo + (k + 1) * step
        big = 400
        band = [(a - big, big), (a + big, -big), (b + big, -big), (b - big, big)]  # x+y ∈ [a,b]
        out[k % 2].append(poly(clip(band)))
    return out

# ── 25 面旗 ──
def flag(letter):
    parts, swallow = [], False
    tail = [(R, L), (R - 18, 50), (R, R)]
    if letter == "A":
        swallow = True
        parts.append(path(poly([(50, L), (R, L), (R - 18, 50), (R, R), (50, R)]), BLUE))
    elif letter == "B":
        swallow = True
        parts.append(path(poly([(L, L), (R, L), (R - 18, 50), (R, R), (L, R)]), RED))
    elif letter == "C":
        parts.append(path(" ".join(hband(k, 5) for k in (0, 2, 4)), BLUE))
        parts.append(path(hband(2, 5), RED))                       # 中带是红
        parts[0] = path(" ".join(hband(k, 5) for k in (0, 4)), BLUE)
    elif letter == "D":
        parts.append(path(rect(L, L + W / 4, R, R - W / 4), BLUE))
        parts.append(path(rect(L, L, R, L + W / 4) + " " + rect(L, R - W / 4, R, R), YEL))
    elif letter == "E":
        parts.append(path(hband(0, 2), BLUE)); parts.append(path(hband(1, 2), RED))
    elif letter == "F":
        parts.append(path(poly([(50, L), (R, 50), (50, R), (L, 50)]), RED))
    elif letter == "G":
        parts.append(path(" ".join(vband(k, 6) for k in (1, 3, 5)), BLUE))
        parts.append(path(" ".join(vband(k, 6) for k in (0, 2, 4)), YEL))
    elif letter == "H":
        parts.append(path(vband(1, 2), RED))
    elif letter == "I":
        parts.append(path(rect(L, L, R, R), YEL))
        parts.append(path(f"M50 28 A22 22 0 1 0 50 72 A22 22 0 1 0 50 28 Z", BLUE))
    elif letter == "J":
        parts.append(path(hband(0, 3) + " " + hband(2, 3), BLUE))
    elif letter == "K":
        parts.append(path(vband(1, 2), BLUE)); parts.append(path(vband(0, 2), YEL))
    elif letter == "L":
        parts.append(path(cell(0, 1, 2) + " " + cell(1, 0, 2), BLUE))
        parts.append(path(cell(0, 0, 2) + " " + cell(1, 1, 2), YEL))
    elif letter == "M":
        parts.append(path(saltire_holes(), BLUE))
    elif letter == "N":
        parts.append(path(" ".join(cell(r, c, 4) for r in range(4) for c in range(4) if (r + c) % 2 == 0), BLUE))
    elif letter == "O":
        parts.append(path(poly([(L, L), (R, L), (R, R)]), RED))
        parts.append(path(poly([(L, L), (R, R), (L, R)]), YEL))
    elif letter == "P":
        parts.append(path(rect(L, L, R, R) + " " + rect(66, 66, 34, 34), BLUE, "evenodd"))
    elif letter == "Q":
        parts.append(path(rect(L, L, R, R), YEL))
    elif letter == "R":
        parts.append(path(rect(L, L, R, R) + " " + plus(), RED, "evenodd"))
        parts.append(path(plus(), YEL))
    elif letter == "S":
        parts.append(path(rect(32, 32, 68, 68), BLUE))
    elif letter == "T":
        parts.append(path(vband(0, 3), RED)); parts.append(path(vband(2, 3), BLUE))
    elif letter == "U":
        parts.append(path(cell(0, 0, 2) + " " + cell(1, 1, 2), RED))
    elif letter == "V":
        parts.append(path(saltire(), RED))
    elif letter == "W":
        parts.append(path(rect(L, L, R, R) + " " + rect(72, 72, 28, 28), BLUE, "evenodd"))
        parts.append(path(rect(40, 40, 60, 60), RED))
    elif letter == "X":
        parts.append(path(plus(), BLUE))
    elif letter == "Y":
        d = diagonals(10)
        parts.append(path(" ".join(d[0]), RED)); parts.append(path(" ".join(d[1]), YEL))
    else:
        raise ValueError(letter)
    parts.append(frame(swallow))
    return svg(parts)

# ── 锚 ──
def anchor():
    ring = "M50 6 A10 10 0 1 0 50 26 A10 10 0 1 0 50 6 Z M50 11 A5 5 0 1 1 50 21 A5 5 0 1 1 50 11 Z"
    shank = rect(46, 24, 54, 86)
    stock = "M28 33 H72 A3.5 3.5 0 0 1 72 40 H28 A3.5 3.5 0 0 1 28 33 Z"
    arms = "M13 52 A37 37 0 0 0 87 52 L79 52 A29 29 0 0 1 21 52 Z"
    flukes = poly([(6, 54), (27, 54), (16, 38)]) + " " + poly([(94, 54), (73, 54), (84, 38)])
    return svg([path(ring, rule="evenodd"), path(shank), path(stock), path(arms), path(flukes)])

if __name__ == "__main__":
    letters = "ABCDEFGHIJKLMNOPQRSTUVWXY"
    for i, letter in enumerate(letters, start=1):
        p = ASSETS / f"crew-hand-{i:02d}.imageset" / f"crew-hand-{i:02d}.svg"
        assert p.exists(), p
        p.write_text(flag(letter), encoding="utf-8")
    p = ASSETS / "crew-mark.imageset" / "crew-mark.svg"; assert p.exists(), p
    p.write_text(anchor(), encoding="utf-8")
    print(f"✓ 写入 {len(letters)} 面旗 + 锚 → {ASSETS.relative_to(ROOT)}")
