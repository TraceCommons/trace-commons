#!/usr/bin/env python3
"""Assert a compiled icon catalogue draws distinct artwork in dark mode.

Reads the JSON that `assetutil --info` prints for an Assets.car and checks the
one property the rest of the build cannot see.

Why this exists as its own check
--------------------------------

`verify-icon.swift` inspects the fallback `.icns`, which carries only the
light drawing -- the dark artwork exists solely inside `Assets.car`. And
asserting merely that a dark composition exists is not enough: before the
`image-name-specializations` shape was understood, this build produced three
appearance compositions that all referenced one vector, so the icon rendered
the light inks in dark mode while every structural check passed. Nothing
downstream said so, because the icon did render -- it rendered wrong.

So this asserts both halves: the dark appearance draws the dark glyph, and its
artwork digest differs from the light one.
"""

import json
import sys


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: verify-icon-appearances.py <assetutil-info.json>", file=sys.stderr)
        return 2

    with open(sys.argv[1]) as handle:
        catalogue = json.load(handle)

    layers = {}
    for entry in catalogue:
        if entry.get("AssetType") != "IconGroup":
            continue
        for layer in entry.get("Layers", []):
            layers[entry.get("Appearance")] = (layer.get("Name"), layer.get("SHA1Digest"))

    dark = layers.get("NSAppearanceNameDarkAqua")
    light = layers.get("NSAppearanceNameAqua")
    if dark is None or light is None:
        print(
            "FATAL: the catalogue has no light and dark icon groups to compare.",
            file=sys.stderr,
        )
        return 1
    if not dark[0].endswith("mark-glyph-dark"):
        print(
            f"FATAL: the dark appearance draws {dark[0]}, not the dark glyph.\n"
            "The image-name-specializations entry was dropped or ignored, and\n"
            "dark mode would show the light palette.",
            file=sys.stderr,
        )
        return 1
    if dark[1] == light[1]:
        print(
            "FATAL: the dark and light appearances share one artwork digest.\n"
            "Dark mode would show the light palette.",
            file=sys.stderr,
        )
        return 1

    print(f"  dark appearance draws {dark[0].split('/')[-1]}, distinct from light")
    return 0


if __name__ == "__main__":
    sys.exit(main())
