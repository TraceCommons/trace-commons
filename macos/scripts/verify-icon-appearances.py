#!/usr/bin/env python3
"""Assert a compiled icon catalogue draws what its Icon Composer document says.

Usage: verify-icon-appearances.py <icon.json> <assetutil-info.json>

Reads the document and the JSON that `assetutil --info` prints for the
Assets.car actool made from it, and checks what the rest of the build cannot
see.

Why this exists
---------------

`verify-icon.swift` inspects the fallback `.icns`, which carries only the
light drawing -- the dark appearance exists solely inside `Assets.car`. And
asserting merely that a dark composition exists is not enough: an earlier
build of this icon produced three appearance compositions that all drew the
same thing while every structural check passed. Nothing downstream said so,
because the icon did render -- it rendered wrong.

So this asserts, from the compiled catalogue:

- each appearance (light, dark, tintable) has its own composition;
- the light composition's ground is the document's base fill and the dark
  one's is the document's dark fill, and the two differ;
- every layer the document declares was compiled, and nothing else was;
- the pre-rendered images macOS 14 and 15 draw are there at every size from
  16 to 1024, since only macOS 26 and later composite the layers live.
"""

import json
import os
import sys

# The compiler stores fill components rounded to three places.
TOLERANCE = 0.0015

LEGACY_SIZES = [16, 32, 64, 128, 256, 512, 1024]


def fail(message):
    print(f"FATAL: {message}", file=sys.stderr)
    sys.exit(1)


def document_fills(doc):
    fills = {}
    for entry in doc.get("fill-specializations", []):
        solid = entry.get("value", {}).get("solid", "")
        components = [float(c) for c in solid.partition(":")[2].split(",")]
        fills[entry.get("appearance", "base")] = components
    if "base" not in fills or "dark" not in fills:
        fail("the document has no base and dark fill; run verify-icon-document.py first")
    return fills


def document_layers(doc):
    names = set()
    for group in doc.get("groups", []):
        for layer in group.get("layers", []):
            if isinstance(layer.get("image-name"), str):
                names.add(layer["image-name"])
            for entry in layer.get("image-name-specializations", []):
                names.add(entry["value"])
    return {os.path.splitext(name)[0] for name in names}


def close(a, b):
    return len(a) == len(b) and all(abs(x - y) <= TOLERANCE for x, y in zip(a, b))


def main():
    if len(sys.argv) != 3:
        print("usage: verify-icon-appearances.py <icon.json> <assetutil-info.json>", file=sys.stderr)
        return 2

    with open(sys.argv[1]) as handle:
        doc = json.load(handle)
    with open(sys.argv[2]) as handle:
        catalogue = json.load(handle)

    fills = document_fills(doc)
    declared = document_layers(doc)

    stacks = {}
    compiled = {}
    legacy = None
    for entry in catalogue:
        kind = entry.get("AssetType")
        if kind == "IconImageStack":
            stacks[entry.get("Appearance")] = entry
        elif kind == "IconGroup":
            appearance = entry.get("Appearance")
            for layer in entry.get("Layers", []):
                # "AppIcon_Assets/near-n" -> "near-n"
                compiled.setdefault(appearance, set()).add(layer.get("Name", "").split("/")[-1])
        elif kind == "MultiSized Image":
            legacy = entry

    for appearance in ("NSAppearanceNameAqua", "NSAppearanceNameDarkAqua", "ISAppearanceTintable"):
        if appearance not in stacks:
            fail(f"the catalogue has no {appearance} composition")

    def ground(appearance):
        colours = [
            layer.get("Color components", [])
            for layer in stacks[appearance].get("Layers", [])
            if layer.get("AssetType") == "Color"
        ]
        if len(colours) != 1:
            fail(f"the {appearance} composition has {len(colours)} ground colours, expected 1")
        return colours[0]

    light_ground = ground("NSAppearanceNameAqua")
    dark_ground = ground("NSAppearanceNameDarkAqua")
    if not close(light_ground, fills["base"]):
        fail(f"the light ground is {light_ground}, the document's base fill is {fills['base']}")
    if not close(dark_ground, fills["dark"]):
        fail(
            f"the dark ground is {dark_ground}, the document's dark fill is {fills['dark']}.\n"
            "The dark fill-specialization was dropped or ignored."
        )
    if close(light_ground, dark_ground):
        fail("the light and dark compositions share one ground colour")

    for appearance in ("NSAppearanceNameAqua", "NSAppearanceNameDarkAqua"):
        layers = compiled.get(appearance, set())
        if layers != declared:
            fail(
                f"the {appearance} composition draws {sorted(layers)}, "
                f"the document declares {sorted(declared)}"
            )

    if legacy is None:
        fail("the catalogue has no pre-rendered icon for macOS 14 and 15")
    sizes = {int(size.split("x")[0]) for size in legacy.get("Sizes", [])}
    missing = [size for size in LEGACY_SIZES if size not in sizes]
    if missing:
        fail(f"the pre-rendered icon is missing sizes {missing}")

    print(
        f"  light and dark grounds match the document; layers {sorted(declared)}; "
        f"pre-rendered at {sorted(sizes)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
