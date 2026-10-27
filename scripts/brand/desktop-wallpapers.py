#!/usr/bin/env python3
"""Makes the desktop's landscape wallpapers (`crates/reins-desktop-app/assets/wallpapers`) from the owner's full-size art.

The phones ship 780 x 1000 portrait art. The desktop covers a wide window with its art (a 360 px band across Home and
Activity, the whole window on the welcome), so it ships landscape versions, 3200 x 1800 (16:9), made from the
owner's full-size art. Each is drawn the way the Next boards' `source/art/fx.py` drew the phones' version:

- the same framing: `load(cx, cy, zoom)`, but with a 16:9 window in place of 780:1000, so the art spans the same
  height of the source and stays centred on the same point (moved in only where the source ends);
- the same effect with the same numbers, every size (dither cell, scanline pitch and shift, halftone pitch) scaled by
  the art's pixel density: 1800 rows here span what 1000 rows span on the phones, so sizes are 1.8 times theirs and
  the effect is as fine against the picture as on the boards. (Scaled by the width instead, 3200 / 780, a cell would
  be as big on screen as on the boards' 1280 px window, but over a picture shown at half the size: Figure's halftone
  turns into a few dozen dots.)
- the same fade into the paper at the foot (`fade(start, 0.97)`).

The dithered and halftone art is PNG: lossless, so every cell stays sharp, and a fraction of a JPEG's size (rows
repeat cell by cell, which PNG's filters take for free). The photos (Crescent and Eyes under scanlines, Torii) are JPEG at quality 85.

Halo (Mono dither), Crescent (scanlines), Figure (halftone) and Ruins (colour dither) keep the boards' framing. Eyes
(scanlines), Torii (the photo, and Torii dither in the light Mono) and Hands (its own dot art, no effect) are framed
for the desktop's band, which shows about the top half of the art (`object-position: 50% 30%`): Eyes and Torii 1.2
times into the height and as low as the source goes, so the eyes and the whole gate sit in it; Hands across its full
width on black, the hands at a third of the height. Torii is moved left a little, so the source's watermark at the
bottom right stays out. Hands' artist credit (bottom right) stays in, under the foot's fade.

Ruins is only 1586 x 992. Its source here is that image upscaled 4x by Real-ESRGAN (`realesrgan-x4plus`, saved at 2x:
`image3-realesrgan-x4plus.png`), which keeps its painted edges crisper than Lanczos; without that file, Lanczos and a
light unsharp mask. Either way it is dithered in 5 px cells.

Needs Pillow and NumPy. Run it from the repository's root:

    python3 -I scripts/brand/desktop-wallpapers.py ~/Documents/dev/reins-designs/wallpapers/originals \\
        crates/reins-desktop-app/assets/wallpapers

The originals folder holds the owner's art: image4.png (Halo), image2.png (Crescent), image1.webp (Figure), image3.png
(Ruins, and its upscale), eyes.webp, torii.jpg, hands.webp.
"""

import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageEnhance, ImageFilter, ImageOps

W, H = 3200, 1800
# The art spans the same height of its source as the boards' 1000-row art: a size on the boards' art, here.
K = H / 1000
QUALITY = 85
# The art kept as JPEG: photos. The rest is cells of a few colours, smaller and exact as PNG.
JPEG = {"crescent", "eyes", "torii"}

DARK = (18, 18, 21)  # --paper dark #121215
LIGHT = (246, 246, 248)  # --paper light #f6f6f8

BAYER = (
    np.array(
        [
            [0, 32, 8, 40, 2, 34, 10, 42],
            [48, 16, 56, 24, 50, 18, 58, 26],
            [12, 44, 4, 36, 14, 46, 6, 38],
            [60, 28, 52, 20, 62, 30, 54, 22],
            [3, 35, 11, 43, 1, 33, 9, 41],
            [51, 19, 59, 27, 49, 17, 57, 25],
            [15, 47, 7, 39, 13, 45, 5, 37],
            [63, 31, 55, 23, 61, 29, 53, 21],
        ]
    )
    / 64.0
)


def px(n: float, least: int = 1) -> int:
    """A size on the boards' 780 x 1000 art, at this density."""
    return max(least, round(n * K))


def load(path: Path, cx: float, cy: float, zoom: float, sharpen: bool = False) -> Image.Image:
    """fx.py's `load` with a 16:9 window: `zoom` times into the height, centred on (cx, cy), kept inside the image."""
    im = Image.open(path).convert("RGB")
    iw, ih = im.size
    ch = ih / zoom
    cw = ch * W / H
    if cw > iw:
        cw = iw / zoom
        ch = cw * H / W
    x0 = max(0, min(iw - cw, cx * iw - cw / 2))
    y0 = max(0, min(ih - ch, cy * ih - ch / 2))
    crop = im.crop((round(x0), round(y0), round(x0 + cw), round(y0 + ch)))
    out = crop.resize((W, H), Image.LANCZOS)
    if sharpen and cw < W:
        out = out.filter(ImageFilter.UnsharpMask(radius=2, percent=60, threshold=2))
    print(f"  {path.name} {iw}x{ih}: {round(cw)}x{round(ch)} at ({round(x0)}, {round(y0)}), x{W / cw:.2f}")
    return out


def cells(cell: int) -> tuple[int, int]:
    """The grid of `cell`-px squares covering the art (the last row and column cut)."""
    return math.ceil(W / cell), math.ceil(H / cell)


def blow_up(small: Image.Image, cell: int) -> Image.Image:
    """Each pixel of `small` as a `cell`-px square, cut to the art."""
    return small.resize((small.width * cell, small.height * cell), Image.NEAREST).crop((0, 0, W, H))


def bayer(h: int, w: int) -> np.ndarray:
    return np.tile(BAYER, (h // 8 + 1, w // 8 + 1))[:h, :w]


def fade(im: Image.Image, bg, start: float, end: float = 0.97) -> Image.Image:
    """fx.py's `fade`: into `bg` from `start` to `end` of the height, smoothstep."""
    a = np.asarray(im).astype(float)
    y = np.linspace(0, 1, H)[:, None, None]
    t = np.clip((y - start) / (end - start), 0, 1)
    t = t * t * (3 - 2 * t)
    return Image.fromarray((a * (1 - t) + np.array(bg, float) * t).astype("uint8"))


def dither(im: Image.Image, cell: int, fg, bg, gamma: float = 1.0, contrast: float = 1.3) -> Image.Image:
    """fx.py's `dither`: ordered (Bayer) dither of the luma in `cell`-px squares, two colours."""
    small = im.resize(cells(cell), Image.LANCZOS).convert("L")
    small = ImageEnhance.Contrast(small).enhance(contrast)
    g = (np.asarray(small) / 255.0) ** gamma
    on = g > bayer(*g.shape)
    return blow_up(Image.fromarray(np.where(on[..., None], np.array(fg), np.array(bg)).astype("uint8")), cell)


def cdither(im: Image.Image, cell: int, levels: int = 3, bright: float = 1.0) -> Image.Image:
    """fx.py's `cdither`: each channel dithered on its own to `levels` levels, in `cell`-px squares."""
    small = ImageEnhance.Brightness(im.resize(cells(cell), Image.LANCZOS)).enhance(bright)
    a = np.asarray(small) / 255.0
    th = bayer(*a.shape[:2])[..., None]
    q = np.floor(a * (levels - 1) + th) / (levels - 1)
    return blow_up(Image.fromarray((q.clip(0, 1) * 255).astype("uint8")), cell)


def scanlines(im: Image.Image, period: int, dark: float, shift: int) -> Image.Image:
    """fx.py's `scanlines`: the lower half of every `period` rows at `dark`, red `shift` px right, blue left."""
    a = np.asarray(im).astype(float)
    m = np.where(np.arange(H) % period < period // 2, 1.0, dark)[:, None, None]
    a = a * m
    a = np.stack([np.roll(a[..., 0], shift, axis=1), a[..., 1], np.roll(a[..., 2], -shift, axis=1)], -1)
    return Image.fromarray(a.clip(0, 255).astype("uint8"))


def halftone(im: Image.Image, cell: int, fg=(236, 226, 206), bg=(18, 16, 14)) -> Image.Image:
    """fx.py's `halftone`: a dot per `cell`-px square, as big as the square is light; drawn at 2x, so edges are smooth."""
    cols, rows = cells(cell)
    g = np.asarray(ImageEnhance.Contrast(im.resize((cols, rows), Image.LANCZOS).convert("L")).enhance(1.4)) / 255.0
    s = 2
    out = Image.new("RGB", (W * s, H * s), bg)
    d = ImageDraw.Draw(out)
    for r in range(rows):
        for c in range(cols):
            rad = cell * 0.52 * g[r, c] ** 0.8
            if rad > 0.4 * K:
                x, y = (c * cell + cell / 2) * s, (r * cell + cell / 2) * s
                d.ellipse((x - rad * s, y - rad * s, x + rad * s, y + rad * s), fill=fg)
    return out.resize((W, H), Image.LANCZOS)


def on_black(path: Path, middle: float, at: float) -> Image.Image:
    """The whole width of `path` on black (fx.py's `fit_width`), its row at `middle` (a share of its height) placed at
    `at` of the art's height; what falls outside is cut, what is missing is black."""
    im = Image.open(path).convert("RGB")
    iw, ih = im.size
    h = round(ih * W / iw)
    top = round(at * H - middle * h)
    print(f"  {path.name} {iw}x{ih}: whole width, x{W / iw:.2f}, top at {top}")
    out = Image.new("RGB", (W, H), (0, 0, 0))
    out.paste(im.resize((W, h), Image.LANCZOS), (0, top))
    return out


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    art, out = Path(sys.argv[1]), Path(sys.argv[2])
    slate = (70, 70, 78)
    ruins = art / "image3-realesrgan-x4plus.png"
    if not ruins.exists():
        ruins = art / "image3.png"
    jobs = {
        # fx.py: fade(dither(load('halo', .25, .45, 1.15), 3, (232, 236, 255), (0, 0, 0), .9, 1.2), DARK, .45)
        "halo": lambda: fade(
            dither(load(art / "image4.png", 0.25, 0.45, 1.15), px(3), (232, 236, 255), (0, 0, 0), 0.9, 1.2), DARK, 0.45
        ),
        # fx.py: fade(scanlines(load('crescent', .5, .45, 1.0)), DARK, .5), scanlines(period=4, dark=.55), 2 px split
        "crescent": lambda: fade(scanlines(load(art / "image2.png", 0.5, 0.45, 1.0), px(4, 2), 0.55, px(2)), DARK, 0.5),
        # fx.py: fade(halftone(load('figure', .45, .4, 1.15)), DARK, .5), 9 px cells
        "figure": lambda: fade(halftone(load(art / "image1.webp", 0.45, 0.4, 1.15), px(9, 3)), DARK, 0.5),
        # fx.py ('forest'): fade(cdither(autocontrast(load(.5, .45, 1.0), cutoff=1), 3, 3, 1.15), DARK, .5)
        "ruins": lambda: fade(
            cdither(
                ImageOps.autocontrast(load(ruins, 0.5, 0.45, 1.0, sharpen=ruins.name == "image3.png"), cutoff=1),
                px(3),
                3,
                1.15,
            ),
            DARK,
            0.5,
        ),
        # fx.py: fade(scanlines(load('eyes', .5, .45, 1.1), 3, .6), DARK, .5); framed lower, for the band
        "eyes": lambda: fade(scanlines(load(art / "eyes.webp", 0.5, 1.0, 1.2), px(3, 2), 0.6, px(2)), DARK, 0.5),
        # fx.py: fade(load('torii', .5, .42, 1.0), LIGHT, .55); framed lower, for the band, and clear of the watermark
        "torii": lambda: fade(load(art / "torii.jpg", 0.47, 0.6, 1.2), LIGHT, 0.55),
        # fx.py: fade(dither(load('torii', .5, .45, 1.0), 2, LIGHT, (70, 70, 78), 1.0, 1.4), LIGHT, .5); framed as Torii
        "torii-dither": lambda: fade(
            dither(load(art / "torii.jpg", 0.47, 0.6, 1.2), px(2), LIGHT, slate, 1.0, 1.4), LIGHT, 0.5
        ),
        # fx.py: fade(fit_width('hands', 60, 1.5), DARK, .62, .98); the whole width here, the hands a third down
        "hands": lambda: fade(on_black(art / "hands.webp", 0.5, 0.36), DARK, 0.62, 0.98),
    }
    for name, make in jobs.items():
        print(name)
        im = make()
        if name in JPEG:
            target = out / f"{name}.jpg"
            im.save(target, quality=QUALITY, optimize=True)
        else:
            target = out / f"{name}.png"
            im.save(target, optimize=True)
        (out / f"{name}.{'png' if name in JPEG else 'jpg'}").unlink(missing_ok=True)
        print(f"  -> {target} {target.stat().st_size // 1024} KiB")


if __name__ == "__main__":
    main()
