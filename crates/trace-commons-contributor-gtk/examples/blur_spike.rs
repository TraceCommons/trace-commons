// Spike S1/S2 of docs/superpowers/specs/2026-11-16-linux-mac-window-and-map-design.md.
// Not for merge. Pure decisions live here, tested on any host.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// What the window asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Auto,
    BlurOff,
    Painted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Glass {
    Painted,
    Translucent { request_blur: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn parse_mode(raw: Option<&str>) -> Mode {
    match raw.map(str::trim) {
        Some("blur-off") => Mode::BlurOff,
        Some("painted") => Mode::Painted,
        _ => Mode::Auto,
    }
}

/// Translucency without blur fails the tokens' contrast floors over an
/// arbitrary desktop, so `Auto` is translucent only where blur was actually
/// bound. `BlurOff` exists only to compare against.
fn glass_for(mode: Mode, blur_available: bool) -> Glass {
    match mode {
        Mode::Auto if blur_available => Glass::Translucent { request_blur: true },
        Mode::Auto | Mode::Painted => Glass::Painted,
        Mode::BlurOff => Glass::Translucent {
            request_blur: false,
        },
    }
}

/// The window widget's box in surface coordinates: GTK draws client-side
/// decorations, so the surface is larger than the window by the shadow
/// margin, and blur must not reach under the shadow.
fn content_rect(offset: (f64, f64), width: i32, height: i32) -> Rect {
    Rect {
        x: offset.0.round() as i32,
        y: offset.1.round() as i32,
        width,
        height,
    }
}

/// `_KDE_NET_WM_BLUR_BEHIND_REGION` is CARDINAL/32, x y w h per rectangle,
/// in device pixels (KWindowSystem kwindoweffects.cpp, KWin blur.cpp).
fn x11_blur_cardinals(rect: Rect, scale: f64) -> Vec<u32> {
    let scaled = |v: i32| f64::from(v) * scale;
    vec![
        scaled(rect.x).floor().max(0.0) as u32,
        scaled(rect.y).floor().max(0.0) as u32,
        scaled(rect.width).ceil().max(0.0) as u32,
        scaled(rect.height).ceil().max(0.0) as u32,
    ]
}

#[cfg(target_os = "linux")]
fn main() -> gtk::glib::ExitCode {
    linux::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("blur_spike linux-only");
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Glass, Rect, content_rect, glass_for, parse_mode, x11_blur_cardinals};
    use adw::prelude::*;
    use gtk::{gdk, glib};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const CSS: &str = r#"
window.blur-spike-translucent { background-color: rgba(246, 245, 250, 0.42); }
window.blur-spike-painted { background-color: rgb(246, 245, 250); }
window.blur-spike-translucent headerbar { background-color: rgba(255, 255, 255, 0.30); box-shadow: none; }
window.blur-spike-translucent .veil { background-color: rgba(255, 255, 255, 0.28); }
window.blur-spike-painted .veil { background-color: rgb(236, 234, 244); }
window.blur-spike-translucent .tier { background-color: rgba(110, 86, 207, 0.22); border-radius: 12px; }
window.blur-spike-painted .tier { background-color: rgb(214, 208, 240); border-radius: 12px; }
window.blur-spike-translucent .pane { background-color: rgba(255, 255, 255, 0.50); }
window.blur-spike-painted .pane { background-color: rgb(250, 249, 253); }
"#;

    pub(super) fn run() -> glib::ExitCode {
        let app = adw::Application::builder()
            .application_id("ai.tracecommons.BlurSpike")
            .build();
        app.connect_activate(activate);
        app.run_with_args::<&str>(&[])
    }

    fn env(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    fn env_ms(name: &str) -> Option<u64> {
        env(name).and_then(|v| v.parse().ok())
    }

    fn activate(app: &adw::Application) {
        let display = gdk::Display::default().expect("display");
        let provider = gtk::CssProvider::new();
        provider.load_from_data(CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let mode = parse_mode(env("TC_BLUR_SPIKE_MODE").as_deref());
        let backend = Backend::probe(&display);
        let glass = glass_for(mode, backend.blur_available());
        eprintln!(
            "blur_spike backend={} blur_available={} mode={mode:?} glass={glass:?}",
            backend.name(),
            backend.blur_available()
        );

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("blur_spike")
            .default_width(760)
            .default_height(480)
            .build();
        window.set_titlebar(Some(&gtk::HeaderBar::new()));
        window.add_css_class(match glass {
            Glass::Painted => "blur-spike-painted",
            Glass::Translucent { .. } => "blur-spike-translucent",
        });

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 8);
        sidebar.add_css_class("veil");
        sidebar.set_size_request(200, -1);
        for icon in [
            "go-home-symbolic",
            "view-grid-symbolic",
            "emblem-shared-symbolic",
        ] {
            let image = gtk::Image::from_icon_name(icon);
            image.set_pixel_size(24);
            image.set_margin_top(12);
            sidebar.append(&image);
        }
        let main = gtk::Box::new(gtk::Orientation::Vertical, 12);
        main.set_hexpand(true);
        main.set_margin_start(16);
        main.set_margin_end(16);
        main.set_margin_top(16);
        main.set_margin_bottom(16);
        for _ in 0..3 {
            let card = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            card.add_css_class("tier");
            card.set_size_request(-1, 96);
            main.append(&card);
        }
        let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        pane.add_css_class("pane");
        pane.set_size_request(640, -1);
        pane.set_visible(false);
        row.append(&sidebar);
        row.append(&main);
        row.append(&pane);
        window.set_child(Some(&row));

        let blur = Rc::new(RefCell::new(BlurState::new(backend, glass)));
        {
            let blur = blur.clone();
            window.connect_map(move |window| {
                let Some(surface) = window.surface() else {
                    return;
                };
                let w = window.clone();
                let b = blur.clone();
                surface.connect_layout(move |_, _, _| b.borrow_mut().apply(&w));
                blur.borrow_mut().apply(window);
            });
        }

        if env("TC_BLUR_SPIKE_MAXIMIZE").is_some() {
            window.maximize();
        }
        window.present();
        schedule_s2(&window, &pane);

        if let Some(ms) = env_ms("TC_BLUR_SPIKE_QUIT_MS") {
            let app = app.clone();
            glib::timeout_add_local_once(Duration::from_millis(ms), move || app.quit());
        }
    }

    /// Spike S2: grow at the right edge after map, then shrink back.
    /// `TC_BLUR_SPIKE_GROW=default-size` calls set_default_size on the mapped
    /// window; `size-request` shows a 640 px pane, raising the minimum width past the
    /// current width. Both then shrink back to the first width.
    fn schedule_s2(window: &gtk::ApplicationWindow, pane: &gtk::Box) {
        let Some(method) = env("TC_BLUR_SPIKE_GROW") else {
            return;
        };
        let step = Duration::from_millis(env_ms("TC_BLUR_SPIKE_STEP_MS").unwrap_or(2000));
        let report = |phase: &str, window: &gtk::ApplicationWindow| {
            let (dw, dh) = window.default_size();
            let surface = window
                .surface()
                .map(|s| (s.width(), s.height()))
                .unwrap_or((-1, -1));
            eprintln!(
                "s2 phase={phase} method={method} widget={}x{} default={dw}x{dh} surface={}x{} maximized={}",
                window.width(),
                window.height(),
                surface.0,
                surface.1,
                window.is_maximized(),
                method = env("TC_BLUR_SPIKE_GROW").unwrap_or_default(),
            );
        };
        let (w1, w2, w3) = (window.clone(), window.clone(), window.clone());
        let (p1, p2) = (pane.clone(), pane.clone());
        let m1 = method.clone();
        glib::timeout_add_local_once(step, move || {
            report("before", &w1);
            if m1 == "size-request" {
                p1.set_visible(true);
            } else {
                let (w, h) = (w1.width(), w1.height());
                w1.set_default_size(w + 320, h);
            }
        });
        glib::timeout_add_local_once(step * 2, move || {
            report("grown", &w2);
            if method == "size-request" {
                p2.set_visible(false);
            }
            w2.set_default_size(760, w2.height());
        });
        glib::timeout_add_local_once(step * 3, move || report("shrunk", &w3));
    }

    enum Backend {
        Wayland(Box<wayland::Blur>),
        WaylandNoBlur,
        X11(x11::Blur),
        X11NoBlur,
        Other,
    }

    impl Backend {
        fn probe(display: &gdk::Display) -> Backend {
            if let Some(wl) = display.downcast_ref::<gdk4_wayland::WaylandDisplay>() {
                return match wayland::Blur::bind(wl) {
                    Some(blur) => Backend::Wayland(Box::new(blur)),
                    None => Backend::WaylandNoBlur,
                };
            }
            if display.downcast_ref::<gdk4_x11::X11Display>().is_some() {
                return match x11::Blur::connect() {
                    Some(blur) => Backend::X11(blur),
                    None => Backend::X11NoBlur,
                };
            }
            Backend::Other
        }

        fn name(&self) -> &'static str {
            match self {
                Backend::Wayland(_) | Backend::WaylandNoBlur => "wayland",
                Backend::X11(_) | Backend::X11NoBlur => "x11",
                Backend::Other => "other",
            }
        }

        fn blur_available(&self) -> bool {
            matches!(self, Backend::Wayland(_) | Backend::X11(_))
        }
    }

    struct BlurState {
        backend: Backend,
        request: bool,
        last: Option<Rect>,
    }

    impl BlurState {
        fn new(backend: Backend, glass: Glass) -> Self {
            BlurState {
                backend,
                request: matches!(glass, Glass::Translucent { request_blur: true }),
                last: None,
            }
        }

        fn apply(&mut self, window: &gtk::ApplicationWindow) {
            if !self.request {
                return;
            }
            let Some(surface) = window.surface() else {
                return;
            };
            let rect = content_rect(window.surface_transform(), window.width(), window.height());
            if rect.width <= 0 || rect.height <= 0 || self.last == Some(rect) {
                return;
            }
            self.last = Some(rect);
            let sent = match &mut self.backend {
                Backend::Wayland(blur) => blur.set(&surface, rect),
                Backend::X11(blur) => blur.set(
                    &surface,
                    x11_blur_cardinals(rect, f64::from(surface.scale_factor())),
                ),
                _ => false,
            };
            eprintln!("blur_spike request rect={rect:?} sent={sent}");
            // The blur state is double-buffered on the wl_surface: it lands
            // with GTK's next commit, so ask for one.
            window.queue_draw();
        }
    }

    mod wayland {
        use super::Rect;
        use gdk4_wayland::prelude::*;
        use gtk::gdk;
        use wayland_client::globals::{GlobalListContents, registry_queue_init};
        use wayland_client::protocol::{wl_compositor, wl_region, wl_registry, wl_surface};
        use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
        use wayland_protocols_plasma::blur::client::{
            org_kde_kwin_blur, org_kde_kwin_blur_manager,
        };

        pub(crate) struct State;

        impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
            fn event(
                _: &mut Self,
                _: &wl_registry::WlRegistry,
                _: wl_registry::Event,
                _: &GlobalListContents,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
        delegate_noop!(State: ignore wl_compositor::WlCompositor);
        delegate_noop!(State: ignore wl_region::WlRegion);
        delegate_noop!(State: ignore org_kde_kwin_blur_manager::OrgKdeKwinBlurManager);
        delegate_noop!(State: ignore org_kde_kwin_blur::OrgKdeKwinBlur);

        pub(crate) struct Blur {
            conn: Connection,
            _queue: EventQueue<State>,
            qh: QueueHandle<State>,
            compositor: wl_compositor::WlCompositor,
            manager: org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
            current: Option<(wl_surface::WlSurface, org_kde_kwin_blur::OrgKdeKwinBlur)>,
        }

        impl Blur {
            /// Binds org_kde_kwin_blur_manager on GTK's own connection, through
            /// a private event queue. None where the compositor does not
            /// advertise it (GNOME, weston).
            pub(crate) fn bind(display: &gdk4_wayland::WaylandDisplay) -> Option<Blur> {
                let wl_display = display.wl_display()?;
                let backend = wl_display.backend().upgrade()?;
                let conn = Connection::from_backend(backend);
                let (globals, queue) = match registry_queue_init::<State>(&conn) {
                    Ok(pair) => pair,
                    Err(err) => {
                        eprintln!("blur_spike registry_error={err}");
                        return None;
                    }
                };
                let names: Vec<String> = globals
                    .contents()
                    .clone_list()
                    .into_iter()
                    .map(|g| format!("{}@{}", g.interface, g.version))
                    .filter(|n| n.contains("blur") || n.contains("contrast"))
                    .collect();
                eprintln!("blur_spike wayland_globals_matching={names:?}");
                let qh = queue.handle();
                let manager = globals
                    .bind::<org_kde_kwin_blur_manager::OrgKdeKwinBlurManager, _, _>(&qh, 1..=1, ())
                    .ok()?;
                let compositor = globals
                    .bind::<wl_compositor::WlCompositor, _, _>(&qh, 1..=4, ())
                    .ok()?;
                Some(Blur {
                    conn,
                    _queue: queue,
                    qh,
                    compositor,
                    manager,
                    current: None,
                })
            }

            pub(crate) fn set(&mut self, surface: &gdk::Surface, rect: Rect) -> bool {
                let Some(wl_surface) = surface
                    .downcast_ref::<gdk4_wayland::WaylandSurface>()
                    .and_then(|s| s.wl_surface())
                else {
                    return false;
                };
                if self.current.as_ref().map(|(s, _)| s.id()) != Some(wl_surface.id()) {
                    let blur = self.manager.create(&wl_surface, &self.qh, ());
                    self.current = Some((wl_surface, blur));
                }
                let (_, blur) = self.current.as_ref().expect("set above");
                let region = self.compositor.create_region(&self.qh, ());
                region.add(rect.x, rect.y, rect.width, rect.height);
                blur.set_region(Some(&region));
                blur.commit();
                region.destroy();
                self.conn.flush().is_ok()
            }
        }
    }

    mod x11 {
        use gtk::gdk;
        use gtk::prelude::*;
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
        use x11rb::rust_connection::RustConnection;
        use x11rb::wrapper::ConnectionExt as _;

        const ATOM: &[u8] = b"_KDE_NET_WM_BLUR_BEHIND_REGION";

        pub(crate) struct Blur {
            conn: RustConnection,
            atom: u32,
        }

        impl Blur {
            /// A second, pure-Rust X connection (no Xlib, no unsafe). KWin
            /// announces support by putting the atom on the root window
            /// (EffectsHandler::announceSupportProperty), so its presence
            /// there is the availability test.
            pub(crate) fn connect() -> Option<Blur> {
                let (conn, screen) = x11rb::connect(None).ok()?;
                let root = conn.setup().roots.get(screen)?.root;
                let atom = conn.intern_atom(false, ATOM).ok()?.reply().ok()?.atom;
                let props = conn.list_properties(root).ok()?.reply().ok()?.atoms;
                let announced = props.contains(&atom);
                eprintln!("blur_spike x11_root_announces_blur={announced}");
                announced.then_some(Blur { conn, atom })
            }

            pub(crate) fn set(&mut self, surface: &gdk::Surface, cardinals: Vec<u32>) -> bool {
                let Some(xid) = surface
                    .downcast_ref::<gdk4_x11::X11Surface>()
                    .map(|s| s.xid())
                else {
                    return false;
                };
                let Ok(xid) = u32::try_from(xid) else {
                    return false;
                };
                self.conn
                    .change_property32(
                        PropMode::REPLACE,
                        xid,
                        self.atom,
                        AtomEnum::CARDINAL,
                        &cardinals,
                    )
                    .is_ok()
                    && self.conn.flush().is_ok()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_defaults_to_auto_and_reads_the_two_overrides() {
        assert_eq!(parse_mode(None), Mode::Auto);
        assert_eq!(parse_mode(Some("")), Mode::Auto);
        assert_eq!(parse_mode(Some("auto")), Mode::Auto);
        assert_eq!(parse_mode(Some("blur-off")), Mode::BlurOff);
        assert_eq!(parse_mode(Some("painted")), Mode::Painted);
        assert_eq!(parse_mode(Some("bogus")), Mode::Auto);
    }

    #[test]
    fn auto_is_translucent_only_where_blur_is_available() {
        assert_eq!(
            glass_for(Mode::Auto, true),
            Glass::Translucent { request_blur: true }
        );
        assert_eq!(glass_for(Mode::Auto, false), Glass::Painted);
    }

    #[test]
    fn blur_off_is_translucent_without_a_request_for_comparison() {
        assert_eq!(
            glass_for(Mode::BlurOff, true),
            Glass::Translucent {
                request_blur: false
            }
        );
        assert_eq!(
            glass_for(Mode::BlurOff, false),
            Glass::Translucent {
                request_blur: false
            }
        );
    }

    #[test]
    fn painted_is_painted_everywhere() {
        assert_eq!(glass_for(Mode::Painted, true), Glass::Painted);
        assert_eq!(glass_for(Mode::Painted, false), Glass::Painted);
    }

    #[test]
    fn content_rect_skips_the_csd_shadow_margin() {
        assert_eq!(
            content_rect((23.0, 17.6), 800, 600),
            Rect {
                x: 23,
                y: 18,
                width: 800,
                height: 600
            }
        );
    }

    #[test]
    fn x11_cardinals_match_kwindowsystem_rounding() {
        // KWindowEffectsPrivateX11::enableBlurBehind: floor x, y and ceil
        // width, height after multiplying by the device pixel ratio.
        let rect = Rect {
            x: 23,
            y: 17,
            width: 801,
            height: 601,
        };
        assert_eq!(x11_blur_cardinals(rect, 1.0), vec![23, 17, 801, 601]);
        assert_eq!(x11_blur_cardinals(rect, 1.5), vec![34, 25, 1202, 902]);
        assert_eq!(x11_blur_cardinals(rect, 2.0), vec![46, 34, 1602, 1202]);
    }
}
