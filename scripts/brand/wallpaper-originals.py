#!/usr/bin/env python3
"""Makes the untreated originals of the wallpapers that ship treated (Halo, Crescent, Figure, Ruins).

The shipped `halo.jpg`, `crescent.jpg`, `figure.jpg` and `ruins.jpg` have their effect baked in (the Next boards'
`source/art/fx.py`). They stay as they are: each is its image with its default effect. Every other effect, and
Original, is drawn from `<image>-original.jpg`, which this makes from the owner's full-size art with the boards' own
framing (`fx.py`'s `load(cx, cy, zoom)`, cropped to 780 x 1000) and the boards' fade into the dark paper (`fade(start,
0.97)`), as the other shipped originals (Eyes, Hands, Torii) were made. JPEG quality 88, as theirs.

Needs Pillow and NumPy. Run it from outside the repo with the folder holding the owner's art:

    python3 -I scripts/brand/wallpaper-originals.py ~/Documents/dev/reins-designs/wallpapers/originals \
        android/app/src/main/assets/wallpapers ios/Reins/Assets.xcassets
"""

import sys
from pathlib import Path

import numpy as np
from PIL import Image

W, H = 780, 1000
DARK = (18, 18, 21)

# image: (file in the owner's folder, fx.py's load(cx, cy, zoom), fade start), as fx.py made the shipped looks.
ORIGINALS = {
    "halo": ("image4.png", (0.25, 0.45, 1.15), 0.45),
    "crescent": ("image2.png", (0.5, 0.45, 1.0), 0.5),
    "figure": ("image1.webp", (0.45, 0.4, 1.15), 0.5),
    "ruins": ("image3.png", (0.5, 0.45, 1.0), 0.5),
}

CONTENTS = """{
  "images" : [
    { "idiom" : "universal", "scale" : "1x" },
    { "filename" : "FILE", "idiom" : "universal", "scale" : "2x" },
    { "idiom" : "universal", "scale" : "3x" }
  ],
  "info" : { "author" : "xcode", "version" : 1 }
}
"""


def crop_box(size, cx, cy, zoom):
    """fx.py's `load`: a 780:1000 window `zoom` times into the height, centred on (cx, cy), kept inside the image."""
    iw, ih = size
    ch = ih / zoom
    cw = ch * W / H
    if cw > iw:
        cw = iw / zoom
        ch = cw * H / W
    x0 = max(0, min(iw - cw, cx * iw - cw / 2))
    y0 = max(0, min(ih - ch, cy * ih - ch / 2))
    return int(x0), int(y0), int(x0 + cw), int(y0 + ch)


def fade(im, bg, start, end=0.97):
    """fx.py's `fade`: into `bg` from `start` to `end` of the height, smoothstep."""
    a = np.asarray(im).astype(float)
    y = np.linspace(0, 1, H)[:, None, None]
    t = np.clip((y - start) / (end - start), 0, 1)
    t = t * t * (3 - 2 * t)
    return Image.fromarray((a * (1 - t) + np.array(bg, float) * t).astype("uint8"))


def main() -> None:
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    art, assets, catalog = (Path(p) for p in sys.argv[1:])
    for name, (file, (cx, cy, zoom), start) in ORIGINALS.items():
        src = Image.open(art / file).convert("RGB")
        box = crop_box(src.size, cx, cy, zoom)
        out = fade(src.crop(box).resize((W, H), Image.LANCZOS), DARK, start)
        target = assets / f"{name}-original.jpg"
        out.save(target, quality=88)
        imageset = catalog / f"wallpaper-{name}-original.imageset"
        imageset.mkdir(exist_ok=True)
        (imageset / f"wallpaper-{name}-original@2x.jpg").write_bytes(target.read_bytes())
        (imageset / "Contents.json").write_text(CONTENTS.replace("FILE", f"wallpaper-{name}-original@2x.jpg"))
        print(f"{name}: {file} {src.size[0]}x{src.size[1]}, crop {box}, fade from {start}: {target.stat().st_size} bytes")


if __name__ == "__main__":
    main()
