//! The two surfaces as wlr-layer-shell surfaces: the only code that calls
//! the layer-shell library, kept to the calls themselves so the rules
//! (`capability`, `geometry`) stay testable on any platform.
//!
//! Checked on Hyprland 0.56 (Omarchy 4.5, gtk4-layer-shell 1.3.0) on
//! 2026-11-18: the closed rail is a 4 x 240 surface at the right edge's
//! middle, the open one 484 x 680 at (1436, 200) on a 1920 x 1080 output,
//! and the panel's top margin starts below Waybar's exclusive zone. The
//! binary loads `libgtk4-layer-shell.so.0` ahead of `libgtk-4.so.1` and so
//! ahead of `libwayland-client`, which the library requires.

use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use super::geometry::PanelAnchor;

/// Whether the compositor advertises `zwlr_layer_shell_v1`. Only asked on
/// a Wayland display.
pub fn supported() -> bool {
    gtk4_layer_shell::is_supported()
}

/// The rail: on the overlay layer, over full-screen windows as the Mac's
/// `.statusBar` panel is, anchored to the right edge alone so the
/// compositor centres it vertically, reserving no space, and taking no
/// keyboard until clicked open (`set_rail_keyboard`). Pinned to `monitor`,
/// the first, as the Mac's is to its first screen; with none, the
/// compositor picks.
pub fn make_rail(window: &gtk::Window, monitor: Option<&gtk::gdk::Monitor>) {
    window.init_layer_shell();
    window.set_monitor(monitor);
    window.set_namespace(Some("edge-rail"));
    window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Right, true);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
}

/// Whether the rail takes the keyboard, as `edge::keyboard` decides: only
/// after a click inside the open rail (owner, 2026-11-18, "Keyboard only
/// after a click"), so Escape and Tab reach it; `None` again as it closes.
/// Asked for on demand (`edge::keyboard::ask`). Checked on 2026-11-18:
/// KWin 6.6.6 gives it on the press; Hyprland 0.56.2 only on the next
/// pointer motion over the rail, not on the press. Exclusive would give it
/// on the press there, but Hyprland then sends the rail every pointer
/// event too, so the rail could not be clicked away from.
pub fn set_rail_keyboard(window: &gtk::Window, takes: bool) {
    window.set_keyboard_mode(match crate::edge::keyboard::ask(takes) {
        crate::edge::keyboard::Ask::OnDemand => KeyboardMode::OnDemand,
        crate::edge::keyboard::Ask::Nothing => KeyboardMode::None,
    });
}

/// Maps the closed rail afresh, so a compositor that kept the keyboard on
/// it after it asked for none lets it go (KWin 6.6.6, 2026-11-18; Hyprland
/// took it back at once). The layer-shell settings carry over.
pub fn map_afresh(window: &gtk::Window) {
    use gtk::prelude::WidgetExt;
    window.set_visible(false);
    window.set_visible(true);
}

/// The panel: on the top layer, over windows and under full-screen ones
/// as a popover from a menu bar is, taking the keyboard as it opens so
/// Escape and the arrow keys reach it.
pub fn make_panel(window: &gtk::Window) {
    window.init_layer_shell();
    window.set_namespace(Some("menu-panel"));
    window.set_layer(Layer::Top);
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
}

/// Anchors the panel as `geometry::panel_anchor` decided, on `monitor`.
pub fn place_panel(window: &gtk::Window, anchor: PanelAnchor, monitor: &gtk::gdk::Monitor) {
    window.set_monitor(Some(monitor));
    window.set_anchor(Edge::Top, anchor.top);
    window.set_anchor(Edge::Bottom, !anchor.top);
    window.set_anchor(Edge::Left, anchor.left);
    window.set_anchor(Edge::Right, !anchor.left);
    let vertical = if anchor.top { Edge::Top } else { Edge::Bottom };
    let horizontal = if anchor.left { Edge::Left } else { Edge::Right };
    window.set_margin(vertical, anchor.vertical);
    window.set_margin(horizontal, anchor.horizontal);
}
