#!/usr/bin/env python3
"""Check an Icon Composer document before actool compiles it.

Usage: verify-icon-document.py <AppIcon.icon> <production|staging>

Why this exists
---------------

actool validates nothing in icon.json. An asset name with a typo, a key with
the wrong JSON type, or a dark fill that was never declared all compile
without a warning, and the result is a different icon rather than a failed
build. So the document is read here first, against what the shipped icon is
meant to be:

- every layer names a file in Assets, and every file in Assets is a layer,
  so an edit to a file the document no longer uses cannot pass for a change;
- the background is an Icon Composer fill with a base and a dark entry that
  differ: Apple's guidance and the brand book both want a coloured ground in
  dark mode, and without the entry the system substitutes its own;
- no layer carries text or embedded raster art, which Icon Composer rejects
  or renders soft;
- the N layer is the kit's N, and only a staging document carries the
  staging ribbon, as its top group so the N never covers it.
"""

import json
import os
import sys

# The first and last commands of the kit's N path
# (Tertiary/01_Color/tile-color.svg). A redrawn or truncated N fails here.
N_PATH_START = 'd="M292.106 82.1357C283.445 82.1357 275.379 86.5848 270.847 93.9047'
N_PATH_END = 'C317.034 93.2012 305.876 82.1548 292.106 82.1548V82.1357Z"'

N_LAYER = "near-n.svg"
RIBBON_LAYER = "staging-ribbon.svg"

# Icon Composer allows at most four groups on a square icon.
MAX_GROUPS = 4


def fail(message):
    print(f"FATAL: {message}", file=sys.stderr)
    sys.exit(1)


def parse_solid(entry, where):
    value = entry.get("value")
    if not isinstance(value, dict) or not isinstance(value.get("solid"), str):
        fail(f"{where} is not a solid colour: {value!r}")
    space, _, components = value["solid"].partition(":")
    if space not in ("srgb", "extended-srgb", "display-p3"):
        fail(f"{where} names an unknown colour space {space!r}")
    try:
        parts = [float(c) for c in components.split(",")]
    except ValueError:
        fail(f"{where} has unreadable components {components!r}")
    if len(parts) != 4:
        fail(f"{where} has {len(parts)} components, expected 4")
    return space, parts


def fills(doc):
    """The base and dark background fills, as (space, components)."""
    specializations = doc.get("fill-specializations")
    if not isinstance(specializations, list):
        fail("icon.json has no fill-specializations; the dark appearance needs its own ground")
    base = [e for e in specializations if "appearance" not in e]
    dark = [e for e in specializations if e.get("appearance") == "dark"]
    # The entry without an appearance is what every appearance starts from.
    # An array holding only overrides is discarded by actool in silence.
    if len(base) != 1:
        fail(f"fill-specializations has {len(base)} base entries, expected 1")
    if len(dark) != 1:
        fail(f"fill-specializations has {len(dark)} dark entries, expected 1")
    return parse_solid(base[0], "the base fill"), parse_solid(dark[0], "the dark fill")


def layer_images(layer):
    names = []
    if isinstance(layer.get("image-name"), str):
        names.append(layer["image-name"])
    for entry in layer.get("image-name-specializations", []):
        if isinstance(entry.get("value"), str):
            names.append(entry["value"])
    return names


def main():
    if len(sys.argv) != 3 or sys.argv[2] not in ("production", "staging"):
        print("usage: verify-icon-document.py <AppIcon.icon> <production|staging>", file=sys.stderr)
        return 2
    document, channel = sys.argv[1], sys.argv[2]

    try:
        with open(os.path.join(document, "icon.json")) as handle:
            doc = json.load(handle)
    except (OSError, ValueError) as err:
        fail(f"{document}/icon.json does not read as JSON: {err}")

    squares = doc.get("supported-platforms", {}).get("squares")
    if squares != "shared" and not (isinstance(squares, list) and "macOS" in squares):
        fail(f"icon.json does not declare macOS squares: {squares!r}")

    (_, base), (_, dark) = fills(doc)
    if base == dark:
        fail("the dark fill is the base fill; dark mode would show the light ground")

    groups = doc.get("groups")
    if not isinstance(groups, list) or not groups:
        fail("icon.json has no groups")
    if len(groups) > MAX_GROUPS:
        fail(f"icon.json has {len(groups)} groups; Icon Composer allows {MAX_GROUPS}")

    referenced = []
    for index, group in enumerate(groups):
        layers = group.get("layers")
        if not isinstance(layers, list) or not layers:
            fail(f"group {index} has no layers")
        for layer in layers:
            names = layer_images(layer)
            if not names:
                fail(f"a layer in group {index} names no image")
            referenced.extend((index, name) for name in names)

    assets_dir = os.path.join(document, "Assets")
    present = set(os.listdir(assets_dir)) if os.path.isdir(assets_dir) else set()
    present.discard(".DS_Store")
    named = {name for _, name in referenced}
    for name in sorted(named - present):
        fail(f"icon.json names {name}, which is not in Assets")
    for name in sorted(present - named):
        fail(f"Assets holds {name}, which no layer uses")

    for name in sorted(present):
        with open(os.path.join(assets_dir, name), encoding="utf-8") as handle:
            svg = handle.read()
        if not name.endswith(".svg") or not svg.lstrip().startswith("<svg"):
            fail(f"{name} is not an SVG document")
        for element in ("<text", "<image", "<filter"):
            if element in svg:
                fail(f"{name} carries {element}>; outline text, keep raster and effects out")

    if N_LAYER not in named:
        fail(f"icon.json has no {N_LAYER} layer")
    with open(os.path.join(assets_dir, N_LAYER), encoding="utf-8") as handle:
        n_svg = handle.read()
    if N_PATH_START not in n_svg or N_PATH_END not in n_svg:
        fail(f"{N_LAYER} does not carry the kit's N path")

    ribbon_groups = [index for index, name in referenced if name == RIBBON_LAYER]
    if channel == "production" and ribbon_groups:
        fail("the production icon carries the staging ribbon")
    if channel == "staging":
        if not ribbon_groups:
            fail("the staging icon has no staging ribbon")
        # groups[0] draws on top.
        if ribbon_groups != [0]:
            fail("the staging ribbon is not the top group; the N would cover it")

    print(f"  {os.path.basename(document)}: {len(groups)} group(s), layers {sorted(named)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
