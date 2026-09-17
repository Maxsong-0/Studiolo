#!/usr/bin/env python3
"""Renders the onboarding tour's captures from the real app, then imports them as assets.

The app runs once in its tour capture mode (see DroppyCode/Support/TourCaptures.swift):
it opens the real window with mock data over a curated gradient backdrop, photographs
each tour scene as a 16:10 still into build.noindex/tour-captures, and quits. This
script then resizes every still to exactly 1320x824 and writes it into the tour
imagesets (welcome comes from the web hero, themes from a seamless 2x2 collage).

A still's sidecar may name a `focus`, the part of the scene the page is about (the
open popover with its chip, the team's panel); the page is then cut around that focus
instead of the whole window, so the slider, recipes and Hydra pages zoom on their subject.

The website's set is NOT cut here: scripts/website_captures.py encodes the same
tour run whole into build.noindex/website-tour and uploads it to the R2 tour/v4
prefix with per-key verification, so only one uploader writes those keys. With
--website/--export-only/--upload this script delegates to that pipeline. Needs
Pillow (pip).

    scripts/tour_captures.py                 # build, capture, import into the asset catalog
    scripts/tour_captures.py --website       # ...and encode the website's set via website_captures.py
    scripts/tour_captures.py --export-only   # encode the website's set from the last run
    scripts/tour_captures.py --upload        # ...and push it to R2 (wrangler signed in)
"""

import json
import pathlib
import shutil
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
CAPTURES = ROOT / "build.noindex" / "tour-captures"
DERIVED = ROOT / "build.noindex" / "tour"
APP = DERIVED / "Build/Products/Debug/Droppy Code.app"
ASSETS = ROOT / "DroppyCode" / "Resources" / "Assets.xcassets"

WIDTH = 1320
HEIGHT = 824
SCENES = ["welcome", "hydra", "pairs", "slider", "threads", "recipes", "panels"]
CONTENTS = {"info": {"author": "xcode", "version": 1}}

# App window size in points for each still, matching the stage sizes in
# DroppyCode/Support/TourCaptures.swift (and TourCaptures+Hydra.swift). Every
# still is a 16:10 rect around the window: the frame grown by 72 pt on every
# side, then widened/heightened to 1.6. The popover scenes (tour-pairs,
# tour-slider, tour-recipes, web-hero) photograph the window unioned with the open popover
# (see Stage.tourCaptureRect(including:)), so their PNGs are larger than the
# window-only rect below; the website pipeline serves those whole, uncropped.
WINDOW_SIZES = {
    "tour-welcome": (1280, 800),
    "web-hero": (1280, 800),
    "tour-hydra": (960, 600),
    "tour-pairs": (960, 600),
    "tour-slider": (960, 600),
    "tour-threads": (960, 600),
    "tour-recipes": (960, 600),
    "tour-panels": (1280, 800),
    "tour-window": (660, 600),
    "web-diff": (1200, 660),
    "web-palette": (1200, 660),
    "web-plans": (1200, 660),
    "web-question": (1200, 660),
    "web-queue": (1200, 660),
    "web-intro": (1200, 660),
    "tour-theme-tokyoNight": (960, 600),
    "tour-theme-gruvbox": (960, 600),
    "tour-theme-catppuccinLatte": (960, 600),
    "tour-theme-rosePine": (960, 600),
    "web-theme-tokyoNight": (960, 600),
    "web-theme-gruvbox": (960, 600),
    "web-theme-catppuccinLatte": (960, 600),
    "web-theme-rosePine": (960, 600),
}
THEME_ORDER = ["tokyoNight", "gruvbox", "catppuccinLatte", "rosePine"]


def window_box(name):
    """Return (rw, rh, mx, my, w, h): still rect and margins in points."""
    stem = name.removesuffix(".png")
    if stem not in WINDOW_SIZES:
        raise KeyError(f"No window size for {name}")
    w, h = WINDOW_SIZES[stem]
    rw, rh = w + 144, h + 144
    if rw / rh < 1.6:
        rw = round(rh * 1.6)
    else:
        rh = round(rw / 1.6)
    return (rw, rh, (rw - w) / 2, (rh - h) / 2, w, h)


def step(title):
    print(f"\n==> {title}", flush=True)


def build():
    step("Building Droppy Code (Debug)")
    subprocess.run(["xcodegen", "generate", "--quiet"], cwd=ROOT, check=True)
    result = subprocess.run([
        "xcodebuild", "-project", "DroppyCode.xcodeproj", "-scheme", "DroppyCode", "-configuration", "Debug",
        "-destination", "platform=macOS", "-derivedDataPath", str(DERIVED), "-skipPackagePluginValidation", "build",
    ], cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        print("\n".join(line for line in result.stdout.splitlines() if "error:" in line))
        sys.exit("Build failed")


def capture():
    step("Capturing the tour scenes")
    shutil.rmtree(CAPTURES, ignore_errors=True)
    CAPTURES.mkdir(parents=True)
    # Through LaunchServices, not the binary: an app launched from a shell is refused
    # activation, and the captures need the key window of the active app. Not waited on:
    # the run is force-killed at its budget, so a stall can never leave it running.
    marker = "tour-captures " + str(CAPTURES)
    subprocess.run(["open", "-n", str(APP), "--args", "--tour-captures", str(CAPTURES)], check=True)
    deadline = time.time() + 630
    while time.time() < deadline:
        time.sleep(2)
        alive = subprocess.run(["pgrep", "-f", marker], capture_output=True, text=True).stdout.strip()
        if not alive:
            break
    else:
        subprocess.run(["pkill", "-9", "-f", marker])
        print("  the run overran its budget and was killed")
    log = CAPTURES / "run.log"
    if log.exists():
        print("  " + log.read_text().strip().splitlines()[-1])


def fit(image, width, height):
    """Centre-crop to the target aspect (a no-op when already exact, up to rounding),
    then resize to exactly width x height."""
    from PIL import Image, ImageDraw
    if abs(image.width / image.height - width / height) > 1e-9:
        target = width / height
        if image.width / image.height > target:
            want = round(image.height * target)
            left = (image.width - want) // 2
            image = image.crop((left, 0, left + want, image.height))
        else:
            want = round(image.width / target)
            top = (image.height - want) // 2
            image = image.crop((0, top, image.width, top + want))
    return image.resize((width, height), Image.LANCZOS)


def write_imageset(name, image):
    folder = ASSETS / f"{name}.imageset"
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / f"{name}@2x.png"
    image.save(path, "PNG")
    (folder / "Contents.json").write_text(json.dumps({
        "images": [{"filename": f"{name}@2x.png", "idiom": "universal", "scale": "2x"}],
        **CONTENTS,
    }, indent=2) + "\n")
    print(f"  {path.relative_to(ROOT)}")


def interior(name, inset=8):
    """The window's inside, `inset` points in from its edges: the tour card rounds its own
    corners, so a picture that still showed the window's corners would draw a second,
    tighter corner inside them.

    The app writes <name>.json beside each still with the still's rect and the window's
    place in it (top-left origin, points), and that is what is cut: a still framed
    around an open popover is wider than the window, so the window's size alone put
    the box in the wrong place. Older runs without the sidecar fall back to the size
    table, which is exact only for window-only stills."""
    from PIL import Image
    image = Image.open(CAPTURES / f"{name}.png").convert("RGB")
    sidecar = CAPTURES / f"{name}.json"
    if sidecar.exists():
        frames = json.loads(sidecar.read_text())
        _, _, rect_w, rect_h = frames["rect"]
        x, y, w, h = frames["window"]
        scale = image.width / rect_w
    else:
        rw, rh, x, y, w, h = window_box(name)
        scale = 2
    box = (round(scale * (x + inset)), round(scale * (y + inset)), round(scale * (x + w - inset)), round(scale * (y + h - inset)))
    return image.crop(box)


def focus_box(inner, focus, aspect=WIDTH / HEIGHT, min_share=0.55):
    """The 16:10 box (left, top, right, bottom) in points a page is cut to, inside the
    window's inside `inner`, around `focus` (x, y, w, h): the focus padded, grown to the
    aspect about its centre, never narrower than `min_share` of the inside's width, and
    shifted (never shrunk) to lie inside. With no focus, or a focus the inside cannot
    hold, the whole inside."""
    il, it, ir, ib = inner
    iw, ih = ir - il, ib - it
    if focus is None:
        return inner
    fx, fy, fw, fh = focus
    pad = max(40, 0.12 * max(fw, fh))
    w, h = fw + 2 * pad, fh + 2 * pad
    cx, cy = fx + fw / 2, fy + fh / 2
    if w / h < aspect:
        w = h * aspect
    else:
        h = w / aspect
    if w < iw * min_share:
        w = iw * min_share
        h = w / aspect
    if w > iw or h > ih:
        return inner
    left, top = cx - w / 2, cy - h / 2
    left = min(max(left, il), ir - w)
    top = min(max(top, it), ib - h)
    return (left, top, left + w, top + h)


def frame(name, inset=8):
    """The tour page's picture: the window's inside, zoomed on the still's focus when the
    sidecar names one (see `focus_box`); `interior` otherwise."""
    from PIL import Image
    sidecar = CAPTURES / f"{name}.json"
    if not sidecar.exists():
        return interior(name, inset)
    image = Image.open(CAPTURES / f"{name}.png").convert("RGB")
    frames = json.loads(sidecar.read_text())
    _, _, rect_w, rect_h = frames["rect"]
    x, y, w, h = frames["window"]
    scale = image.width / rect_w
    inner = (x + inset, y + inset, x + w - inset, y + h - inset)
    focus = frames.get("focus")
    box = focus_box(inner, focus)
    if focus and box == inner:
        # A focus the window cannot hold (the cookbook's popover is taller than the
        # window it hangs from): cut around it within the whole still instead, so the
        # page shows the popover entire beside the window rather than clipped at its edge.
        box = focus_box((0, 0, rect_w, rect_h), focus)
    print(f"  {name}: {'focus' if focus and box != inner else 'whole window'} {tuple(round(v) for v in box)}")
    return image.crop(tuple(round(scale * v) for v in box))


def theme_mosaic():
    """The four theme windows' insides, two by two with a thin dark seam, for the tour."""
    from PIL import Image
    tiles = [interior(f"tour-theme-{theme}", inset=8) for theme in THEME_ORDER]
    gap = 6
    tile_w = (WIDTH - gap) // 2
    tile_h = (HEIGHT - gap) // 2
    mosaic = Image.new("RGB", (tile_w * 2 + gap, tile_h * 2 + gap), (16, 16, 20))
    for index, tile in enumerate(tiles):
        mosaic.paste(fit(tile, tile_w, tile_h), ((index % 2) * (tile_w + gap), (index // 2) * (tile_h + gap)))
    return mosaic


def encode():
    from PIL import Image, ImageDraw
    step("Importing")
    # The welcome page is its own scene: the whole window with the sidebar open, no
    # popover, so the first page is the overview and not the hero's close-up.
    for name, source in [("tour-welcome", "tour-welcome"), ("tour-hydra", "tour-hydra"), ("tour-pairs", "tour-pairs"),
                         ("tour-slider", "tour-slider"), ("tour-threads", "tour-threads"), ("tour-recipes", "tour-recipes"), ("tour-panels", "tour-panels")]:
        if not (CAPTURES / f"{source}.png").exists():
            sys.exit(f"The capture run wrote no {source}.png; see {CAPTURES / 'run.log'}")
        write_imageset(name, fit(frame(source), WIDTH, HEIGHT))
    write_imageset("tour-themes", fit(theme_mosaic(), WIDTH, HEIGHT))


# The website's set used to be cut and uploaded from here: cropped-on-the-action
# WebP files into website/assets/app/tour/, pushed to the same R2 tour/v4 prefix
# website_captures.py writes. That path is superseded and removed, so only one
# uploader writes those keys: --website/--export-only/--upload delegate to
# website_captures.py, which encodes every still whole (popovers uncropped) into
# build.noindex/website-tour and verifies each key serves byte-identical.
def _tour_pipeline():
    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
    import website_captures
    return website_captures


def website():
    _tour_pipeline().encode_tour()


def upload():
    _tour_pipeline().upload_tour()


if __name__ == "__main__":
    args = sys.argv[1:]
    if "--export-only" not in args:
        build()
        capture()
        encode()
        print(f"\nReady: {ASSETS}")
    if "--website" in args or "--export-only" in args or "--upload" in args:
        website()
    if "--upload" in args:
        upload()
