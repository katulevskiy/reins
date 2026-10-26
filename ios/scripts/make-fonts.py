#!/usr/bin/env python3
"""Builds the app's static fonts from the variable OFL fonts in github.com/google/fonts (needs fontTools).

    ios/scripts/make-fonts.py <google-fonts>/ofl ios/Shared/Fonts

Sora 400/500/600, each with Geologica's Cyrillic merged in at the same weight (Sora has no Cyrillic; Latin stays
Sora), and JetBrains Mono 400/500. One font per weight keeps `Font.custom` and Dynamic Type working everywhere,
widgets included, where a cascade list would need a UIFont per call site.
"""
import sys, tempfile, os
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools.ttLib.scaleUpem import scale_upem
from fontTools import subset
from fontTools.merge import Merger

OFL, OUT = sys.argv[1], sys.argv[2]
CYRILLIC = [*range(0x0400, 0x0530), *range(0x1C80, 0x1C90), *range(0x2DE0, 0x2E00), *range(0xA640, 0xA6A0)]


def instance(path, weight):
    font = TTFont(path)
    axes = {a.axisTag: a.defaultValue for a in font["fvar"].axes}
    axes["wght"] = weight
    font = instancer.instantiateVariableFont(font, axes)
    for table in ("STAT", "DSIG", "meta"):
        if table in font:
            del font[table]
    return font


def name(font, family, style, weight):
    ps = family.replace(" ", "") + "-" + style
    full = family if style == "Regular" else f"{family} {style}"
    table = font["name"]
    table.names = [r for r in table.names if r.platformID == 3 and r.nameID not in (1, 2, 3, 4, 6, 16, 17, 21, 22, 25)]
    for nid, value in ((1, full), (2, "Regular"), (3, f"{ps};reins"), (4, full), (6, ps), (16, family), (17, style)):
        table.setName(value, nid, 3, 1, 0x409)
    font["OS/2"].usWeightClass = weight
    font["OS/2"].fsSelection = (font["OS/2"].fsSelection & ~0b1100001) | 0b1000000
    font["head"].macStyle = 0
    return ps


with tempfile.TemporaryDirectory() as tmp:
    for weight, style in ((400, "Regular"), (500, "Medium"), (600, "SemiBold")):
        sora = instance(f"{OFL}/sora/Sora[wght].ttf", weight)
        name(sora, "Sora", style, weight)
        cyr = instance(f"{OFL}/geologica/Geologica[CRSV,SHRP,slnt,wght].ttf", weight)
        scale_upem(cyr, sora["head"].unitsPerEm)
        opts = subset.Options()
        opts.layout_features = ["ccmp", "locl", "kern", "mark", "mkmk"]
        opts.notdef_outline = False
        opts.name_IDs = []
        sub = subset.Subsetter(opts)
        sub.populate(unicodes=CYRILLIC)
        sub.subset(cyr)
        a, b = os.path.join(tmp, "a.ttf"), os.path.join(tmp, "b.ttf")
        sora.save(a)
        cyr.save(b)
        merged = Merger().merge([a, b])
        merged["name"] = sora["name"]
        merged["OS/2"].usWeightClass = weight
        for k in ("ascent", "descent", "lineGap"):
            setattr(merged["hhea"], k, getattr(sora["hhea"], k))
        merged.save(f"{OUT}/Sora-{style}.ttf")
    for weight, style in ((400, "Regular"), (500, "Medium")):
        mono = instance(f"{OFL}/jetbrainsmono/JetBrainsMono[wght].ttf", weight)
        name(mono, "JetBrains Mono", style, weight)
        mono.save(f"{OUT}/JetBrainsMono-{style}.ttf")
print("fonts written to", OUT)
