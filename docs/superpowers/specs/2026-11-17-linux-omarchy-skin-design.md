# Linux app: Omarchy skin

Date: 2026-11-17. Status: draft for owner review. Base: `linux-mac-look`
(the Mac-style window stack, #2160-#2165). Input: the owner's design
excerpt of 2026-11-17 (Omarchy integration, approaches, touchpoints,
acceptance criteria) and the owner's request for "an omarchy skin version of
the app as part of the linux portfolio", tested against an Omarchy instance.

## Decisions to confirm

Each is a default chosen for this draft. None is approved yet.

| # | Decision | Default in this PR | Alternatives |
| --- | --- | --- | --- |
| D1 | Integration approach | App-owned Omarchy mode: the app reads the active `colors.toml` itself, the excerpt's recommendation | An Omarchy template (`trace-commons.css.tpl`) generating a CSS fragment; generic light/dark only |
| D2 | Where the choice lives | A new Appearance card at the end of the Startup section of Settings. `Section::ALL` is pinned to the macOS groups by a test, so no new navigation entry | A new Settings section (needs a shared nav label and a macOS counterpart) |
| D3 | When the choice is offered | Always, on every Linux desktop. Choosing Omarchy where no Omarchy theme exists shows the fallback line and keeps the standard look | Offer it only when an Omarchy state directory exists |
| D4 | Flatpak | **No new filesystem access in this PR.** In the Flatpak the palette is unreadable, so Omarchy mode shows the fallback line. See "Flatpak options" for the owner's pick | See below |
| D5 | Words | New UI text lives in a const-only shared module, `trace_commons_contributor::appearance_copy`, every entry marked DRAFT, NEEDS APPROVAL. It is outside `ShellWordsCopy` and the C ABI, so the Swift and C# fixtures do not move | Add the fields to `SettingsWords` once macOS or Windows want them |
| D6 | The fallback line | One caption under the choice, shown only while Omarchy is chosen and no valid palette has been read since launch. It is the "concise explanation in Appearance settings" the excerpt asks for; no banner, toast, notification or status channel | Show nothing and only keep the standard look |
| D7 | Fonts | Not changed. `colors.toml` carries no font; Omarchy's font is a system-wide monospace choice the app already inherits through fontconfig | Read Omarchy's font setting |
| D8 | Brand panel and illustration | The community-site brand panel (`community_brand.rs`, the public profile in the site's own visual language) and the map's fixed illustration colours (rocket flame, smoke, the white cores of transfer orbs) stay as they are in Omarchy mode | Recolour them too |
| D9 | Legacy Omarchy | Also read the pre-4.x layout (`~/.config/omarchy/current/theme`, ANSI `color0..15` keys), behind the 4.x layout | 4.x only |

## What was confirmed upstream, and what is assumed

Fetched 2026-11-17 from `omacom/omarchy`. The default branch is `quattro`
(at `3437cfb482d603a34c1292db2d425cacd0f80281`); the latest release, `v4.5.0`
(tag object `97e664bd971fa5a8691d89440d3738143499c438`), is cut from it.
`master` (`f4378f0de5b44d331ee943746a97872b718a6c18`, last commit
2026-08-14) is the older 1.x-3.x line. Omarchy is MIT-licensed.

Confirmed from source:

- **Palette path.** `bin/omarchy-theme-set` and `bin/omarchy-theme-color`
  hardcode `$HOME/.local/state/omarchy/current/theme/colors.toml`. They do
  NOT consult `XDG_STATE_HOME`. On `master` the same tree is under
  `$HOME/.config/omarchy/current/theme`.
- **Keys.** Flat TOML, all values quoted `#rrggbb`: `mode`, `accent`,
  `selection`, `muted`, `background`, `dark_background`,
  `darker_background`, `lighter_background`, `foreground`,
  `dark_foreground`, `light_foreground`, `bright_foreground`, `red`,
  `yellow`, `orange`, `green`, `cyan`, `blue`, `magenta`, `brown`, and
  `bright_*` twins. Checked in tokyo-night and gruvbox (dark),
  catppuccin-latte, rose-pine and flexoki-light (light). catppuccin-latte
  on `quattro` also has `icon_mark`, absent at `v4.5.0`. `master` themes use
  `accent`, `cursor`, `foreground`, `background`, `selection_foreground`,
  `selection_background` and `color0..color15`.
- **Mode.** `omarchy-theme-color`'s precedence: the `mode` key, the legacy
  `theme_type` key, a `light.mode` file beside `colors.toml`, then
  background luminance (sum of the three 8-bit channels above 382 is
  light), else dark. `omarchy-theme-set-gnome` maps it to
  `org.gnome.desktop.interface color-scheme` `prefer-light`/`prefer-dark`.
- **Aliases.** The same script resolves legacy names before deriving:
  `bg`/`fg`/`lighter_bg`/... for the canonical names, `color0` for
  `background`, `color7` for `foreground`, `color1..6` and `color9..14`
  for the hues, `color8` for `muted`, `selection_background` for
  `selection`.
- **Switching is a directory rename, not a link swap.** `omarchy-theme-set`
  stages the new theme in `current/next-theme`, then runs
  `rm -rf current/theme; mv current/next-theme current/theme`, and writes
  `current/theme.name` and `current/theme.generation`. A watch on the theme
  directory itself dies with it; the watch has to sit on `current/`.
- **Fonts.** `manual/38-fonts.md`: JetBrainsMono Nerd Font is the default
  terminal and system font, chosen in the Omarchy menu, set as the
  monospace font everywhere. Not part of the palette.
- **Templates.** `omarchy-theme-set-templates` renders
  `~/.config/omarchy/themed/*.tpl` into the next theme directory, with
  `{{ key }}` substitutions. Not used by D1.

Assumed, to be checked on the VM:

- A fresh Omarchy 4.x install writes `~/.local/state/omarchy/current/theme`
  as a real directory (the scripts say so; not yet observed on a machine).
- GLib's inotify backend reports the `rm -rf`/`mv` pair on `current/` as
  delete and move-in (or create) events. The design does not depend on
  which: any event re-resolves.
- Flatpak: the sandbox's `XDG_STATE_HOME` is the app's own
  (`~/.var/app/<id>/.local/state`), and the host's `~/.local/state` is not
  readable without a grant (the manifest grants only the two session roots).

## Resolution

Candidates for the theme directory, first with a readable `colors.toml`
wins:

1. `$XDG_STATE_HOME/omarchy/current/theme`, when `XDG_STATE_HOME` is set and
   absolute (the excerpt's XDG convention);
2. `$HOME/.local/state/omarchy/current/theme` (what Omarchy 4.x writes);
3. `$XDG_CONFIG_HOME/omarchy/current/theme`, when set and absolute;
4. `$HOME/.config/omarchy/current/theme` (pre-4.x).

Duplicates are removed. Resolution is a pure function of the environment
values passed in, so tests never read the real home. The file is read with
a 64 KiB cap; nothing in the theme directory is executed, sourced or
followed beyond the one read.

## Role mapping

Parsing: the file must be valid TOML. Only string values that are exactly
`#rrggbb` count; anything else under a key is treated as absent. Legacy
aliases are applied as above. `background` and `foreground` are required;
without both the palette is invalid.

| Stack role | Omarchy source | Derivation when absent | Floor |
| --- | --- | --- | --- |
| `tc_bg`, glass pane | `background` | required | |
| `tc_surface`, glass card, card-quiet, control, popover, menu, node card | `lighter_background` | mix(bg, fg, 6%) | |
| `tc_surface2` (inset), glass well | `dark_background` | mix(bg, fg, 10%) | |
| glass scene (map) | `darker_background` | mix(bg, black, 25%) dark / mix(bg, fg, 8%) light | |
| control hover / row hover | derived | mix(surface, fg, 8%) / mix(bg, fg, 6%) | |
| `tc_ink`, glass ink, toolbar glyph | `foreground` | required | 4.5 on every ground |
| strong text (feeds selected text only; no `@define-color` uses it) | `bright_foreground` | foreground | 4.5 on every ground |
| `tc_muted`, glass secondary ink | derived: mix(fg, bg, 30%) | `light_foreground` is not used: whether it sits closer to or further from the background than `foreground` differs from theme to theme | 4.5 on every ground |
| `tc_ink_tertiary`, hidden toolbar glyph | derived: mix(fg, bg, 40%) | | 4.5 on every ground |
| `tc_line`, `tc_line_divider` (decorative) | `muted` | dark_foreground, then mix(bg, fg, 20%) | none (decorative) |
| `tc_accent` (fill), glass selected control, tab ember | `accent` | `blue` | 3 on bg and surface; 4.5 against `on_accent` |
| `tc_on_accent`, glass selected ink | chosen | the candidate (bg, bright fg, fg, white, black) needing the smallest accent shift | |
| `tc_accent_line` | accent | | 3 on every ground |
| `tc_accent_text`, `accent_color`, focus | accent | | 4.5 on every ground |
| `tc_status_on` / `_text` | `green` | | 3 / 4.5 on every ground |
| `tc_status_ask` / `_text` | `yellow` | | 3 / 4.5 |
| `tc_status_outside` / `_text` | `red` | | 3 / 4.5 |
| held (`tc_blue`, `tc_blue_text`, `tc_blue_icon`) | `blue` | accent | 3 / 4.5 |
| `tc_on_status` | background | | 4.5 against each status fill |
| text selection background (scoped rule only) | `selection` | mix(bg, accent, 25%) | ink 4.5 on it |
| `tc_selected` (selected rows), `tc_scrim` | kept: the standard neutral translucent overlays | | every ink keeps its contrast through them |
| selection text | `bright_foreground` | | 4.5 on selection |
| redaction wash / ink | mix(bg, yellow, 25%) / ink | | 4.5 |
| gold highlight (preview excerpt) | yellow at 30% alpha | | |

"Every ground" is bg, surface, inset, scene and the two hover tiers.

**Contrast derivation.** WCAG 2.1 relative luminance. A colour that misses
its floor is moved toward black and toward white in 2% steps; the smaller
move that clears every constraint wins, so the theme's hue survives where it
can. A palette where neither end clears a floor is rejected as a whole (it
is kept out, the last valid palette stays). Strong ink is moved the same
way. Fills are solved jointly against their label: for the accent, each
label candidate is tried and the one needing the smallest move of the fill
wins (measured, not assumed from mode). Status fills are moved until
`on_status` (the background) clears 4.5 on them and they clear 3 on every
ground. In light mode the standard sheet labels status fills white; the
skin labels them `on_status` instead.

Measured on the fixtures: Tokyo Night's accent and status hues already
clear every floor and are used unchanged; Gruvbox's accent clears its
ground but not the hovered control (4.1:1) and is lifted a few steps; Rose
Pine's accent is darkened for text on its light ground.

**Mode.** `mode`, else `theme_type`, else a `light.mode` file, else the
luminance rule, as upstream. While Omarchy mode is on, libadwaita's
`StyleManager` is forced to that scheme (so stock widgets follow), and the
stylesheet's structure is chosen by it (the dark or light token block).
Switching back to Standard restores `ColorScheme::Default`, or the
`TRACE_COMMONS_APPEARANCE` development hook if set.

## CSS and drawing

`@define-color` cannot be scoped to a selector. Omarchy mode therefore
replaces values, not structure: the generated brand block, the glass tiers,
the token block and the Insights block are the standard ones with every
colour definition and every literal colour in a rule rewritten from the
resolved palette, by role name. A test pins that the Omarchy sheet defines
exactly the same `@define-color` names as the standard one, and that no
literal from the standard palette survives. Rules that only make sense in
the skin (text selection colours) are scoped under `.tc-omarchy`, a class
the shell puts on every toplevel window while Omarchy mode is on (including
windows opened later). With Standard chosen the stylesheet is byte-identical
to today's, which a test pins.

One accessor, `style::inks()`, returns the colours custom drawing uses;
in Standard it returns today's constants. Routed through it: the mark
(frame, hairline, open bracket, close bracket, template ink), History's
status glyphs, the map's accent and status dots, and the preview's
redaction tags and highlighted excerpt. The graph footer and the monitor
glyphs already draw in the widget's CSS colour and follow without change.
After each reload the shell redraws every drawing area in every window,
because a dark-to-dark switch fires no `dark` notification.

Known limit: the transcript preview's redaction tags and highlighted
excerpt take their colours when the preview is rendered. A preview already
open keeps the old colours until it is opened again; everything else
follows a switch live.

Off is off: with Standard chosen the loaded sheet is `stylesheet(dark)`,
and the branch leaves that function, the token blocks, `style.css` and
`brand_tokens.rs` untouched (no diff against `linux-mac-look`), so the
standard look is byte-identical. Whether to reload is decided by comparing
the palette the provider holds with the one that should show, so turning
the skin off and on again with an unchanged theme loads it again.

## Live reload

A `gio::FileMonitor` on each candidate's `current/` directory (or its
nearest existing ancestor, so a later Omarchy install is noticed) and one on
each candidate theme directory that exists, so an edit in place is noticed
whether or not the last read succeeded. Watching only the palette that read
cleanly missed a broken or deleted `colors.toml` fixed without a theme
switch; that was seen on the Omarchy 4.5 test box. Any event schedules a re-resolve 300 ms later;
events inside that window restart it, which absorbs the `rm`/`mv` pair. A
re-resolve re-reads, re-arms the monitors on whatever now exists, and, on a
valid palette that differs from the current one, reloads the one CSS
provider on the main thread and redraws. A missing or invalid palette keeps
the last valid one; with none since launch the standard look stays and the
fallback line shows. Monitors exist only while Omarchy mode is on.

## Preference

`appearance.json` beside the window's `monitor-window.json`
(`$XDG_STATE_HOME/trace-commons/`), the same small-file pattern as
`monitor/prefs.rs`: `{"appearance": "standard" | "omarchy"}`. Absent or
damaged is Standard; a failed write is dropped. It is the shell's own
preference, not a daemon setting. `TRACE_COMMONS_APPEARANCE=omarchy` turns
the mode on for a capture run without touching the file.

## Flatpak options (owner decision D4)

The manifest grants `--filesystem` only for the two session roots, read-only.
Ordered from narrowest:

1. **Explicit import**: a file chooser (the portal hands back one document)
   copies a `colors.toml` into the app's own state. No standing access; no
   live switching.
2. **Read-only grant of the theme tree only**:
   `--filesystem=~/.local/state/omarchy/current:ro`. Live switching works.
   Grants read of the whole current theme (backgrounds, generated configs),
   colour data only by upstream's own staging rule.
3. **Through the host daemon**: the contributor daemon runs on the host
   and could hand the palette over its socket. Puts cosmetic code into the
   daemon, which the excerpt rules out ("keep theme logic in the Linux
   shell").

This PR does none of them.

## Words (DRAFT, NEEDS APPROVAL)

In `trace_commons_contributor::appearance_copy`:

- `APPEARANCE_HEADING`: "Appearance"
- `APPEARANCE_STANDARD`: "Standard"
- `APPEARANCE_OMARCHY`: "Omarchy"
- `APPEARANCE_OMARCHY_UNAVAILABLE`: "No Omarchy theme found. Using the standard appearance."

## Where it lives

- `crates/trace-commons-contributor-gtk/src/ui/omarchy.rs`: the pure adapter
  (parse, aliases, mode, derivation, contrast, path candidates, last-valid
  retention).
- `.../src/ui/appearance.rs`: the preference file, state, monitors,
  debounce, window class, redraw, and the Settings card.
- `.../src/ui/style.rs`: `omarchy_stylesheet`, `sheet_for`, `reload`, and
  `Inks` with `style::inks()`.
- Drawing sites: `ui/mark.rs`, `ui/history.rs` (`Glyph::ink`),
  `ui/monitor/map.rs` (`Palette::with_inks`), `ui/preview.rs`
  (redaction tag, `Excerpt::markup_with`).
- `crates/trace-commons-contributor/src/appearance_copy.rs`: the words.
- `toml = "0.9"` declared in the GTK crate: already in its tree at that
  version with default features; `Cargo.lock` gains only the edge.

## Tests

- Adapter, over the vendored fixtures (`tests/fixtures/omarchy/`, MIT, with
  provenance): parsing, legacy aliases, validation of hex, mode precedence,
  derivation, and that every role pair meets its floor for every fixture.
- Malformed TOML, a missing core colour, an invalid hex: rejected; the state
  keeps the last valid palette; a later valid palette replaces it.
- Path resolution from injected env values.
- The Omarchy stylesheet for every fixture: same define names, no standard
  literal left, the existing contrast matrix in `style.rs` holds, and it
  parses with GTK's CSS parser (the parse test is `#[ignore]`d like the
  existing display tests; `scripts/weston-portal-verify-inner.sh` runs it
  beside `both_schemes_parse_with_insights_styles` in the Linux weston job).
- Drawing: `inks()` in Standard equals today's constants; with a palette it
  returns the palette's colours, and the map palette, History glyphs and the
  mark read it.
- Preference: round trip, damaged file, reverse to Standard.
- `tests/shell_wording.rs` passes without raising any baseline.

## Testing on Omarchy (GCP VM)

An Omarchy 4.x VM in the pilot GCP project, hardened like the existing Linux
test VMs. The checklist is in the PR. Native build only on the VM; the
Flatpak is checked for the fallback line.

## Out of scope

- Hyprland window rules (`looknfeel.lua` rounding, gaps, borders, opacity),
  the Waybar/tray look, and window decorations Hyprland draws.
- Fonts (D7), icons, the background image.
- The template approach (D1 alternative).
- The macOS, Windows and Tauri shells.
- Any change to daemon, sharing, enrolment, inference or autostart
  behaviour.
