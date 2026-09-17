#!/usr/bin/env python3
"""Renders the marketing site's app captures from the real app, then cuts and encodes them.

    scripts/website_captures.py            # build, capture the tour run, encode the stills
    scripts/website_captures.py --encode   # encode again from the last tour run
    scripts/website_captures.py --upload   # push the encoded stills to R2 (with either of the above)

The app runs once in its tour capture mode (see DroppyCode/Support/TourCaptures.swift):
it opens the real window over gradient backdrops with mock data, photographs the tour
scenes (tour-*.png) and the site's composer scenes (web-*.png), and quits. This script
then encodes every still as WebP at the site's sizes into build.noindex/website-tour
(which mirrors the R2 tour/v5 prefix one to one) and uploads the set with immutable cache
headers, verifying each key afterwards. The site loads these stills straight from R2,
so nothing under website/ is touched.

The older desktop-wallpaper run (see DroppyCode/Support/WebsiteCaptures.swift) is still
available behind --website: it captures films and stills over this Mac's wallpaper into
build.noindex/website-captures and encodes them into website/assets/app. Needs ffmpeg
(brew), Pillow (pip) and, for --upload, wrangler signed in to the Droppy Cloudflare
account.
"""

import json
import pathlib
import shutil
import subprocess
import sys
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
CAPTURES = ROOT / "build.noindex" / "website-captures"
DERIVED = ROOT / "build.noindex" / "website"
APP = DERIVED / "Build/Products/Debug/Droppy Code Dev.app"
OUT = ROOT / "website" / "assets" / "app"
FFMPEG = shutil.which("ffmpeg") or "/opt/homebrew/bin/ffmpeg"
FFPROBE = shutil.which("ffprobe") or "/opt/homebrew/bin/ffprobe"
R2_BUCKET = "droppy-releases"
R2_PREFIX = "site-assets/droppy-code"
SITE_ORIGIN = "https://droppy-releases.jordylegrand.workers.dev"

# The tour run's output: stills photographed over gradient backdrops, already 16:10,
# at the display's scale (2x). Encoded one to one into TOUR_OUT, which mirrors the R2
# tour/v5 prefix, so the site serves them straight from R2 and nothing under website/ is
# touched.
TOUR_CAPTURES = ROOT / "build.noindex" / "tour-captures"
TOUR_OUT = ROOT / "build.noindex" / "website-tour"
R2_TOUR_PREFIX = "site-assets/droppy-code/tour/v5"
TOUR_STILLS = {
    # name: (source still from the tour run, output width)
    "hero": ("web-hero", 2400),
    "welcome": ("tour-welcome", 2080),
    "hydra": ("tour-hydra", 2080),
    "pairs": ("tour-pairs", 2080),
    "slider": ("tour-slider", 2080),
    "panels": ("tour-panels", 2080),
    "threads": ("tour-threads", 2080),
    "recipes": ("tour-recipes", 2080),
    "window": ("tour-window", 2080),
    "diff": ("web-diff", 2080),
    "palette": ("web-palette", 2080),
    "plans": ("web-plans", 2080),
    "question": ("web-question", 2080),
    "queue": ("web-queue", 2080),
    "slash": ("web-slash", 2080),
    "quote": ("web-quote", 2080),
    "notify": ("web-notify", 2080),
    "intro": ("web-intro", 2080),
}
# The four themes whose quadrants tile into one seamless themes still, in tile order:
# top-left, top-right, bottom-left, bottom-right (see TourCaptures.run).
TOUR_THEME_QUADRANTS = ["tokyoNight", "gruvbox", "catppuccinLatte", "rosePine"]
TOUR_THEMES_WIDTH = 2080

# Every capture is 2x with a 72 point (144 px) wallpaper margin on every side. The wide
# window is 1320x860 points, so its captures are 2928x2008; the narrow one is 860x560 points,
# 2008x1408. A width here only ever caps, never scales up.
MARGIN = 144
FILMS = {
    # name: (master, output width, hevc bitrate)
    "hero": ("editing", 2400, "4500k"),
    "question": ("question", 2080, "3500k"),
    "queue": ("queue", 2080, "3500k"),
    "slider": ("slider", 2008, "3000k"),
}
STILLS = {
    # name: (source still, crop, output width)
    "hero": ("editing", None, 2400),
    "diff": ("diff", None, 2080),
    "palette": ("palette", None, 2080),
    "switcher": ("switcher", None, 2008),
    "sidebar": ("editing", (0, MARGIN, MARGIN + 540, MARGIN + 1720), 684),
    "plans": ("plans", "pane", 2080),
}
THEME_WIDTH = 2080


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
    step("Capturing the scenes")
    shutil.rmtree(CAPTURES, ignore_errors=True)
    CAPTURES.mkdir(parents=True)
    # Through LaunchServices, not the binary: an app launched from a shell is refused activation,
    # and the captures need the key window of the active app, traffic lights lit. Not waited on:
    # the run is force-killed at its budget, so a stall can never leave a recording running.
    marker = "website-captures " + str(CAPTURES)
    subprocess.run(["open", "-n", str(APP), "--args", "--website-captures", str(CAPTURES)], check=True)
    wait_for_run(marker, 120)
    log = CAPTURES / "run.log"
    if log.exists():
        print("  " + log.read_text().strip().splitlines()[-1])
    for master, _, _ in FILMS.values():
        film = CAPTURES / "films" / f"{master}.mov"
        if not film.exists() or film.stat().st_size == 0:
            sys.exit(f"The capture run wrote no {master} film; see {log}")


def wait_for_run(marker, budget):
    """Waits for a capture run to quit on its own, killing it at its budget so a stall
    can never leave a recording running. The app takes a moment to appear through
    LaunchServices, so the run is first waited into existence."""
    # A fresh bundle's first launch can take a minute to clear LaunchServices.
    for _ in range(60):
        time.sleep(2)
        if subprocess.run(["pgrep", "-f", "--", marker], capture_output=True, text=True).stdout.strip():
            break
    else:
        sys.exit(f"the run never launched ({marker})")
    deadline = time.time() + budget
    while time.time() < deadline:
        time.sleep(2)
        alive = subprocess.run(["pgrep", "-f", "--", marker], capture_output=True, text=True).stdout.strip()
        if not alive:
            return
    subprocess.run(["pkill", "-9", "-f", "--", marker])
    print("  the run overran its budget and was killed")


def capture_tour():
    step("Capturing the tour and web stills")
    shutil.rmtree(TOUR_CAPTURES, ignore_errors=True)
    TOUR_CAPTURES.mkdir(parents=True)
    # Through LaunchServices, not the binary: an app launched from a shell is refused
    # activation, and the captures need the key window of the active app, traffic lights
    # lit. Not waited on: the run is force-killed at its budget, so a stall can never
    # leave a recording running.
    marker = "--tour-captures " + str(TOUR_CAPTURES)
    subprocess.run(["open", "-n", str(APP), "--args", "--tour-captures", str(TOUR_CAPTURES)], check=True)
    wait_for_run(marker, 330)
    log = TOUR_CAPTURES / "run.log"
    if log.exists():
        print("  " + log.read_text().strip().splitlines()[-1])
    missing = [f"{source}.png" for source, _ in TOUR_STILLS.values()
               if not (TOUR_CAPTURES / f"{source}.png").exists()]
    missing += [f"tour-theme-{name}.png" for name in TOUR_THEME_QUADRANTS
                if not (TOUR_CAPTURES / f"tour-theme-{name}.png").exists()]
    if missing:
        sys.exit(f"The tour run wrote no {', '.join(missing)}; see {log}")


def tour_still(name, source, width, quality=90):
    from PIL import Image
    image = Image.open(source).convert("RGB")
    if image.width > width:
        image = image.resize((width, round(image.height * width / image.width)), Image.LANCZOS)
    TOUR_OUT.mkdir(parents=True, exist_ok=True)
    image.save(TOUR_OUT / f"{name}.webp", "WEBP", quality=quality, method=6)
    return image.size


def tour_themes():
    """Tiles the four theme quadrants into one seamless themes still, the way the tour
    stages them: each still shows one quarter of a double-size gradient."""
    from PIL import Image
    tiles = [Image.open(TOUR_CAPTURES / f"tour-theme-{name}.png").convert("RGB")
             for name in TOUR_THEME_QUADRANTS]
    cell = (min(tile.width for tile in tiles), min(tile.height for tile in tiles))
    tiles = [tile if tile.size == cell else tile.resize(cell, Image.LANCZOS) for tile in tiles]
    sheet = Image.new("RGB", (cell[0] * 2, cell[1] * 2))
    sheet.paste(tiles[0], (0, 0))
    sheet.paste(tiles[1], (cell[0], 0))
    sheet.paste(tiles[2], (0, cell[1]))
    sheet.paste(tiles[3], (cell[0], cell[1]))
    if sheet.width > TOUR_THEMES_WIDTH:
        sheet = sheet.resize(
            (TOUR_THEMES_WIDTH, round(sheet.height * TOUR_THEMES_WIDTH / sheet.width)), Image.LANCZOS)
    TOUR_OUT.mkdir(parents=True, exist_ok=True)
    sheet.save(TOUR_OUT / "themes.webp", "WEBP", quality=86, method=6)
    return sheet.size


def encode_tour():
    step("Encoding the tour and web stills")
    TOUR_OUT.mkdir(parents=True, exist_ok=True)
    for name, (source, width) in TOUR_STILLS.items():
        path = TOUR_CAPTURES / f"{source}.png"
        if not path.exists():
            sys.exit(f"No {path.name} from the last tour run; rerun without --encode")
        size = tour_still(name, path, width)
        print(f"  {name}.webp {size[0]}x{size[1]}  {(TOUR_OUT / f'{name}.webp').stat().st_size // 1024} KB")
    (TOUR_OUT / "themes").mkdir(parents=True, exist_ok=True)
    for path in sorted(TOUR_CAPTURES.glob("web-theme-*.png")):
        theme_id = path.stem.removeprefix("web-theme-")
        size = tour_still(f"themes/{theme_id}", path, 2080, quality=86)
        print(f"  themes/{theme_id}.webp {size[0]}x{size[1]}  {(TOUR_OUT / 'themes' / f'{theme_id}.webp').stat().st_size // 1024} KB")
    # The sidebar tile: the floating list itself, cut from the threads still around its
    # popover (the sidecar's focus: the popover with the button it hangs from, in still
    # points, top-left origin; the still is 2x).
    threads_path = TOUR_CAPTURES / "tour-threads.png"
    sidecar_path = TOUR_CAPTURES / "tour-threads.json"
    if not threads_path.exists() or not sidecar_path.exists():
        sys.exit(f"No tour-threads still and sidecar from the last tour run; rerun without --encode")
    from PIL import Image
    threads = Image.open(threads_path).convert("RGB")
    frames = json.loads(sidecar_path.read_text())
    scale = threads.width / frames["rect"][2]
    fx, fy, fw, fh = frames["focus"]
    box = tuple(round(scale * v) for v in (fx, fy, fx + fw, fy + fh))
    sidebar = threads.crop(box)
    sidebar.save(TOUR_OUT / "sidebar.webp", "WEBP", quality=86, method=6)
    print(f"  sidebar.webp {sidebar.width}x{sidebar.height}  {(TOUR_OUT / 'sidebar.webp').stat().st_size // 1024} KB")
    for name in TOUR_THEME_QUADRANTS:
        path = TOUR_CAPTURES / f"tour-theme-{name}.png"
        if not path.exists():
            sys.exit(f"No {path.name} from the last tour run; rerun without --encode")
    size = tour_themes()
    print(f"  themes.webp {size[0]}x{size[1]}  {(TOUR_OUT / 'themes.webp').stat().st_size // 1024} KB")


def verify_key(key, local):
    """The uploaded key serves from the site's origin, byte-identical to the still."""
    url = f"{SITE_ORIGIN}/{key}"
    request = urllib.request.Request(url, headers={"User-Agent": "DroppyCode-website-captures"})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            remote = response.read()
    except Exception as error:
        sys.exit(f"  {key}: uploaded but does not serve ({url}: {error})")
    if remote != local.read_bytes():
        sys.exit(f"  {key}: serves {len(remote)} bytes but the still is {local.stat().st_size}")
    print(f"  {key} verified ({len(remote) // 1024} KB, identical)")


def upload_tour():
    step(f"Uploading to R2 ({R2_BUCKET}/{R2_TOUR_PREFIX})")
    paths = sorted(TOUR_OUT.rglob("*.webp"))
    if not paths:
        sys.exit(f"No .webp under {TOUR_OUT}; encode first")
    for local in paths:
        key = f"{R2_TOUR_PREFIX}/{local.relative_to(TOUR_OUT).as_posix()}"
        subprocess.run([
            "wrangler", "r2", "object", "put", f"{R2_BUCKET}/{key}", "--file", str(local),
            "--content-type", "image/webp", "--cache-control", "public, max-age=31536000, immutable", "--remote",
        ], check=True, capture_output=True)
        print(f"  {key}")
    step("Verifying every key serves")
    for local in paths:
        verify_key(f"{R2_TOUR_PREFIX}/{local.relative_to(TOUR_OUT).as_posix()}", local)


def probe(path):
    result = subprocess.run([
        FFPROBE, "-v", "error", "-select_streams", "v:0",
        "-show_entries", "stream=width,height,avg_frame_rate,color_space,color_transfer,color_primaries,color_range",
        "-of", "json", str(path),
    ], capture_output=True, text=True, check=True)
    return json.loads(result.stdout)["streams"][0]


def color_flags(stream):
    """The master's own colour tags, carried through so nothing shifts on the way to the web."""
    flags = []
    for key, flag in (("color_primaries", "-color_primaries"), ("color_transfer", "-color_trc"), ("color_space", "-colorspace")):
        value = stream.get(key)
        if value and value != "unknown":
            flags += [flag, value]
    return flags


def encode_film(name, master, width, hevc_bitrate):
    source = CAPTURES / "films" / f"{master}.mov"
    stream = probe(source)
    scale = f"scale='min({width},iw)':-2:flags=lanczos"
    common = ["-vf", scale, "-r", "60", "-fps_mode", "cfr", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-an"]
    common += color_flags(stream)
    subprocess.run([
        FFMPEG, "-y", "-loglevel", "error", "-i", str(source), *common,
        "-c:v", "libx264", "-preset", "slow", "-crf", "19", "-profile:v", "high", "-level", "5.1", "-g", "120",
        str(OUT / f"{name}.mp4"),
    ], check=True)
    subprocess.run([
        FFMPEG, "-y", "-loglevel", "error", "-i", str(source), *common,
        "-c:v", "hevc_videotoolbox", "-b:v", hevc_bitrate, "-tag:v", "hvc1", "-g", "120",
        str(OUT / f"{name}-hevc.mp4"),
    ], check=True)
    # The poster is the film's own first frame.
    poster = CAPTURES / "films" / f"{master}-first.png"
    subprocess.run([FFMPEG, "-y", "-loglevel", "error", "-i", str(source), "-frames:v", "1", str(poster)], check=True)
    still(f"{name}-poster", poster, None, width)


def pane_crop(image, blank):
    """The chat pane, from just above its first content down to the window's bottom edge, for a
    scene staged on an otherwise empty thread. Found by comparing against the empty thread."""
    from PIL import ImageChops
    pane = (MARGIN + 540, MARGIN + 100, MARGIN + 2620, MARGIN + 1480)
    diff = ImageChops.difference(image.crop(pane), blank.crop(pane)).convert("L").point(lambda v: 255 if v > 18 else 0)
    box = diff.getbbox()
    top = pane[1] + (box[1] if box else 0) - 40
    return (MARGIN + 540, max(MARGIN, top), MARGIN + 2620, MARGIN + 1720)


def still(name, source, crop, width, quality=92):
    from PIL import Image
    image = Image.open(source).convert("RGB")
    if crop == "pane":
        crop = pane_crop(image, Image.open(CAPTURES / "blank-wide.png").convert("RGB"))
    if crop:
        image = image.crop(crop)
    if image.width > width:
        image = image.resize((width, round(image.height * width / image.width)), Image.LANCZOS)
    image.save(OUT / f"{name}.webp", "WEBP", quality=quality, method=6)
    return image.size


def encode():
    step("Encoding")
    OUT.mkdir(parents=True, exist_ok=True)
    for name, (master, width, bitrate) in FILMS.items():
        encode_film(name, master, width, bitrate)
        size = probe(OUT / f"{name}.mp4")
        print(f"  {name}.mp4 {size['width']}x{size['height']}  "
              f"{(OUT / f'{name}.mp4').stat().st_size // 1024} KB  + hevc {(OUT / f'{name}-hevc.mp4').stat().st_size // 1024} KB")
    for name, (source, crop, width) in STILLS.items():
        size = still(name, CAPTURES / f"{source}.png", crop, width)
        print(f"  {name}.webp {size[0]}x{size[1]}  {(OUT / f'{name}.webp').stat().st_size // 1024} KB")
    themes = OUT / "themes"
    themes.mkdir(exist_ok=True)
    for source in sorted(CAPTURES.glob("theme-*.png")):
        name = source.stem.removeprefix("theme-")
        still(f"themes/{name}", source, None, THEME_WIDTH, quality=86)
    print(f"  themes/  {len(list(themes.glob('*.webp')))} stills")


def upload():
    step(f"Uploading to R2 ({R2_BUCKET}/{R2_PREFIX})")
    for path in sorted(OUT.rglob("*")):
        if not path.is_file():
            continue
        key = f"{R2_PREFIX}/{path.relative_to(OUT)}"
        content_type = {"mp4": "video/mp4", "webp": "image/webp", "png": "image/png"}.get(path.suffix[1:], "application/octet-stream")
        subprocess.run([
            "wrangler", "r2", "object", "put", f"{R2_BUCKET}/{key}", "--file", str(path),
            "--content-type", content_type, "--cache-control", "public, max-age=31536000, immutable", "--remote",
        ], check=True, capture_output=True)
        print(f"  {key}")


if __name__ == "__main__":
    website = "--website" in sys.argv
    if "--encode" not in sys.argv and "--upload-only" not in sys.argv:
        build()
        if website:
            capture()
        capture_tour()
    if "--upload-only" not in sys.argv:
        if website:
            encode()
        encode_tour()
    if "--upload" in sys.argv or "--upload-only" in sys.argv:
        if website:
            upload()
        upload_tour()
    print(f"\nReady: {TOUR_OUT}")
