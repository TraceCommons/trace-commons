//! The edge rail and the menu-bar panel, outside the window.
//!
//! Ports of the Mac's `EdgeRailController` / `EdgeRailView` (the optional
//! rail on the screen's right edge) and `MenuBarGlassPanel` (the popover
//! under the menu-bar item). The Swift is the reference behaviour; what is
//! ported is in this module and `ui::edge_rail` / `ui::menu_panel`, and each
//! deliberate platform difference is recorded beside the code that makes it.
//!
//! A Linux desktop has no one way to put a window at a screen edge, so the
//! surfaces take whichever the session offers ([`capability`]):
//!
//! * **wlr-layer-shell** (KDE Plasma, Hyprland, Sway and other wlroots
//!   compositors): both are layer surfaces, anchored to the edge or near the
//!   tray, over every window, taking keyboard focus only while open
//!   ([`layer`], behind the `layer-shell` feature, Linux only).
//! * **the GNOME Shell extension** in `gnome-shell-extension/`: both are
//!   ordinary windows the extension places and keeps above ([`gnome`]).
//! * **neither** (GNOME without the extension, any X11 session): no rail,
//!   since a window that cannot be put at the edge cannot be one, and the
//!   panel is a plain small window. GTK 4 has no X11 positioning or
//!   keep-above call; the X11 window hints the Mac's panel would need are
//!   only reachable through `gdk4-x11`, which this build does not take.
//!
//! The rules -- which mode, where, what size, the order of opening and
//! closing, the switch -- build no widget and are tested without a display.

pub mod capability;
pub mod geometry;
pub mod gnome;
#[cfg(all(target_os = "linux", feature = "layer-shell"))]
pub mod layer;
pub mod opening;
pub mod prefs;
pub mod status;
