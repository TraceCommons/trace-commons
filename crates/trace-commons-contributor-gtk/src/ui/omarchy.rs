//! The Omarchy palette adapter: the active theme's `colors.toml` in, a
//! validated palette for this shell's semantic roles out.
//!
//! Pure: no GTK, no global state. `ui::appearance` owns the preference, the
//! file monitors and the reload; `ui::style` turns a [`Palette`] into CSS and
//! drawing inks. See `docs/superpowers/specs/2026-11-17-linux-omarchy-skin-design.md`.
//!
//! What is read, and from where, follows Omarchy's own resolver
//! (`bin/omarchy-theme-color` on its `quattro` branch): the same key
//! aliases, the same mode precedence. Only the file is read. Nothing in the
//! theme directory is executed, sourced or interpreted beyond its colours.

use std::path::{Path, PathBuf};

/// The largest `colors.toml` this adapter reads. Upstream's are under 1 KiB.
pub const MAX_PALETTE_BYTES: u64 = 64 * 1024;

/// An opaque sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);
    pub const WHITE: Rgb = Rgb(255, 255, 255);

    /// Exactly `#rrggbb`, either case. Nothing else is a colour here.
    pub fn parse(text: &str) -> Option<Rgb> {
        let hex = text.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Rgb(channel(0)?, channel(2)?, channel(4)?))
    }

    /// `#RRGGBB`.
    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }

    /// The channels as cairo wants them.
    pub fn unit(self) -> (f64, f64, f64) {
        (
            f64::from(self.0) / 255.0,
            f64::from(self.1) / 255.0,
            f64::from(self.2) / 255.0,
        )
    }

    /// WCAG 2.1 relative luminance.
    pub fn luminance(self) -> f64 {
        let linear = |c: u8| {
            let v = f64::from(c) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.0) + 0.7152 * linear(self.1) + 0.0722 * linear(self.2)
    }

    /// `self` moved `t` (0..=1) of the way to `other`, per channel.
    pub fn mix(self, other: Rgb, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

/// WCAG 2.1 contrast ratio, 1..=21.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Body text.
pub const TEXT: f64 = 4.5;
/// Meaningful boundaries, fills, rules and focus indicators.
pub const NON_TEXT: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

/// Every role this shell draws with, resolved and contrast-checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub mode: Mode,
    /// Window ground, the glass pane.
    pub bg: Rgb,
    /// Raised panel: cards, controls, popovers, menus.
    pub surface: Rgb,
    /// Inset surface, the glass well.
    pub inset: Rgb,
    /// The map's scene.
    pub scene: Rgb,
    pub control_hover: Rgb,
    pub row_hover: Rgb,
    pub ink: Rgb,
    pub strong: Rgb,
    /// Supporting text.
    pub muted: Rgb,
    pub tertiary: Rgb,
    /// Decorative hairline. No contrast floor.
    pub line: Rgb,
    /// Accent fill.
    pub accent: Rgb,
    /// The label on an accent fill.
    pub on_accent: Rgb,
    /// Accent rule or glyph on a ground.
    pub accent_line: Rgb,
    /// Accent text on a ground, and the focus colour.
    pub accent_text: Rgb,
    pub status_on: Rgb,
    pub status_on_text: Rgb,
    pub status_ask: Rgb,
    pub status_ask_text: Rgb,
    pub status_outside: Rgb,
    pub status_outside_text: Rgb,
    pub held: Rgb,
    pub held_text: Rgb,
    /// The label on a status fill.
    pub on_status: Rgb,
    pub selection: Rgb,
    pub selection_ink: Rgb,
    pub redaction_bg: Rgb,
    pub redaction_ink: Rgb,
    /// The search hit's wash, drawn at [`HIGHLIGHT_ALPHA`]: the theme's
    /// yellow, unmoved wherever an ink can be solved on it, so the hit
    /// stays marked.
    pub highlight: Rgb,
    /// The hit's text on that wash: the ink, moved until it clears 4.5:1 on
    /// the wash over every ground, bare and under each kept overlay.
    pub highlight_ink: Rgb,
    /// A control on a selected row (the Mac's `controlOnSelection`): the
    /// accent fill moved away from its label, so [`Palette::on_accent`]
    /// reads on it at least as well as on the accent, and its hover.
    pub control_on_selection: Rgb,
    pub control_on_selection_hover: Rgb,
}

/// The highlighted excerpt's wash, as Pango's `bgalpha`.
pub const HIGHLIGHT_ALPHA: &str = "30%";
/// [`HIGHLIGHT_ALPHA`] as a fraction, for CSS and for the contrast check.
pub const HIGHLIGHT_WASH: f64 = 0.30;
/// The least the wash must stand off its ground, so the hit stays marked.
/// Measured: the fixtures' own yellows at 30% clear it on every ground and
/// overlay (Rose Pine's scene under `tc_selected` is 1.12:1), the standard
/// gold 1.32-1.96:1, and a yellow darkened until a light ink cleared 4.5:1
/// on it 1.01-1.14:1 on Tokyo Night, where the hit no longer showed.
pub const HIGHLIGHT_VISIBLE: f64 = 1.1;

/// The standard sheet's neutral translucent overlays, kept in the skin:
/// `tc_scrim` (the `.tc-code` recess, among others) and `tc_selected`, as
/// (tint, alpha) for the token block `mode` chooses. A test in `ui::style`
/// pins them to the sheet.
pub fn overlays(mode: Mode) -> [(Rgb, f64); 2] {
    match mode {
        Mode::Light => [(Rgb::BLACK, 0.06), (Rgb::BLACK, 0.07)],
        Mode::Dark => [(Rgb::WHITE, 0.08), (Rgb::WHITE, 0.10)],
    }
}

/// Why a palette was not accepted. The variants carry no file content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteError {
    Unreadable,
    TooLarge,
    NotToml,
    Missing(&'static str),
    Unreachable(&'static str),
}

impl Palette {
    /// The grounds every text and rule role is checked against.
    pub fn grounds(&self) -> [Rgb; 6] {
        [
            self.bg,
            self.surface,
            self.inset,
            self.scene,
            self.control_hover,
            self.row_hover,
        ]
    }
}

/// Parse a `colors.toml`. `light_marker` is whether a `light.mode` file sits
/// beside it (the legacy mode marker).
pub fn parse(text: &str, light_marker: bool) -> Result<Palette, PaletteError> {
    let table: toml::Table = text.parse().map_err(|_| PaletteError::NotToml)?;
    let colour = |key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_str)
            .and_then(Rgb::parse)
    };
    // The canonical name first, then upstream's legacy aliases for it.
    let pick = |keys: &[&str]| keys.iter().find_map(|key| colour(key));
    // Upstream keeps any non-empty value and stops there, and its consumers
    // read only "light" as light: an unknown value (or a non-string one,
    // which upstream reads as its bare text) is dark.
    let word = |key: &str| match table.get(key) {
        None => None,
        Some(toml::Value::String(value)) if value.is_empty() => None,
        Some(toml::Value::String(value)) if value == "light" => Some(Mode::Light),
        Some(_) => Some(Mode::Dark),
    };

    let bg = pick(&["background", "bg", "color0"]).ok_or(PaletteError::Missing("background"))?;
    let fg = pick(&["foreground", "fg", "color7"]).ok_or(PaletteError::Missing("foreground"))?;

    let mode = word("mode")
        .or_else(|| word("theme_type"))
        .or(light_marker.then_some(Mode::Light))
        .unwrap_or({
            let sum = u32::from(bg.0) + u32::from(bg.1) + u32::from(bg.2);
            if sum > 382 { Mode::Light } else { Mode::Dark }
        });

    let surface = pick(&["lighter_background", "lighter_bg"]).unwrap_or(bg.mix(fg, 0.06));
    let inset = pick(&["dark_background", "dark_bg"]).unwrap_or(bg.mix(fg, 0.10));
    let scene = pick(&["darker_background", "darker_bg"]).unwrap_or(match mode {
        Mode::Dark => bg.mix(Rgb::BLACK, 0.25),
        Mode::Light => bg.mix(fg, 0.08),
    });
    let control_hover = surface.mix(fg, 0.08);
    let row_hover = bg.mix(fg, 0.06);
    let grounds = [bg, surface, inset, scene, control_hover, row_hover];
    let on = |floor: f64| grounds.map(|g| (g, floor));

    let ink = need("ink", fg, &on(TEXT))?;
    let strong_src = pick(&["bright_foreground", "bright_fg", "color15"]).unwrap_or(fg);
    let strong = need("strong", strong_src, &on(TEXT))?;
    let muted = need("muted", fg.mix(bg, 0.30), &on(TEXT))?;
    let tertiary = need("tertiary", fg.mix(bg, 0.40), &on(TEXT))?;
    let line = pick(&["muted", "color8", "dark_foreground", "dark_fg"]).unwrap_or(bg.mix(fg, 0.20));

    let blue_src = pick(&["blue", "color4"]);
    let accent_src = pick(&["accent"]).or(blue_src).unwrap_or(fg);
    let (accent, on_accent) = accent_pair(accent_src, bg, surface, [bg, strong, ink])?;
    let accent_line = need("accent", accent_src, &on(NON_TEXT))?;
    let accent_text = need("accent", accent_src, &on(TEXT))?;

    let on_status = bg;
    let status = |role: &'static str, hue: Rgb| -> Result<(Rgb, Rgb), PaletteError> {
        let mut fill_need = on(NON_TEXT).to_vec();
        fill_need.push((on_status, TEXT));
        let fill = need(role, hue, &fill_need)?;
        let mut text_need = on(TEXT).to_vec();
        text_need.push((on_status, TEXT));
        Ok((fill, need(role, hue, &text_need)?))
    };
    let green = pick(&["green", "color2"]).unwrap_or(accent_src);
    let yellow = pick(&["yellow", "color3"]).unwrap_or(accent_src);
    let red = pick(&["red", "color1"]).unwrap_or(accent_src);
    let (status_on, status_on_text) = status("green", green)?;
    let (status_ask, status_ask_text) = status("yellow", yellow)?;
    let (status_outside, status_outside_text) = status("red", red)?;
    let blue = blue_src.unwrap_or(accent_src);
    let held = need("blue", blue, &on(NON_TEXT))?;
    let held_text = need("blue", blue, &on(TEXT))?;

    let selection_src =
        pick(&["selection", "selection_background"]).unwrap_or(bg.mix(accent_src, 0.25));
    let selection = need("selection", selection_src, &[(ink, TEXT)])?;
    let selection_ink = need("selection", strong, &[(selection, TEXT)])?;
    let redaction_bg = bg.mix(yellow, 0.25);
    let redaction_ink = need("redaction", ink, &[(redaction_bg, TEXT)])?;
    // The search hit: the theme's yellow, unmoved, washed at its alpha, and
    // the hit's own ink solved on that wash over every ground, bare and
    // under each kept overlay, as the redaction ink is on its wash. Where
    // no ink clears every wash, or the unmoved wash would not stand
    // out from its ground (a pale yellow on a light one), the ink stays and the wash colour moves instead.
    let bases: Vec<Rgb> = grounds
        .iter()
        .flat_map(|g| {
            std::iter::once(*g).chain(overlays(mode).map(|(tint, alpha)| g.mix(tint, alpha)))
        })
        .collect();
    let washes: Vec<(Rgb, f64)> = bases
        .iter()
        .map(|base| (base.mix(yellow, HIGHLIGHT_WASH), TEXT))
        .collect();
    let unmoved_visible = bases
        .iter()
        .all(|base| contrast(base.mix(yellow, HIGHLIGHT_WASH), *base) >= HIGHLIGHT_VISIBLE);
    let (highlight, highlight_ink) = match nearest_passing(ink, &washes) {
        Some((solved, _)) if unmoved_visible => (yellow, solved),
        _ => {
            let moved =
                moved_wash(ink, yellow, &bases).ok_or(PaletteError::Unreachable("yellow"))?;
            (moved.0, ink)
        }
    };

    // Black under a light label, white under a dark one.
    let away = if on_accent.luminance() > 0.18 {
        Rgb::BLACK
    } else {
        Rgb::WHITE
    };
    Ok(Palette {
        mode,
        bg,
        surface,
        inset,
        scene,
        control_hover,
        row_hover,
        ink,
        strong,
        muted,
        tertiary,
        line,
        accent,
        on_accent,
        accent_line,
        accent_text,
        status_on,
        status_on_text,
        status_ask,
        status_ask_text,
        status_outside,
        status_outside_text,
        held,
        held_text,
        on_status,
        selection,
        selection_ink,
        redaction_bg,
        redaction_ink,
        highlight,
        highlight_ink,
        control_on_selection: accent.mix(away, 0.16),
        control_on_selection_hover: accent.mix(away, 0.26),
    })
}

/// Steps toward black or white a colour may take, 2% each.
const STEPS: u32 = 50;

/// `colour`, or the nearest colour on its way to black or to white that
/// clears every `(against, floor)` pair, with how many steps it moved.
/// Unmoved when it already clears them, so a theme's own colour survives
/// wherever it can.
fn nearest_passing(colour: Rgb, needs: &[(Rgb, f64)]) -> Option<(Rgb, u32)> {
    nearest_by(colour, |c| {
        needs
            .iter()
            .map(|(against, floor)| contrast(c, *against) / floor)
            .fold(f64::INFINITY, f64::min)
    })
}

/// [`nearest_passing`] for any measure: `worst` is the smallest
/// ratio-over-floor a candidate colour gives, passing at 1 or more.
fn nearest_by(colour: Rgb, worst: impl Fn(Rgb) -> f64) -> Option<(Rgb, u32)> {
    (0..=STEPS).find_map(|step| {
        let t = f64::from(step) / f64::from(STEPS);
        [Rgb::BLACK, Rgb::WHITE]
            .into_iter()
            .map(|end| colour.mix(end, t))
            .filter(|c| worst(*c) >= 1.0)
            .max_by(|a, b| worst(*a).total_cmp(&worst(*b)))
            .map(|c| (c, step))
    })
}

/// The fallback wash colour: the nearest to `yellow` where `ink` clears
/// [`TEXT`] on the wash over every base and the wash still stands
/// [`HIGHLIGHT_VISIBLE`] off that base. A colour that fails either is
/// refused, never returned.
fn moved_wash(ink: Rgb, yellow: Rgb, bases: &[Rgb]) -> Option<(Rgb, u32)> {
    nearest_by(yellow, |c| {
        bases
            .iter()
            .map(|base| {
                let wash = base.mix(c, HIGHLIGHT_WASH);
                (contrast(ink, wash) / TEXT).min(contrast(wash, *base) / HIGHLIGHT_VISIBLE)
            })
            .fold(f64::INFINITY, f64::min)
    })
}

fn need(role: &'static str, colour: Rgb, needs: &[(Rgb, f64)]) -> Result<Rgb, PaletteError> {
    nearest_passing(colour, needs)
        .map(|(c, _)| c)
        .ok_or(PaletteError::Unreachable(role))
}

/// The accent fill and the label on it, solved together. Each candidate
/// label is tried; the one that needs the smallest move of the theme's
/// accent wins, and a tie goes to the higher contrast. Measured, never
/// assumed from the mode or from a key's name.
fn accent_pair(
    accent: Rgb,
    bg: Rgb,
    surface: Rgb,
    theme_labels: [Rgb; 3],
) -> Result<(Rgb, Rgb), PaletteError> {
    theme_labels
        .into_iter()
        .chain([Rgb::WHITE, Rgb::BLACK])
        .filter_map(|label| {
            let needs = [(label, TEXT), (bg, NON_TEXT), (surface, NON_TEXT)];
            nearest_passing(accent, &needs).map(|(fill, step)| (fill, label, step))
        })
        .min_by(|a, b| {
            a.2.cmp(&b.2)
                .then(contrast(b.0, b.1).total_cmp(&contrast(a.0, a.1)))
        })
        .map(|(fill, label, _)| (fill, label))
        .ok_or(PaletteError::Unreachable("accent"))
}

/// The environment the theme directory is resolved from, passed in so tests
/// never read the real one.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
}

impl Env {
    pub fn from_process() -> Self {
        let absolute = |key: &str| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Self {
            home: absolute("HOME"),
            xdg_state_home: absolute("XDG_STATE_HOME"),
            xdg_config_home: absolute("XDG_CONFIG_HOME"),
        }
    }
}

/// The candidate theme directories, in the order they are tried.
pub fn theme_dirs(env: &Env) -> Vec<PathBuf> {
    let home = |rel: &str| env.home.as_ref().map(|h| h.join(rel));
    let candidates = [
        env.xdg_state_home.clone(),
        home(".local/state"),
        env.xdg_config_home.clone(),
        home(".config"),
    ];
    let mut dirs: Vec<PathBuf> = Vec::new();
    for base in candidates.into_iter().flatten() {
        let dir = base.join("omarchy").join("current").join("theme");
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Read and parse one theme directory's palette.
pub fn read_dir_palette(dir: &Path) -> Result<Palette, PaletteError> {
    use std::io::Read;
    let file =
        std::fs::File::open(dir.join("colors.toml")).map_err(|_| PaletteError::Unreadable)?;
    let mut bytes = Vec::new();
    file.take(MAX_PALETTE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PaletteError::Unreadable)?;
    if bytes.len() as u64 > MAX_PALETTE_BYTES {
        return Err(PaletteError::TooLarge);
    }
    let text = String::from_utf8(bytes).map_err(|_| PaletteError::NotToml)?;
    parse(&text, dir.join("light.mode").exists())
}

/// The palette of the first candidate that exists, or why it failed.
///
/// A candidate exists when its `current/` directory does: that survives a
/// theme switch, where the theme directory itself is removed and moved back
/// in. The search stops there, valid or not, so a broken current palette
/// leaves the caller keeping the last valid one rather than reading a stale
/// legacy directory behind it.
pub fn resolve(dirs: &[PathBuf]) -> Result<(PathBuf, Palette), PaletteError> {
    let dir = dirs
        .iter()
        .find(|dir| dir.parent().is_some_and(Path::is_dir))
        .ok_or(PaletteError::Unreadable)?;
    read_dir_palette(dir).map(|palette| (dir.clone(), palette))
}

/// The last valid palette, kept across failed reads.
#[derive(Clone, Debug, Default)]
pub struct Retained {
    current: Option<Palette>,
}

/// What [`Retained::offer`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offered {
    /// A new valid palette replaced the old one (or the absence of one).
    Replaced,
    /// The same palette again: nothing to reload.
    Unchanged,
    /// The read failed; the last valid palette stays.
    KeptLast,
    /// The read failed and there has never been a valid palette.
    NoneYet,
}

impl Retained {
    pub fn current(&self) -> Option<&Palette> {
        self.current.as_ref()
    }

    pub fn offer(&mut self, read: Result<Palette, PaletteError>) -> Offered {
        match (read, &self.current) {
            (Ok(palette), Some(current)) if *current == palette => Offered::Unchanged,
            (Ok(palette), _) => {
                self.current = Some(palette);
                Offered::Replaced
            }
            (Err(_), Some(_)) => Offered::KeptLast,
            (Err(_), None) => Offered::NoneYet,
        }
    }
}

/// The vendored upstream palettes (`tests/fixtures/omarchy/`, MIT).
#[cfg(test)]
pub(crate) mod fixtures {
    pub(crate) const TOKYO: &str =
        include_str!("../../tests/fixtures/omarchy/tokyo-night.colors.toml");
    pub(crate) const GRUVBOX: &str =
        include_str!("../../tests/fixtures/omarchy/gruvbox.colors.toml");
    pub(crate) const LATTE: &str =
        include_str!("../../tests/fixtures/omarchy/catppuccin-latte.colors.toml");
    pub(crate) const ROSE: &str =
        include_str!("../../tests/fixtures/omarchy/rose-pine.colors.toml");
    pub(crate) const FLEXOKI: &str =
        include_str!("../../tests/fixtures/omarchy/flexoki-light.colors.toml");
    pub(crate) const LEGACY: &str =
        include_str!("../../tests/fixtures/omarchy/legacy-tokyo-night.colors.toml");

    pub(crate) const FIXTURES: [(&str, &str); 6] = [
        ("tokyo-night", TOKYO),
        ("gruvbox", GRUVBOX),
        ("catppuccin-latte", LATTE),
        ("rose-pine", ROSE),
        ("flexoki-light", FLEXOKI),
        ("legacy-tokyo-night", LEGACY),
    ];
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn rgb(hex: &str) -> Rgb {
        Rgb::parse(hex).unwrap()
    }

    #[test]
    fn a_colour_is_exactly_six_hex_digits_after_a_hash() {
        assert_eq!(Rgb::parse("#1a1B26"), Some(Rgb(0x1a, 0x1b, 0x26)));
        for bad in [
            "1a1b26",
            "#1a1b2",
            "#1a1b266",
            "#1a1b2g",
            "#fff",
            "",
            "#",
            "rgb(1,2,3)",
        ] {
            assert_eq!(Rgb::parse(bad), None, "{bad:?}");
        }
        assert_eq!(rgb("#c0caf5").hex(), "#C0CAF5");
    }

    #[test]
    fn contrast_is_wcag() {
        assert!((contrast(Rgb::BLACK, Rgb::WHITE) - 21.0).abs() < 1e-9);
        // The stack's own measured pair: ink #20241F on bg #F6F7F4 is 14.64:1.
        assert!((contrast(rgb("#20241F"), rgb("#F6F7F4")) - 14.64).abs() < 0.01);
    }

    #[test]
    fn every_fixture_parses_with_the_mode_it_declares() {
        let expect = [
            ("tokyo-night", Mode::Dark),
            ("gruvbox", Mode::Dark),
            ("catppuccin-latte", Mode::Light),
            ("rose-pine", Mode::Light),
            ("flexoki-light", Mode::Light),
            // No `mode` key: background luminance, as upstream.
            ("legacy-tokyo-night", Mode::Dark),
        ];
        for ((name, text), (expected_name, mode)) in FIXTURES.iter().zip(expect) {
            assert_eq!(*name, expected_name);
            let palette = parse(text, false).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(palette.mode, mode, "{name}");
        }
    }

    #[test]
    fn canonical_keys_map_onto_their_roles() {
        let p = parse(TOKYO, false).unwrap();
        assert_eq!(p.bg, rgb("#1a1b26"));
        assert_eq!(p.surface, rgb("#24283b"));
        assert_eq!(p.inset, rgb("#13141c"));
        assert_eq!(p.scene, rgb("#0e0e14"));
        assert_eq!(p.ink, rgb("#a9b1d6"));
        assert_eq!(p.line, rgb("#414868"));
        // Already clear of their floors, so kept exactly.
        assert_eq!(p.accent_text, rgb("#7aa2f7"));
        assert_eq!(p.status_on_text, rgb("#9ece6a"));
        assert_eq!(p.status_outside_text, rgb("#f7768e"));
        assert_eq!(p.status_ask_text, rgb("#e0af68"));
        assert_eq!(p.held_text, rgb("#7aa2f7"));
    }

    #[test]
    fn legacy_ansi_keys_fill_the_canonical_roles() {
        let p = parse(LEGACY, false).unwrap();
        assert_eq!(p.bg, rgb("#1a1b26"));
        assert_eq!(p.ink, rgb("#a9b1d6"));
        // selection_background, then contrast-adjusted against ink.
        assert_eq!(p.status_on_text, rgb("#9ece6a")); // color2
        assert_eq!(p.status_outside_text, rgb("#f7768e")); // color1
        assert_eq!(p.line, rgb("#444b6a")); // color8 is muted
    }

    #[test]
    fn short_legacy_names_are_aliases() {
        let text = "bg = \"#101010\"\nfg = \"#e0e0e0\"\naccent = \"#5599ff\"\n";
        let p = parse(text, false).unwrap();
        assert_eq!(p.bg, rgb("#101010"));
        assert_eq!(p.ink, rgb("#e0e0e0"));
    }

    /// The floors the spec states, for every vendored palette: 4.5:1 text
    /// on every ground, 3:1 rules and fills, 4.5:1 labels on fills.
    #[test]
    fn every_role_pair_meets_its_floor_for_every_fixture() {
        for (name, text) in FIXTURES {
            let p = parse(text, false).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let mut failures = Vec::new();
            let mut need = |label: &str, a: Rgb, b: Rgb, floor: f64| {
                let ratio = contrast(a, b);
                if ratio < floor {
                    failures.push(format!("{name}: {label} {ratio:.2}:1 < {floor}"));
                }
            };
            for (i, ground) in p.grounds().into_iter().enumerate() {
                for (label, ink) in [
                    ("ink", p.ink),
                    ("strong", p.strong),
                    ("muted", p.muted),
                    ("tertiary", p.tertiary),
                    ("accent_text", p.accent_text),
                    ("on_text", p.status_on_text),
                    ("ask_text", p.status_ask_text),
                    ("outside_text", p.status_outside_text),
                    ("held_text", p.held_text),
                ] {
                    need(&format!("{label} on ground {i}"), ink, ground, TEXT);
                }
                for (label, rule) in [
                    ("accent_line", p.accent_line),
                    ("status_on", p.status_on),
                    ("status_ask", p.status_ask),
                    ("status_outside", p.status_outside),
                    ("held", p.held),
                ] {
                    need(&format!("{label} on ground {i}"), rule, ground, NON_TEXT);
                }
            }
            need("accent fill on bg", p.accent, p.bg, NON_TEXT);
            need("accent fill on surface", p.accent, p.surface, NON_TEXT);
            need("on_accent on accent", p.on_accent, p.accent, TEXT);
            for (label, fill) in [
                ("status_on", p.status_on),
                ("status_ask", p.status_ask),
                ("status_outside", p.status_outside),
                ("status_on_text", p.status_on_text),
                ("status_ask_text", p.status_ask_text),
                ("status_outside_text", p.status_outside_text),
            ] {
                need(&format!("on_status on {label}"), p.on_status, fill, TEXT);
            }
            need("ink on selection", p.ink, p.selection, TEXT);
            need(
                "selection_ink on selection",
                p.selection_ink,
                p.selection,
                TEXT,
            );
            need("redaction ink", p.redaction_ink, p.redaction_bg, TEXT);
            // The search hit: its ink on the highlight at its alpha, over
            // every ground, bare and under each kept translucent overlay
            // (the excerpt sits in a `.tc-code` recess, which is
            // `tc_scrim`).
            for (i, ground) in p.grounds().into_iter().enumerate() {
                let under = std::iter::once(ground)
                    .chain(overlays(p.mode).map(|(tint, alpha)| ground.mix(tint, alpha)));
                for (j, base) in under.enumerate() {
                    let wash = base.mix(p.highlight, HIGHLIGHT_WASH);
                    need(
                        &format!("ink on highlight {i}/{j}"),
                        p.highlight_ink,
                        wash,
                        TEXT,
                    );
                    // ...and the wash still marks the hit.
                    need(
                        &format!("highlight wash {i}/{j}"),
                        wash,
                        base,
                        HIGHLIGHT_VISIBLE,
                    );
                }
            }
            assert!(failures.is_empty(), "{failures:#?}");
        }
    }

    /// The fallback never hands back a wash that vanishes off its ground,
    /// even where the ink would pass on it.
    #[test]
    fn moved_wash_stays_visible_off_its_ground() {
        let bases = [Rgb::BLACK];
        let (c, _) = moved_wash(Rgb::WHITE, Rgb::BLACK, &bases).unwrap();
        let wash = Rgb::BLACK.mix(c, HIGHLIGHT_WASH);
        assert!(contrast(wash, Rgb::BLACK) >= HIGHLIGHT_VISIBLE);
        assert!(contrast(Rgb::WHITE, wash) >= TEXT);
    }

    /// A pale yellow on a light ground passes the ink test unmoved, but its
    /// wash vanishes off the ground; the highlight must still be visible.
    #[test]
    fn pale_yellow_on_light_ground_keeps_a_visible_wash() {
        for yellow in ["#f9e2af", "#ffff00"] {
            let text = format!(
                "mode = \"light\"\nbackground = \"#eff1f5\"\nforeground = \"#4c4f69\"\nyellow = \"{yellow}\"\n"
            );
            let p = parse(&text, false).unwrap();
            for ground in p.grounds() {
                let under = std::iter::once(ground)
                    .chain(overlays(p.mode).map(|(tint, alpha)| ground.mix(tint, alpha)));
                for base in under {
                    let wash = base.mix(p.highlight, HIGHLIGHT_WASH);
                    assert!(
                        contrast(wash, base) >= HIGHLIGHT_VISIBLE,
                        "{yellow}: wash {:.3}:1",
                        contrast(wash, base)
                    );
                    assert!(contrast(p.highlight_ink, wash) >= TEXT);
                }
            }
        }
    }

    /// The Pango alpha string and the CSS/contrast fraction are one value.
    #[test]
    fn highlight_alpha_and_wash_agree() {
        let pct: f64 = HIGHLIGHT_ALPHA.trim_end_matches('%').parse().unwrap();
        assert!((pct / 100.0 - HIGHLIGHT_WASH).abs() < 1e-9);
    }

    /// An accent and the foreground are not a button pair by name: the label
    /// is chosen by measuring, and it differs between light and dark themes.
    #[test]
    fn on_accent_is_chosen_by_contrast_not_by_name() {
        let dark = parse(TOKYO, false).unwrap();
        let light = parse(LATTE, false).unwrap();
        assert!(contrast(dark.on_accent, dark.accent) >= TEXT);
        assert!(contrast(light.on_accent, light.accent) >= TEXT);
        // Tokyo Night's accent is light, so its label is dark; Latte's blue
        // is dark, so its label is light.
        assert!(dark.on_accent.luminance() < dark.accent.luminance());
        assert!(light.on_accent.luminance() > light.accent.luminance());
    }

    /// Where a theme colour already clears its floor it is used unchanged:
    /// the hue a person chose survives.
    #[test]
    fn a_passing_theme_colour_is_not_moved() {
        let p = parse(TOKYO, false).unwrap();
        assert_eq!(p.accent_text, rgb("#7aa2f7"));
        // Gruvbox's accent clears its ground but not the hovered control
        // (4.1:1), so it is lifted, by the fewest 2% steps that clear it.
        let g = parse(GRUVBOX, false).unwrap();
        let source = rgb("#7daea3");
        let step = (1..=STEPS)
            .find(|k| source.mix(Rgb::WHITE, f64::from(*k) / f64::from(STEPS)) == g.accent_text)
            .expect("lifted toward white");
        let one_less = source.mix(Rgb::WHITE, f64::from(step - 1) / f64::from(STEPS));
        assert!(
            g.grounds()
                .iter()
                .any(|ground| contrast(one_less, *ground) < TEXT)
        );
        // Rose Pine's accent on its light ground needs darkening as text.
        let r = parse(ROSE, false).unwrap();
        assert_ne!(r.accent_text, rgb("#56949f"));
        assert!(r.accent_text.luminance() < rgb("#56949f").luminance());
    }

    #[test]
    fn mode_precedence_is_upstreams() {
        let base = "background = \"#ffffff\"\nforeground = \"#000000\"\n";
        // Luminance: white ground is light.
        assert_eq!(parse(base, false).unwrap().mode, Mode::Light);
        // The legacy marker file.
        let dark_ground = "background = \"#101010\"\nforeground = \"#f0f0f0\"\n";
        assert_eq!(parse(dark_ground, false).unwrap().mode, Mode::Dark);
        assert_eq!(parse(dark_ground, true).unwrap().mode, Mode::Light);
        // theme_type beats the marker; mode beats theme_type.
        let typed = format!("{dark_ground}theme_type = \"dark\"\n");
        assert_eq!(parse(&typed, true).unwrap().mode, Mode::Dark);
        let moded = format!("{typed}mode = \"light\"\n");
        assert_eq!(parse(&moded, false).unwrap().mode, Mode::Light);
        // As upstream, any non-empty `mode` stops the search, and only
        // "light" is light: an unknown value is dark, even on a light ground
        // with the marker file beside it.
        let light_ground = "background = \"#ffffff\"\nforeground = \"#000000\"\n";
        let odd = format!("{light_ground}mode = \"sepia\"\n");
        assert_eq!(parse(&odd, true).unwrap().mode, Mode::Dark);
        let odd_type = format!("{light_ground}theme_type = \"Light\"\n");
        assert_eq!(parse(&odd_type, true).unwrap().mode, Mode::Dark);
        // An empty value is absent: the search goes on.
        let empty = format!("{dark_ground}mode = \"\"\ntheme_type = \"light\"\n");
        assert_eq!(parse(&empty, false).unwrap().mode, Mode::Light);
    }

    #[test]
    fn malformed_input_is_rejected_without_its_content() {
        assert_eq!(
            parse("background = \"#1a1b26", false),
            Err(PaletteError::NotToml)
        );
        assert_eq!(
            parse("foreground = \"#ffffff\"\n", false),
            Err(PaletteError::Missing("background"))
        );
        assert_eq!(
            parse("background = \"#000000\"\n", false),
            Err(PaletteError::Missing("foreground"))
        );
        // A key with a value that is not a colour counts as absent.
        assert_eq!(
            parse("background = \"black\"\nforeground = \"#ffffff\"\n", false),
            Err(PaletteError::Missing("background"))
        );
        assert_eq!(
            parse("background = 7\nforeground = \"#ffffff\"\n", false),
            Err(PaletteError::Missing("background"))
        );
    }

    #[test]
    fn an_invalid_optional_colour_is_derived_instead() {
        let text = "background = \"#1a1b26\"\nforeground = \"#a9b1d6\"\naccent = \"blue\"\n\
                    lighter_background = \"#zzzzzz\"\n";
        let p = parse(text, false).unwrap();
        // Derived surface: mix(bg, fg, 6%).
        assert_eq!(p.surface, rgb("#1a1b26").mix(rgb("#a9b1d6"), 0.06));
        assert!(contrast(p.on_accent, p.accent) >= TEXT);
    }

    #[test]
    fn a_palette_with_no_readable_ink_is_rejected() {
        // Foreground equal to background: moving the ink to either end is
        // possible, so this one is adjusted, not rejected...
        let same = "background = \"#808080\"\nforeground = \"#808080\"\n";
        let p = parse(same, false).unwrap();
        assert!(contrast(p.ink, p.bg) >= TEXT);
        // ...but a mid-grey ground with a mid-grey surface leaves no colour
        // that clears 4.5:1 on both, so the palette is refused.
        let hopeless = "background = \"#767676\"\nforeground = \"#767676\"\n\
                        lighter_background = \"#000000\"\ndark_background = \"#ffffff\"\n";
        assert!(matches!(
            parse(hopeless, false),
            Err(PaletteError::Unreachable(_))
        ));
    }

    #[test]
    fn theme_dirs_follow_xdg_then_omarchys_own_paths() {
        let env = Env {
            home: Some("/home/a".into()),
            xdg_state_home: Some("/xdg/state".into()),
            xdg_config_home: Some("/xdg/config".into()),
        };
        assert_eq!(
            theme_dirs(&env),
            vec![
                PathBuf::from("/xdg/state/omarchy/current/theme"),
                PathBuf::from("/home/a/.local/state/omarchy/current/theme"),
                PathBuf::from("/xdg/config/omarchy/current/theme"),
                PathBuf::from("/home/a/.config/omarchy/current/theme"),
            ]
        );
        // Unset XDG variables and duplicates collapse.
        let plain = Env {
            home: Some("/home/a".into()),
            xdg_state_home: Some("/home/a/.local/state".into()),
            xdg_config_home: None,
        };
        assert_eq!(
            theme_dirs(&plain),
            vec![
                PathBuf::from("/home/a/.local/state/omarchy/current/theme"),
                PathBuf::from("/home/a/.config/omarchy/current/theme"),
            ]
        );
        assert!(theme_dirs(&Env::default()).is_empty());
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tc-omarchy-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_stops_at_the_first_candidate_that_exists() {
        let root = scratch("resolve");
        // Each candidate is <base>/omarchy/current/theme.
        let theme = |base: &str| root.join(base).join("omarchy/current/theme");
        let (missing, broken, good) = (theme("missing"), theme("broken"), theme("good"));
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(broken.join("colors.toml"), "not toml [").unwrap();
        std::fs::write(good.join("colors.toml"), GRUVBOX).unwrap();
        // A candidate that does not exist is passed over.
        let (dir, palette) = resolve(&[missing.clone(), good.clone()]).unwrap();
        assert_eq!(dir, good);
        assert_eq!(palette, parse(GRUVBOX, false).unwrap());
        // A broken palette in the first one that exists is that one's
        // answer: the caller keeps the last valid palette, and a stale
        // legacy directory behind it is never read instead.
        assert_eq!(
            resolve(&[missing.clone(), broken.clone(), good.clone()]),
            Err(PaletteError::NotToml)
        );
        // So is the gap of a theme switch: `current/` is there, the theme
        // directory is not (yet).
        std::fs::remove_dir_all(&broken).unwrap();
        assert_eq!(
            resolve(&[broken.clone(), good.clone()]),
            Err(PaletteError::Unreadable)
        );
        assert_eq!(resolve(&[missing]), Err(PaletteError::Unreadable));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_light_mode_file_beside_the_palette_is_read() {
        let dir = scratch("marker");
        std::fs::write(
            dir.join("colors.toml"),
            "background = \"#101010\"\nforeground = \"#f0f0f0\"\n",
        )
        .unwrap();
        assert_eq!(read_dir_palette(&dir).unwrap().mode, Mode::Dark);
        std::fs::write(dir.join("light.mode"), "").unwrap();
        assert_eq!(read_dir_palette(&dir).unwrap().mode, Mode::Light);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_oversized_palette_is_not_read() {
        let dir = scratch("large");
        let mut text = String::from(TOKYO);
        text.push_str(&"# padding\n".repeat(8000));
        std::fs::write(dir.join("colors.toml"), text).unwrap();
        assert_eq!(read_dir_palette(&dir), Err(PaletteError::TooLarge));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_read_keeps_the_last_valid_palette() {
        let mut kept = Retained::default();
        assert_eq!(kept.offer(Err(PaletteError::Unreadable)), Offered::NoneYet);
        assert!(kept.current().is_none());
        let tokyo = parse(TOKYO, false).unwrap();
        assert_eq!(kept.offer(Ok(tokyo.clone())), Offered::Replaced);
        assert_eq!(kept.offer(Ok(tokyo.clone())), Offered::Unchanged);
        // The rm/mv gap of a theme switch, then a malformed file.
        assert_eq!(kept.offer(Err(PaletteError::Unreadable)), Offered::KeptLast);
        assert_eq!(kept.offer(Err(PaletteError::NotToml)), Offered::KeptLast);
        assert_eq!(kept.current(), Some(&tokyo));
        let latte = parse(LATTE, false).unwrap();
        assert_eq!(kept.offer(Ok(latte.clone())), Offered::Replaced);
        assert_eq!(kept.current(), Some(&latte));
    }
}
