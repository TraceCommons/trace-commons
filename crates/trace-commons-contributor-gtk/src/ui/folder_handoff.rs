//! The launch dialog's art: a folder sliding into a terminal window that
//! shows the tool's logo (`ManagedFolderHandoff` in
//! `macos/Sources/TraceCommonsApp/Views/ManagedFolderExplainer.swift`).
//!
//! Once, for 1.2 s on the token curve, the folder tile moves from outside
//! the terminal's left edge to rest inside it, left of the logo, shrinking
//! from 1 to 0.9; then it rests. Flat: a faint rim, flat fills, no glow.
//! With `gtk-enable-animations` off it is the rest frame, still.
//! Decorative, so hidden from assistive tech.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

use super::glass_motion::{self, Driver};
use super::style::brand_tokens::tile;
use super::tool_tile::{self, Colour, Kind};

/// The art's frame, in points (`ManagedFolderHandoff.width`, `.height`).
pub const WIDTH: f64 = 260.0;
pub const HEIGHT: f64 = 108.0;
/// The terminal window, centred.
pub const TERMINAL: (f64, f64) = (156.0, 100.0);
/// The ruling's duration, on the token curve.
pub const DURATION: f64 = 1.2;
/// How far below the frame's centre the folder and logo sit.
const CONTENT_Y: f64 = 9.0;
const FOLDER_START_X: f64 = -112.0;
const FOLDER_REST_X: f64 = -34.0;
const LOGO_X: f64 = 34.0;
const LOGO: f64 = 28.0;

/// Where the folder tile is at time fraction `t` (0 the start, 1 at rest):
/// its centre and its side.
pub fn folder_at(t: f64) -> (f64, f64, f64) {
    let p = glass_motion::curve(t);
    let x = FOLDER_START_X + (FOLDER_REST_X - FOLDER_START_X) * p;
    let scale = 1.0 + (0.9 - 1.0) * p;
    (
        WIDTH / 2.0 + x,
        HEIGHT / 2.0 + CONTENT_Y,
        tile::TOOL_TILE_LARGE * scale,
    )
}

/// The terminal's top-left corner.
pub fn terminal_origin() -> (f64, f64) {
    ((WIDTH - TERMINAL.0) / 2.0, (HEIGHT - TERMINAL.1) / 2.0)
}

/// The logo's square: its top-left and side.
pub fn logo_square() -> (f64, f64, f64) {
    (
        WIDTH / 2.0 + LOGO_X - LOGO / 2.0,
        HEIGHT / 2.0 + CONTENT_Y - LOGO / 2.0,
        LOGO,
    )
}

/// Draws the frame at time fraction `t`.
pub fn draw(cr: &gtk::cairo::Context, tool: &Kind, t: f64, dark: bool) {
    let (tx, ty) = terminal_origin();
    let (tw, th) = TERMINAL;
    let radius = tile::RADIUS_CONTROL;
    let hairline = Colour::token(if dark {
        tile::dark::HAIRLINE
    } else {
        tile::light::HAIRLINE
    });
    // The window: ink at 0.06, a title bar of three dots over a hairline.
    tool_tile::ink(0.06, dark).set(cr);
    tool_tile::rounded(cr, tx, ty, tw, th, radius);
    let _ = cr.fill();
    tool_tile::ink(0.18, dark).set(cr);
    for i in 0..3 {
        cr.new_sub_path();
        cr.arc(
            tx + 9.0 + 3.0 + 10.0 * f64::from(i),
            ty + 9.0,
            3.0,
            0.0,
            2.0 * std::f64::consts::PI,
        );
    }
    let _ = cr.fill();
    hairline.set(cr);
    cr.rectangle(tx, ty + 18.0, tw, 1.0);
    let _ = cr.fill();
    if let Kind::Tool {
        logo: Some(logo),
        tint,
        ..
    } = tool
    {
        let (lx, ly, side) = logo_square();
        tool_tile::tint(*tint, dark).set(cr);
        logo.fill(cr, (lx, ly), side);
    }
    // The rim, inside the window's edge.
    hairline.set(cr);
    cr.set_line_width(1.0);
    tool_tile::rounded(cr, tx + 0.5, ty + 0.5, tw - 1.0, th - 1.0, radius - 0.5);
    let _ = cr.stroke();
    // The folder, over the window.
    let (cx, cy, side) = folder_at(t);
    let scale = side / tile::TOOL_TILE_LARGE;
    let _ = cr.save();
    cr.translate(cx, cy);
    cr.scale(scale, scale);
    cr.translate(-tile::TOOL_TILE_LARGE / 2.0, -tile::TOOL_TILE_LARGE / 2.0);
    tool_tile::draw(cr, &Kind::Folder, true, dark);
    let _ = cr.restore();
}

/// The art as a widget for `tool`; `set_tool` follows the dialog's picker.
pub struct FolderHandoff {
    pub area: gtk::DrawingArea,
    tool: Rc<RefCell<Kind>>,
}

impl FolderHandoff {
    pub fn new(tool: Kind) -> Self {
        let area = gtk::DrawingArea::new();
        area.set_content_width(WIDTH as i32);
        area.set_content_height(HEIGHT as i32);
        area.set_halign(gtk::Align::Center);
        area.set_accessible_role(gtk::AccessibleRole::Presentation);
        let tool = Rc::new(RefCell::new(tool));
        let t = Rc::new(Cell::new(0.0));
        {
            let tool = Rc::clone(&tool);
            let t = Rc::clone(&t);
            area.set_draw_func(move |_, cr, w, _| {
                cr.translate((f64::from(w) - WIDTH) / 2.0, 0.0);
                draw(cr, &tool.borrow(), t.get(), tool_tile::dark());
            });
        }
        // Once, the first time it is shown; the rest frame from then on.
        let driver = Rc::new(Driver::default());
        let played = Cell::new(false);
        area.connect_map(move |area| {
            if played.replace(true) {
                return;
            }
            let weak = area.downgrade();
            let frame = Rc::clone(&t);
            driver.start(
                area,
                DURATION,
                0.0,
                move |elapsed| {
                    frame.set(elapsed / DURATION);
                    if let Some(area) = weak.upgrade() {
                        area.queue_draw();
                    }
                },
                || {},
            );
        });
        {
            let weak = area.downgrade();
            let scheme = adw::StyleManager::default().connect_dark_notify(move |_| {
                if let Some(area) = weak.upgrade() {
                    area.queue_draw();
                }
            });
            let scheme = RefCell::new(Some(scheme));
            area.connect_destroy(move |_| {
                if let Some(id) = scheme.borrow_mut().take() {
                    adw::StyleManager::default().disconnect(id);
                }
            });
        }
        Self { area, tool }
    }

    pub fn set_tool(&self, tool: Kind) {
        *self.tool.borrow_mut() = tool;
        self.area.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::monitor::traces_tree::SourceKind;
    use crate::ui::style::brand_tokens::map;

    /// `ManagedFolderHandoff`'s layout: a 260 x 108 frame, the 156 x 100
    /// terminal centred, the folder from -112 to -34 off centre and 9
    /// below it, shrinking to 0.9, the 28-point logo 34 right of centre.
    #[test]
    fn the_folder_slides_where_the_macs_does() {
        assert_eq!(terminal_origin(), (52.0, 4.0));
        let (x, y, side) = folder_at(0.0);
        assert!((x - 18.0).abs() < 1e-9 && y == 63.0 && (side - 30.0).abs() < 1e-9);
        let (x, y, side) = folder_at(1.0);
        assert!((x - 96.0).abs() < 1e-9 && y == 63.0 && (side - 27.0).abs() < 1e-9);
        assert_eq!(logo_square(), (150.0, 49.0, 28.0));
        // On the token curve: most of the way there by half time.
        let (half, _, _) = folder_at(0.5);
        assert!((half - (18.0 + 78.0 * 0.946_079)).abs() < 1e-3, "{half}");
        assert_eq!(DURATION, 1.2);
    }

    /// The rest frame's pixels where the Mac's render (2x) has them: the
    /// folder tile's ground at its centre's left, the logo's tint at its
    /// centre, the window's tint on its body.
    #[test]
    fn the_rest_frame_matches_the_macs_pixels() {
        let s = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 520, 216).unwrap();
        {
            let cr = gtk::cairo::Context::new(&s).unwrap();
            cr.scale(2.0, 2.0);
            Colour::token(map::light::PANE_OPAQUE).set(&cr);
            let _ = cr.paint();
            draw(&cr, &Kind::source(SourceKind::ClaudeCode), 1.0, false);
        }
        let mut s = s;
        let stride = s.stride() as usize;
        let data = s.data().unwrap();
        let px = |x: f64, y: f64| {
            let at = (y * 2.0) as usize * stride + (x * 2.0) as usize * 4;
            [data[at + 2], data[at + 1], data[at]]
        };
        let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3);
        // The folder ground (#e6e6ea) inside the scaled tile's left edge.
        assert!(
            near(px(85.0, 63.0), [230, 230, 234]),
            "{:?}",
            px(85.0, 63.0)
        );
        // The window body: black at 0.06 over the pane (#f2f3f6).
        assert!(
            near(px(70.0, 90.0), [227, 228, 231]),
            "{:?}",
            px(70.0, 90.0)
        );
        // A title-bar dot: black at 0.18 over the window.
        assert!(
            near(px(64.0, 13.0), [186, 187, 189]),
            "{:?}",
            px(64.0, 13.0)
        );
    }
}
