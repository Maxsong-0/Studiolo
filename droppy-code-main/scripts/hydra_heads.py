#!/usr/bin/env python3
"""Draws Hydra's heads: twenty-five dragon heads in profile and the one-headed mark, as
template SVGs for the asset catalog. Every head is one closed contour built from parts
(neck, skull and horns, snout, mouth, throat) plus an eye and sometimes a nostril, cut as
holes by winding. Run it to regenerate the assets; pass --render for a contact sheet at
/tmp/hydra-svg/sheet.png (needs Pillow and qlmanage) to look at them.

    scripts/hydra_heads.py            # writes the SVGs into the asset catalog
    scripts/hydra_heads.py --render   # and a contact sheet to look at
"""
import math, os, sys, subprocess, json, shutil

ASSETS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "DroppyCode", "Resources", "Assets.xcassets")
OUT = "/tmp/hydra-svg/out"
os.makedirs(OUT, exist_ok=True)

# ---------- geometry helpers ----------
def signed_area(pts):
    a = 0
    for i in range(len(pts)):
        x1, y1 = pts[i][0], pts[i][1]
        x2, y2 = pts[(i+1) % len(pts)][0], pts[(i+1) % len(pts)][1]
        a += x1*y2 - x2*y1
    return a/2

def ensure_dir(pts, positive=True):
    # screen coords (y down): visually clockwise => positive shoelace
    return pts if (signed_area(pts) > 0) == positive else list(reversed(pts))

def smooth_path(pts):
    """Closed Catmull-Rom spline -> cubic beziers. pts: (x, y, t) with t = tension
    multiplier (1 smooth, 0 corner)."""
    n = len(pts)
    def P(i): return pts[i % n]
    d = []
    x0, y0 = P(0)[0], P(0)[1]
    d.append(f"M{x0:.2f} {y0:.2f}")
    for i in range(n):
        p0, p1, p2, p3 = P(i-1), P(i), P(i+1), P(i+2)
        t1 = p1[2] if len(p1) > 2 else 1
        t2 = p2[2] if len(p2) > 2 else 1
        c1x = p1[0] + (p2[0]-p0[0])/6*t1
        c1y = p1[1] + (p2[1]-p0[1])/6*t1
        c2x = p2[0] - (p3[0]-p1[0])/6*t2
        c2y = p2[1] - (p3[1]-p1[1])/6*t2
        d.append(f"C{c1x:.2f} {c1y:.2f} {c2x:.2f} {c2y:.2f} {p2[0]:.2f} {p2[1]:.2f}")
    d.append("Z")
    return " ".join(d)

def circle_path(cx, cy, r, clockwise=True, ry=None):
    ry = ry or r
    k = 0.5523
    if clockwise:
        return (f"M{cx:.2f} {cy-ry:.2f} C{cx+r*k:.2f} {cy-ry:.2f} {cx+r:.2f} {cy-ry*k:.2f} {cx+r:.2f} {cy:.2f} "
                f"C{cx+r:.2f} {cy+ry*k:.2f} {cx+r*k:.2f} {cy+ry:.2f} {cx:.2f} {cy+ry:.2f} "
                f"C{cx-r*k:.2f} {cy+ry:.2f} {cx-r:.2f} {cy+ry*k:.2f} {cx-r:.2f} {cy:.2f} "
                f"C{cx-r:.2f} {cy-ry*k:.2f} {cx-r*k:.2f} {cy-ry:.2f} {cx:.2f} {cy-ry:.2f} Z")
    else:
        return (f"M{cx:.2f} {cy-ry:.2f} C{cx-r*k:.2f} {cy-ry:.2f} {cx-r:.2f} {cy-ry*k:.2f} {cx-r:.2f} {cy:.2f} "
                f"C{cx-r:.2f} {cy+ry*k:.2f} {cx-r*k:.2f} {cy+ry:.2f} {cx:.2f} {cy+ry:.2f} "
                f"C{cx+r*k:.2f} {cy+ry:.2f} {cx+r:.2f} {cy+ry*k:.2f} {cx+r:.2f} {cy:.2f} "
                f"C{cx+r:.2f} {cy-ry*k:.2f} {cx+r*k:.2f} {cy-ry:.2f} {cx:.2f} {cy-ry:.2f} Z")

def transform(pts, scale=1.0, angle=0.0, mirror=False, dx=0.0, dy=0.0, cx=50, cy=50):
    out = []
    a = math.radians(angle)
    for p in pts:
        x, y = p[0]-cx, p[1]-cy
        if mirror: x = -x
        x, y = x*scale, y*scale
        xr = x*math.cos(a) - y*math.sin(a)
        yr = x*math.sin(a) + y*math.cos(a)
        out.append((xr+cx+dx, yr+cy+dy) + tuple(p[2:]))
    return out

S = 0  # corner

# ---------- head parts (100x100, facing right, clockwise) ----------
NECK = {
    "plain":  [(16,94,S),(17,74),(21,56)],
    "spiked": [(16,94,S),(17,82),(10,74,S),(19,68),(11,60,S),(21,54),(15,46,S),(24,44)],
    "fin":    [(16,94,S),(13,80),(9,68),(11,56),(19,48)],
    "mane":   [(16,94,S),(15,84),(6,78,S),(16,72),(7,64,S),(18,58),(9,50,S),(24,46)],
}
SKULL = {
    "none":     [(26,44),(30,34),(42,26),(54,25),(64,29)],
    "single":   [(26,44),(28,36),(21,22),(16,7,S),(27,18),(37,26),(50,24),(64,29)],
    "double":   [(26,44),(24,36),(13,27),(7,17,S),(20,27),(28,32),(23,20),(20,5,S),(31,16),(41,25),(54,24),(64,29)],
    "straight": [(26,44),(30,34),(31,22),(30,8,S),(36,24),(38,30),(46,26),(47,16),(48,4,S),(52,18),(54,26),(64,29)],
    "ram":      [(26,44),(28,36),(16,28),(12,14),(20,5),(33,7),(37,14,S),(31,14),(24,15),(20,20),(23,26),(32,30),(40,26),(52,24),(64,29)],
    "nubs":     [(26,44),(30,34),(35,27),(38,31),(46,25),(50,29),(64,29)],
    "antlers":  [(26,44),(28,36),(22,26),(12,14,S),(21,23),(21,10,S),(26,20),(30,7,S),(32,22),(40,25),(54,24),(64,29)],
    "ears":     [(26,44),(30,34),(34,31),(25,20),(28,8,S),(41,23),(52,24),(64,29)],
    "crest":    [(26,44),(30,34),(33,20,S),(38,28),(43,16,S),(48,26),(53,18,S),(58,27),(64,29)],
    "swept":    [(26,44),(29,36),(18,30),(8,26,S),(20,30),(30,30),(50,24),(64,29)],
}
SNOUT = {
    "long":     [(72,38),(84,50),(94,60),(95,66)],
    "short":    [(72,40),(80,50),(86,58),(87,66)],
    "upturned": [(74,40),(86,48),(94,52),(96,58),(92,64)],
    "beak":     [(72,38),(84,48),(94,58),(96,66),(92,74,S),(89,70)],
    "bumped":   [(72,38),(77,40),(82,30,S),(88,46),(94,58),(95,66)],
}
MOUTH = {
    "closed":     [(92,70),(84,72),(72,72),(64,75)],
    "open":       [(90,70),(80,70),(71,68),(74,76),(80,80),(88,86,S),(78,90),(66,88),(58,80)],
    "teeth":      [(90,70),(84,70),(82,76,S),(80,70),(71,68),(74,76),(78,74),(80,80,S),(82,79),(88,86,S),(78,90),(66,88),(58,80)],
    "tongue":     [(90,70),(80,70),(71,68),(73,74),(79,73),(85,75),(87,78),(83,80),(90,88,S),(78,91),(66,88),(58,80)],
    "smirk":      [(92,70),(82,71),(70,71),(73,77),(80,80),(88,83,S),(78,87),(66,86),(58,80)],
}
THROAT = {
    "plain":  [(52,86),(44,92),(40,94,S)],
    "beard":  [(56,84),(54,90),(52,100,S),(46,92),(40,94,S)],
    "frill":  [(58,82),(63,90),(57,97),(48,97),(42,94,S),(40,94,S)],
    "wattle": [(54,86),(53,96),(46,98),(40,94,S)],
}

def head_points(neck="plain", skull="single", snout="long", mouth="closed", throat="plain"):
    pts = []
    pts += NECK[neck]
    pts += SKULL[skull]
    pts += SNOUT[snout]
    pts += MOUTH[mouth]
    pts += THROAT[throat]
    # de-duplicate consecutive equal points
    out = []
    for p in pts:
        if out and abs(out[-1][0]-p[0]) < 0.01 and abs(out[-1][1]-p[1]) < 0.01: continue
        out.append(p)
    return out

def head_svg(spec):
    pts = head_points(spec.get("neck","plain"), spec.get("skull","single"), spec.get("snout","long"), spec.get("mouth","closed"), spec.get("throat","plain"))
    pts = ensure_dir(pts, True)
    d = smooth_path(pts)
    eye = spec.get("eye", ("round", 63, 43, 4.6))
    if eye[0] == "round":
        d += " " + circle_path(eye[1], eye[2], eye[3], clockwise=False)
    elif eye[0] == "slit":
        d += " " + circle_path(eye[1], eye[2], eye[3], clockwise=False, ry=eye[4])
    if spec.get("nostril"):
        nx, ny = spec["nostril"]
        d += " " + circle_path(nx, ny, 1.8, clockwise=False)
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="100" height="100"><path fill="#000000" fill-rule="nonzero" d="{d}"/></svg>'

SPECS = [
 ("Hank",   dict(skull="single",   snout="long",     mouth="closed", neck="plain",  throat="plain",  nostril=(86,63))),
 ("Walter", dict(skull="double",   snout="short",    mouth="smirk",  neck="plain",  throat="beard")),
 ("Ada",    dict(skull="ears",     snout="upturned", mouth="closed", neck="fin",    throat="plain",  eye=("round",62,42,5))),
 ("Otto",   dict(skull="ram",      snout="short",    mouth="closed", neck="plain",  throat="wattle")),
 ("Nova",   dict(skull="crest",    snout="long",     mouth="open",   neck="spiked", throat="plain")),
 ("Remy",   dict(skull="straight", snout="beak",     mouth="closed", neck="plain",  throat="plain",  eye=("slit",62,43,3,6))),
 ("Iris",   dict(skull="swept",    snout="long",     mouth="closed", neck="mane",   throat="plain",  nostril=(87,63))),
 ("Milo",   dict(skull="nubs",     snout="bumped",   mouth="closed", neck="plain",  throat="frill")),
 ("Juno",   dict(skull="antlers",  snout="short",    mouth="smirk",  neck="plain",  throat="plain")),
 ("Ezra",   dict(skull="single",   snout="beak",     mouth="teeth",  neck="spiked", throat="plain")),
 ("Lena",   dict(skull="ears",     snout="long",     mouth="closed", neck="plain",  throat="beard",  eye=("slit",62,43,3,6))),
 ("Bo",     dict(skull="none",     snout="short",    mouth="tongue", neck="fin",    throat="plain")),
 ("Kai",    dict(skull="double",   snout="long",     mouth="open",   neck="plain",  throat="plain",  nostril=(87,63))),
 ("Vera",   dict(skull="crest",    snout="upturned", mouth="closed", neck="plain",  throat="wattle")),
 ("Finn",   dict(skull="straight", snout="long",     mouth="smirk",  neck="mane",   throat="plain")),
 ("Mira",   dict(skull="swept",    snout="short",    mouth="closed", neck="plain",  throat="frill",  eye=("round",62,42,5))),
 ("Odin",   dict(skull="ram",      snout="bumped",   mouth="teeth",  neck="spiked", throat="beard")),
 ("Suki",   dict(skull="nubs",     snout="upturned", mouth="closed", neck="fin",    throat="plain",  eye=("slit",62,43,3,6))),
 ("Rex",    dict(skull="antlers",  snout="long",     mouth="open",   neck="plain",  throat="plain")),
 ("Zola",   dict(skull="single",   snout="short",    mouth="closed", neck="mane",   throat="wattle", nostril=(80,63))),
 ("Pip",    dict(skull="none",     snout="upturned", mouth="smirk",  neck="plain",  throat="plain",  eye=("round",62,42,5.4))),
 ("Ivo",    dict(skull="ears",     snout="beak",     mouth="closed", neck="spiked", throat="plain")),
 ("Lux",    dict(skull="crest",    snout="bumped",   mouth="closed", neck="plain",  throat="beard")),
 ("Tova",   dict(skull="double",   snout="upturned", mouth="tongue", neck="fin",    throat="plain")),
 ("Gus",    dict(skull="swept",    snout="beak",     mouth="closed", neck="plain",  throat="wattle", nostril=(88,64))),
]

# ---------- the mark ----------
def mark_svg():
    """The Hydra mark is one clean dragon head, drawn from the same parts as the roster
    but bolder: two horns, a long snout, a closed mouth and a nostril, filling the frame.
    An earlier mark fanned three half-size heads on one collar; at the 15 to 30 points
    the mark is shown at, that read as a blob. One head reads at any size."""
    return head_svg(dict(skull="double", snout="long", mouth="closed", neck="plain", throat="plain",
                         eye=("round", 63, 43, 5), nostril=(86, 63)))

# ---------- write + render ----------
def install(name, svg):
    open(f"{OUT}/{name}.svg", "w").write(svg)
    folder = os.path.join(ASSETS, f"{name}.imageset")
    if os.path.isdir(ASSETS):
        os.makedirs(folder, exist_ok=True)
        open(os.path.join(folder, f"{name}.svg"), "w").write(svg)
        with open(os.path.join(folder, "Contents.json"), "w") as f:
            json.dump({"images": [{"filename": f"{name}.svg", "idiom": "universal"}],
                       "info": {"author": "xcode", "version": 1},
                       "properties": {"preserves-vector-representation": True, "template-rendering-intent": "template"}}, f, indent=2)
            f.write("\n")

for i, (name, spec) in enumerate(SPECS, 1):
    install(f"hydra-head-{i:02d}", head_svg(spec))
install("hydra-mark", mark_svg())

if "--render" in sys.argv:
    from PIL import Image, ImageDraw
    rdir = "/tmp/hydra-svg/render"; shutil.rmtree(rdir, ignore_errors=True); os.makedirs(rdir)
    pdir = "/tmp/hydra-svg/preview"; shutil.rmtree(pdir, ignore_errors=True); os.makedirs(pdir)
    names = [f"hydra-head-{i:02d}.svg" for i in range(1, 26)] + ["hydra-mark.svg"]
    files = []
    for n in names:
        svg = open(f"{OUT}/{n}").read().replace('width="100" height="100"', 'width="300" height="300"')
        open(f"{pdir}/{n}", "w").write(svg); files.append(f"{pdir}/{n}")
    subprocess.run(["qlmanage", "-t", "-s", "300", "-o", rdir] + files, capture_output=True)
    cols = 6; cell = 310
    sheet = Image.new("RGB", (cols*cell, 5*cell), "white")
    draw = ImageDraw.Draw(sheet)
    for idx, f in enumerate(files):
        png = os.path.join(rdir, os.path.basename(f) + ".png")
        if not os.path.exists(png): continue
        im = Image.open(png).convert("RGBA")
        bg = Image.new("RGBA", im.size, "white"); bg.alpha_composite(im)
        x, y = (idx % cols)*cell, (idx // cols)*cell
        sheet.paste(bg.convert("RGB"), (x+5, y+5))
        label = SPECS[idx][0] if idx < 25 else "MARK"
        draw.text((x+8, y+4), f"{idx+1} {label}", fill="red")
    strip = Image.new("RGB", (cols*cell, 60), (30, 30, 34))
    for idx, f in enumerate(files):
        png = os.path.join(rdir, os.path.basename(f) + ".png")
        if not os.path.exists(png): continue
        im = Image.open(png).convert("RGBA")
        # tint white: use alpha as the mask
        flat = Image.new("RGBA", im.size, "white"); flat.alpha_composite(im)
        from PIL import ImageOps
        alpha = ImageOps.invert(flat.convert("L")).resize((20, 20), Image.LANCZOS)
        glyph = Image.new("RGBA", (20, 20), (255, 255, 255, 0)); glyph.putalpha(alpha)
        strip.paste(glyph, (20 + idx*66, 20), glyph)
    full = Image.new("RGB", (sheet.width, sheet.height + 60), "white")
    full.paste(sheet, (0, 0)); full.paste(strip, (0, sheet.height))
    full.save("/tmp/hydra-svg/sheet.png")
    print("sheet at /tmp/hydra-svg/sheet.png")
