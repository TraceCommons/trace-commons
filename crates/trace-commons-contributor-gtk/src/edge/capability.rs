//! What this session offers the rail and the panel, and so how each is shown.
//!
//! [`modes`] is the whole decision and is pure. It keys on what the session
//! can actually do -- the display backend, whether the compositor
//! advertises `zwlr_layer_shell_v1`, whether the GNOME Shell extension owns
//! its bus name -- and never on a desktop's name: KDE Plasma and Hyprland
//! both speak layer-shell, GNOME does not, and a desktop that adds or drops
//! the protocol is followed without an edit here. [`detect`] gathers the
//! facts from the running session.

/// The display backend GTK opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Wayland,
    X11,
    /// Anything else (Broadway, macOS in this crate's own tests).
    Other,
}

/// What the session offers, as [`detect`] finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub backend: Backend,
    /// This build links the layer-shell library and the compositor
    /// advertises the protocol. False whenever the backend is not Wayland:
    /// the library is never asked about any other.
    pub layer_shell: bool,
    /// The GNOME Shell extension owns its bus name on the session bus.
    pub gnome_extension: bool,
}

/// How the edge rail is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailMode {
    /// A layer surface anchored to the right edge.
    LayerShell,
    /// A plain window the GNOME Shell extension places and keeps above.
    GnomeExtension,
    /// No rail, and no switch for it in Settings: a window that cannot be
    /// put at the screen's edge is not an edge rail.
    Unavailable,
}

/// How the menu-bar panel is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelMode {
    /// A layer surface anchored near the tray icon that opened it.
    LayerShell,
    /// A plain window the GNOME Shell extension places near the pointer
    /// and keeps above.
    GnomeExtension,
    /// A plain small window, wherever the window manager puts it.
    PlainWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modes {
    pub rail: RailMode,
    pub panel: PanelMode,
}

/// The modes this session supports.
///
/// Layer-shell first: where the compositor speaks it, it is the native way
/// and needs nothing installed. The extension second, on any backend: Mutter
/// places windows for it under Wayland and under X11 alike. Otherwise the
/// panel is a plain window and there is no rail.
pub fn modes(capabilities: Capabilities) -> Modes {
    if capabilities.backend == Backend::Wayland && capabilities.layer_shell {
        return Modes {
            rail: RailMode::LayerShell,
            panel: PanelMode::LayerShell,
        };
    }
    if capabilities.gnome_extension && capabilities.backend != Backend::Other {
        return Modes {
            rail: RailMode::GnomeExtension,
            panel: PanelMode::GnomeExtension,
        };
    }
    Modes {
        rail: RailMode::Unavailable,
        panel: PanelMode::PlainWindow,
    }
}

impl Modes {
    /// Whether Settings offers the rail's switch at all.
    pub fn offers_rail(self) -> bool {
        self.rail != RailMode::Unavailable
    }
}

/// The backend, by the display's GObject type name.
pub fn backend_of(type_name: &str) -> Backend {
    match type_name {
        "GdkWaylandDisplay" => Backend::Wayland,
        "GdkX11Display" => Backend::X11,
        _ => Backend::Other,
    }
}

/// The session's facts. `gnome_extension` is the extension client's answer
/// ([`super::gnome`]), asked separately because it is a bus round trip.
pub fn detect(display: &gtk::gdk::Display, gnome_extension: bool) -> Capabilities {
    use gtk::prelude::*;
    let backend = backend_of(display.type_().name());
    Capabilities {
        backend,
        layer_shell: backend == Backend::Wayland && layer_shell_supported(),
        gnome_extension,
    }
}

#[cfg(all(target_os = "linux", feature = "layer-shell"))]
fn layer_shell_supported() -> bool {
    super::layer::supported()
}

#[cfg(not(all(target_os = "linux", feature = "layer-shell")))]
fn layer_shell_supported() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(backend: Backend, layer_shell: bool, gnome_extension: bool) -> Capabilities {
        Capabilities {
            backend,
            layer_shell,
            gnome_extension,
        }
    }

    #[test]
    fn a_layer_shell_compositor_gets_both_as_layer_surfaces() {
        // KDE Plasma 6 and Hyprland (Omarchy), with or without the extension.
        for extension in [false, true] {
            assert_eq!(
                modes(caps(Backend::Wayland, true, extension)),
                Modes {
                    rail: RailMode::LayerShell,
                    panel: PanelMode::LayerShell
                }
            );
        }
    }

    #[test]
    fn gnome_with_the_extension_gets_both_as_placed_windows() {
        // GNOME on Wayland has no layer-shell; GNOME on Xorg has neither.
        for backend in [Backend::Wayland, Backend::X11] {
            assert_eq!(
                modes(caps(backend, false, true)),
                Modes {
                    rail: RailMode::GnomeExtension,
                    panel: PanelMode::GnomeExtension
                }
            );
        }
    }

    #[test]
    fn without_either_there_is_no_rail_and_a_plain_panel() {
        for backend in [Backend::Wayland, Backend::X11, Backend::Other] {
            let modes = modes(caps(backend, false, false));
            assert_eq!(modes.rail, RailMode::Unavailable);
            assert_eq!(modes.panel, PanelMode::PlainWindow);
            assert!(!modes.offers_rail());
        }
    }

    #[test]
    fn layer_shell_is_never_taken_off_wayland() {
        // `detect` never reports it there, but the rule holds on its own.
        assert_eq!(
            modes(caps(Backend::X11, true, false)).rail,
            RailMode::Unavailable
        );
        assert_eq!(
            modes(caps(Backend::X11, true, true)).rail,
            RailMode::GnomeExtension
        );
    }

    #[test]
    fn the_switch_is_offered_wherever_a_rail_can_be_shown() {
        assert!(modes(caps(Backend::Wayland, true, false)).offers_rail());
        assert!(modes(caps(Backend::Wayland, false, true)).offers_rail());
    }

    #[test]
    fn backends_are_read_from_gdks_type_names() {
        assert_eq!(backend_of("GdkWaylandDisplay"), Backend::Wayland);
        assert_eq!(backend_of("GdkX11Display"), Backend::X11);
        assert_eq!(backend_of("GdkMacosDisplay"), Backend::Other);
        assert_eq!(backend_of("GdkBroadwayDisplay"), Backend::Other);
    }
}
