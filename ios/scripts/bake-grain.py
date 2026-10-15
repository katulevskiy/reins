#!/usr/bin/env python3
"""Bakes the field grain (design/signal/assets/textures/grain.png) into ios/Shared/Signal.xcassets.

- `grain`: for the app, drawn tiled with a soft-light blend. Soft light is linear in the blend colour on each side of
  mid-grey, so the texture's grey at alpha `a` blends exactly like an opaque grey of 0.5 + a * (grey - 0.5): the tile
  becomes one grey channel, which is smaller and draws without alpha. Greys are rounded to steps of 3, which soft light
  makes invisible.
- `grain-widget`: for widgets and Live Activities, which can't blend: white specks where the grain lightens, black
  where it darkens, at about 6 % opacity on average, drawn with a normal blend.

Both are 360 px @2x, so they tile at 180 pt. Needs numpy and Pillow.

    python3 ios/scripts/bake-grain.py
"""

import json
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent.parent
SOURCE = ROOT / "design" / "signal" / "assets" / "textures" / "grain.png"
CATALOG = ROOT / "ios" / "Shared" / "Signal.xcassets"


def image_set(name: str, image: Image.Image) -> None:
    folder = CATALOG / f"{name}.imageset"
    folder.mkdir(parents=True, exist_ok=True)
    image.save(folder / f"{name}@2x.png", optimize=True)
    contents = {
        "images": [
            {"idiom": "universal", "scale": "1x"},
            {"filename": f"{name}@2x.png", "idiom": "universal", "scale": "2x"},
            {"idiom": "universal", "scale": "3x"},
        ],
        "info": {"author": "xcode", "version": 1},
    }
    (folder / "Contents.json").write_text(json.dumps(contents, indent=2) + "\n")


def main() -> None:
    rgba = np.asarray(Image.open(SOURCE).convert("RGBA")).astype(float) / 255
    grey, alpha = rgba[..., 0], rgba[..., 3]
    flat = 0.5 + alpha * (grey - 0.5)
    levels = np.clip(np.round((flat * 255 - 128) / 3) * 3 + 128, 0, 255).astype(np.uint8)
    image_set("grain", Image.fromarray(levels, "L"))

    deviation = flat - 0.5
    speck = np.round(np.clip(np.abs(deviation) * (0.06 / np.abs(deviation).mean()), 0, 1) * 15) / 15
    widget = np.dstack([np.where(deviation > 0, 255, 0), np.round(speck * 255)]).astype(np.uint8)
    image_set("grain-widget", Image.fromarray(widget, "LA"))
    print(f"grain and grain-widget into {CATALOG.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
