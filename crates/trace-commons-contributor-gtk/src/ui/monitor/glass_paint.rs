//! Small cairo pieces the Monitor window draws the way the Mac's TCDesign
//! does: the overlay ink (`GlassColor.ink`), a status dot with its halo
//! (`GlassStatusDot`), the Private AI tab's dot colours, and the selected
//! tab's ember line and halo (`GlassRuleTabs.emberGlow`).
//!
//! Each is a pure function of a cairo context and plain numbers, so a test
//! can paint it into an image surface and read the pixels back.

use crate::ui::style::{self, brand_tokens::glass};

/// `GlassColor.ink(alpha)`: black at `alpha` in light, white in dark. An
/// overlay ink, never the text colour.
pub fn ink(dark: bool, alpha: f64) -> (f64, f64, f64, f64) {
    let value = if dark { 1.0 } else { 0.0 };
    (value, value, value, alpha)
}

/// `#RRGGBB` as cairo channels; black for anything else.
pub fn channels(hex: &str) -> (f64, f64, f64) {
    let hex = hex.trim_start_matches('#');
    let channel = |at: usize| {
        hex.get(at..at + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map_or(0.0, |value| f64::from(value) / 255.0)
    };
    if hex.len() == 6 {
        (channel(0), channel(2), channel(4))
    } else {
        (0.0, 0.0, 0.0)
    }
}

/// The Private AI dot's colour for a tone: the Mac's one rule for every
/// Private AI dot (`PrivateInferenceView.status`): green while clear, amber
/// for attention, red for refused, and grey (`statusOff`) for both neutral
/// and held, "so one state is one colour everywhere".
pub fn tab_dot_ink(tone: style::Tone, dark: bool) -> &'static str {
    use glass::status::*;
    match (tone, dark) {
        (style::Tone::Clear, false) => ON_LIGHT,
        (style::Tone::Clear, true) => ON_DARK,
        (style::Tone::Attention, false) => ASK_LIGHT,
        (style::Tone::Attention, true) => ASK_DARK,
        (style::Tone::Refused, false) => OUTSIDE_LIGHT,
        (style::Tone::Refused, true) => OUTSIDE_DARK,
        (style::Tone::Neutral | style::Tone::Held, false) => OFF_LIGHT,
        (style::Tone::Neutral | style::Tone::Held, true) => OFF_DARK,
    }
}

/// The tab dot's diameter (`GlassSegmentMarkView`: `GlassStatusDot(status,
/// size: 6)`).
pub const TAB_DOT: i32 = 6;

/// `GlassStatusDot`: a filled circle `size` across, centred on `(x, y)`.
/// With `halo`, the neutral ring around it: `ink(0.08)`, 2 wide, its path
/// on a circle 1 larger all round than the dot (`.padding(-1)`), so it
/// runs from the dot's edge to 2 outside it.
pub fn paint_status_dot(
    context: &gtk::cairo::Context,
    x: f64,
    y: f64,
    size: f64,
    fill: &str,
    halo: bool,
    dark: bool,
) {
    let (red, green, blue) = channels(fill);
    context.set_source_rgb(red, green, blue);
    context.new_path();
    context.arc(x, y, size / 2.0, 0.0, std::f64::consts::TAU);
    let _ = context.fill();
    if halo {
        let (red, green, blue, alpha) = ink(dark, 0.08);
        context.set_source_rgba(red, green, blue, alpha);
        context.set_line_width(2.0);
        context.new_path();
        context.arc(x, y, size / 2.0 + 1.0, 0.0, std::f64::consts::TAU);
        let _ = context.stroke();
    }
}

/// How far the ember line reaches past its tab on each side, as a share of
/// the tab's width (`GlassRuleTabs.emberReach`).
pub const EMBER_REACH: f64 = 0.3;
/// The ember line's thickness (`emberLine`).
pub const EMBER_LINE: f64 = 2.0;
/// How far the halo rises above the rule (`haloHeight`).
pub const HALO_HEIGHT: f64 = 26.0;
/// The halo's half-width, as a share of the tab's width: its frame is 1.1
/// tab widths and the gradient ends at half of it.
pub const HALO_HALF_WIDTH: f64 = 0.55;

/// The selected tab's glow, in a strip `height` high whose rule is its
/// bottom pixel: the halo, brightest at the rule under the tab's middle and
/// gone `HALO_HEIGHT` up and `HALO_HALF_WIDTH` tab widths out; then the
/// ember line, a single peak from clear to `tabEmber` and back over 1.6 tab
/// widths, centred on the 1pt rule, so it runs into the neighbouring tabs.
pub fn paint_ember(
    context: &gtk::cairo::Context,
    height: f64,
    tab_left: f64,
    tab_width: f64,
    dark: bool,
) {
    if tab_width <= 0.0 {
        return;
    }
    let (ember, (halo, halo_alpha)) = if dark {
        (glass::tabs::EMBER_DARK, glass::tabs::HALO_DARK)
    } else {
        (glass::tabs::EMBER_LIGHT, glass::tabs::HALO_LIGHT)
    };
    let middle = tab_left + tab_width / 2.0;
    let (red, green, blue) = channels(halo);
    let glow = gtk::cairo::RadialGradient::new(0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
    glow.add_color_stop_rgba(0.0, red, green, blue, halo_alpha);
    glow.add_color_stop_rgba(1.0, red, green, blue, 0.0);
    let _ = context.save();
    context.translate(middle, height);
    context.scale(tab_width * HALO_HALF_WIDTH, HALO_HEIGHT);
    let _ = context.set_source(&glow);
    context.new_path();
    context.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
    let _ = context.fill();
    let _ = context.restore();

    let reach = tab_width * EMBER_REACH;
    let (start, end) = (tab_left - reach, tab_left + tab_width + reach);
    let (red, green, blue) = channels(ember);
    let line = gtk::cairo::LinearGradient::new(start, 0.0, end, 0.0);
    line.add_color_stop_rgba(0.0, red, green, blue, 0.0);
    line.add_color_stop_rgba(0.5, red, green, blue, 1.0);
    line.add_color_stop_rgba(1.0, red, green, blue, 0.0);
    let _ = context.set_source(&line);
    // Centred on the 1pt rule: half a point below the strip's foot.
    let bottom = height + (EMBER_LINE - 1.0) / 2.0;
    context.new_path();
    context.rectangle(start, bottom - EMBER_LINE, end - start, EMBER_LINE);
    let _ = context.fill();
}

#[cfg(test)]
pub(crate) mod paint_tests {
    /// Paints `ground` over a `width` by `height` surface, then `paint`,
    /// and returns a reader for the pixel at `(x, y)` as 0..=255 channels.
    pub fn paint(
        width: i32,
        height: i32,
        ground: (u8, u8, u8),
        paint: impl FnOnce(&gtk::cairo::Context),
    ) -> impl Fn(i32, i32) -> (i32, i32, i32) {
        let surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height).unwrap();
        {
            let context = gtk::cairo::Context::new(&surface).unwrap();
            context.set_source_rgb(
                f64::from(ground.0) / 255.0,
                f64::from(ground.1) / 255.0,
                f64::from(ground.2) / 255.0,
            );
            let _ = context.paint();
            paint(&context);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let mut surface = surface;
        let data = surface.data().unwrap().to_vec();
        move |x, y| {
            let at = y as usize * stride + x as usize * 4;
            // Little-endian ARGB32: B, G, R, A.
            (
                i32::from(data[at + 2]),
                i32::from(data[at + 1]),
                i32::from(data[at]),
            )
        }
    }

    /// `ground` with `ink` laid over it at `alpha`, per channel.
    pub fn over(ink: (u8, u8, u8), alpha: f64, ground: (u8, u8, u8)) -> (i32, i32, i32) {
        let mix =
            |a: u8, b: u8| (f64::from(a) * alpha + f64::from(b) * (1.0 - alpha)).round() as i32;
        (
            mix(ink.0, ground.0),
            mix(ink.1, ground.1),
            mix(ink.2, ground.2),
        )
    }

    pub fn near(actual: (i32, i32, i32), expected: (i32, i32, i32), slack: i32) -> bool {
        (actual.0 - expected.0).abs() <= slack
            && (actual.1 - expected.1).abs() <= slack
            && (actual.2 - expected.2).abs() <= slack
    }
}

#[cfg(test)]
mod tests {
    use super::paint_tests::{near, over, paint};
    use super::*;

    /// `PrivateInferenceView.status` maps clear, attention and refused to
    /// on, ask and outside, and both neutral and held to statusOff; the
    /// flat theme's values from `GlassTokens.swift` (:52-58).
    #[test]
    fn the_tab_dot_takes_the_macs_one_colour_per_state() {
        for (tone, light, dark) in [
            (style::Tone::Clear, "#1A8F4C", "#30D158"),
            (style::Tone::Attention, "#A8690F", "#FFC75C"),
            (style::Tone::Refused, "#D0342C", "#FF6B6B"),
            (style::Tone::Held, "#8A8A90", "#A9A9B0"),
            (style::Tone::Neutral, "#8A8A90", "#A9A9B0"),
        ] {
            assert_eq!(tab_dot_ink(tone, false), light, "{tone:?} light");
            assert_eq!(tab_dot_ink(tone, true), dark, "{tone:?} dark");
        }
        // `Navigation.swift:70`: `GlassStatusDot(status, size: 6)`.
        assert_eq!(TAB_DOT, 6);
    }

    /// The legend's halo (`GlassStatusDot(halo: true)`, `Indicators.swift`
    /// :22-24): a neutral `ink(0.08)` ring from the dot's edge to 2 past it,
    /// and nothing beyond. Not the series-coloured `ring` variant.
    #[test]
    fn the_legend_halo_is_a_neutral_ring_hugging_the_dot() {
        for (dark, ground, ink_rgb) in [
            (false, (0xdb, 0xdd, 0xe3), (0, 0, 0)),
            (true, (0x0e, 0x11, 0x1d), (255, 255, 255)),
        ] {
            let shared = if dark { "#8A3DFF" } else { "#7A2CF5" };
            let pixel = paint(28, 28, ground, |context| {
                paint_status_dot(context, 14.0, 14.0, 8.0, shared, true, dark);
            });
            // Inside the ring (4.5 out from the centre): the ground under
            // ink at 0.08, with no trace of the series colour.
            let ring = pixel(18, 13);
            assert!(
                near(ring, over(ink_rgb, 0.08, ground), 2),
                "dark {dark}: ring {ring:?}"
            );
            // Past it (6.5 out): the bare ground.
            let outside = pixel(20, 13);
            assert!(
                near(
                    outside,
                    (ground.0.into(), ground.1.into(), ground.2.into()),
                    1
                ),
                "dark {dark}: outside {outside:?}"
            );
            // The dot itself is the series colour.
            let (red, green, blue) = channels(shared);
            let centre = pixel(14, 14);
            let expected = (
                (red * 255.0).round() as i32,
                (green * 255.0).round() as i32,
                (blue * 255.0).round() as i32,
            );
            assert!(near(centre, expected, 1), "dark {dark}: centre {centre:?}");
        }
    }

    /// `GlassRuleTabs.emberGlow` for the second of four 100-wide tabs in a
    /// 38-high strip: a single peak 1.6 tab widths wide, so it starts 30
    /// left of the tab and is at three quarters 30 into it, and a halo that
    /// is gone 26 above the rule.
    #[test]
    fn the_ember_is_one_peak_reaching_past_its_tab_under_a_26_high_halo() {
        let ground = (0xf2, 0xf3, 0xf6);
        let pixel = paint(400, 38, ground, |context| {
            paint_ember(context, 38.0, 100.0, 100.0, false);
        });
        let ember = (0x05, 0x6b, 0xdc);
        // The peak, at the tab's middle on the rule: the ember itself.
        assert!(near(pixel(150, 37), ember, 4), "peak {:?}", pixel(150, 37));
        // 30% into the tab the line is at 0.75, not yet at full strength.
        let (red, _, _) = pixel(130, 37);
        assert!((red - 54).abs() <= 6, "30% in: red {red}");
        // It runs 15 into the tab on the left at about a fifth.
        let (red, _, _) = pixel(85, 37);
        assert!((red - 196).abs() <= 6, "into the neighbour: red {red}");
        // Before the reach: the ground.
        assert!(near(pixel(60, 37), (0xf2, 0xf3, 0xf6), 1));
        // The halo: faint 20 above the rule under the middle, gone at 27.
        let (red, _, _) = pixel(150, 18);
        assert!((red - 227).abs() <= 4, "halo 20 up: red {red}");
        assert!(
            near(pixel(150, 11), (0xf2, 0xf3, 0xf6), 1),
            "halo 27 up {:?}",
            pixel(150, 11)
        );
        // Gone past 0.55 of the tab's width from the middle, above the line.
        assert!(near(pixel(150 + 56, 30), (0xf2, 0xf3, 0xf6), 1));
        // In dark, the dark ember.
        let pixel = paint(400, 38, (0x17, 0x1b, 0x2d), |context| {
            paint_ember(context, 38.0, 100.0, 100.0, true);
        });
        assert!(near(pixel(150, 37), (0x00, 0x91, 0xfd), 4));
    }

    /// Overlay ink is pure black or white (`GlassTokens.Color.ink`, :252).
    #[test]
    fn the_overlay_ink_is_black_in_light_and_white_in_dark() {
        assert_eq!(ink(false, 0.07), (0.0, 0.0, 0.0, 0.07));
        assert_eq!(ink(true, 0.14), (1.0, 1.0, 1.0, 0.14));
    }
}
