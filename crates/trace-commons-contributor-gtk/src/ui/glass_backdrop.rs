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
//! Blur alone does not make the shell's text read over every desktop, so
//! on Wayland KWin's background contrast (`org_kde_kwin_contrast`) is asked
//! for behind the same region, with a [`Frost`] per scheme, and the window
//! goes translucent on its own only where [`readable`] says the text then
//! reads: today the dark scheme with the contrast granted. KWin 6.4 to 6.6
//! advertise the contrast global; KWin's development branch has removed
//! it, and there, and on X11, the window stays painted.
//!
//! Not verified on a running KWin: the GCP KDE box has no DRM device to
//! run one on. What is verified is KWin's own source for the matrix
//! ([`frosted`]) and the contrast measurement in `style.rs`.
//!
//! Every decision is pure and tested here ([`decide`], [`Requests`],
//! [`blur_region`], [`watch`], [`frosted`]); the protocol plumbing sits
//! behind [`BlurBackend`], in the Linux-only `kde` module.

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
    /// KWin's background contrast is reachable too: the Wayland
    /// `org_kde_kwin_contrast_manager` global is bound. Only then is the
    /// [`Frost`] behind the panes, and only then can [`readable`] hold.
    pub contrast: bool,
}

impl Capability {
    /// Nothing to ask: not Linux, or no display backend this module knows.
    pub const NONE: Capability = Capability {
        backend: Backend::Other,
        granted: false,
        composited: false,
        contrast: false,
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

/// KWin's background contrast, asked for behind the same pane region as
/// the blur (`org_kde_kwin_contrast`, plasma-wayland-protocols
/// `contrast.xml`, v2 in `wayland-protocols-plasma` 0.3.12). KWin draws the
/// blurred desktop through a colour matrix built from `contrast`,
/// `intensity` and `saturation` ([`frosted`]) before the translucent pane
/// is laid over it.
///
/// `colour` goes out with `set_frost` (v2), the pane's own fill as the
/// protocol asks. No KWin release draws it: KWin 6.4's contrast effect and
/// 6.5's and 6.6's blur effect store it (`src/wayland/contrast.cpp`,
/// `ContrastInterface::frost`) and never read it, so [`readable`] does
/// not count on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frost {
    pub contrast: f64,
    pub intensity: f64,
    pub saturation: f64,
    /// `set_frost`'s red, green, blue and alpha, 0 to 255.
    pub colour: [u8; 4],
}

/// The frost asked for in a scheme. Only `contrast` changes the desktop:
/// KWin 6.5.2 to 6.6 apply `contrast` and `saturation` and ignore
/// `intensity` (`BlurEffect::updateBlurRegion`), so `intensity` stays at
/// 1 and every KWin that advertises the global draws the same. (6.5.0
/// and 6.5.1 advertise none; they draw a fixed contrast of their own.)
/// `saturation` stays at 1: it does not move a grey desktop, the worst
/// case measured.
pub fn frost(dark: bool) -> Frost {
    Frost {
        contrast: if dark {
            FROST_CONTRAST_DARK
        } else {
            FROST_CONTRAST_LIGHT
        },
        intensity: 1.0,
        saturation: 1.0,
        colour: pane_colour(dark),
    }
}

/// The largest `contrast`, in steps of 0.05, at which every text colour
/// reads on the frosted dark panes: the most desktop that can show through
/// (`style.rs`, `the_frost_shows_as_much_desktop_as_the_text_allows`). The
/// worst text there is 4.63:1; at 0.40 `tc_muted` falls to 4.49:1 over a
/// white desktop.
pub const FROST_CONTRAST_DARK: f64 = 0.35;
/// None in light: no contrast makes its text read (at 0, a mid-grey
/// desktop, `tc_muted` is 3.93:1), and the intensity that would lift it is
/// ignored by KWin 6.5 and 6.6. Light asks for no change and stays painted.
pub const FROST_CONTRAST_LIGHT: f64 = 1.0;

/// The translucent pane's fill (`tc_glass_pane` in the blur sheet), as
/// `set_frost` takes it.
pub fn pane_colour(dark: bool) -> [u8; 4] {
    let sheet = if dark {
        super::style::brand_tokens::GLASS_DARK_BLUR_CSS
    } else {
        super::style::brand_tokens::GLASS_LIGHT_BLUR_CSS
    };
    let parsed = sheet.lines().find_map(|line| {
        let inner = line
            .strip_prefix("@define-color tc_glass_pane rgba(")?
            .strip_suffix(");")?;
        let parts: Vec<f64> = inner
            .split(',')
            .map(|part| part.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [r, g, b, a] = parts[..] else { return None };
        Some([r, g, b, a * 255.0].map(|v| v.round().clamp(0.0, 255.0) as u8))
    });
    parsed.unwrap_or([0, 0, 0, 255])
}

/// What KWin draws for one blurred desktop colour (channels 0 to 1) under
/// `frost`: `tex * colorMatrix`, the matrix being the product
/// `contrast * saturation * intensity` (KWin 6.4
/// `ContrastEffect::colorMatrix` and its shader; 6.5/6.6
/// `colorTransformMatrix` and `onscreen.frag`, intensity left out there). Read as a row vector, the contrast matrix applies first:
/// `c * x + (1 - c) / 2` per channel; then saturation, mixing toward the
/// Rec. 709 luma (0.2126, 0.7152, 0.0722); then intensity scales. The
/// framebuffer clamps the result to 0..1.
///
/// The channels are encoded values, not linear light: KWin blends in gamma
/// 2.2 (`DrmOutput`, `blendingSpace`, 6.6), and the blur samples what was
/// blended. Were the matrix applied in linear light, no contrast would make
/// the dark scheme read (4.36:1 at best). An HDR output raises white past
/// the SDR reference, which this does not model.
pub fn frosted(desktop: [f64; 3], frost: Frost) -> [f64; 3] {
    let c = frost.contrast;
    let shift = (1.0 - c) / 2.0;
    let contrasted = desktop.map(|x| c * x + shift);
    let s = frost.saturation;
    let luma = [0.2126, 0.7152, 0.0722];
    let weighted: f64 = (0..3).map(|i| contrasted[i] * (1.0 - s) * luma[i]).sum();
    let saturated = contrasted.map(|x| x * s + weighted);
    saturated.map(|x| (x * frost.intensity).clamp(0.0, 1.0))
}

/// Whether every text colour the panes' screens use reads at 4.5:1 on the
/// translucent panes over a black and a white desktop, in and out of focus,
/// in this scheme, with what KWin granted behind them. Measured, not
/// chosen: `style.rs`'s `every_text_colour_reads_on_the_translucent_panes`
/// fails if this disagrees with the measurement either way.
///
/// Blur alone never reads: KWin's blur averages the desktop and adds no
/// frost of its own, so over a dark desktop a light pane is mid-grey (worst
/// 2.51:1) and over a white one a dark pane is too light (worst 3.33:1).
/// With KWin's background contrast granted, the [`frost`] behind the panes
/// is measured too: dark then reads everywhere, light still does not.
pub fn readable(dark: bool, contrast: bool) -> bool {
    // Dark reads at FROST_CONTRAST_DARK; light reads at no contrast.
    contrast && dark
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

/// What the follower keeps running between events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    /// Re-decide after every painted frame: the panes move with resizes,
    /// maximizing, tiling and pane widths without the surface resizing.
    pub frames: bool,
    /// Re-probe the grant once a second: KWin drops it when its blur effect
    /// unloads.
    pub poll: bool,
}

/// What to keep running: nothing while the window is unmapped, and
/// otherwise no per-frame or timed work unless the grant is held or blur
/// is forced. Frames only while blurred, since only a sent region follows
/// the panes. Without the grant, a focus change, a map, a compositing or a
/// high-contrast change probes again instead.
pub fn watch(choice: Override, granted: bool, glass: Glass, mapped: bool) -> Watch {
    if !mapped {
        return Watch {
            frames: false,
            poll: false,
        };
    }
    Watch {
        frames: glass == Glass::Blurred,
        poll: match choice {
            Override::Painted => false,
            Override::Auto => granted,
            Override::Blur => true,
        },
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

/// The region to blur: every shown pane, offset by `offset`, rounded
/// at the pane radius. Only the panes: on the Mac the gaps between them show
/// the desktop unblurred. `panes` are bounds in the window's coordinates
/// (`compute_bounds`); `offset` is where the window's origin sits in the
/// coordinates the region is sent in ([`region_offset`]).
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

/// Where the window's origin sits in the coordinates the region is sent in.
/// KWin reads a Wayland window's blur and contrast regions from the
/// window's own geometry (`contentsRect`), which leaves out the client-side
/// shadow, so there the region is in the window's coordinates; sent from
/// the surface's, it landed one shadow margin inside every pane. X11 keeps
/// the surface offset: not seen on a running KWin, and the window stays
/// painted there ([`readable`]).
pub fn region_offset(backend: Backend, surface_transform: (f64, f64)) -> (f64, f64) {
    match backend {
        Backend::Wayland => (0.0, 0.0),
        Backend::X11 | Backend::Other => surface_transform,
    }
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
    /// The frost that went with the last request.
    frost: Option<Frost>,
    /// A request did not go out. Translucency without blur is never shown,
    /// so the window paints until the grant is lost and given again.
    refused: bool,
}

impl Requests {
    /// How the window draws now, a refused request included.
    pub fn glass(&self) -> Glass {
        self.glass.unwrap_or(Glass::Painted)
    }

    /// `glass` as [`decide`] chose it, with `granted` the backend's grant
    /// and `frost` what goes behind the panes with the blur (`None` where
    /// KWin's contrast is not granted). A changed frost asks again.
    pub fn update(
        &mut self,
        glass: Glass,
        granted: bool,
        region: Vec<Rect>,
        frost: Option<Frost>,
    ) -> Vec<Effect> {
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
        if self.sent.as_ref() == Some(&region) && self.frost == frost {
            return effects;
        }
        self.frost = frost;
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
        self.frost = None;
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
    /// With `frost`, KWin's background contrast goes behind the same
    /// region; without it, any contrast asked for before is withdrawn.
    fn request(
        &mut self,
        surface: &gtk::gdk::Surface,
        region: &[Rect],
        scale: f64,
        frost: Option<Frost>,
    ) -> bool;
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
    fn request(&mut self, _: &gtk::gdk::Surface, _: &[Rect], _: f64, _: Option<Frost>) -> bool {
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

/// One window's follower: what it decided, and the frame and timer
/// watches it keeps only while [`watch`] asks for them.
struct Follower {
    window: glib::WeakRef<adw::ApplicationWindow>,
    panes: Vec<gtk::Widget>,
    state: std::cell::RefCell<State>,
    /// The `after-paint` handler, with the clock it is on: at most one.
    paint: std::cell::RefCell<Option<(gtk::gdk::FrameClock, glib::SignalHandlerId)>>,
    timer: std::cell::RefCell<Option<glib::SourceId>>,
    /// Inside the timer's own tick, which ends itself by returning
    /// `Break` rather than removing its source.
    ticking: std::cell::Cell<bool>,
}

impl Follower {
    /// Re-decide, then keep exactly the watches [`watch`] asks for.
    fn refresh(self: &std::rc::Rc<Self>) {
        let Some(window) = self.window.upgrade() else {
            self.stop();
            return;
        };
        let wanted = {
            let mut state = self.state.borrow_mut();
            apply(&window, &self.panes, &mut state);
            watch(
                state.choice,
                state.backend.capability().granted,
                state.requests.glass(),
                window.is_mapped(),
            )
        };
        self.keep(&window, wanted);
    }

    /// Ask the desktop again, then re-decide.
    fn probe(self: &std::rc::Rc<Self>) {
        self.state.borrow_mut().backend.poll();
        self.refresh();
    }

    fn keep(self: &std::rc::Rc<Self>, window: &adw::ApplicationWindow, wanted: Watch) {
        if !wanted.frames {
            if let Some((clock, handler)) = self.paint.borrow_mut().take() {
                clock.disconnect(handler);
            }
        } else if self.paint.borrow().is_none()
            && let Some(clock) = window.surface().map(|surface| surface.frame_clock())
        {
            let weak = std::rc::Rc::downgrade(self);
            let handler = clock.connect_after_paint(move |_| {
                if let Some(follower) = weak.upgrade() {
                    follower.refresh();
                }
            });
            *self.paint.borrow_mut() = Some((clock, handler));
        }
        if !wanted.poll {
            if let Some(timer) = self.timer.borrow_mut().take()
                && !self.ticking.get()
            {
                timer.remove();
            }
        } else if self.timer.borrow().is_none() {
            let weak = std::rc::Rc::downgrade(self);
            let timer = glib::timeout_add_seconds_local(1, move || {
                let Some(follower) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                follower.ticking.set(true);
                follower.probe();
                follower.ticking.set(false);
                if follower.timer.borrow().is_some() {
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
            *self.timer.borrow_mut() = Some(timer);
        }
    }

    /// Unmapped or gone: nothing runs until the next map.
    fn stop(self: &std::rc::Rc<Self>) {
        if let Some((clock, handler)) = self.paint.borrow_mut().take() {
            clock.disconnect(handler);
        }
        if let Some(timer) = self.timer.borrow_mut().take()
            && !self.ticking.get()
        {
            timer.remove();
        }
    }
}

/// Follow the desktop's blur for `window`, whose glass panes are `panes`.
/// Probes once, then re-decides on a map, a focus change, a compositing
/// change, a high-contrast or scheme change, and, only while [`watch`]
/// asks for them, after every painted frame (the panes move with resizes,
/// maximizing, tiling and pane widths without the surface resizing) and on
/// a one-second poll (a grant can be revoked: KWin drops the global when
/// its blur effect unloads). Unmapped, nothing runs.
pub fn follow(window: &adw::ApplicationWindow, panes: Vec<gtk::Widget>) {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let follower = std::rc::Rc::new(Follower {
        window: window.downgrade(),
        panes,
        state: std::cell::RefCell::new(State {
            backend: probe(&display),
            requests: Requests::default(),
            choice: parse_override(std::env::var(OVERRIDE_ENV).ok().as_deref()),
            logged: None,
        }),
        paint: Default::default(),
        timer: Default::default(),
        ticking: Default::default(),
    });

    let weak = std::rc::Rc::downgrade(&follower);
    window.connect_map(move |_| {
        if let Some(follower) = weak.upgrade() {
            follower.refresh();
        }
    });
    let weak = std::rc::Rc::downgrade(&follower);
    window.connect_unmap(move |_| {
        if let Some(follower) = weak.upgrade() {
            follower.stop();
        }
    });
    // Without the grant nothing polls: a focus change asks again, so a
    // blur effect loaded later is found the next time the window is used.
    let weak = std::rc::Rc::downgrade(&follower);
    window.connect_is_active_notify(move |_| {
        if let Some(follower) = weak.upgrade() {
            follower.probe();
        }
    });
    let manager = adw::StyleManager::default();
    let weak = std::rc::Rc::downgrade(&follower);
    manager.connect_high_contrast_notify(move |_| {
        if let Some(follower) = weak.upgrade() {
            follower.refresh();
        }
    });
    // The scheme picks the frost and whether its text reads.
    let weak = std::rc::Rc::downgrade(&follower);
    manager.connect_dark_notify(move |_| {
        if let Some(follower) = weak.upgrade() {
            follower.refresh();
        }
    });
    let weak = std::rc::Rc::downgrade(&follower);
    display.connect_composited_notify(move |_| {
        if let Some(follower) = weak.upgrade() {
            follower.probe();
        }
    });
    // A window already on screen has had its map: decide now.
    if window.is_mapped() {
        follower.refresh();
    }
    // Leaving the window, leave the display painted for any other, and
    // let the follower go with it.
    let held = std::cell::RefCell::new(Some(follower));
    window.connect_destroy(move |_| {
        if let Some(follower) = held.borrow_mut().take() {
            follower.stop();
        }
        super::style::set_translucent(false);
    });
}

fn apply(window: &adw::ApplicationWindow, panes: &[gtk::Widget], state: &mut State) {
    let display = gtk::prelude::WidgetExt::display(window);
    let mut capability = state.backend.capability();
    capability.composited = display.is_composited() && display.is_rgba();
    let manager = adw::StyleManager::default();
    let high_contrast = manager.is_high_contrast();
    let dark = manager.is_dark();
    // The Omarchy skin paints its own palette (`style::sheet_for_window`),
    // which the readability measurement does not cover: painted, as under
    // high contrast.
    let skinned = super::appearance::active_palette().is_some();
    let glass = decide(
        state.choice,
        capability,
        high_contrast || skinned,
        readable(dark, capability.contrast),
    );
    if state.logged != Some((capability, glass)) {
        state.logged = Some((capability, glass));
        eprintln!(
            "trace-commons-shell: glass backend={} blur_granted={} contrast_granted={} composited={} high_contrast={high_contrast} glass={glass:?}",
            capability.backend.name(),
            capability.granted,
            capability.contrast,
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
    let region = blur_region(
        &bounds,
        region_offset(capability.backend, window.surface_transform()),
        PANE_RADIUS,
    );
    let Some(surface) = window.surface() else {
        return;
    };
    let frost = capability.contrast.then(|| frost(dark));
    let mut effects = state
        .requests
        .update(glass, capability.granted, region, frost)
        .into_iter()
        .collect::<std::collections::VecDeque<_>>();
    while let Some(effect) = effects.pop_front() {
        match effect {
            Effect::Sheet(translucent) => super::style::set_translucent(translucent),
            Effect::Request(region) => {
                let scale = f64::from(surface.scale_factor());
                if !state.backend.request(&surface, &region, scale, frost) {
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
        contrast: false,
    };

    /// A changed frost (the scheme flipped, or KWin's contrast came or
    /// went) asks again over the same region; an unchanged one does not.
    #[test]
    fn a_changed_frost_asks_again() {
        let region = vec![Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        }];
        let mut requests = Requests::default();
        assert_eq!(
            requests.update(Glass::Blurred, true, region.clone(), None),
            vec![Effect::Sheet(true), Effect::Request(region.clone())]
        );
        assert_eq!(
            requests.update(Glass::Blurred, true, region.clone(), Some(frost(true))),
            vec![Effect::Request(region.clone())]
        );
        assert_eq!(
            requests.update(Glass::Blurred, true, region.clone(), Some(frost(true))),
            vec![]
        );
        assert_eq!(
            requests.update(Glass::Blurred, true, region.clone(), Some(frost(false))),
            vec![Effect::Request(region.clone())]
        );
        assert_eq!(
            requests.update(Glass::Blurred, true, region.clone(), None),
            vec![Effect::Request(region)]
        );
    }

    /// No frame or timer work without the grant (or forced blur), none
    /// while unmapped, and frames only while a region is being followed.
    #[test]
    fn the_follower_idles_unless_blur_is_in_play() {
        let idle = Watch {
            frames: false,
            poll: false,
        };
        for choice in [Override::Auto, Override::Painted, Override::Blur] {
            for granted in [false, true] {
                for glass in [Glass::Painted, Glass::Blurred] {
                    assert_eq!(watch(choice, granted, glass, false), idle);
                }
            }
        }
        // GNOME, weston, any desktop without the grant: nothing runs.
        assert_eq!(watch(Override::Auto, false, Glass::Painted, true), idle);
        assert_eq!(watch(Override::Painted, false, Glass::Painted, true), idle);
        // Granted but painted (Auto, unreadable): poll for revocation only.
        assert_eq!(
            watch(Override::Auto, true, Glass::Painted, true),
            Watch {
                frames: false,
                poll: true
            }
        );
        // Blurred: follow the panes and the grant.
        assert_eq!(
            watch(Override::Auto, true, Glass::Blurred, true),
            Watch {
                frames: true,
                poll: true
            }
        );
        // Forced blur waits for a grant on the timer.
        assert_eq!(
            watch(Override::Blur, false, Glass::Painted, true),
            Watch {
                frames: false,
                poll: true
            }
        );
    }

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
                                    contrast: false,
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

    /// Blur without KWin's contrast never reads over every desktop, so a
    /// KDE box with blur alone granted stays painted unless the override
    /// asks for blur.
    #[test]
    fn a_granted_kde_box_without_contrast_is_painted() {
        for dark in [false, true] {
            assert!(!readable(dark, false));
            assert_eq!(
                decide(Override::Auto, WAYLAND, false, readable(dark, false)),
                Glass::Painted
            );
            assert_eq!(
                decide(Override::Blur, WAYLAND, false, readable(dark, false)),
                Glass::Blurred
            );
        }
    }

    /// The frost is KWin's colour matrix, read off its source: contrast
    /// first (`c * x + (1 - c) / 2`), then saturation toward the Rec. 709
    /// luma, then intensity, clamped to 0..1.
    #[test]
    fn the_frost_is_kwins_colour_matrix() {
        let plain = Frost {
            contrast: 1.0,
            intensity: 1.0,
            saturation: 1.0,
            colour: [0, 0, 0, 255],
        };
        let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(close(frosted([0.2, 0.5, 0.9], plain), [0.2, 0.5, 0.9]));
        let half = Frost {
            contrast: 0.5,
            ..plain
        };
        assert!(close(frosted([0.0; 3], half), [0.25; 3]));
        assert!(close(frosted([1.0; 3], half), [0.75; 3]));
        let grey = Frost {
            saturation: 0.0,
            ..plain
        };
        assert!(close(frosted([1.0, 0.0, 0.0], grey), [0.2126; 3]));
        let dim = Frost {
            intensity: 0.5,
            ..plain
        };
        assert!(close(frosted([1.0, 0.4, 0.0], dim), [0.5, 0.2, 0.0]));
        // Contrast before saturation: a saturated colour pushed past 1 by
        // contrast is clamped only at the end.
        let both = Frost {
            contrast: 2.0,
            saturation: 0.0,
            ..plain
        };
        let luma: f64 = 0.2126 * 1.5 + 0.7152 * -0.5 + 0.0722 * -0.5;
        assert!(close(
            frosted([1.0, 0.0, 0.0], both),
            [luma.clamp(0.0, 1.0); 3]
        ));
    }

    /// The frost colour is the translucent pane's own fill.
    #[test]
    fn the_frost_colour_is_the_panes_fill() {
        for dark in [false, true] {
            let [_, _, _, alpha] = frost(dark).colour;
            assert!(alpha > 0, "dark={dark}");
            assert_ne!(frost(dark).colour, [0, 0, 0, 255], "dark={dark}");
        }
        assert_eq!(frost(true).colour, [20, 23, 38, 190]);
        assert_eq!(frost(false).colour, [229, 232, 237, 191]);
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

    /// KWin places a Wayland window's blur region from the window's own
    /// geometry (`contentsRect`), which excludes the client-side shadow, so
    /// the region is in window coordinates there. Taken from the surface
    /// instead, it landed one shadow margin (about 26 px) down and right of
    /// every pane: the second rounded edge inside each pane in the KDE
    /// screenshots. X11 keeps the surface offset (not seen on a KWin).
    #[test]
    fn on_wayland_the_region_is_in_the_windows_own_coordinates() {
        let shadow = (26.0, 26.0);
        let pane = [(10.0, 10.0, 300.0, 400.0)];
        let region = blur_region(&pane, region_offset(Backend::Wayland, shadow), PANE_RADIUS);
        // The pane's top-left, inside its rounded corner, is blurred, and
        // nothing past its right and bottom edges is.
        assert!(covers(&region, 10 + PANE_RADIUS, 10 + PANE_RADIUS));
        assert!(covers(&region, 10 + 150, 10 + 1));
        assert!(!covers(&region, 10 + 300 + 5, 10 + 200));
        assert!(!covers(&region, 10 + 150, 10 + 400 + 5));
        assert_eq!(region.iter().map(|r| r.y).min(), Some(10));
        assert_eq!(region.iter().map(|r| r.x).min(), Some(10));
        assert_eq!(region_offset(Backend::X11, shadow), shadow);
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
            requests.update(Glass::Painted, false, band(0), None),
            vec![Effect::Sheet(false)]
        );
        assert_eq!(
            requests.update(Glass::Blurred, true, band(0), None),
            vec![Effect::Sheet(true), Effect::Request(band(0))]
        );
        // A frame that moves nothing sends nothing.
        assert_eq!(requests.update(Glass::Blurred, true, band(0), None), vec![]);
        // A pane that moves is asked for again.
        assert_eq!(
            requests.update(Glass::Blurred, true, band(5), None),
            vec![Effect::Request(band(5))]
        );
    }

    #[test]
    fn a_revoked_grant_withdraws_the_request_and_paints() {
        let mut requests = Requests::default();
        requests.update(Glass::Blurred, true, band(0), None);
        assert_eq!(
            requests.update(Glass::Painted, false, band(0), None),
            vec![Effect::Sheet(false), Effect::Unset]
        );
        // Granted again: asked again, though the region is the same.
        assert_eq!(
            requests.update(Glass::Blurred, true, band(0), None),
            vec![Effect::Sheet(true), Effect::Request(band(0))]
        );
    }

    #[test]
    fn an_empty_region_is_withdrawn_never_sent() {
        // To both protocols an empty region means the whole surface.
        let mut requests = Requests::default();
        assert_eq!(
            requests.update(Glass::Blurred, true, vec![], None),
            vec![Effect::Sheet(true)]
        );
        requests.update(Glass::Blurred, true, band(0), None);
        assert_eq!(
            requests.update(Glass::Blurred, true, vec![], None),
            vec![Effect::Unset]
        );
        assert_eq!(requests.update(Glass::Blurred, true, vec![], None), vec![]);
        assert_eq!(
            requests.update(Glass::Blurred, true, band(0), None),
            vec![Effect::Request(band(0))]
        );
    }

    /// A request that does not go out leaves no translucent window behind
    /// it: paint, withdraw, and stay painted while the same grant lasts.
    #[test]
    fn an_unsent_request_paints_and_withdraws_until_the_grant_returns() {
        let mut requests = Requests::default();
        requests.update(Glass::Blurred, true, band(0), None);
        assert_eq!(
            requests.refused(),
            vec![Effect::Sheet(false), Effect::Unset]
        );
        assert_eq!(requests.update(Glass::Blurred, true, band(0), None), vec![]);
        assert_eq!(requests.update(Glass::Blurred, true, band(5), None), vec![]);
        // The grant goes and comes back: asked again.
        assert_eq!(
            requests.update(Glass::Painted, false, band(5), None),
            vec![]
        );
        assert_eq!(
            requests.update(Glass::Blurred, true, band(5), None),
            vec![Effect::Sheet(true), Effect::Request(band(5))]
        );
    }
}
