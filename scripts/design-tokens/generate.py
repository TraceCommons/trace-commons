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


def gradient_expr(entry: dict, where: str) -> str:
    stops = ", ".join(
        f"GlassStop({rgb(stop['hex'], where)}, alpha: {alpha(stop, where)}, at: {number(stop['at'], where)}{light_expr(stop, where)})"
        for stop in entry["stops"]
    )
    return f"GlassGradient(angle: {number(entry['angle'], where)}, stops: [{stops}])"


def shadow_expr(layers: list, where: str) -> str:
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
    return "[" + ", ".join(parts) + "]"


# The flat theme (design exploration) names its values per appearance:
# `flatLight` is its light appearance and `flatDark` its dark one, each
# naming only what it changes. A colour one leaves alone keeps its classic
# value for that appearance. Gradients and shadows are named by both or by
# neither, with the same stops or layers, because each stop and layer is one
# value with a light and a dark side. Only colours, gradients, shadows and
# radii take overrides.
THEME_APPEARANCES = ("flatLight", "flatDark")
THEMED_SECTIONS = ("color", "gradient", "shadow", "radius")


def check_themes(tokens: dict) -> None:
    themes = {key: value for key, value in tokens.get("themes", {}).items() if not key.startswith("$")}
    for name, theme in themes.items():
        if name not in THEME_APPEARANCES:
            raise TokenError(f"themes.{name}: not one of {', '.join(THEME_APPEARANCES)}")
        for json_key, entries in theme.items():
            if json_key not in THEMED_SECTIONS:
                raise TokenError(f"themes.{name}.{json_key}: a theme overrides only {', '.join(THEMED_SECTIONS)}")
            for key in entries:
                if key not in tokens[json_key]:
                    raise TokenError(f"themes.{name}.{json_key}.{key}: no such {json_key} token")
    if not themes:
        return
    light, dark = (themes.get(name, {}) for name in THEME_APPEARANCES)
    for json_key in THEMED_SECTIONS:
        a, b = light.get(json_key, {}), dark.get(json_key, {})
        if json_key != "color" and set(a) != set(b):
            raise TokenError(f"themes.{json_key}: flatLight and flatDark override different tokens: {sorted(set(a) ^ set(b))}")
        for key in a:
            where = f"themes.*.{json_key}.{key}"
            if json_key == "gradient" and [x["at"] for x in a[key]["stops"]] != [x["at"] for x in b[key]["stops"]]:
                raise TokenError(f"{where}: the two appearances need the same stops")
            if json_key == "shadow" and [bool(x.get("inset")) for x in a[key]] != [bool(x.get("inset")) for x in b[key]]:
                raise TokenError(f"{where}: the two appearances need the same layers")
            if json_key == "radius" and a[key] != b[key]:
                raise TokenError(f"{where}: a radius is the same in both appearances")


def dark_only(entry: dict) -> dict:
    """An entry with its light value dropped: its dark value everywhere."""
    return {k: v for k, v in entry.items() if k != "light"}


def paired(light: dict, dark: dict) -> dict:
    """A colour entry whose dark value is `dark` and light value `light`."""
    out = dark_only(dark)
    if (light["hex"].lower(), light.get("alpha", 1)) != (dark["hex"].lower(), dark.get("alpha", 1)):
        out["light"] = dark_only(light)
    return out


def flat_entry(tokens: dict, json_key: str, key: str):
    """The flat theme's entry for a token, or None where it is the classic one."""
    themes = tokens.get("themes", {})
    light = themes.get("flatLight", {}).get(json_key, {}).get(key)
    dark = themes.get("flatDark", {}).get(json_key, {}).get(key)
    base = tokens[json_key][key]
    if json_key == "radius":
        return light
    if json_key == "color":
        if light is None and dark is None:
            return None
        classic_light = base.get("light", dark_only(base))
        entry = paired(light if light is not None else classic_light, dark if dark is not None else dark_only(base))
        return None if entry == base else entry
    if json_key == "gradient":
        if light is None:
            return None
        else:
            entry = {**dark, "stops": [{**paired(a, b), "at": b["at"]} for a, b in zip(light["stops"], dark["stops"])]}
        return None if entry == base else entry
    if json_key == "shadow":
        if light is None:
            return None
        else:
            entry = [{**paired(a, b), **{k: b[k] for k in ("x", "y", "blur", "inset") if k in b}} for a, b in zip(light, dark)]
        return None if entry == base else entry
    raise TokenError(f"{json_key}: not themed")


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

    check_themes(tokens)

    def section(name: str, kind: str, items: list[tuple], json_key: str | None = None, expr=None) -> None:
        out.append(f"    public enum {name} {{")
        themed = False
        for key, value, note in items:
            out.extend(doc(note, "        "))
            flat = flat_entry(tokens, json_key, key) if json_key else None
            if flat is not None:
                # Read each time, so it follows the theme the app launched in.
                themed = True
                flat_value = expr(flat, f"themes.{json_key}.{key}")
                out.append(f"        public static var {key}: {kind} {{ GlassTheme.pick({value}, flat: {flat_value}) }}")
            else:
                out.append(f"        public static let {key}: {kind} = {value}")
        out.append("")
        out.append(f"        /// Every {name.lower()} token by its JSON name.")
        if themed:
            out.append(f"        public static var all: [String: {kind}] {{ [")
        else:
            out.append(f"        public static let all: [String: {kind}] = [")
        for key, _, _ in items:
            out.append(f'            "{key}": {key},')
        out.append("        ] }" if themed else "        ]")
        out.append("    }")
        out.append("")

    colors = []
    for key, entry in tokens["color"].items():
        where = f"color.{key}"
        colors.append((identifier(key, where), color_expr(entry, where), entry.get("note")))
    section("Color", "GlassRGBA", colors, "color", color_expr)

    gradients = []
    for key, entry in tokens["gradient"].items():
        where = f"gradient.{key}"
        gradients.append((identifier(key, where), gradient_expr(entry, where), entry.get("note")))
    section("Gradient", "GlassGradient", gradients, "gradient", gradient_expr)

    shadows = []
    for key, layers in tokens["shadow"].items():
        where = f"shadow.{key}"
        shadows.append((identifier(key, where), shadow_expr(layers, where), None))
    section("Shadow", "[GlassShadow]", shadows, "shadow", shadow_expr)

    for name, json_key in (("Radius", "radius"), ("Space", "space"), ("Size", "size"), ("Opacity", "opacity"), ("Motion", "motion")):
        kind = "Double" if name in ("Opacity", "Motion") else "CGFloat"
        items = [
            (identifier(key, f"{json_key}.{key}"), number(value, f"{json_key}.{key}"), None)
            for key, value in tokens[json_key].items()
        ]
        if json_key == "radius":
            section(name, kind, items, json_key, number)
        else:
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


def insights_entry(tokens: dict, section: str, name: str, scheme: str) -> dict:
    """Resolve the same flat-theme override and appearance as native macOS."""
    override = tokens["themes"]["flat" + scheme.title()].get(section, {}).get(name)
    entry = override if override is not None else tokens[section][name]
    return entry.get("light", entry) if scheme == "light" else entry


def opaque_over(entry: dict, ground: str) -> str:
    """Flatten a glass fill over the Linux solid ground, never the desktop."""
    alpha = entry.get("alpha", 1)
    rgb(entry["hex"], "Insights colour")
    channels = [round(int(entry["hex"][i:i + 2], 16) * alpha
                      + int(ground[i:i + 2], 16) * (1 - alpha)) for i in (1, 3, 5)]
    return "#" + "".join(f"{c:02x}" for c in channels)


def insights_palette(tokens: dict, scheme: str, dimmed: bool = False) -> dict[str, str]:
    """Insights' opaque colours; `dimmed`, over the pane as an unfocused
    window dims it (`GlassPaneFill`'s inactiveDim)."""
    def color(name):
        return insights_entry(tokens, "color", name, scheme)

    def fill(name):
        entry = insights_entry(tokens, "gradient", name, scheme)
        # Flat gradients are uniform fills. Refuse a future nonuniform gradient
        # rather than silently discarding its stops in the Linux translation.
        stops = entry["stops"]
        if any((s["hex"], s.get("alpha", 1)) !=
               (stops[0]["hex"], stops[0].get("alpha", 1)) for s in stops):
            raise TokenError(f"Insights {name}: expected a uniform flat fill")
        return stops[0]

    ground = opaque_over(fill("paneFill"), color("paneOpaque")["hex"])
    if dimmed:
        ground = opaque_over(color("inactiveDim"), ground)
    card = opaque_over(fill("cardFill"), ground)
    control = opaque_over(fill("controlFill"), ground)
    # White hover on Linux's already-white solid surfaces is imperceptible.
    # Use the flat Light card-control slate on each actual backing; Dark
    # retains its card hover and uses the raised menu-control hover on
    # the equally dark switcher track (the black control hover disappears).
    card_hover = (opaque_over(color("controlOnCardHover"), card) if scheme == "light"
                  else opaque_over(color("cardHover"), ground))
    control_hover = opaque_over(color("controlOnCardHover" if scheme == "light"
                                      else "controlOnMenuHover"), control)
    palette = {"ground": ground, "card": card,
               "hover": card_hover,
               "control": control,
               "control_hover": control_hover,
               "selected": opaque_over(color("controlSelected"), ground),
               "selected_ink": opaque_over(color("textOnSelected"), ground),
               "ink": opaque_over(color("textPrimary"), ground),
               "secondary": opaque_over(color("textSecondary"), ground)}
    for ink in ("ink", "secondary"):
        for surface in ("ground", "card", "hover", "control", "control_hover"):
            if contrast(palette[ink], palette[surface]) < 4.5:
                raise TokenError(f"Insights {scheme}: {ink} on {surface} under 4.5:1")
    if contrast(palette["selected_ink"], palette["selected"]) < 4.5:
        raise TokenError(f"Insights {scheme}: selection text under 4.5:1")
    for hover, surface in (("hover", "card"), ("control_hover", "control")):
        if contrast(palette[hover], palette[surface]) < 1.15:
            raise TokenError(f"Insights {scheme}: {hover} on {surface} under 1.15:1")
    return palette


def render_insights_css(tokens: dict, scheme: str) -> str:
    p = insights_palette(tokens, scheme)
    theme = tokens["themes"]["flat" + scheme.title()]
    radius = {**tokens["radius"], **theme.get("radius", {})}
    space = tokens["space"]
    weights = {"regular": 400, "medium": 500, "semibold": 600, "bold": 700, "heavy": 800}
    out = [f".tc-insights {{ background: {p['ground']}; color: {p['ink']}; }}",
           f".tc-insights .tc-insights-card, .tc-insights button.tc-insights-summary {{ background: {p['card']}; border: none; box-shadow: none; border-radius: {radius['card']}px; padding: {space['s5']}px; }}",
           ".tc-insights button.tc-insights-summary { min-width: 10em; }",
           ".tc-insights .tc-insights-summary-grid { background: transparent; }",
           ".tc-insights .tc-insights-summary-grid > flowboxchild { padding: 0; background: transparent; }",
           f".tc-insights button.tc-insights-card:hover, .tc-insights button.tc-insights-summary:hover {{ background: {p['hover']}; }}",
           f".tc-insights button:focus-visible, .tc-insights dropdown:focus-visible {{ outline: 2px solid {p['selected']}; outline-offset: 2px; }}",
           f".tc-insights stackswitcher {{ background: {p['control']}; border-radius: {radius['control']}px; padding: {space['s2']}px; }}",
           f".tc-insights stackswitcher button {{ background: transparent; color: {p['ink']}; border: none; box-shadow: none; border-radius: {radius['control']}px; }}",
           f".tc-insights stackswitcher button:hover {{ background: {p['control_hover']}; }}",
           f".tc-insights stackswitcher button:checked {{ background: {p['selected']}; color: {p['selected_ink']}; }}",
           f".tc-insights .caption, .tc-insights .dim-label {{ color: {p['secondary']}; opacity: 1; }}"]
    # In an unfocused glass window Insights dims with the pane it sits in.
    d = insights_palette(tokens, scheme, dimmed=True)
    backdrop = "window.tc-glass-window:backdrop .tc-insights"
    out += [f"{backdrop} {{ background: {d['ground']}; }}",
            f"{backdrop} .tc-insights-card, {backdrop} button.tc-insights-summary {{ background: {d['card']}; }}",
            f"{backdrop} stackswitcher {{ background: {d['control']}; }}"]
    for selector, role in [("", "body"), (" .title-2", "title"),
                           (" .title-3", "number"), (" .heading", "bodyStrong"),
                           (" .caption", "caption")]:
        t = tokens["type"][role]
        # Preserve the desktop font setting; tokens determine relative hierarchy.
        # Role classes belong on leaf labels, never nested type containers.
        relative_size = t["size"] / tokens["type"]["body"]["size"]
        out.append(f".tc-insights{selector} {{ font-size: {relative_size:.6g}em; font-weight: {weights[t['weight']]}; }}")
    return "\n".join(out)


GLYPHS = REPO / "macos" / "Sources" / "TCDesign" / "Components" / "Glyphs.swift"
LOGOS = REPO / "macos" / "Sources" / "TCDesign" / "Logos" / "GlassLogoPaths.swift"
BRAND_MARK = REPO / "macos" / "Sources" / "TCDesign" / "Logos" / "GlassBrandMark.swift"

# The colours the Traces map draws with on the Mac, as the flat theme draws
# them: its nodes, the pad, the rocket, the runs' orbs, the coins and their
# tints. The Linux map reads them from `brand_tokens::map`.
MAP_COLOURS = [
    "mapOrbShared", "mapOrbKept", "purpleSoft", "blue", "statusOff", "statusOn",
    "statusOnText", "statusAsk", "statusOutside", "mapNodeOff", "mapCredentialOn",
    "mapRuleAutoFill", "mapRuleAskFill", "mapRuleIgnoreFill", "mapRuleUnsetFill",
    "paneOpaque", "wellFill", "mapFieldInner", "mapFieldOuter",
    "textPrimary", "textSecondary", "textTertiary", "purpleText",
    "toolClaude", "toolCodex", "toolAntigravity", "toolOpenCode", "toolTheia",
]

# Glass tiers as the Linux shell paints them: every fill opaque, flattened
# over the surface the tier sits on, as the Mac does under Reduce
# Transparency. (tier, CSS name, radius token, edge shadow token, ground)
# A well is dug into a card, so it is flattened over the card; every other
# tier sits on the pane's opaque ground.
GLASS_TIERS = [
    ("pane", "pane", "pane", "paneEdge", "pane"),
    ("card", "card", "card", "cardEdge", "pane"),
    ("cardQuiet", "card-quiet", "cardQuiet", "cardEdgeQuiet", "pane"),
    ("well", "well", "pill", "wellEdge", "card"),
    ("control", "control", "pill", "controlEdge", "pane"),
    ("controlSelected", "control-selected", "pill", "controlSelectedEdge", "pane"),
    ("popover", "popover", "card", "popoverEdge", "pane"),
    ("menu", "menu", "cardQuiet", "menuEdge", "pane"),
    ("nodeCard", "node-card", "card", "nodeCardEdge", "pane"),
]
# Tier fills that hold text the shell draws in the glass ink. A tier named
# here is checked for 4.5:1 against primary and secondary text.
GLASS_TEXT_TIERS = ["pane", "card", "cardQuiet", "control", "popover", "menu", "nodeCard"]
# The type roles, as classes on a leaf label.
GLASS_WEIGHTS = {"regular": 400, "medium": 500, "semibold": 600, "bold": 700, "heavy": 800}


def resolve_scheme(entry: dict, scheme: str) -> dict:
    """A colour entry's value in one scheme: its `light` side, or itself."""
    if scheme == "light" and "light" in entry:
        return dict(entry["light"])
    return {k: v for k, v in entry.items() if k != "light"}


def flat_value(tokens: dict, section: str, name: str, scheme: str):
    """A token as the flat theme draws it in one scheme (what Linux shows)."""
    override = tokens["themes"]["flat" + scheme.title()].get(section, {}).get(name)
    return override if override is not None else tokens[section][name]


def flat_color(tokens: dict, name: str, scheme: str) -> dict:
    return resolve_scheme(flat_value(tokens, "color", name, scheme), scheme)


def flat_fill(tokens: dict, name: str, scheme: str) -> dict:
    """A gradient token's one colour. The flat theme's fills are uniform."""
    stops = flat_value(tokens, "gradient", name, scheme)["stops"]
    colours = [resolve_scheme(stop, scheme) for stop in stops]
    if any((c["hex"], c.get("alpha", 1)) != (colours[0]["hex"], colours[0].get("alpha", 1)) for c in colours):
        raise TokenError(f"glass {name}: expected a uniform flat fill")
    return colours[0]


def flat_layers(tokens: dict, name: str, scheme: str) -> list[dict]:
    layers = flat_value(tokens, "shadow", name, scheme)
    out = []
    for layer in layers:
        value = resolve_scheme(layer, scheme)
        for key in ("x", "y", "blur", "inset"):
            if key in layer:
                value[key] = layer[key]
        out.append(value)
    return out


def css_rgba(entry: dict, where: str) -> str:
    rgb(entry["hex"], where)
    channels = ", ".join(str(int(entry["hex"][i : i + 2], 16)) for i in (1, 3, 5))
    return f"rgba({channels}, {number(entry.get('alpha', 1), where)})"


def css_shadow(layers: list[dict], where: str) -> str:
    if not layers:
        return "none"
    parts = []
    for layer in layers:
        parts.append(
            ("inset " if layer.get("inset") else "")
            + f"{number(layer.get('x', 0), where)}px {number(layer.get('y', 0), where)}px "
            + f"{number(layer.get('blur', 0), where)}px {css_rgba(layer, where)}"
        )
    return ", ".join(parts)


def glass_fills(tokens: dict, scheme: str, ground: str) -> dict[str, str]:
    """Every tier's opaque fill over a pane ground: a well over a card,
    every other tier over the pane."""
    over = lambda entry, under: opaque_over(entry, under)
    fills = {"pane": ground}
    fills["card"] = over(flat_fill(tokens, "cardFill", scheme), ground)
    fills["cardQuiet"] = over(flat_fill(tokens, "cardFillQuiet", scheme), ground)
    fills["well"] = over(flat_color(tokens, "wellFill", scheme), fills["card"])
    fills["control"] = over(flat_fill(tokens, "controlFill", scheme), ground)
    fills["controlSelected"] = over(flat_color(tokens, "controlSelected", scheme), ground)
    fills["popover"] = over(flat_color(tokens, "popoverFill", scheme), ground)
    fills["menu"] = over(flat_color(tokens, "menuFill", scheme), ground)
    fills["nodeCard"] = over(flat_color(tokens, "nodeCardFill", scheme), ground)
    return fills


def glass_palette(tokens: dict, scheme: str) -> dict[str, str]:
    """Every opaque colour the painted window uses, in one scheme."""
    # Every Mac pane path paints paneFill over its base (`GlassPaneFill`),
    # so the painted ground is paneFill over paneOpaque, as Insights' is.
    ground = opaque_over(flat_fill(tokens, "paneFill", scheme),
                         flat_color(tokens, "paneOpaque", scheme)["hex"])
    over = lambda entry, under: opaque_over(entry, under)
    fills = glass_fills(tokens, scheme, ground)
    palette = dict(fills)
    palette["scene"] = flat_color(tokens, "sceneBase", scheme)["hex"]
    palette["ink"] = over(flat_color(tokens, "textPrimary", scheme), ground)
    palette["inkSecondary"] = over(flat_color(tokens, "textSecondary", scheme), ground)
    palette["selectedInk"] = over(flat_color(tokens, "textOnSelected", scheme), ground)
    palette["toolbarGlyph"] = over(flat_color(tokens, "toolbarGlyph", scheme), ground)
    palette["toolbarGlyphHidden"] = over(flat_color(tokens, "toolbarGlyphHidden", scheme), ground)
    palette["toolbarExpanded"] = over(flat_color(tokens, "toolbarExpanded", scheme), ground)
    # As the Insights palette does: a white hover is invisible on Linux's
    # white control in light, and dark's own hover is a black that vanishes
    # on its dark control, so each takes the raised swatch that shows.
    hover = "controlOnCardHover" if scheme == "light" else "controlOnMenuHover"
    palette["controlHover"] = over(flat_color(tokens, hover, scheme), fills["control"])
    if contrast(palette["controlHover"], fills["control"]) < 1.1:
        raise TokenError(f"glass {scheme}: control hover is invisible on the control")
    palette["rowHover"] = over(flat_color(tokens, "rowHover", scheme), ground)
    palette["tabEmber"] = over(flat_color(tokens, "tabEmber", scheme), ground)
    # The tab strip's 1pt rule, and the halo under the selected tab, which
    # fades to nothing and so stays translucent (`GlassRuleTabs`).
    palette["rule"] = over(flat_color(tokens, "rule", scheme), ground)
    palette["tabEmberHalo"] = css_rgba(flat_color(tokens, "tabEmberHalo", scheme), "tabEmberHalo")
    # `inactiveDim` over the pane while the window is not key (GTK's
    # `:backdrop`), as `GlassPaneFill` draws it.
    # The Mac's material path fills it under the pane's translucent tiers,
    # so cards, wells and controls dim with the pane; here every tier is
    # opaque, so each is flattened again over the dimmed ground.
    dimmed = over(flat_color(tokens, "inactiveDim", scheme), ground)
    inactive = glass_fills(tokens, scheme, dimmed)
    for tier, fill in inactive.items():
        palette[tier + "Inactive"] = fill
    palette["ruleInactive"] = over(flat_color(tokens, "rule", scheme), dimmed)
    for tier in GLASS_TEXT_TIERS:
        for ink in ("ink", "inkSecondary"):
            ratio = contrast(palette[ink], inactive[tier])
            if ratio < 4.5:
                where = "paneInactive" if tier == "pane" else f"{tier} unfocused"
                raise TokenError(f"glass {scheme}: {ink} on {where} is {ratio:.2f}:1, under 4.5:1")
    # The graph's focus glyph (MonitorShell `focusInk`) and the range
    # capsule's ink(0.07) fill, which stays translucent over the pane.
    for token in ("graphFocusOn", "graphFocusIdle", "graphFocusOff"):
        palette[token] = over(flat_color(tokens, token, scheme), ground)
    # The summary's decision-line glyphs (`SummaryDecisionLine`): a
    # status's text colour, or the tertiary ink with no status.
    for token in ("statusOnText", "statusAskText", "textTertiary"):
        palette[token] = over(flat_color(tokens, token, scheme), ground)
    # The Traces tree's rows (`GlassListRow`): a flagged sub-line in
    # statusAsk or statusOn, the chevron in statusOff, and the folder
    # tile's fill (`GlassToolTile`, tileFolder).
    for token in ("statusAsk", "statusOn", "statusOff", "tileFolder"):
        palette[token] = over(flat_color(tokens, token, scheme), ground)
    ink = flat_color(tokens, "ink", scheme)
    palette["rangeFill"] = css_rgba({"hex": ink["hex"], "alpha": 0.07}, "rangeFill")
    # The solid stroke a tier with edge layers takes under Increase Contrast
    # (`GlassEdgeModifier`'s increased branch). Kept translucent: a border
    # paints over its own tier's fill, which is the ground the Mac strokes on.
    palette["edgeHighContrast"] = css_rgba(flat_color(tokens, "edgeHighContrast", scheme), "edgeHighContrast")
    for tier in GLASS_TEXT_TIERS:
        for ink in ("ink", "inkSecondary"):
            ratio = contrast(palette[ink], fills[tier])
            if ratio < 4.5:
                raise TokenError(f"glass {scheme}: {ink} on {tier} is {ratio:.2f}:1, under 4.5:1")
    ratio = contrast(palette["selectedInk"], palette["controlSelected"])
    if ratio < 4.5:
        raise TokenError(f"glass {scheme}: selection text on controlSelected is {ratio:.2f}:1, under 4.5:1")
    return palette



# The translucent window KDE blur shows through (`ui::glass_backdrop`): the
# panes carry the Mac's material stack as one colour, and the tiers inside a
# pane take their flat fills with their own alpha, composited by GTK over it.
# The Liquid Glass tint is the veil at this share of its alpha
# (GlassTheme.swift:82 `glassTintShare`); the compositor's blur stands in for
# the glass itself, which adds no frost of its own.
GLASS_TINT_SHARE = 0.25
# Tiers whose blurred fill is their flat token with alpha. A popover and a
# menu are surfaces of their own, which no blur reaches, and a node card
# floats over the map, which the Mac blurs within the window and Linux does
# not: those three keep their opaque painted fills.
GLASS_BLUR_TIERS = ["card", "cardQuiet", "well", "control", "controlSelected"]
# The desktops a blurred pane may sit over: the blur averages what is behind,
# and these are the extremes it can average to.
GLASS_BLUR_DESKTOPS = ["#000000", "#ffffff"]


def rgba_stack(layers: list[dict]) -> tuple[float, float, float, float]:
    """Source-over compositing of colour entries, bottom first, onto nothing:
    the colour and alpha one layer must carry to look the same."""
    r = g = b = a = 0.0
    for entry in layers:
        alpha = float(entry.get("alpha", 1))
        c = [int(entry["hex"][i:i + 2], 16) for i in (1, 3, 5)]
        out = alpha + a * (1 - alpha)
        if out == 0:
            continue
        r, g, b = ((c[i] * alpha + (r, g, b)[i] * a * (1 - alpha)) / out for i in range(3))
        a = out
    return r, g, b, a


def css_rgba_tuple(colour: tuple[float, float, float, float]) -> str:
    r, g, b, a = colour
    return f"rgba({round(r)}, {round(g)}, {round(b)}, {round(a, 4):g})"


def entry_of(colour: tuple[float, float, float, float]) -> dict:
    """An rgba tuple back as a colour entry, at the precision CSS carries."""
    r, g, b, a = colour
    return {"hex": "#" + "".join(f"{round(c):02x}" for c in (r, g, b)), "alpha": round(a, 4)}


def blur_pane_layers(tokens: dict, scheme: str, share: float, dimmed: bool) -> list[dict]:
    """GlassPaneFill's layers over the glass: the tint, the veil, paneFill,
    and inactiveDim out of focus."""
    veil = flat_color(tokens, "glassVeil", scheme)
    tint = dict(veil, alpha=float(veil.get("alpha", 1)) * share)
    layers = [tint, veil, flat_fill(tokens, "paneFill", scheme)]
    if dimmed:
        layers.append(flat_color(tokens, "inactiveDim", scheme))
    return layers


def blur_failures(tokens: dict, scheme: str, share: float) -> list[str]:
    """Every glass ink (ink, inkSecondary, selection text) that would read
    under 4.5:1 over a black or a white desktop at this tint share, focused
    or not. The shell's own text colours are checked against these panes in
    style.rs, which decides whether a scheme may go translucent at all."""
    painted = glass_palette(tokens, scheme)
    fills = {"card": flat_fill(tokens, "cardFill", scheme),
             "cardQuiet": flat_fill(tokens, "cardFillQuiet", scheme),
             "control": flat_fill(tokens, "controlFill", scheme)}
    failures = []
    for desktop in GLASS_BLUR_DESKTOPS:
        for dimmed in (False, True):
            pane = entry_of(rgba_stack(blur_pane_layers(tokens, scheme, share, dimmed)))
            ground = opaque_over(pane, desktop)
            surfaces = {"pane": ground}
            for tier, fill in fills.items():
                surfaces[tier] = opaque_over(fill, ground)
            for surface, colour in surfaces.items():
                for ink in ("ink", "inkSecondary"):
                    ratio = contrast(painted[ink], colour)
                    if ratio < 4.5:
                        focus = "unfocused" if dimmed else "focused"
                        failures.append(f"{ink} on translucent {surface} over {desktop}, {focus}, "
                                        f"is {ratio:.2f}:1")
            selected = opaque_over(flat_color(tokens, "controlSelected", scheme), ground)
            if contrast(painted["selectedInk"], selected) < 4.5:
                failures.append(f"selection text on translucent controlSelected over {desktop}")
    return failures


def glass_blur_tint_share(tokens: dict, scheme: str) -> float:
    """The Mac's tint share, or the smallest lift in 0.05 steps at which every
    glass ink still reads over a black or white desktop. KWin's blur adds no frost
    of its own, as the Mac's material does; a lift is a deviation from the
    Mac, kept as small as the 4.5:1 floor allows."""
    share = GLASS_TINT_SHARE
    while share <= 0.5 + 1e-9:
        if not blur_failures(tokens, scheme, share):
            return round(share, 2)
        share += 0.05
    raise TokenError(f"glass {scheme}: translucent pane unreadable at every tint share: "
                     + "; ".join(blur_failures(tokens, scheme, GLASS_TINT_SHARE)))


def glass_blur_palette(tokens: dict, scheme: str) -> dict[str, str]:
    """The colours the translucent window redefines, as CSS: the panes as the
    Mac's material stack in one colour, the tiers inside them with alpha."""
    share = glass_blur_tint_share(tokens, scheme)
    palette = {"scene": "transparent"}
    palette["pane"] = css_rgba_tuple(rgba_stack(blur_pane_layers(tokens, scheme, share, False)))
    palette["paneInactive"] = css_rgba_tuple(rgba_stack(blur_pane_layers(tokens, scheme, share, True)))
    fills = {"card": flat_fill(tokens, "cardFill", scheme),
             "cardQuiet": flat_fill(tokens, "cardFillQuiet", scheme),
             "well": flat_color(tokens, "wellFill", scheme),
             "control": flat_fill(tokens, "controlFill", scheme),
             "controlSelected": flat_color(tokens, "controlSelected", scheme),
             "rule": flat_color(tokens, "rule", scheme)}
    for tier, fill in fills.items():
        # Inside a pane a tier dims with the pane beneath it, not by itself.
        palette[tier] = palette[tier + "Inactive"] = css_rgba_tuple(rgba_stack([fill]))
    # As the painted palette: the hover that shows on each control, stacked
    # on the control's own fill, and a row's hover over the pane.
    hover = "controlOnCardHover" if scheme == "light" else "controlOnMenuHover"
    palette["controlHover"] = css_rgba_tuple(rgba_stack([fills["control"], flat_color(tokens, hover, scheme)]))
    palette["rowHover"] = css_rgba_tuple(rgba_stack([flat_color(tokens, "rowHover", scheme)]))
    return palette


GLASS_BLUR_COLOURS = [
    ("scene", "scene"),
    ("pane", "pane"),
    ("paneInactive", "pane_inactive"),
    *[(t, snake(t)) for t in GLASS_BLUR_TIERS],
    *[(t + "Inactive", snake(t) + "_inactive") for t in GLASS_BLUR_TIERS],
    ("rule", "rule"),
    ("ruleInactive", "rule_inactive"),
    ("controlHover", "control_hover"),
    ("rowHover", "row_hover"),
]


def render_glass_blur_css(tokens: dict, scheme: str) -> str:
    """Loaded after the painted sheet while blur is granted: it redefines the
    colours the panes and tiers paint with, so every rule naming them, the
    hand-written ones in style.css included, follows."""
    palette = glass_blur_palette(tokens, scheme)
    out = [f"@define-color tc_glass_{name} {palette[key]};" for key, name in GLASS_BLUR_COLOURS]
    # GlassWindowConfigurator: no background, no window shadow. libadwaita's
    # window.csd outline and 1px ring would draw a rectangle across the gaps;
    # the shadow's extent stays, transparent, as the resize margin (its own
    # `0 0 14px 5px transparent` on :backdrop), and a maximized or tiled
    # window keeps libadwaita's zero or 20px extent without the ring.
    out.append("window.tc-glass-window.csd, window.tc-glass-window.csd:backdrop "
               "{ outline: none; box-shadow: 0 0 14px 5px transparent; }")
    out.append("window.tc-glass-window.maximized, window.tc-glass-window.fullscreen "
               "{ box-shadow: none; }")
    out.append("window.tc-glass-window.tiled, window.tc-glass-window.tiled-top, "
               "window.tc-glass-window.tiled-right, window.tc-glass-window.tiled-bottom, "
               "window.tc-glass-window.tiled-left { box-shadow: 0 0 0 20px transparent; }")
    return "\n".join(out)


def render_insights_blur_css(tokens: dict, scheme: str) -> str:
    """Insights over a translucent pane: no ground of its own, cards and the
    switcher with their fills' alpha, in and out of focus alike."""
    card = flat_fill(tokens, "cardFill", scheme)
    control = flat_fill(tokens, "controlFill", scheme)
    card_hover = (rgba_stack([card, flat_color(tokens, "controlOnCardHover", scheme)]) if scheme == "light"
                  else rgba_stack([flat_color(tokens, "cardHover", scheme)]))
    control_hover = rgba_stack([control, flat_color(tokens, "controlOnCardHover" if scheme == "light"
                                                    else "controlOnMenuHover", scheme)])
    # Scoped to the glass window: the sheet is display-wide while loaded, and
    # an Insights window of its own keeps its painted ground.
    cards = ".tc-insights .tc-insights-card, .tc-insights button.tc-insights-summary"
    out = []
    for prefix in ("window.tc-glass-window ", "window.tc-glass-window:backdrop "):
        out += [f"{prefix}.tc-insights {{ background: transparent; }}",
                ", ".join(prefix + part.strip() for part in cards.split(","))
                + f" {{ background: {css_rgba(card, 'cardFill')}; }}",
                f"{prefix}.tc-insights stackswitcher {{ background: {css_rgba(control, 'controlFill')}; }}"]
    scope = "window.tc-glass-window "
    out += [f"{scope}.tc-insights button.tc-insights-card:hover, "
            f"{scope}.tc-insights button.tc-insights-summary:hover "
            f"{{ background: {css_rgba_tuple(card_hover)}; }}",
            f"{scope}.tc-insights stackswitcher button:hover "
            f"{{ background: {css_rgba_tuple(control_hover)}; }}"]
    return "\n".join(out)


GLASS_COLOURS = [
    ("scene", "scene"),
    ("ink", "ink"),
    ("inkSecondary", "ink_secondary"),
    ("selectedInk", "selected_ink"),
    ("toolbarGlyph", "toolbar_glyph"),
    ("toolbarGlyphHidden", "toolbar_glyph_hidden"),
    ("toolbarExpanded", "toolbar_expanded"),
    ("controlHover", "control_hover"),
    ("rowHover", "row_hover"),
    ("tabEmber", "tab_ember"),
    ("rule", "rule"),
    ("tabEmberHalo", "tab_ember_halo"),
    ("ruleInactive", "rule_inactive"),
    ("edgeHighContrast", "edge_high_contrast"),
    ("graphFocusOn", "graph_focus_on"),
    ("graphFocusIdle", "graph_focus_idle"),
    ("graphFocusOff", "graph_focus_off"),
    ("rangeFill", "range_fill"),
    ("statusOnText", "status_on_text"),
    ("statusAskText", "status_ask_text"),
    ("textTertiary", "ink_tertiary"),
    ("statusAsk", "status_ask"),
    ("statusOn", "status_on"),
    ("statusOff", "status_off"),
    ("tileFolder", "tile_folder"),
]


def render_glass_css(tokens: dict, scheme: str) -> str:
    palette = glass_palette(tokens, scheme)
    theme = tokens["themes"]["flat" + scheme.title()]
    radius = {**tokens["radius"], **theme.get("radius", {})}
    out = []
    for tier, css, _, _, _ in GLASS_TIERS:
        out.append(f"@define-color tc_glass_{snake(tier)} {palette[tier]};")
    for tier, css, _, _, _ in GLASS_TIERS:
        out.append(f"@define-color tc_glass_{snake(tier)}_inactive {palette[tier + 'Inactive']};")
    for key, name in GLASS_COLOURS:
        out.append(f"@define-color tc_glass_{name} {palette[key]};")
    for tier, css, radius_token, edge, _ in GLASS_TIERS:
        where = f"glass tier {tier}"
        out.append(
            f".tc-glass-{css} {{ background-color: @tc_glass_{snake(tier)}; "
            f"border-radius: {number(radius[radius_token], where)}px; "
            f"box-shadow: {css_shadow(flat_layers(tokens, edge, scheme), where)}; }}"
        )
    # Out of focus (GTK's :backdrop) every tier takes its fill over the
    # dimmed pane (`GlassPaneFill`'s inactiveDim, under the tiers).
    for tier, css, _, _, _ in GLASS_TIERS:
        out.append(f"window.tc-glass-window:backdrop .tc-glass-{css} "
                   f"{{ background-color: @tc_glass_{snake(tier)}_inactive; }}")
    # Increase Contrast: a solid 1pt stroke on every tier with edge layers
    # (`GlassEdgeModifier`), drawn as an inset shadow so, like the Mac's
    # strokeBorder, it takes no layout space; the tier keeps its own shadow.
    for tier, css, _, edge, _ in GLASS_TIERS:
        layers = flat_layers(tokens, edge, scheme)
        if layers:
            out.append(f"window.tc-high-contrast .tc-glass-{css} {{ box-shadow: inset 0 0 0 1px "
                       f"@tc_glass_edge_high_contrast, {css_shadow(layers, f'glass tier {tier}')}; }}")
    for role, entry in tokens["type"].items():
        body = tokens["type"]["body"]
        size = entry["size"] / body["size"]
        rule = [f"font-size: {size:.6g}em", f"font-weight: {GLASS_WEIGHTS[entry.get('weight', 'regular')]}"]
        rule.append(f"line-height: {entry['lineHeight'] / entry['size']:.6g}")
        if entry.get("tracking"):
            rule.append(f"letter-spacing: {number(entry['tracking'], role)}px")
        if entry.get("tabular"):
            rule.append('font-feature-settings: "tnum"')
        if entry.get("design") == "monospaced":
            rule.append("font-family: monospace")
        out.append(f".tc-type-{snake(role).replace('_', '-')} {{ {'; '.join(rule)}; }}")
    return "\n".join(out)


def render_glass_rust(tokens: dict) -> list[str]:
    out = ["/// Radii, spacing and sizes of the glass system, as the flat theme draws them.", "pub mod glass {"]
    theme = tokens["themes"]["flatLight"]
    radius = {**tokens["radius"], **theme.get("radius", {})}
    for mod, values in (("radius", radius), ("space", tokens["space"])):
        out.append(f"    pub mod {mod} {{")
        for key, value in values.items():
            if isinstance(value, bool) or not isinstance(value, int):
                raise TokenError(f"{mod}.{key}: {value!r} is not a whole number of px")
            out.append(f"        pub const {snake(key).upper()}: i32 = {value};")
        out.append("    }")
    # The graph's dot inks (`GlassBarGraph`): shared over kept, and the bar
    # labels' statusOff, per scheme, for cairo to draw with.
    out.append("    /// The graph's inks: shared and kept dots, and the bar labels.")
    out.append("    pub mod data {")
    for scheme in SCHEMES:
        for token, name in (("dataShared", "SHARED"), ("dataKept", "KEPT"), ("statusOff", "LABEL"),
                            ("textPrimary", "TEXT_PRIMARY")):
            hex_ = flat_color(tokens, token, scheme)["hex"].upper()
            out.append(f"        /// `{token}`, {scheme}.")
            out.append(f'        pub const {name}_{scheme.upper()}: &str = "{hex_}";')
    out.append("    }")
    # The selected tab's ember line and the halo behind its title
    # (`GlassRuleTabs.emberGlow`), drawn in cairo across the strip.
    out.append("    /// The tab strip's ember line and its halo, as colour and alpha.")
    out.append("    pub mod tabs {")
    for scheme in SCHEMES:
        ember = flat_color(tokens, "tabEmber", scheme)
        halo = flat_color(tokens, "tabEmberHalo", scheme)
        if ember.get("alpha", 1) != 1:
            raise TokenError(f"tabEmber {scheme}: expected an opaque ember")
        out.append(f"        /// `tabEmber`, {scheme}.")
        out.append(f'        pub const EMBER_{scheme.upper()}: &str = "{ember["hex"].upper()}";')
        out.append(f"        /// `tabEmberHalo`, {scheme}.")
        out.append(f'        pub const HALO_{scheme.upper()}: (&str, f64) = '
                   f'("{halo["hex"].upper()}", {number(halo.get("alpha", 1), "tabEmberHalo")});')
    out.append("    }")
    # A status dot's ink (`GlassStatus`), per scheme, for cairo.
    out.append("    /// Status dot inks: on, ask, off and outside.")
    out.append("    pub mod status {")
    for scheme in SCHEMES:
        for token, name in (("statusOn", "ON"), ("statusAsk", "ASK"), ("statusOff", "OFF"),
                            ("statusOutside", "OUTSIDE")):
            hex_ = flat_color(tokens, token, scheme)["hex"].upper()
            out.append(f"        /// `{token}`, {scheme}.")
            out.append(f'        pub const {name}_{scheme.upper()}: &str = "{hex_}";')
    out.append("    }")
    out.append("}")
    return out


def swift_string_literals(text: str) -> str:
    return "".join(re.findall(r'"([^"\n]*)"', text))


def parse_glyphs(source: str) -> list[dict]:
    """The toolbar glyphs from Glyphs.swift: the one copy of their path data."""
    frame = re.search(r"static let frame =\s*((?:\"[^\"\n]*\"\s*\+?\s*)+)", source)
    if frame is None:
        raise TokenError("Glyphs.swift: no shared frame")
    frame_data = swift_string_literals(frame.group(1))
    toolbar = re.search(r"let toolbar = CGRect\(x: 0, y: 0, width: (\d+), height: (\d+)\)", source)
    start = source.index("var artwork: Artwork")
    body = source[start:]
    out = []
    for match in re.finditer(r"case \.(\w+):\s*return Artwork\((.*?)\)\n(?=\s*(?:case \.|\}))", body, re.S):
        name, args = match.group(1), match.group(2)
        data_text = re.search(r"data:\s*(.*?),\s*viewBox:", args, re.S).group(1)
        data = swift_string_literals(data_text)
        if "Self.frame" in data_text:
            data = frame_data + data
        box = re.search(r"viewBox: (?:CGRect\(x: 0, y: 0, width: (\d+), height: (\d+)\)|toolbar)", args)
        if box.group(1):
            width, height = int(box.group(1)), int(box.group(2))
        else:
            width, height = int(toolbar.group(1)), int(toolbar.group(2))
        stroke = re.search(r"stroke: (nil|[\d.]+)", args).group(1)
        size = re.search(r"size: (?:CGSize\(width: (\d+), height: (\d+)\)|toolbar\.size)", args)
        drawn = (int(size.group(1)), int(size.group(2))) if size.group(1) else (width, height)
        out.append({"name": name, "data": data, "width": width, "height": height,
                    "stroke": None if stroke == "nil" else float(stroke), "size": drawn})
    return out


def glyph_svg(glyph: dict) -> str:
    """The glyph as an SVG: stroked in currentColor, or filled when it has no stroke."""
    paint = ('fill="none" stroke="currentColor" stroke-width="' + number(glyph["stroke"], glyph["name"])
             + '" stroke-linejoin="round"') if glyph["stroke"] is not None else 'fill="currentColor"'
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {glyph["width"]} {glyph["height"]}" '
            f'width="{glyph["size"][0]}" height="{glyph["size"][1]}"><path d="{glyph["data"]}" {paint}/></svg>')


def parse_view_box(text: str, where: str) -> tuple:
    box = re.search(r"CGRect\(x: ([\d.]+), y: ([\d.]+), width: ([\d.]+), height: ([\d.]+)\)", text)
    if box is None:
        raise TokenError(f"{where}: no viewBox")
    return tuple(int(v) if v.isdigit() else float(v) for v in box.groups())


def parse_logos(source: str) -> list[dict]:
    """The tool logos from GlassLogoPaths.swift: the one copy of their path data."""
    out = []
    pattern = r"static let (\w+) = GlassLogoArtwork\(\s*viewBox: (CGRect\([^)]*\)),\s*paths: \[(.*?)\]\s*\)"
    for match in re.finditer(pattern, source, re.S):
        name = match.group(1)
        paths = re.findall(r'"([^"\n]*)"', match.group(3))
        if not paths:
            raise TokenError(f"logo {name}: no paths")
        out.append({"name": name, "view_box": parse_view_box(match.group(2), name), "paths": paths})
    if not out:
        raise TokenError("GlassLogoPaths.swift: no logos")
    return out


def parse_brand_mark(source: str) -> dict:
    """The NEAR AI mark from GlassBrandMark.swift: its viewBox and path data."""
    box = re.search(r"static let viewBox = (CGRect\([^)]*\))", source)
    data = re.search(r'static let data =\s*"([^"\n]*)"', source)
    if box is None or data is None:
        raise TokenError("GlassBrandMark.swift: no mark")
    return {"view_box": parse_view_box(box.group(1), "brand mark"), "data": data.group(1)}


def rust_float(value) -> str:
    text = repr(float(value))
    return text


def render_logos_rust(logos: list[dict], mark: dict) -> list[str]:
    out = ["/// The tool logos and the NEAR AI mark, exported from `GlassLogoPaths.swift`",
           "/// and `GlassBrandMark.swift`: SVG path data in each one's own viewBox",
           "/// (x, y, width, height).",
           "pub mod logo {"]
    for logo in logos:
        const = snake(logo["name"]).upper()
        box = ", ".join(rust_float(v) for v in logo["view_box"])
        out.append(f"    pub const {const}_VIEW_BOX: [f64; 4] = [{box}];")
        # On one line where rustfmt would keep it on one, so `cargo fmt`
        # leaves the generated file as it is.
        one_line = f"    pub const {const}: &[&str] = &[" + ", ".join(f'"{p}"' for p in logo["paths"]) + "];"
        if len(one_line) <= 100:
            out.append(one_line)
        else:
            out.append(f"    pub const {const}: &[&str] = &[")
            for path in logo["paths"]:
                out.append(f'        "{path}",')
            out.append("    ];")
    box = ", ".join(rust_float(v) for v in mark["view_box"])
    out.append(f"    pub const BRAND_MARK_VIEW_BOX: [f64; 4] = [{box}];")
    out.append(f'    pub const BRAND_MARK: &str = "{mark["data"]}";')
    out.append("}")
    return out


def render_map_rust(tokens: dict) -> list[str]:
    out = ["/// The Traces map's colours, as the Mac's flat theme draws them: a hex",
           "/// colour and its alpha.",
           "pub mod map {",
           "    pub type Rgba = (&'static str, f64);"]
    for scheme in SCHEMES:
        out.append(f"    /// The {scheme} scheme.")
        out.append(f"    pub mod {scheme} {{")
        out.append("        use super::Rgba;")
        for name in MAP_COLOURS:
            colour = flat_color(tokens, name, scheme)
            alpha = rust_float(colour.get("alpha", 1))
            out.append(f'        pub const {snake(name).upper()}: Rgba = ("{colour["hex"].lower()}", {alpha});')
        stops = [resolve_scheme(stop, scheme) for stop in flat_value(tokens, "gradient", "heroCampaign", scheme)["stops"]]
        for label, stop in (("DEEP", stops[0]), ("BRIGHT", stops[-1])):
            alpha = rust_float(stop.get("alpha", 1))
            out.append(f'        pub const HERO_CAMPAIGN_{label}: Rgba = ("{stop["hex"].lower()}", {alpha});')
        out.append("    }")
    out.append("}")
    return out


def render_glyphs_rust(glyphs: list[dict]) -> list[str]:
    out = ["/// The Mac's toolbar glyphs, as SVG, exported from `Glyphs.swift`'s path data.", "pub mod glyph {"]
    for glyph in glyphs:
        const = snake(glyph["name"]).upper()
        out.append(f"    pub const {const}: &str = r#\"{glyph_svg(glyph)}\"#;")
    out.append("    /// Every glyph by its name in `GlassGlyph`.")
    out.append("    pub const ALL: &[(&str, &str)] = &[")
    for glyph in glyphs:
        out.append(f'        ("{glyph["name"]}", {snake(glyph["name"]).upper()}),')
    out.append("    ];")
    out.append("}")
    return out


# Home's hero promotions on the Mac (`GlassHeroCard`, `GlassHeroSymbol`):
# the three card grounds, the ink and pill on them, and the pixel graphic's
# three blues. The grounds are the same in both appearances; the blues are
# not. Every one of them is opaque, so the Linux shell takes hex alone.
HERO_GROUNDS = ["heroCampaignCard", "heroMissions", "heroInsights"]
HERO_INKS = ["heroInk", "heroButtonFill", "heroButtonInk"]
HERO_TINTS = ["dataKept", "chartSeries1", "firstRunBeamCore"]


def render_hero_rust(tokens: dict) -> list[str]:
    out = ["/// Home's hero cards, as the Mac's flat theme draws them: each ground's",
           "/// CSS angle and stops (offset, hex), the ink and pill on every ground, and",
           "/// the pixel graphic's three blues per scheme.",
           "pub mod hero {",
           "    pub type Ground = (i32, &'static [(f64, &'static str)]);"]
    for name in HERO_GROUNDS:
        ground = flat_value(tokens, "gradient", name, "dark")
        if ground != flat_value(tokens, "gradient", name, "light"):
            raise TokenError(f"hero {name}: a hero's ground is the same in both appearances")
        stops = []
        for stop in ground["stops"]:
            rgb(stop["hex"], name)
            if stop.get("alpha", 1) != 1 or "light" in stop:
                raise TokenError(f"hero {name}: expected opaque stops shared by both appearances")
            number(stop["at"], name)
            stops.append(f'({float(stop["at"])!r}, "{stop["hex"].lower()}")')
        out.append(f"    pub const {snake(name).upper()}: Ground = ({int(ground['angle'])}, &[{', '.join(stops)}]);")
    for name in HERO_INKS:
        colour = flat_color(tokens, name, "dark")
        if colour != flat_color(tokens, name, "light") or colour.get("alpha", 1) != 1:
            raise TokenError(f"hero {name}: expected one opaque ink in both appearances")
        out.append(f'    pub const {snake(name).upper()}: &str = "{colour["hex"].lower()}";')
    for scheme in SCHEMES:
        out.append(f"    /// The pixel graphic's blues, {scheme} scheme.")
        hexes = []
        for name in HERO_TINTS:
            colour = flat_color(tokens, name, scheme)
            if colour.get("alpha", 1) != 1:
                raise TokenError(f"hero {name}: expected an opaque tint")
            hexes.append(f'"{colour["hex"].lower()}"')
        out.append(f"    pub const TINTS_{scheme.upper()}: [&str; 3] = [{', '.join(hexes)}];")
    out.append("}")
    return out


# The Mac's small drawn controls (`GlassToolTile`, `GlassSpinner`,
# `GlassSkeleton`, `GlassRefreshButton`, `ManagedFolderHandoff`): their sizes
# and timings, and the colours they take that the map module does not carry.
TILE_SIZES = ["toolTile", "toolTileLarge", "toolLogo", "toolLogoLarge", "spinner", "spinnerStroke", "skeletonHeight"]
TILE_MOTION = ["spin", "pulse", "easeX1", "easeY1", "easeX2", "easeY2"]
TILE_COLOURS = ["tileFolder", "tileFolderInk", "ink", "hairline"]


def tile_number(value: object, where: str) -> str:
    number(value, where)
    return rust_float(value)


def render_tile_rust(tokens: dict) -> list[str]:
    out = ["/// The Mac's tool tiles, spinner, skeleton and refresh turn, as the flat",
           "/// theme draws them: sizes in points, times in seconds, the motion curve's",
           "/// control points, and per scheme the colours the map module lacks.",
           "pub mod tile {",
           "    pub type Rgba = (&'static str, f64);"]
    for name in TILE_SIZES:
        out.append(f"    pub const {snake(name).upper()}: f64 = {tile_number(tokens['size'][name], name)};")
    for name in ("tile", "control"):
        value = flat_value(tokens, "radius", name, "light")
        if value != flat_value(tokens, "radius", name, "dark"):
            raise TokenError(f"tile radius {name}: expected one value in both appearances")
        out.append(f"    pub const RADIUS_{snake(name).upper()}: f64 = {tile_number(value, name)};")
    for name in TILE_MOTION:
        out.append(f"    pub const {snake(name).upper()}: f64 = {tile_number(tokens['motion'][name], name)};")
    out.append(f"    pub const SKELETON_DIM: f64 = {tile_number(tokens['opacity']['skeletonDim'], 'skeletonDim')};")
    for scheme in SCHEMES:
        out.append(f"    /// The {scheme} scheme.")
        out.append(f"    pub mod {scheme} {{")
        out.append("        use super::Rgba;")
        for name in TILE_COLOURS:
            colour = flat_color(tokens, name, scheme)
            rgb(colour["hex"], name)
            out.append(f'        pub const {snake(name).upper()}: Rgba = ("{colour["hex"].lower()}", {tile_number(colour.get("alpha", 1), name)});')
        out.append("    }")
    out.append("}")
    return out


# The colours the Mac's animated first run draws its stage with
# (`FirstRunStage.swift`, `GlassBrandMarkDrawing`): the scene, the glow
# behind the N and the beams, the tool tiles and their tints, the orbs and
# the dashed rules, and the marker check's well and stroke
# (`FirstRunMarkerCheck`). The Linux stage reads them from
# `brand_tokens::first_run`.
FIRST_RUN_COLOURS = [
    "sceneBase", "sceneWarm", "firstRunGlow", "firstRunBeam", "firstRunBeamCore",
    "firstRunCard", "firstRunCardSolid", "firstRunTile", "firstRunTileShadow", "firstRunRule",
    "firstRunApprove", "firstRunApproveDeep", "heroInk", "wellFill",
    "textPrimary", "textSecondary", "textTertiary",
    "toolClaude", "toolCodex", "toolAntigravity", "toolOpenCode",
    "tintAccent", "toggleOn",
]


def render_first_run_rust(tokens: dict) -> list[str]:
    out = ["/// The first run's stage colours, as the Mac's flat theme draws them: a",
           "/// hex colour and its alpha; and the motion curve's control points.",
           "pub mod first_run {",
           "    pub type Rgba = (&'static str, f64);"]
    motion = tokens["motion"]
    ease = ", ".join(rust_float(motion[k]) for k in ("easeX1", "easeY1", "easeX2", "easeY2"))
    out.append("    /// The token ease's cubic Bezier control points (x1, y1, x2, y2).")
    out.append(f"    pub const EASE: [f64; 4] = [{ease}];")
    for scheme in SCHEMES:
        out.append(f"    /// The {scheme} scheme.")
        out.append(f"    pub mod {scheme} {{")
        out.append("        use super::Rgba;")
        for name in FIRST_RUN_COLOURS:
            colour = flat_color(tokens, name, scheme)
            alpha = rust_float(colour.get("alpha", 1))
            out.append(f'        pub const {snake(name).upper()}: Rgba = ("{colour["hex"].lower()}", {alpha});')
        out.append("    }")
    out.append("}")
    return out


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
    out.append("/// Insights layout roles shared with the macOS Insights layout.")
    out.append("pub mod insights {")
    for role, token in [("SECTION_GAP", "s8"), ("HEADING_GAP", "s4"),
                        ("LINE_GAP", "s2"), ("BODY_INSET", "s5")]:
        out.append(f"    pub const {role}: i32 = {tokens['space'][token]};")
    out.append("}")
    for scheme in SCHEMES:
        out.append(f'pub const INSIGHTS_{scheme.upper()}_CSS: &str = r#"')
        out.append(render_insights_css(tokens, scheme))
        out.append('"#;')
    out.append("")
    for scheme in SCHEMES:
        out.append(f"/// The painted Monitor window's tiers, type roles and inks, {scheme} scheme.")
        out.append(f'pub const GLASS_{scheme.upper()}_CSS: &str = r#"')
        out.append(render_glass_css(tokens, scheme))
        out.append('"#;')
    for scheme in SCHEMES:
        out.append(f"/// The translucent window's colours over the painted ones, {scheme} scheme,")
        out.append("/// loaded after them while KDE blur is granted (`ui::glass_backdrop`).")
        out.append(f'pub const GLASS_{scheme.upper()}_BLUR_CSS: &str = r#"')
        out.append(render_glass_blur_css(tokens, scheme))
        out.append('"#;')
        out.append(f"/// Insights over the translucent pane, {scheme} scheme.")
        out.append(f'pub const INSIGHTS_{scheme.upper()}_BLUR_CSS: &str = r#"')
        out.append(render_insights_blur_css(tokens, scheme))
        out.append('"#;')
    out.append("")
    out.extend(render_glass_rust(tokens))
    out.append("")
    out.extend(render_hero_rust(tokens))
    out.append("")
    out.extend(render_tile_rust(tokens))
    out.append("")
    out.extend(render_glyphs_rust(parse_glyphs(GLYPHS.read_text())))
    out.append("")
    out.extend(render_logos_rust(parse_logos(LOGOS.read_text()), parse_brand_mark(BRAND_MARK.read_text())))
    out.append("")
    out.extend(render_map_rust(tokens))
    out.append("")
    out.extend(render_first_run_rust(tokens))
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
