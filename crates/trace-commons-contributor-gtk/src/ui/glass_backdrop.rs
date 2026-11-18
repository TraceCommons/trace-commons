//! Real blur behind the glass panes, where the desktop grants it.
//!
//! The Mac's panes are a material over the desktop (`GlassPaneFill` over
//! `GlassBackdrop`), and the gaps between them show the desktop itself
//! (`GlassWindowConfigurator`). On Linux only KDE's KWin lets a client ask
//! for that blur, through `org_kde_kwin_blur_manager` on Wayland and the
//! `_KDE_NET_WM_BLUR_BEHIND_REGION` window property on X11. This module asks
//! for it there and makes the window translucent ONLY while the blur is
//! granted (the global bound, or the atom announced on the root window) and
//! the display composites. Everywhere else -- GNOME, other compositors,
//! high contrast, a Flatpak that cannot reach the protocol -- the painted
//! look stays, because translucency without blur fails the contrast floors
//! over an arbitrary desktop.
//!
//! Every decision is pure and tested here ([`decide`], [`Requests`],
//! [`blur_region`]); the protocol plumbing sits behind [`BlurBackend`], in
//! the Linux-only `kde` module.

use gtk::prelude::*;

/// The glass radius of a pane (`GlassTier.pane.radius`, flat 20).
use super::style::brand_tokens::glass::radius::PANE as PANE_RADIUS;

/// The display backend the window is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Wayland,
    X11,
    Other,
}

impl Backend {
    fn name(self) -> &'static str {
        match self {
            Backend::Wayland => "wayland",
            Backend::X11 => "x11",
            Backend::Other => "other",
        }
    }
}

/// What the desktop offers, as last probed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub backend: Backend,
    /// KWin's blur is reachable: the Wayland global is bound, or the X11
    /// atom is announced on the root window. KWin offers either only while
    /// its blur effect is loaded.
    pub granted: bool,
    /// `gdk::Display::is_composited() && is_rgba()`: a translucent window
    /// shows what is behind it.
    pub composited: bool,
}

impl Capability {
    /// Nothing to ask: not Linux, or no display backend this module knows.
    pub const NONE: Capability = Capability {
        backend: Backend::Other,
        granted: false,
        composited: false,
    };
}

/// A developer override, `TRACE_COMMONS_GLASS`; unset, the window follows
/// the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Override {
    Auto,
    /// Always painted.
    Painted,
    /// Blur wherever it is granted, even in a scheme whose text does not
    /// yet read over every desktop ([`readable`]): for verifying the blur
    /// itself on a KDE box. Still needs the grant, compositing and no high
    /// contrast; it never makes the window translucent without blur.
    Blur,
}

/// The environment variable [`Override`] is read from.
pub const OVERRIDE_ENV: &str = "TRACE_COMMONS_GLASS";

pub fn parse_override(raw: Option<&str>) -> Override {
    match raw.map(str::trim) {
        Some("painted") => Override::Painted,
        Some("blur") => Override::Blur,
        _ => Override::Auto,
    }
}

/// How the window draws its glass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glass {
    /// Opaque tiers on an opaque ground: the look everywhere blur is not
    /// granted.
    Painted,
    /// Translucent panes over KWin's blur of the desktop.
    Blurred,
}

impl Glass {
    /// Whether the translucent sheet is loaded.
    pub fn translucent(self) -> bool {
        self == Glass::Blurred
    }
}

/// Whether every text colour the panes' screens use reads at 4.5:1 on the
/// translucent panes over a black and a white desktop, in and out of focus,
/// in this scheme. Measured, not chosen: `style.rs`'s
/// `every_text_colour_reads_on_the_translucent_panes` fails if this
/// disagrees with the measurement either way.
///
/// False in both schemes today. KWin's blur averages the desktop and adds
/// no frost of its own, as the Mac's material does, so over a dark desktop a
/// light pane is mid-grey (worst 2.51:1, `tc_blue_text` out of focus;
/// `tc_muted` 2.57:1) and over a white one a dark pane is too light (worst
/// 3.33:1, `tc_muted`). No tint share reaches 4.5 in light
/// (3.54:1 at the full veil); dark would need 0.65 against the Mac's 0.25.
/// Until that is settled the window stays painted under [`Override::Auto`],
/// on KDE too.
pub fn readable(dark: bool) -> bool {
    let _ = dark;
    false
}

/// The one decision. High contrast always paints: the Mac's flat theme
/// deepens its veil under Increase Contrast, and a painted pane is the
/// deepest there is. Otherwise the window is translucent only where blur is
/// granted and the display composites, and under [`Override::Auto`] only
/// where its text reads over any desktop (`readable`).
pub fn decide(
    choice: Override,
    capability: Capability,
    high_contrast: bool,
    readable: bool,
) -> Glass {
    let granted =
        capability.granted && capability.composited && capability.backend != Backend::Other;
    match choice {
        _ if high_contrast || !granted => Glass::Painted,
        Override::Painted => Glass::Painted,
        Override::Auto if !readable => Glass::Painted,
        Override::Auto | Override::Blur => Glass::Blurred,
    }
}

/// A rectangle in surface coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A rounded rectangle as horizontal bands, for a region protocol that
/// takes only rectangles. Each band of a corner is as wide as the circle is
/// at the band's middle row, so the region stays within half a pixel of
/// the rounded edge and no blur shows outside a pane's corner. Rows of
/// equal width are merged.
pub fn rounded_bands(rect: Rect, radius: i32) -> Vec<Rect> {
    if rect.width <= 0 || rect.height <= 0 {
        return Vec::new();
    }
    let r = radius.clamp(0, rect.width.min(rect.height) / 2);
    let inset = |row: i32| -> i32 {
        // Distance from the corner circle's centre to the middle of `row`.
        let dy = f64::from(r) - (f64::from(row) + 0.5);
        if dy <= 0.0 {
            return 0;
        }
        let half = (f64::from(r) * f64::from(r) - dy * dy).max(0.0).sqrt();
        (f64::from(r) - half).round() as i32
    };
    let mut bands: Vec<Rect> = Vec::new();
    let mut push = |y: i32, height: i32, inset: i32| {
        let band = Rect {
            x: rect.x + inset,
            y,
            width: rect.width - 2 * inset,
            height,
        };
        if band.width <= 0 || band.height <= 0 {
            return;
        }
        match bands.last_mut() {
            Some(last)
                if last.x == band.x && last.width == band.width && last.y + last.height == y =>
            {
                last.height += height;
            }
            _ => bands.push(band),
        }
    };
    for row in 0..r {
        push(rect.y + row, 1, inset(row));
    }
    push(rect.y + r, rect.height - 2 * r, 0);
    for row in (0..r).rev() {
        push(rect.y + rect.height - 1 - row, 1, inset(row));
    }
    bands
}

/// The region to blur: every shown pane, in surface coordinates, rounded
/// at the pane radius. Only the panes: on the Mac the gaps between them show
/// the desktop unblurred. `panes` are bounds in the window's coordinates
/// (`compute_bounds`); `offset` is where the window's origin sits in its
/// surface (`surface_transform`, the client-side shadow's margin).
pub fn blur_region(panes: &[(f64, f64, f64, f64)], offset: (f64, f64), radius: i32) -> Vec<Rect> {
    let mut region = Vec::new();
    for &(x, y, width, height) in panes {
        let left = (x + offset.0).round() as i32;
        let top = (y + offset.1).round() as i32;
        let right = (x + offset.0 + width).round() as i32;
        let bottom = (y + offset.1 + height).round() as i32;
        region.extend(rounded_bands(
            Rect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            },
            radius,
        ));
    }
    region
}

/// `_KDE_NET_WM_BLUR_BEHIND_REGION`: CARDINAL/32, `x y w h` per rectangle
/// in device pixels. KWindowSystem's `enableBlurBehind` floors x and y and
/// ceils the width and height after scaling.
pub fn x11_cardinals(region: &[Rect], scale: f64) -> Vec<u32> {
    let scaled = |v: i32| f64::from(v) * scale;
    region
        .iter()
        .flat_map(|rect| {
            [
                scaled(rect.x).floor().max(0.0) as u32,
                scaled(rect.y).floor().max(0.0) as u32,
                scaled(rect.width).ceil().max(0.0) as u32,
                scaled(rect.height).ceil().max(0.0) as u32,
            ]
        })
        .collect()
}

/// What to do after a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Load the painted sheet (`false`) or the translucent one (`true`).
    Sheet(bool),
    /// Ask for blur over exactly this non-empty region.
    Request(Vec<Rect>),
    /// Withdraw the blur request. Never an empty request: to both
    /// protocols an empty or absent region means the WHOLE surface.
    Unset,
}

/// The requests already made, so a frame that changes nothing sends
/// nothing, and a revoked grant withdraws what was asked.
#[derive(Debug, Default)]
pub struct Requests {
    glass: Option<Glass>,
    sent: Option<Vec<Rect>>,
    /// A request did not go out. Translucency without blur is never shown,
    /// so the window paints until the grant is lost and given again.
    refused: bool,
}

impl Requests {
    /// `glass` as [`decide`] chose it, with `granted` the backend's grant.
    pub fn update(&mut self, glass: Glass, granted: bool, region: Vec<Rect>) -> Vec<Effect> {
        if !granted {
            self.refused = false;
        }
        let glass = if self.refused { Glass::Painted } else { glass };
        let mut effects = Vec::new();
        let was = self.glass.replace(glass);
        if was.map(Glass::translucent) != Some(glass.translucent()) {
            effects.push(Effect::Sheet(glass.translucent()));
        }
        if glass != Glass::Blurred {
            if self.sent.take().is_some_and(|sent| !sent.is_empty()) {
                effects.push(Effect::Unset);
            }
            return effects;
        }
        if self.sent.as_ref() == Some(&region) {
            return effects;
        }
        if region.is_empty() {
            if self.sent.as_ref().is_some_and(|sent| !sent.is_empty()) {
                effects.push(Effect::Unset);
            }
        } else {
            effects.push(Effect::Request(region.clone()));
        }
        self.sent = Some(region);
        effects
    }

    /// The backend did not send the last request: paint and withdraw.
    pub fn refused(&mut self) -> Vec<Effect> {
        self.refused = true;
        let mut effects = Vec::new();
        if self
            .glass
            .replace(Glass::Painted)
            .is_some_and(Glass::translucent)
        {
            effects.push(Effect::Sheet(false));
        }
        self.sent = None;
        effects.push(Effect::Unset);
        effects
    }
}

/// The protocol seam: KWin's blur on Wayland or X11, or nothing.
pub trait BlurBackend {
    /// What was last probed. Cheap: called on every frame.
    fn capability(&self) -> Capability;
    /// Re-probe: dispatch pending Wayland events (a removed global), or ask
    /// the X server again. Called on a timer and when compositing changes.
    fn poll(&mut self);
    /// Ask for blur over `region` behind `surface`. False if not sent.
    fn request(&mut self, surface: &gtk::gdk::Surface, region: &[Rect], scale: f64) -> bool;
    /// Withdraw the request on `surface`.
    fn unset(&mut self, surface: &gtk::gdk::Surface);
}

/// No blur anywhere: not Linux, or no backend this module knows.
struct NoBlur;

impl BlurBackend for NoBlur {
    fn capability(&self) -> Capability {
        Capability::NONE
    }
    fn poll(&mut self) {}
    fn request(&mut self, _: &gtk::gdk::Surface, _: &[Rect], _: f64) -> bool {
        false
    }
    fn unset(&mut self, _: &gtk::gdk::Surface) {}
}

#[cfg(target_os = "linux")]
mod kde;

fn probe(display: &gtk::gdk::Display) -> Box<dyn BlurBackend> {
    #[cfg(target_os = "linux")]
    if let Some(backend) = kde::probe(display) {
        return backend;
    }
    let _ = display;
    Box::new(NoBlur)
}

/// What `display` offers right now, probed afresh: the backend's grant and
/// whether the display composites.
pub fn probe_capability(display: &gtk::gdk::Display) -> Capability {
    let mut capability = probe(display).capability();
    capability.composited = display.is_composited() && display.is_rgba();
    capability
}

struct State {
    backend: Box<dyn BlurBackend>,
    requests: Requests,
    choice: Override,
    logged: Option<(Capability, Glass)>,
}

/// Follow the desktop's blur for `window`, whose glass panes are `panes`.
/// Probes once, then re-decides on every painted frame (the panes move with
/// resizes, maximizing, tiling and pane widths without the surface
/// resizing), on a one-second poll (a grant can be revoked: KWin drops the
/// global when its blur effect unloads), on a compositing change and on a
/// high-contrast change.
pub fn follow(window: &adw::ApplicationWindow, panes: Vec<gtk::Widget>) {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let state = std::rc::Rc::new(std::cell::RefCell::new(State {
        backend: probe(&display),
        requests: Requests::default(),
        choice: parse_override(std::env::var(OVERRIDE_ENV).ok().as_deref()),
        logged: None,
    }));
    let panes = std::rc::Rc::new(panes);

    let refresh = {
        let state = state.clone();
        let panes = panes.clone();
        let window = window.downgrade();
        std::rc::Rc::new(move || {
            if let Some(window) = window.upgrade() {
                apply(&window, &panes, &mut state.borrow_mut());
            }
        })
    };

    let r = refresh.clone();
    window.connect_map(move |window| {
        if let Some(clock) = window.surface().map(|surface| surface.frame_clock()) {
            let r = r.clone();
            clock.connect_after_paint(move |_| r());
        }
        r();
    });
    let r = refresh.clone();
    adw::StyleManager::default().connect_high_contrast_notify(move |_| r());
    let (s, r) = (state.clone(), refresh.clone());
    display.connect_composited_notify(move |_| {
        s.borrow_mut().backend.poll();
        r();
    });
    let weak = window.downgrade();
    glib::timeout_add_seconds_local(1, move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        state.borrow_mut().backend.poll();
        refresh();
        glib::ControlFlow::Continue
    });
    // Leaving the window, leave the display painted for any other.
    window.connect_destroy(|_| super::style::set_translucent(false));
}

fn apply(window: &adw::ApplicationWindow, panes: &[gtk::Widget], state: &mut State) {
    let display = gtk::prelude::WidgetExt::display(window);
    let mut capability = state.backend.capability();
    capability.composited = display.is_composited() && display.is_rgba();
    let manager = adw::StyleManager::default();
    let high_contrast = manager.is_high_contrast();
    let glass = decide(
        state.choice,
        capability,
        high_contrast,
        readable(manager.is_dark()),
    );
    if state.logged != Some((capability, glass)) {
        state.logged = Some((capability, glass));
        eprintln!(
            "trace-commons-shell: glass backend={} blur_granted={} composited={} high_contrast={high_contrast} glass={glass:?}",
            capability.backend.name(),
            capability.granted,
            capability.composited,
        );
    }
    let bounds: Vec<(f64, f64, f64, f64)> = panes
        .iter()
        .filter(|pane| pane.is_mapped())
        .filter_map(|pane| pane.compute_bounds(window))
        .map(|b| {
            (
                f64::from(b.x()),
                f64::from(b.y()),
                f64::from(b.width()),
                f64::from(b.height()),
            )
        })
        .collect();
    let region = blur_region(&bounds, window.surface_transform(), PANE_RADIUS);
    let Some(surface) = window.surface() else {
        return;
    };
    let mut effects = state
        .requests
        .update(glass, capability.granted, region)
        .into_iter()
        .collect::<std::collections::VecDeque<_>>();
    while let Some(effect) = effects.pop_front() {
        match effect {
            Effect::Sheet(translucent) => super::style::set_translucent(translucent),
            Effect::Request(region) => {
                let scale = f64::from(surface.scale_factor());
                if !state.backend.request(&surface, &region, scale) {
                    eprintln!(
                        "trace-commons-shell: glass request=unsent bands={}",
                        region.len()
                    );
                    // Never translucent without the blur behind it.
                    effects.clear();
                    effects.extend(state.requests.refused());
                }
                // Wayland's blur state is double-buffered on the surface:
                // it lands with GTK's next commit, so ask for one.
                window.queue_draw();
            }
            Effect::Unset => {
                state.backend.unset(&surface);
                window.queue_draw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAYLAND: Capability = Capability {
        backend: Backend::Wayland,
        granted: true,
        composited: true,
    };

    #[test]
    fn the_override_defaults_to_following_the_desktop() {
        assert_eq!(parse_override(None), Override::Auto);
        assert_eq!(parse_override(Some("")), Override::Auto);
        assert_eq!(parse_override(Some("bogus")), Override::Auto);
        assert_eq!(parse_override(Some("translucent")), Override::Auto);
        assert_eq!(parse_override(Some("painted")), Override::Painted);
        assert_eq!(parse_override(Some(" blur ")), Override::Blur);
    }

    /// Every capability, high-contrast and readability combination, under
    /// each override: blurred in exactly the granted, compositing, known
    /// backend without high contrast, and under Auto only where readable.
    #[test]
    fn blurred_only_where_blur_is_granted_and_composited() {
        for choice in [Override::Auto, Override::Painted, Override::Blur] {
            let mut blurred = Vec::new();
            for backend in [Backend::Wayland, Backend::X11, Backend::Other] {
                for granted in [false, true] {
                    for composited in [false, true] {
                        for high_contrast in [false, true] {
                            for readable in [false, true] {
                                let capability = Capability {
                                    backend,
                                    granted,
                                    composited,
                                };
                                if decide(choice, capability, high_contrast, readable)
                                    == Glass::Blurred
                                {
                                    blurred.push((backend, readable));
                                }
                            }
                        }
                    }
                }
            }
            let expected = match choice {
                Override::Painted => vec![],
                Override::Auto => vec![(Backend::Wayland, true), (Backend::X11, true)],
                Override::Blur => vec![
                    (Backend::Wayland, false),
                    (Backend::Wayland, true),
                    (Backend::X11, false),
                    (Backend::X11, true),
                ],
            };
            assert_eq!(blurred, expected, "{choice:?}");
        }
    }

    #[test]
    fn nothing_probed_is_painted_whatever_the_override() {
        for choice in [Override::Auto, Override::Painted, Override::Blur] {
            assert_eq!(
                decide(choice, Capability::NONE, false, true),
                Glass::Painted
            );
        }
    }

    #[test]
    fn high_contrast_paints_whatever_the_override() {
        for choice in [Override::Auto, Override::Painted, Override::Blur] {
            assert_eq!(
                decide(choice, WAYLAND, true, true),
                Glass::Painted,
                "{choice:?}"
            );
        }
    }

    /// Today neither scheme's text reads over every desktop, so a KDE box
    /// with blur granted stays painted unless the override asks for blur.
    #[test]
    fn a_granted_kde_box_is_painted_until_its_text_reads() {
        for dark in [false, true] {
            assert!(!readable(dark));
            assert_eq!(
                decide(Override::Auto, WAYLAND, false, readable(dark)),
                Glass::Painted
            );
            assert_eq!(
                decide(Override::Blur, WAYLAND, false, readable(dark)),
                Glass::Blurred
            );
        }
    }

    fn area(region: &[Rect]) -> i64 {
        region
            .iter()
            .map(|r| i64::from(r.width) * i64::from(r.height))
            .sum()
    }

    fn covers(region: &[Rect], x: i32, y: i32) -> bool {
        region
            .iter()
            .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
    }

    #[test]
    fn a_pane_is_blurred_to_its_rounded_corner_and_no_further() {
        let pane = Rect {
            x: 10,
            y: 30,
            width: 400,
            height: 300,
        };
        let region = rounded_bands(pane, 20);
        // The rounded rectangle's area: w*h less (4 - pi) r^2.
        let exact = 400.0 * 300.0 - (4.0 - std::f64::consts::PI) * 400.0;
        // Within 0.1%: each corner row is rounded by at most half a pixel.
        assert!(
            (area(&region) as f64 - exact).abs() < exact / 1000.0,
            "area {} vs {exact}",
            area(&region)
        );
        // The corners' outer pixels are outside; the edges' middles inside.
        for (x, y) in [(10, 30), (409, 30), (10, 329), (409, 329), (12, 32)] {
            assert!(!covers(&region, x, y), "({x}, {y}) is outside the curve");
        }
        for (x, y) in [(210, 30), (210, 329), (10, 180), (409, 180), (30, 30)] {
            assert!(covers(&region, x, y), "({x}, {y}) is on the pane");
        }
        // Nothing reaches past the pane, and no band overlaps another.
        for band in &region {
            assert!(band.x >= 10 && band.x + band.width <= 410, "{band:?}");
            assert!(band.y >= 30 && band.y + band.height <= 330, "{band:?}");
        }
        let mut rows: Vec<i32> = region.iter().flat_map(|r| r.y..r.y + r.height).collect();
        let all = rows.len();
        rows.dedup();
        assert_eq!(rows.len(), all);
        assert_eq!(all, 300);
        // Mirror images top to bottom.
        let top: Vec<i32> = region.iter().take(5).map(|r| r.width).collect();
        let bottom: Vec<i32> = region.iter().rev().take(5).map(|r| r.width).collect();
        assert_eq!(top, bottom);
    }

    #[test]
    fn a_band_count_stays_small() {
        // One band per corner row at most, plus the middle.
        let region = rounded_bands(
            Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            20,
        );
        assert!(region.len() <= 41, "{}", region.len());
    }

    #[test]
    fn a_radius_larger_than_the_pane_is_clamped_and_nothing_is_empty_or_negative() {
        let small = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 30,
        };
        let region = rounded_bands(small, 20);
        assert!(region.iter().all(|r| r.width > 0 && r.height > 0));
        assert!(covers(&region, 5, 15));
        assert!(
            rounded_bands(
                Rect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 30
                },
                20
            )
            .is_empty()
        );
    }

    #[test]
    fn the_region_is_the_panes_in_surface_coordinates_and_not_the_gaps() {
        // Two panes 10 apart (`space.paneGap`), the window 23,17 into its
        // surface (the shadow margin).
        let region = blur_region(
            &[(10.0, 10.0, 300.0, 400.0), (320.0, 10.0, 200.0, 400.0)],
            (23.0, 17.4),
            PANE_RADIUS,
        );
        assert!(covers(&region, 23 + 160, 17 + 200));
        assert!(covers(&region, 23 + 420, 17 + 200));
        // The gap between them and the margin round the panes are not.
        for x in [23 + 312, 23 + 318] {
            assert!(!covers(&region, x, 17 + 200), "gap at {x}");
        }
        assert!(!covers(&region, 23 + 5, 17 + 200));
        assert!(!covers(&region, 23 + 160, 17 + 5));
        // The top edge lands on the rounded row: 10 + 17.4 rounds to 27.
        assert_eq!(region.iter().map(|r| r.y).min(), Some(27));
        assert!(blur_region(&[], (0.0, 0.0), PANE_RADIUS).is_empty());
    }

    #[test]
    fn the_pane_radius_is_the_macs_flat_pane() {
        // GlassTokens.swift:477, `pane: GlassTheme.pick(16, flat: 20)`.
        assert_eq!(PANE_RADIUS, 20);
    }

    #[test]
    fn x11_cardinals_match_kwindowsystem_rounding() {
        let rect = Rect {
            x: 23,
            y: 17,
            width: 801,
            height: 601,
        };
        assert_eq!(x11_cardinals(&[rect], 1.0), vec![23, 17, 801, 601]);
        assert_eq!(x11_cardinals(&[rect], 1.5), vec![34, 25, 1202, 902]);
        assert_eq!(
            x11_cardinals(&[rect, rect], 2.0),
            vec![46, 34, 1602, 1202, 46, 34, 1602, 1202]
        );
    }

    fn band(y: i32) -> Vec<Rect> {
        vec![Rect {
            x: 0,
            y,
            width: 100,
            height: 100,
        }]
    }

    #[test]
    fn a_grant_loads_the_translucent_sheet_and_asks_for_the_panes() {
        let mut requests = Requests::default();
        assert_eq!(
            requests.update(Glass::Painted, false, band(0)),
            vec![Effect::Sheet(false)]
        );
        assert_eq!(
            requests.update(Glass::Blurred, true, band(0)),
            vec![Effect::Sheet(true), Effect::Request(band(0))]
        );
        // A frame that moves nothing sends nothing.
        assert_eq!(requests.update(Glass::Blurred, true, band(0)), vec![]);
        // A pane that moves is asked for again.
        assert_eq!(
            requests.update(Glass::Blurred, true, band(5)),
            vec![Effect::Request(band(5))]
        );
    }

    #[test]
    fn a_revoked_grant_withdraws_the_request_and_paints() {
        let mut requests = Requests::default();
        requests.update(Glass::Blurred, true, band(0));
        assert_eq!(
            requests.update(Glass::Painted, false, band(0)),
            vec![Effect::Sheet(false), Effect::Unset]
        );
        // Granted again: asked again, though the region is the same.
        assert_eq!(
            requests.update(Glass::Blurred, true, band(0)),
            vec![Effect::Sheet(true), Effect::Request(band(0))]
        );
    }

    #[test]
    fn an_empty_region_is_withdrawn_never_sent() {
        // To both protocols an empty region means the whole surface.
        let mut requests = Requests::default();
        assert_eq!(
            requests.update(Glass::Blurred, true, vec![]),
            vec![Effect::Sheet(true)]
        );
        requests.update(Glass::Blurred, true, band(0));
        assert_eq!(
            requests.update(Glass::Blurred, true, vec![]),
            vec![Effect::Unset]
        );
        assert_eq!(requests.update(Glass::Blurred, true, vec![]), vec![]);
        assert_eq!(
            requests.update(Glass::Blurred, true, band(0)),
            vec![Effect::Request(band(0))]
        );
    }

    /// A request that does not go out leaves no translucent window behind
    /// it: paint, withdraw, and stay painted while the same grant lasts.
    #[test]
    fn an_unsent_request_paints_and_withdraws_until_the_grant_returns() {
        let mut requests = Requests::default();
        requests.update(Glass::Blurred, true, band(0));
        assert_eq!(
            requests.refused(),
            vec![Effect::Sheet(false), Effect::Unset]
        );
        assert_eq!(requests.update(Glass::Blurred, true, band(0)), vec![]);
        assert_eq!(requests.update(Glass::Blurred, true, band(5)), vec![]);
        // The grant goes and comes back: asked again.
        assert_eq!(requests.update(Glass::Painted, false, band(5)), vec![]);
        assert_eq!(
            requests.update(Glass::Blurred, true, band(5)),
            vec![Effect::Sheet(true), Effect::Request(band(5))]
        );
    }
}
