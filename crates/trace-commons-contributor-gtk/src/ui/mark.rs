//! The NEAR AI mark, drawn in the window.
//!
//! The Linux shell shows the NEAR AI "N" wherever it shows its mark (owner,
//! 2026-11-18), as the Mac does: the app icon's tile on the larger surfaces
//! and the N alone, in the logo kit's single ink, where the Mac's menu-bar
//! item draws it. "The Turn" is no longer drawn here; the Windows tiles still
//! come from it in `trace-commons-mark`.
//!
//! ## Where the numbers live
//!
//! The N's path is the brand kit's, exported from `GlassBrandMark.swift` to
//! `brand_tokens::logo::BRAND_MARK` and read by `monitor::glyph::Artwork`;
//! the tile's inks and proportion are `trace_commons_mark::near`, which the
//! installed app icon is generated from. A test below pins the two copies of
//! the path to each other.
//!
//! ## Colour
//!
//! Neither ink is recoloured (the kit's rule). The tile is `brand-tile`
//! #0B8FF6, #024175 under the dark style as in the Mac icon's dark
//! appearance, with the N in `brand-paper`. The single-ink N is the Mac's
//! `menuItemMark`: black on light, white on dark.

use std::cell::RefCell;

use adw::prelude::*;
use gtk::cairo;
use trace_commons_mark::near;

use super::monitor::glyph::Artwork;

pub use trace_commons_mark::Scheme;

/// The scheme the desktop is currently asking for.
pub fn current_scheme() -> Scheme {
    if adw::StyleManager::default().is_dark() {
        Scheme::Dark
    } else {
        Scheme::Light
    }
}

/// The single ink the N is drawn in on `scheme`: the Mac's `menuItemMark`,
/// the logo kit's black on light and white on dark.
pub fn ink(scheme: Scheme) -> &'static str {
    scheme.ink()
}

/// The tile's ground on `scheme`.
pub fn tile_ground(scheme: Scheme) -> &'static str {
    match scheme {
        Scheme::Light => near::TILE,
        Scheme::Dark => near::TILE_DARK,
    }
}

/// The app icon's tile as a widget, following the desktop's style.
///
/// `size` is the edge in logical pixels: 20 in a header bar, 84 on the
/// welcome screen.
pub fn framed(size: i32) -> gtk::DrawingArea {
    let area = base(size);
    area.set_draw_func(move |_, cr, width, height| {
        draw_tile(cr, current_scheme(), f64::from(width), f64::from(height));
    });
    follow_scheme(&area);
    area
}

/// The N alone in the scheme's single ink, as a widget.
pub fn template(size: i32) -> gtk::DrawingArea {
    let area = base(size);
    area.set_draw_func(move |_, cr, width, height| {
        let side = f64::from(width.min(height));
        let origin = (
            (f64::from(width) - side) / 2.0,
            (f64::from(height) - side) / 2.0,
        );
        set_source(cr, ink(current_scheme()));
        fill_n(cr, origin, side);
    });
    follow_scheme(&area);
    area
}

fn base(size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(size)
        .content_height(size)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    // Decoration. Whatever surface this sits on already names the product
    // in text, and a screen reader announcing a logo on every screen is
    // noise.
    area.set_can_focus(false);
    area.update_property(&[gtk::accessible::Property::Label(crate::copy::APP_NAME)]);
    area
}

/// Redraw when the desktop flips between light and dark.
///
/// The style manager is a process-wide singleton, so a mark that simply
/// connected to it would be kept alive by that connection for the life of
/// the application. The handler holds a weak reference and is disconnected
/// when the widget goes away.
fn follow_scheme(area: &gtk::DrawingArea) {
    let weak = area.downgrade();
    let handler = RefCell::new(Some(adw::StyleManager::default().connect_dark_notify(
        move |_| {
            if let Some(area) = weak.upgrade() {
                area.queue_draw();
            }
        },
    )));
    area.connect_destroy(move |_| {
        if let Some(id) = handler.borrow_mut().take() {
            adw::StyleManager::default().disconnect(id);
        }
    });
}

/// Fills the N in the source already set on `cr`, `side` across with its
/// top-left at `origin`, aspect kept and centred, as `GlassBrandMark.path`.
pub fn fill_n(cr: &cairo::Context, origin: (f64, f64), side: f64) {
    Artwork::brand_mark().fill(cr, origin, side);
}

/// The app icon's tile, centred in `width` x `height`: the rounded ground,
/// then the N in paper at the kit's proportion.
pub fn draw_tile(cr: &cairo::Context, scheme: Scheme, width: f64, height: f64) {
    let side = width.min(height);
    if side <= 0.0 {
        return;
    }
    let unit = side / near::SIDE;
    let _ = cr.save();
    cr.translate((width - side) / 2.0, (height - side) / 2.0);
    cr.scale(unit, unit);
    rounded_square(cr, near::SIDE, near::SIDE * near::CORNER_SHARE);
    set_source(cr, tile_ground(scheme));
    let _ = cr.fill();
    let (scale, tx, ty) = near::n_on_tile();
    let [bx, by, bw, bh] = near::VIEW_BOX;
    let n_side = bw.max(bh) * scale;
    // `fill_n` centres the N in a square; this square is the one the kit's
    // placement puts it in.
    let origin = (
        bx * scale + tx - (n_side - bw * scale) / 2.0,
        by * scale + ty - (n_side - bh * scale) / 2.0,
    );
    set_source(cr, near::PAPER);
    fill_n(cr, origin, n_side);
    let _ = cr.restore();
}

fn rounded_square(cr: &cairo::Context, side: f64, radius: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(side - radius, radius, radius, -FRAC_PI_2, 0.0);
    cr.arc(side - radius, side - radius, radius, 0.0, FRAC_PI_2);
    cr.arc(radius, side - radius, radius, FRAC_PI_2, PI);
    cr.arc(radius, radius, radius, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// Set a `#rrggbb` literal as the source colour.
///
/// The inks are compile-time constants, so a malformed one is a typo rather
/// than a runtime condition; it sets nothing rather than panicking a draw
/// handler.
pub fn set_source(cr: &cairo::Context, hex: &str) {
    let (r, g, b) = match rgb(hex) {
        Some(components) => components,
        None => return,
    };
    cr.set_source_rgb(r, g, b);
}

pub fn rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |i: usize| {
        u8::from_str_radix(&hex[i..i + 2], 16)
            .ok()
            .map(|v| f64::from(v) / 255.0)
    };
    Some((channel(0)?, channel(2)?, channel(4)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::brand_tokens::logo;

    /// The window draws the same N the app icon is generated from.
    #[test]
    fn the_window_and_the_app_icon_share_one_n() {
        assert_eq!(logo::BRAND_MARK, near::PATH);
        assert_eq!(logo::BRAND_MARK_VIEW_BOX, near::VIEW_BOX);
    }

    /// Light and dark grounds are the Mac icon's two fills, and the N's
    /// single ink is `menuItemMark`.
    #[test]
    fn the_inks_are_the_macs() {
        assert_eq!(tile_ground(Scheme::Light), "#0B8FF6");
        assert_eq!(tile_ground(Scheme::Dark), "#024175");
        assert_eq!(ink(Scheme::Light), "#000000");
        assert_eq!(ink(Scheme::Dark), "#FFFFFF");
    }

    fn pixels(draw: impl Fn(&cairo::Context), side: i32) -> (Vec<u8>, i32) {
        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, side, side).expect("surface");
        {
            let cr = cairo::Context::new(&surface).expect("context");
            draw(&cr);
        }
        surface.flush();
        let stride = surface.stride();
        let data = surface.data().expect("data").to_vec();
        (data, stride)
    }

    /// Cairo's native-endian premultiplied pixel at (x, y), as (r, g, b, a).
    fn at(data: &[u8], stride: i32, x: i32, y: i32) -> (u8, u8, u8, u8) {
        let i = (y * stride + x * 4) as usize;
        let word = u32::from_ne_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        (
            (word >> 16) as u8,
            (word >> 8) as u8,
            word as u8,
            (word >> 24) as u8,
        )
    }

    /// The tile at 64px: corners clear, the ground blue, the N's left stem
    /// paper. Kit coordinates (100, 200) are inside the stem and (200, 50)
    /// above the N; at 64px that is (16, 32) and (32, 8).
    #[test]
    fn the_tile_paints_the_ground_and_the_n() {
        for (scheme, ground) in [
            (Scheme::Light, (0x0B, 0x8F, 0xF6)),
            (Scheme::Dark, (0x02, 0x41, 0x75)),
        ] {
            let (data, stride) = pixels(|cr| draw_tile(cr, scheme, 64.0, 64.0), 64);
            assert_eq!(
                at(&data, stride, 0, 0).3,
                0,
                "{scheme:?}: the corner is rounded off"
            );
            let (r, g, b, a) = at(&data, stride, 32, 8);
            assert_eq!(
                (r, g, b, a),
                (ground.0, ground.1, ground.2, 255),
                "{scheme:?}"
            );
            assert_eq!(
                at(&data, stride, 16, 32),
                (0xF3, 0xF3, 0xF2, 255),
                "{scheme:?}"
            );
        }
    }

    /// The N alone fills only itself, in the ink it is given.
    #[test]
    fn the_n_fills_only_itself() {
        let (data, stride) = pixels(
            |cr| {
                set_source(cr, ink(Scheme::Light));
                fill_n(cr, (0.0, 0.0), 64.0);
            },
            64,
        );
        // Kit (100, 200) is in the stem; in a 64px square around the
        // 234.08-unit N that is ((100-82.96)/234.08*64, (200-80.95)/234.08*64).
        assert_eq!(at(&data, stride, 4, 32), (0, 0, 0, 255));
        // Kit (150, 200), between the left stem, the diagonal and the
        // flag, is empty.
        assert_eq!(at(&data, stride, 18, 32).3, 0);
    }

    #[test]
    fn hex_parses_to_unit_components() {
        assert_eq!(rgb("#000000"), Some((0.0, 0.0, 0.0)));
        assert_eq!(rgb("#FFFFFF"), Some((1.0, 1.0, 1.0)));
        assert_eq!(rgb("315FBA"), None);
        assert_eq!(rgb("#FFF"), None);
    }
}
