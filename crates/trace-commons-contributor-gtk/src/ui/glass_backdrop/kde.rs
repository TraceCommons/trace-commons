//! KWin's blur, behind [`BlurBackend`]: the protocol plumbing and nothing
//! else. Every decision is in the parent module, tested on any host. On
//! Wayland, KWin's background contrast (`org_kde_kwin_contrast_manager`
//! v2, plasma-wayland-protocols `contrast.xml`) goes behind the same region
//! with the frost the parent chose, and is withdrawn wherever the blur is.
//!
//! Lifted from the blur spike (#2162, `examples/blur_spike.rs`). Names are
//! from source: `org_kde_kwin_blur_manager` v1 and `org_kde_kwin_blur` v1
//! (plasma-wayland-protocols `blur.xml`, vendored in
//! `wayland-protocols-plasma` 0.3), and `_KDE_NET_WM_BLUR_BEHIND_REGION`
//! (KWindowSystem `kwindoweffects.cpp`, KWin `plugins/blur/blur.cpp`). KWin
//! advertises the Wayland global, and announces the atom on the X11 root
//! window, only while its blur effect is loaded, so what can be bound is the
//! grant. No `unsafe`: every FFI step is inside `gdk4-wayland`, `gdk4-x11`,
//! `wayland-client` and `x11rb`.

use super::BlurBackend;
use gtk::gdk;
use gtk::prelude::*;

/// The backend for `display`, or None where it is neither Wayland nor X11.
pub(super) fn probe(display: &gdk::Display) -> Option<Box<dyn BlurBackend>> {
    if let Some(display) = display.downcast_ref::<gdk4_wayland::WaylandDisplay>() {
        return Some(Box::new(wayland::Blur::connect(display)));
    }
    if display.downcast_ref::<gdk4_x11::X11Display>().is_some() {
        return Some(Box::new(x11::Blur::connect()));
    }
    None
}

mod wayland {
    use crate::ui::glass_backdrop::{Backend, BlurBackend, Capability, Frost, Rect};
    use gdk4_wayland::prelude::*;
    use gtk::gdk;
    use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_compositor, wl_region, wl_registry, wl_surface};
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
    use wayland_protocols_plasma::blur::client::{org_kde_kwin_blur, org_kde_kwin_blur_manager};
    use wayland_protocols_plasma::contrast::client::{
        org_kde_kwin_contrast, org_kde_kwin_contrast_manager,
    };

    const BLUR_MANAGER: &str = "org_kde_kwin_blur_manager";
    const CONTRAST_MANAGER: &str = "org_kde_kwin_contrast_manager";

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
    delegate_noop!(State: ignore org_kde_kwin_contrast_manager::OrgKdeKwinContrastManager);
    delegate_noop!(State: ignore org_kde_kwin_contrast::OrgKdeKwinContrast);

    struct BoundBlur {
        name: u32,
        manager: org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
        current: Option<(wl_surface::WlSurface, org_kde_kwin_blur::OrgKdeKwinBlur)>,
    }

    /// KWin's background contrast: its own global, advertised while the
    /// effect that draws it is loaded (KWin 6.4's contrast effect, 6.5's
    /// and 6.6's blur effect), and gone from KWin's development branch.
    struct BoundContrast {
        name: u32,
        manager: org_kde_kwin_contrast_manager::OrgKdeKwinContrastManager,
        current: Option<(
            wl_surface::WlSurface,
            org_kde_kwin_contrast::OrgKdeKwinContrast,
        )>,
    }

    impl BoundContrast {
        fn withdraw(&mut self, surface: &wl_surface::WlSurface) {
            if let Some((_, contrast)) = self.current.take() {
                contrast.release();
            }
            self.manager.unset(surface);
        }
    }

    struct Session {
        conn: Connection,
        queue: EventQueue<State>,
        globals: GlobalList,
        compositor: Option<wl_compositor::WlCompositor>,
        blur: Option<BoundBlur>,
        contrast: Option<BoundContrast>,
    }

    pub(crate) struct Blur {
        session: Option<Session>,
    }

    impl Blur {
        /// A registry of our own on GTK's `wl_display`, through a private
        /// event queue. GTK reads the socket; [`BlurBackend::poll`]
        /// dispatches what it queued for us (a global added or removed).
        pub(crate) fn connect(display: &gdk4_wayland::WaylandDisplay) -> Blur {
            let session = (|| {
                let wl_display = display.wl_display()?;
                let backend = wl_display.backend().upgrade()?;
                let conn = Connection::from_backend(backend);
                match registry_queue_init::<State>(&conn) {
                    Ok((globals, queue)) => Some(Session {
                        conn,
                        queue,
                        globals,
                        compositor: None,
                        blur: None,
                        contrast: None,
                    }),
                    Err(err) => {
                        eprintln!("trace-commons-shell: glass wayland_registry_error={err}");
                        None
                    }
                }
            })();
            let mut blur = Blur { session };
            blur.rebind();
            blur
        }

        /// A global's current name in the registry, if advertised.
        fn advertised(session: &Session, interface: &str) -> Option<u32> {
            session.globals.contents().with_list(|list| {
                list.iter()
                    .find(|global| global.interface == interface)
                    .map(|global| global.name)
            })
        }

        /// Bind each manager when it appears, and forget it when KWin
        /// removes it (its effect unloaded). The two come and go apart.
        fn rebind(&mut self) {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            let qh = session.queue.handle();
            let blur = Self::advertised(session, BLUR_MANAGER);
            if session.blur.as_ref().map(|bound| bound.name) != blur {
                if let Some(bound) = session.blur.take()
                    && let Some((_, old)) = bound.current
                {
                    old.release();
                }
                if let Some(name) = blur
                    && let Ok(manager) = session
                        .globals
                        .bind::<org_kde_kwin_blur_manager::OrgKdeKwinBlurManager, _, _>(
                        &qh,
                        1..=1,
                        (),
                    )
                {
                    session.blur = Some(BoundBlur {
                        name,
                        manager,
                        current: None,
                    });
                }
            }
            let contrast = Self::advertised(session, CONTRAST_MANAGER);
            if session.contrast.as_ref().map(|bound| bound.name) != contrast {
                if let Some(bound) = session.contrast.take()
                    && let Some((_, old)) = bound.current
                {
                    old.release();
                }
                if let Some(name) = contrast
                    && let Ok(manager) = session
                        .globals
                        .bind::<org_kde_kwin_contrast_manager::OrgKdeKwinContrastManager, _, _>(
                        &qh,
                        1..=2,
                        (),
                    )
                {
                    session.contrast = Some(BoundContrast {
                        name,
                        manager,
                        current: None,
                    });
                }
            }
            if session.compositor.is_none() && session.blur.is_some() {
                session.compositor = session
                    .globals
                    .bind::<wl_compositor::WlCompositor, _, _>(&qh, 1..=4, ())
                    .ok();
            }
        }
    }

    impl BlurBackend for Blur {
        fn capability(&self) -> Capability {
            let session = self.session.as_ref();
            Capability {
                backend: Backend::Wayland,
                granted: session
                    .is_some_and(|session| session.blur.is_some() && session.compositor.is_some()),
                // Filled in by the caller from the display.
                composited: false,
                contrast: session.is_some_and(|session| session.contrast.is_some()),
            }
        }

        fn poll(&mut self) {
            if let Some(session) = self.session.as_mut()
                && let Err(err) = session.queue.dispatch_pending(&mut State)
            {
                eprintln!("trace-commons-shell: glass wayland_dispatch_error={err}");
            }
            self.rebind();
        }

        fn request(
            &mut self,
            surface: &gdk::Surface,
            region: &[Rect],
            _scale: f64,
            frost: Option<Frost>,
        ) -> bool {
            let Some(session) = self.session.as_mut() else {
                return false;
            };
            let qh = session.queue.handle();
            let (Some(bound), Some(compositor)) =
                (session.blur.as_mut(), session.compositor.as_ref())
            else {
                return false;
            };
            let Some(wl_surface) = surface
                .downcast_ref::<gdk4_wayland::WaylandSurface>()
                .and_then(|surface| surface.wl_surface())
            else {
                return false;
            };
            if bound.current.as_ref().map(|(s, _)| s.id()) != Some(wl_surface.id()) {
                if let Some((_, old)) = bound.current.take() {
                    old.release();
                }
                let blur = bound.manager.create(&wl_surface, &qh, ());
                bound.current = Some((wl_surface.clone(), blur));
            }
            let Some((_, blur)) = bound.current.as_ref() else {
                return false;
            };
            // Surface-local, in logical pixels: the compositor scales it.
            let wl_region = compositor.create_region(&qh, ());
            for rect in region {
                wl_region.add(rect.x, rect.y, rect.width, rect.height);
            }
            blur.set_region(Some(&wl_region));
            blur.commit();
            // The frost behind exactly the same region, or none.
            if let Some(contrast) = session.contrast.as_mut() {
                match frost {
                    Some(frost) => {
                        if contrast.current.as_ref().map(|(s, _)| s.id()) != Some(wl_surface.id()) {
                            if let Some((_, old)) = contrast.current.take() {
                                old.release();
                            }
                            let object = contrast.manager.create(&wl_surface, &qh, ());
                            contrast.current = Some((wl_surface.clone(), object));
                        }
                        if let Some((_, object)) = contrast.current.as_ref() {
                            object.set_region(Some(&wl_region));
                            object.set_contrast(frost.contrast);
                            object.set_intensity(frost.intensity);
                            object.set_saturation(frost.saturation);
                            // `set_frost` is v2; a v1 object would be a
                            // protocol error.
                            if object.version() >= 2 {
                                let [red, green, blue, alpha] = frost.colour.map(i32::from);
                                object.set_frost(red, green, blue, alpha);
                            }
                            object.commit();
                        }
                    }
                    None => contrast.withdraw(&wl_surface),
                }
            }
            wl_region.destroy();
            session.conn.flush().is_ok()
        }

        fn unset(&mut self, surface: &gdk::Surface) {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            let wl_surface = surface
                .downcast_ref::<gdk4_wayland::WaylandSurface>()
                .and_then(|surface| surface.wl_surface());
            if let Some(bound) = session.blur.as_mut() {
                if let Some((_, blur)) = bound.current.take() {
                    blur.release();
                }
                if let Some(wl_surface) = wl_surface.as_ref() {
                    bound.manager.unset(wl_surface);
                }
            }
            // The frost goes wherever the blur does.
            if let Some(contrast) = session.contrast.as_mut() {
                match wl_surface.as_ref() {
                    Some(wl_surface) => contrast.withdraw(wl_surface),
                    None => {
                        if let Some((_, object)) = contrast.current.take() {
                            object.release();
                        }
                    }
                }
            }
            let _ = session.conn.flush();
        }
    }
}

mod x11 {
    use crate::ui::glass_backdrop::{Backend, BlurBackend, Capability, Frost, Rect, x11_cardinals};
    use gtk::gdk;
    use gtk::prelude::*;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    const ATOM: &[u8] = b"_KDE_NET_WM_BLUR_BEHIND_REGION";

    struct Session {
        conn: RustConnection,
        root: u32,
        atom: u32,
    }

    pub(crate) struct Blur {
        session: Option<Session>,
        announced: bool,
    }

    impl Blur {
        /// A second, pure-Rust X connection (no Xlib, no unsafe), on the
        /// same `DISPLAY` and Xauthority GTK uses.
        pub(crate) fn connect() -> Blur {
            let session = (|| {
                let (conn, screen) = x11rb::connect(None).ok()?;
                let root = conn.setup().roots.get(screen)?.root;
                let atom = conn.intern_atom(false, ATOM).ok()?.reply().ok()?.atom;
                Some(Session { conn, root, atom })
            })();
            if session.is_none() {
                eprintln!("trace-commons-shell: glass x11_connect=failed");
            }
            let mut blur = Blur {
                session,
                announced: false,
            };
            blur.poll();
            blur
        }

        fn xid(surface: &gdk::Surface) -> Option<u32> {
            surface
                .downcast_ref::<gdk4_x11::X11Surface>()
                .and_then(|surface| u32::try_from(surface.xid()).ok())
        }
    }

    impl BlurBackend for Blur {
        fn capability(&self) -> Capability {
            Capability {
                backend: Backend::X11,
                granted: self.announced,
                composited: false,
                // KWin reads background contrast from Wayland surfaces
                // only (`BlurEffect::updateBlurRegion`).
                contrast: false,
            }
        }

        /// KWin announces support by putting the atom on the root window
        /// (`EffectsHandler::announceSupportProperty`) and takes it off when
        /// the effect unloads.
        fn poll(&mut self) {
            self.announced = self.session.as_ref().is_some_and(|session| {
                session
                    .conn
                    .list_properties(session.root)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .is_some_and(|reply| reply.atoms.contains(&session.atom))
            });
        }

        fn request(
            &mut self,
            surface: &gdk::Surface,
            region: &[Rect],
            scale: f64,
            _frost: Option<Frost>,
        ) -> bool {
            let (Some(session), Some(xid)) = (self.session.as_ref(), Self::xid(surface)) else {
                return false;
            };
            session
                .conn
                .change_property32(
                    PropMode::REPLACE,
                    xid,
                    session.atom,
                    AtomEnum::CARDINAL,
                    &x11_cardinals(region, scale),
                )
                .is_ok()
                && session.conn.flush().is_ok()
        }

        fn unset(&mut self, surface: &gdk::Surface) {
            let (Some(session), Some(xid)) = (self.session.as_ref(), Self::xid(surface)) else {
                return;
            };
            let _ = session.conn.delete_property(xid, session.atom);
            let _ = session.conn.flush();
        }
    }
}
