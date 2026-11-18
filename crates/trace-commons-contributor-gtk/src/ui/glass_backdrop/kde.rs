//! KWin's blur, behind [`BlurBackend`]: the protocol plumbing and nothing
//! else. Every decision is in the parent module, tested on any host.
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
    use crate::ui::glass_backdrop::{Backend, BlurBackend, Capability, Rect};
    use gdk4_wayland::prelude::*;
    use gtk::gdk;
    use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_compositor, wl_region, wl_registry, wl_surface};
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
    use wayland_protocols_plasma::blur::client::{org_kde_kwin_blur, org_kde_kwin_blur_manager};

    const MANAGER: &str = "org_kde_kwin_blur_manager";

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

    struct Bound {
        name: u32,
        manager: org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
        compositor: wl_compositor::WlCompositor,
        current: Option<(wl_surface::WlSurface, org_kde_kwin_blur::OrgKdeKwinBlur)>,
    }

    struct Session {
        conn: Connection,
        queue: EventQueue<State>,
        globals: GlobalList,
        bound: Option<Bound>,
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
                        bound: None,
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

        /// The manager's current name in the registry, if advertised.
        fn advertised(session: &Session) -> Option<u32> {
            session.globals.contents().with_list(|list| {
                list.iter()
                    .find(|global| global.interface == MANAGER)
                    .map(|global| global.name)
            })
        }

        /// Bind the manager when it appears, and forget it when KWin removes
        /// it (the blur effect unloaded).
        fn rebind(&mut self) {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            let advertised = Self::advertised(session);
            if session.bound.as_ref().map(|bound| bound.name) == advertised {
                return;
            }
            if let Some(bound) = session.bound.take()
                && let Some((_, blur)) = bound.current
            {
                blur.release();
            }
            let Some(name) = advertised else {
                return;
            };
            let qh = session.queue.handle();
            let manager = session
                .globals
                .bind::<org_kde_kwin_blur_manager::OrgKdeKwinBlurManager, _, _>(&qh, 1..=1, ());
            let compositor =
                session
                    .globals
                    .bind::<wl_compositor::WlCompositor, _, _>(&qh, 1..=4, ());
            if let (Ok(manager), Ok(compositor)) = (manager, compositor) {
                session.bound = Some(Bound {
                    name,
                    manager,
                    compositor,
                    current: None,
                });
            }
        }
    }

    impl BlurBackend for Blur {
        fn capability(&self) -> Capability {
            Capability {
                backend: Backend::Wayland,
                granted: self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.bound.is_some()),
                // Filled in by the caller from the display.
                composited: false,
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

        fn request(&mut self, surface: &gdk::Surface, region: &[Rect], _scale: f64) -> bool {
            let Some(session) = self.session.as_mut() else {
                return false;
            };
            let qh = session.queue.handle();
            let Some(bound) = session.bound.as_mut() else {
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
                bound.current = Some((wl_surface, blur));
            }
            let Some((_, blur)) = bound.current.as_ref() else {
                return false;
            };
            // Surface-local, in logical pixels: the compositor scales it.
            let wl_region = bound.compositor.create_region(&qh, ());
            for rect in region {
                wl_region.add(rect.x, rect.y, rect.width, rect.height);
            }
            blur.set_region(Some(&wl_region));
            blur.commit();
            wl_region.destroy();
            session.conn.flush().is_ok()
        }

        fn unset(&mut self, surface: &gdk::Surface) {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            let Some(bound) = session.bound.as_mut() else {
                return;
            };
            if let Some((_, blur)) = bound.current.take() {
                blur.release();
            }
            if let Some(wl_surface) = surface
                .downcast_ref::<gdk4_wayland::WaylandSurface>()
                .and_then(|surface| surface.wl_surface())
            {
                bound.manager.unset(&wl_surface);
            }
            let _ = session.conn.flush();
        }
    }
}

mod x11 {
    use crate::ui::glass_backdrop::{Backend, BlurBackend, Capability, Rect, x11_cardinals};
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

        fn request(&mut self, surface: &gdk::Surface, region: &[Rect], scale: f64) -> bool {
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
