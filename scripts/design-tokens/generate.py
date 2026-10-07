#!/usr/bin/env python3
"""Generate every shell's colour tokens from design-tokens/glass.tokens.json.

The JSON file is the one source for the glass design system's values. This
script writes them out for each shell that consumes them:

    macos/Sources/TCDesign/Generated/GlassTokens.swift
        every token, as Swift constants for the native macOS app;
    crates/trace-commons-contributor-gtk/src/ui/brand_tokens.rs
        the brand and status roles (SHELL_ROLES below), light and dark, as
        GTK `@define-color` sheets and Rust constants for the Linux app;
    windows/src/TraceCommons.App/Themes/BrandTokens.xaml
        the same roles as a WinUI ResourceDictionary with Light, Default
        (dark) and HighContrast theme dictionaries.

Run it after editing the JSON:

    python3 scripts/design-tokens/generate.py

`--check` exits non-zero if any output is out of date instead of writing
it. The Swift suite also guards drift: TCDesignTests decodes the JSON and
compares every token with the generated constants. The GTK and Windows
suites measure the generated roles for contrast on their own grounds.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SOURCE = REPO / "design-tokens" / "glass.tokens.json"
OUTPUT = REPO / "macos" / "Sources" / "TCDesign" / "Generated" / "GlassTokens.swift"
GTK_OUTPUT = REPO / "crates" / "trace-commons-contributor-gtk" / "src" / "ui" / "brand_tokens.rs"
WINDOWS_OUTPUT = REPO / "windows" / "src" / "TraceCommons.App" / "Themes" / "BrandTokens.xaml"

# The roles the GTK and Windows shells take from the tokens. Those shells keep
# their own grounds, ink and control styling (libadwaita, WinUI); what they
# share with the glass system is the brand accent and the text-safe status
# colours. Each role names the token it reads in each scheme: the accent
# fill is the brand purple on light grounds and its brighter twin on dark.
#
# `accentLine` is the accent drawn as a stroke, rule, border or indicator
# straight on a ground. In dark, purpleSoft is a fill colour: it carries a
# white label at 5:1 but sits under 3:1 on the shells' lighter dark grounds
# (window chrome, inset wells), so a line takes purpleText there.
#
# (role, light token, dark token)
SHELL_ROLES = [
    ("accent", "purple", "purpleSoft"),
    ("accentLine", "purple", "purpleText"),
    ("accentText", "purpleText", "purpleText"),
    ("onAccent", "textOnAccent", "textOnAccent"),
    ("statusOn", "statusOn", "statusOn"),
    ("statusOnText", "statusOnText", "statusOnText"),
    ("statusAsk", "statusAsk", "statusAsk"),
    ("statusAskText", "statusAskText", "statusAskText"),
    ("statusOutside", "statusOutside", "statusOutside"),
    ("statusOutsideText", "statusOutsideText", "statusOutsideText"),
    ("onStatus", "textOnStatus", "textOnStatus"),
]
SCHEMES = ("light", "dark")
# The label-on-fill pair every shell draws on its primary action, checked
# here so a token edit that breaks it fails generation rather than a shell's
# test run. 4.5:1, because the label is text.
FILL_LABEL_PAIRS = [("accent", "onAccent")]

WEIGHTS = {"regular", "medium", "semibold", "bold", "heavy"}
DESIGNS = {"default", "monospaced"}
# macOS text styles and their sizes at the default system text size. A type
# step names its style and states that size, so the scale follows the system
# setting the way the rest of the shell does (see TC.Font_ in the app).
TEXT_STYLES = {
    "largeTitle": 26,
    "title": 22,
    "title2": 17,
    "title3": 15,
    "headline": 13,
    "body": 13,
    "callout": 12,
    "subheadline": 11,
    "footnote": 10,
    "caption": 10,
    "caption2": 10,
}
IDENTIFIER = re.compile(r"^[a-z][A-Za-z0-9]*$")


class TokenError(ValueError):
    pass


def identifier(name: str, where: str) -> str:
    if not IDENTIFIER.match(name):
        raise TokenError(f"{where}: {name!r} is not a lowerCamelCase identifier")
    return name


def rgb(value: str, where: str) -> str:
    if not re.fullmatch(r"#[0-9a-fA-F]{6}", value):
        raise TokenError(f"{where}: {value!r} is not #rrggbb")
    return "0x" + value[1:].upper()


def number(value: object, where: str) -> str:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TokenError(f"{where}: {value!r} is not a number")
    text = repr(float(value))
    return text[:-2] if text.endswith(".0") else text


def alpha(entry: dict, where: str) -> str:
    value = entry.get("alpha", 1)
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not 0 <= value <= 1:
        raise TokenError(f"{where}: alpha {value!r} is not in 0...1")
    return number(value, where)


def doc(note: str | None, indent: str) -> list[str]:
    return [f"{indent}/// {note}"] if note else []


def light_expr(entry: dict, where: str) -> str:
    """The `light:` argument for an entry with a light appearance value."""
    light = entry.get("light")
    if light is None:
        return ""
    return f", light: GlassRGBA({rgb(light['hex'], where + '.light')}, alpha: {alpha(light, where + '.light')})"


def color_expr(entry: dict, where: str) -> str:
    return f"GlassRGBA({rgb(entry['hex'], where)}, alpha: {alpha(entry, where)}{light_expr(entry, where)})"


def render(tokens: dict) -> str:
    if tokens.get("version") != 1:
        raise TokenError("version: expected 1")
    out: list[str] = [
        "// GENERATED by scripts/design-tokens/generate.py from",
        "// design-tokens/glass.tokens.json. Do not edit; edit the JSON and",
        "// regenerate. TCDesignTests fails if this file and the JSON disagree.",
        "",
        "import CoreGraphics",
        "",
        "// swiftlint:disable all",
        "",
        "/// The glass design system's values: light and dark (each colour follows",
        "/// the person's system appearance), purple brand, SF Pro.",
        "public enum GlassTokens {",
    ]

    def section(name: str, kind: str, items: list[tuple[str, str, str | None]]) -> None:
        out.append(f"    public enum {name} {{")
        for key, expr, note in items:
            out.extend(doc(note, "        "))
            out.append(f"        public static let {key}: {kind} = {expr}")
        out.append("")
        out.append(f"        /// Every {name.lower()} token by its JSON name.")
        out.append(f"        public static let all: [String: {kind}] = [")
        for key, _, _ in items:
            out.append(f'            "{key}": {key},')
        out.append("        ]")
        out.append("    }")
        out.append("")

    colors = []
    for key, entry in tokens["color"].items():
        where = f"color.{key}"
        colors.append((identifier(key, where), color_expr(entry, where), entry.get("note")))
    section("Color", "GlassRGBA", colors)

    gradients = []
    for key, entry in tokens["gradient"].items():
        where = f"gradient.{key}"
        stops = ", ".join(
            f"GlassStop({rgb(stop['hex'], where)}, alpha: {alpha(stop, where)}, at: {number(stop['at'], where)}{light_expr(stop, where)})"
            for stop in entry["stops"]
        )
        gradients.append(
            (identifier(key, where), f"GlassGradient(angle: {number(entry['angle'], where)}, stops: [{stops}])", entry.get("note"))
        )
    section("Gradient", "GlassGradient", gradients)

    shadows = []
    for key, layers in tokens["shadow"].items():
        where = f"shadow.{key}"
        parts = []
        for layer in layers:
            parts.append(
                "GlassShadow("
                f"x: {number(layer.get('x', 0), where)}, "
                f"y: {number(layer.get('y', 0), where)}, "
                f"blur: {number(layer.get('blur', 0), where)}, "
                f"color: GlassRGBA({rgb(layer['hex'], where)}, alpha: {alpha(layer, where)}{light_expr(layer, where)}), "
                f"inset: {'true' if layer.get('inset') else 'false'})"
            )
        shadows.append((identifier(key, where), "[" + ", ".join(parts) + "]", None))
    section("Shadow", "[GlassShadow]", shadows)

    for name, json_key in (("Radius", "radius"), ("Space", "space"), ("Size", "size"), ("Opacity", "opacity"), ("Motion", "motion")):
        kind = "Double" if name in ("Opacity", "Motion") else "CGFloat"
        items = [
            (identifier(key, f"{json_key}.{key}"), number(value, f"{json_key}.{key}"), None)
            for key, value in tokens[json_key].items()
        ]
        section(name, kind, items)

    types = []
    for key, entry in tokens["type"].items():
        where = f"type.{key}"
        weight = entry.get("weight", "regular")
        design = entry.get("design", "default")
        if weight not in WEIGHTS:
            raise TokenError(f"{where}: weight {weight!r}")
        if design not in DESIGNS:
            raise TokenError(f"{where}: design {design!r}")
        style = entry.get("textStyle")
        if style not in TEXT_STYLES:
            raise TokenError(f"{where}: textStyle {style!r}")
        if number(entry["size"], where) != str(TEXT_STYLES[style]):
            raise TokenError(f"{where}: size {entry['size']} is not {style}'s {TEXT_STYLES[style]}")
        types.append(
            (
                identifier(key, where),
                "GlassTypeStyle("
                f"textStyle: .{style}, "
                f"size: {number(entry['size'], where)}, "
                f"weight: .{weight}, "
                f"lineHeight: {number(entry['lineHeight'], where)}, "
                f"tracking: {number(entry.get('tracking', 0), where)}, "
                f"design: .{design}, "
                f"uppercase: {'true' if entry.get('uppercase') else 'false'}, "
                f"tabular: {'true' if entry.get('tabular') else 'false'})",
                None,
            )
        )
    section("TypeScale", "GlassTypeStyle", types)

    out[-1:] = ["}", ""]
    return "\n".join(out)


def scheme_hex(tokens: dict, token: str, scheme: str) -> str:
    """A token's #RRGGBB in one scheme: its light value where it has one."""
    entry = tokens["color"].get(token)
    if entry is None:
        raise TokenError(f"shell role: no colour token {token!r}")
    where = f"color.{token}"
    if entry.get("alpha", 1) != 1 or (entry.get("light") or {}).get("alpha", 1) != 1:
        raise TokenError(f"{where}: a shell role must be opaque")
    value = entry["hex"]
    if scheme == "light" and "light" in entry:
        value = entry["light"]["hex"]
        where += ".light"
    return "#" + rgb(value, where)[2:]


def luminance(hex_: str) -> float:
    def channel(c: int) -> float:
        v = c / 255
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4

    r, g, b = (int(hex_[i : i + 2], 16) for i in (1, 3, 5))
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def contrast(a: str, b: str) -> float:
    hi, lo = sorted((luminance(a), luminance(b)), reverse=True)
    return (hi + 0.05) / (lo + 0.05)


def role_values(tokens: dict) -> dict[str, list[tuple[str, str, str]]]:
    """{scheme: [(role, token, #RRGGBB)]} in SHELL_ROLES order."""
    out: dict[str, list[tuple[str, str, str]]] = {}
    for scheme in SCHEMES:
        rows = []
        for role, light, dark in SHELL_ROLES:
            token = light if scheme == "light" else dark
            rows.append((role, token, scheme_hex(tokens, token, scheme)))
        out[scheme] = rows
    for scheme, rows in out.items():
        values = {role: hex_ for role, _, hex_ in rows}
        for fill, label in FILL_LABEL_PAIRS:
            ratio = contrast(values[label], values[fill])
            if ratio < 4.5:
                raise TokenError(f"{scheme}: {label} on {fill} is {ratio:.2f}:1, under 4.5:1")
    return out


def snake(name: str) -> str:
    return re.sub(r"([A-Z])", r"_\1", name).lower()


def pascal(name: str) -> str:
    return name[0].upper() + name[1:]


def render_gtk(tokens: dict) -> str:
    values = role_values(tokens)
    out = [
        "// GENERATED by scripts/design-tokens/generate.py from",
        "// design-tokens/glass.tokens.json. Do not edit; edit the JSON and",
        "// regenerate. `generate.py --check` fails in CI if this file and the",
        "// JSON disagree.",
        "",
        "//! The brand accent and the text-safe status colours, from the glass",
        "//! design tokens. `style.rs` maps the shell's `tc_*` palette onto",
        "//! these; cairo drawing reads the constants, since GTK offers no",
        "//! supported way to read a `@define-color` back out of a provider.",
        "",
    ]
    for scheme in SCHEMES:
        out.append(f"/// The {scheme} scheme.")
        out.append(f"pub mod {scheme} {{")
        for role, token, hex_ in values[scheme]:
            out.append(f"    /// `{token}`.")
            out.append(f'    pub const {snake(role).upper()}: &str = "{hex_}";')
        out.append("}")
        out.append("")
    for scheme in SCHEMES:
        out.append(f"/// The {scheme} roles as GTK `@define-color` rules, loaded ahead of")
        out.append("/// the shell's own palette, which refers to them by name.")
        out.append(f'pub const {scheme.upper()}_CSS: &str = r#"')
        for role, token, hex_ in values[scheme]:
            out.append(f"@define-color tc_{snake(role)} {hex_}; /* {token} */")
        out.append('"#;')
        out.append("")
    out[-1:] = []
    out.append("")
    return "\n".join(out)


# WinUI keys the views already bind to, kept so the views need no edit, and
# the role (and, for a translucent border, the alpha byte per scheme) each
# now reads. The names predate the purple; see DesignSystem.xaml.
WINDOWS_LEGACY_KEYS = [
    ("TcGreenBrandBrush", "accent", None),
    # Selection bars and progress bars on the window and the nav chrome; the
    # primary button's fill is TcAccentBrush (DesignSystem.xaml).
    ("TcGreenFillBrush", "accentLine", None),
    ("TcGreenTextBrush", "accentText", None),
    ("TcGreenChipBorderBrush", "accentLine", {"light": "73", "dark": "80"}),
    ("TcActiveTabBorderBrush", "accentLine", {"light": "8C", "dark": "99"}),
    ("TcMarkAccentBrush", "accent", None),
    ("TcGoldBrandBrush", "statusAsk", None),
    ("TcGoldTextBrush", "statusAskText", None),
    ("TcGoldAttentionBorderBrush", "statusAsk", {"light": "8C", "dark": "99"}),
    ("TcCoralBrandBrush", "statusOutside", None),
    ("TcCoralTextBrush", "statusOutsideText", None),
]
# Under a high contrast theme every brand colour gives way to the system's:
# a fill or ink aliases the primary text brush and a label on a fill the
# page ground, as DesignSystem.xaml does for the rest of the palette.
WINDOWS_HC_GROUND_ROLES = {"onAccent", "onStatus"}


def render_windows(tokens: dict) -> str:
    values = role_values(tokens)
    out = [
        '<?xml version="1.0" encoding="utf-8"?>',
        "<!--",
        "    GENERATED by scripts/design-tokens/generate.py from",
        "    design-tokens/glass.tokens.json. Do not edit; edit the JSON and",
        "    regenerate. The generator's check mode fails in CI if this file and",
        "    the JSON disagree.",
        "",
        "    The brand accent and the text-safe status colours. DesignSystem.xaml",
        "    merges this dictionary and keeps everything else: grounds, ink, type,",
        "    spacing and the WinUI control styles. The TcGreen, TcGold and TcCoral",
        "    keys are the names the views already bind to; they now carry the",
        "    purple accent and the status tokens.",
        "-->",
        "<ResourceDictionary",
        '    xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"',
        '    xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml">',
        "",
        "    <ResourceDictionary.ThemeDictionaries>",
    ]
    for scheme, key in (("dark", "Default"), ("light", "Light")):
        by_role = {role: hex_ for role, _, hex_ in values[scheme]}
        out.append("")
        note = "Dark. WinUI names the dark dictionary Default." if scheme == "dark" else "Light."
        out.append(f"        <!-- {note} -->")
        out.append(f'        <ResourceDictionary x:Key="{key}">')
        for role, token, hex_ in values[scheme]:
            out.append(f'            <SolidColorBrush x:Key="Tc{pascal(role)}Brush" Color="{hex_}" />')
        out.append("")
        for name, role, alphas in WINDOWS_LEGACY_KEYS:
            hex_ = by_role[role]
            color = f"#{alphas[scheme]}{hex_[1:]}" if alphas else hex_
            out.append(f'            <SolidColorBrush x:Key="{name}" Color="{color}" />')
        out.append("        </ResourceDictionary>")
    out.append("")
    out.append("        <!-- High contrast: the system's colours, as in DesignSystem.xaml. -->")
    out.append('        <ResourceDictionary x:Key="HighContrast">')
    for role, _, _ in SHELL_ROLES:
        stock = "ApplicationPageBackgroundThemeBrush" if role in WINDOWS_HC_GROUND_ROLES else "TextFillColorPrimaryBrush"
        out.append(f'            <StaticResource x:Key="Tc{pascal(role)}Brush" ResourceKey="{stock}" />')
    out.append("")
    for name, _, _ in WINDOWS_LEGACY_KEYS:
        out.append(f'            <StaticResource x:Key="{name}" ResourceKey="TextFillColorPrimaryBrush" />')
    out.append("        </ResourceDictionary>")
    out.append("    </ResourceDictionary.ThemeDictionaries>")
    out.append("</ResourceDictionary>")
    out.append("")
    return "\n".join(out)


def outputs(tokens: dict) -> list[tuple[Path, str]]:
    return [
        (OUTPUT, render(tokens)),
        (GTK_OUTPUT, render_gtk(tokens)),
        (WINDOWS_OUTPUT, render_windows(tokens)),
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true", help="fail if any generated file is stale")
    args = parser.parse_args()
    try:
        rendered = outputs(json.loads(SOURCE.read_text()))
    except (TokenError, KeyError, json.JSONDecodeError) as error:
        print(f"glass tokens: {error}", file=sys.stderr)
        return 2
    if args.check:
        stale = [path for path, text in rendered if (path.read_text() if path.exists() else "") != text]
        for path in stale:
            print(f"{path.relative_to(REPO)} is stale; run scripts/design-tokens/generate.py", file=sys.stderr)
        return 1 if stale else 0
    for path, text in rendered:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        print(f"wrote {path.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
