//! "The Turn" -- the adopted product mark, as pure geometry.
//!
//! Two corner brackets facing each other inside a hairline frame: the
//! user's bracket top-left in green, the agent's answer bottom-right in
//! blue, and the session implied in the space between them. No gradients,
//! no fills other than the frame, no asset file.
//!
//! The geometry is `design-import/DESIGN-SPEC.md` §1.2, transcribed unit
//! for unit on its 64-unit coordinate space:
//!
//! ```text
//! frame     rect x=1 y=1 w=62 h=62, stroke-width 2
//! green     M11 28 V11 H28          stroke-width 7
//! blue      M53 36 v17 H36          stroke-width 7
//! template  the same two paths in one ink, stroke-width 8, no frame
//! ```
//!
//! The frame is inset one unit under a two-unit stroke, so its outer edge
//! lands exactly on the 64x64 boundary. The template variant thickens to 8
//! because it loses the frame that was holding the brackets apart.
//!
//! ## Why it is drawn, not shipped as a file
//!
//! It has to be legible at 14px in a tray and at 84px on an onboarding
//! screen, on displays at 1x, 1.5x, 2x and fractional scales in between.
//! A `DrawingArea` is handed the real device scale by GTK and redraws into
//! it, so there is one description of the mark and no size ladder of PNGs
//! to keep in step with it. [`svg`] exists for the surfaces that can only
//! take a serialised icon -- a `StatusNotifierItem` pixmap, a desktop
//! entry, a notification -- and emits exactly the same geometry.
//!
//! ## Colour
//!
//! Every ink here is a palette token from [`super::style`], not a new
//! value: frame fill `tc_surface`, frame stroke `tc_line`, brackets
//! `tc_green` and `tc_blue`, template ink `tc_ink`. They are repeated as
//! literals because a cairo path needs floating-point components and GTK
//! offers no supported way to read a `@define-color` back out of a
//! provider. If a token below ever drifts from `style.rs`, `style.rs` is
//! right.

use std::cell::RefCell;

use adw::prelude::*;
use gtk::cairo;

/// The mark's coordinate space. Every number in this module is in these
/// units and is scaled to the requested pixel size at draw time.
const VIEW: f64 = 64.0;

/// Stroke width of a bracket inside the frame, in view units.
const STROKE_FRAMED: f64 = 7.0;

/// Stroke width of a bracket without the frame. Thicker, because the frame
/// is no longer there to give the two brackets something to sit against.
const STROKE_TEMPLATE: f64 = 8.0;

/// Which palette the mark is drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    Light,
    Dark,
}

impl Scheme {
    /// The scheme the desktop is currently asking for.
    pub fn current() -> Self {
        if adw::StyleManager::default().is_dark() {
            Scheme::Dark
        } else {
            Scheme::Light
        }
    }

    /// Frame fill. `tc_surface`.
    fn surface(self) -> &'static str {
        match self {
            Scheme::Light => "#FFFFFF",
            Scheme::Dark => "#21241E",
        }
    }

    /// Frame stroke. `tc_line`.
    fn line(self) -> &'static str {
        match self {
            Scheme::Light => "#D9DFDC",
            Scheme::Dark => "#3B4038",
        }
    }

    /// The user's bracket. `tc_green`.
    fn green(self) -> &'static str {
        match self {
            Scheme::Light => "#178F70",
            Scheme::Dark => "#3FBE9A",
        }
    }

    /// The agent's bracket. `tc_blue`.
    fn blue(self) -> &'static str {
        match self {
            Scheme::Light => "#315FBA",
            Scheme::Dark => "#7FA0EC",
        }
    }

    /// The single ink of the template variant. `tc_ink`.
    ///
    /// A status area recolours a template icon itself where it can, so this
    /// is what it falls back to when it cannot.
    pub fn ink(self) -> &'static str {
        match self {
            Scheme::Light => "#20241F",
            Scheme::Dark => "#E8EAE3",
        }
    }
}

/// The mark as an SVG document, for surfaces that can only take a
/// serialised icon.
///
/// `size` is the rendered edge in pixels; the geometry stays on its
/// 64-unit `viewBox`, so the document is resolution-independent whatever
/// is passed here.
pub fn svg(scheme: Scheme, size: u32) -> String {
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" "#,
            r#"viewBox="0 0 64 64" role="img" aria-label="Trace Commons">"#,
            r#"<rect x="1" y="1" width="62" height="62" fill="{surface}" stroke="{line}" stroke-width="2"/>"#,
            r#"<path d="M11 28V11h17" fill="none" stroke="{green}" stroke-width="7"/>"#,
            r#"<path d="M53 36v17H36" fill="none" stroke="{blue}" stroke-width="7"/>"#,
            r#"</svg>"#,
        ),
        size = size,
        surface = scheme.surface(),
        line = scheme.line(),
        green = scheme.green(),
        blue = scheme.blue(),
    )
}

/// The frameless, single-ink template variant as an SVG document.
///
/// `ink` is a CSS colour written straight into the document -- pass
/// [`Scheme::ink`] unless a status area has told you what it wants.
pub fn template_svg(ink: &str, size: u32) -> String {
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" "#,
            r#"viewBox="0 0 64 64" role="img" aria-label="Trace Commons">"#,
            r#"<path d="M11 28V11h17" fill="none" stroke="{ink}" stroke-width="8"/>"#,
            r#"<path d="M53 36v17H36" fill="none" stroke="{ink}" stroke-width="8"/>"#,
            r#"</svg>"#,
        ),
        size = size,
        ink = ink,
    )
}

/// The framed mark as a widget, in the palette the desktop is currently
/// using and following it when it changes.
///
/// `size` is the edge in logical pixels: 20 in the GNOME header bar, 22
/// inline, 40 and 84 on the larger surfaces.
pub fn framed(size: i32) -> gtk::DrawingArea {
    let area = base(size);
    area.set_draw_func(move |_, cr, width, height| {
        draw_framed(cr, Scheme::current(), width as f64, height as f64);
    });
    follow_scheme(&area);
    area
}

/// The frameless, single-ink variant as a widget, for a status area or a
/// place that wants the brackets without a card under them.
///
/// It picks up `tc_ink` for the current scheme; a tray that recolours its
/// own icons should take [`template_svg`] instead and hand it its colour.
pub fn template(size: i32) -> gtk::DrawingArea {
    let area = base(size);
    area.set_draw_func(move |_, cr, width, height| {
        let scheme = Scheme::current();
        draw_template(cr, scheme.ink(), width as f64, height as f64);
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
    area.update_property(&[gtk::accessible::Property::Label("Trace Commons")]);
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

/// The framed variant: `tc_surface` card, `tc_line` hairline, two brackets.
fn draw_framed(cr: &cairo::Context, scheme: Scheme, width: f64, height: f64) {
    let unit = width.min(height) / VIEW;
    if unit <= 0.0 {
        return;
    }
    let _ = cr.save();
    cr.scale(unit, unit);
    cr.set_line_cap(cairo::LineCap::Butt);
    cr.set_line_join(cairo::LineJoin::Miter);

    // The frame: inset one unit, two-unit stroke, so its outer edge is the
    // 64x64 boundary exactly.
    cr.rectangle(1.0, 1.0, 62.0, 62.0);
    set_source(cr, scheme.surface());
    let _ = cr.fill_preserve();
    set_source(cr, scheme.line());
    cr.set_line_width(2.0);
    let _ = cr.stroke();

    cr.set_line_width(STROKE_FRAMED);
    set_source(cr, scheme.green());
    bracket_top_left(cr);
    let _ = cr.stroke();

    set_source(cr, scheme.blue());
    bracket_bottom_right(cr);
    let _ = cr.stroke();

    let _ = cr.restore();
}

/// The template variant: the same two brackets, one ink, no frame.
fn draw_template(cr: &cairo::Context, ink: &str, width: f64, height: f64) {
    let unit = width.min(height) / VIEW;
    if unit <= 0.0 {
        return;
    }
    let _ = cr.save();
    cr.scale(unit, unit);
    cr.set_line_cap(cairo::LineCap::Butt);
    cr.set_line_join(cairo::LineJoin::Miter);
    cr.set_line_width(STROKE_TEMPLATE);
    set_source(cr, ink);
    bracket_top_left(cr);
    let _ = cr.stroke();
    bracket_bottom_right(cr);
    let _ = cr.stroke();
    let _ = cr.restore();
}

/// `M11 28V11h17` -- up the left edge, then right along the top.
fn bracket_top_left(cr: &cairo::Context) {
    cr.move_to(11.0, 28.0);
    cr.line_to(11.0, 11.0);
    cr.line_to(28.0, 11.0);
}

/// `M53 36v17H36` -- down the right edge, then left along the bottom.
fn bracket_bottom_right(cr: &cairo::Context) {
    cr.move_to(53.0, 36.0);
    cr.line_to(53.0, 53.0);
    cr.line_to(36.0, 53.0);
}

/// Set a `#rrggbb` literal as the source colour.
///
/// The inks are compile-time constants in this module, so a malformed one
/// is a typo rather than a runtime condition; it falls back to fully
/// transparent rather than panicking a draw handler.
fn set_source(cr: &cairo::Context, hex: &str) {
    let (r, g, b) = match rgb(hex) {
        Some(components) => components,
        None => return,
    };
    cr.set_source_rgb(r, g, b);
}

fn rgb(hex: &str) -> Option<(f64, f64, f64)> {
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

    #[test]
    fn geometry_is_the_spec_verbatim() {
        let light = svg(Scheme::Light, 64);
        assert!(light.contains(r##"viewBox="0 0 64 64""##));
        assert!(light.contains(
            r##"<rect x="1" y="1" width="62" height="62" fill="#FFFFFF" stroke="#D9DFDC" stroke-width="2"/>"##
        ));
        assert!(light.contains(
            r##"<path d="M11 28V11h17" fill="none" stroke="#178F70" stroke-width="7"/>"##
        ));
        assert!(light.contains(
            r##"<path d="M53 36v17H36" fill="none" stroke="#315FBA" stroke-width="7"/>"##
        ));

        let dark = svg(Scheme::Dark, 64);
        assert!(dark.contains(r##"fill="#21241E" stroke="#3B4038""##));
        assert!(dark.contains(r##"stroke="#3FBE9A""##));
        assert!(dark.contains(r##"stroke="#7FA0EC""##));
    }

    #[test]
    fn template_is_frameless_and_thicker() {
        let ink = template_svg(Scheme::Light.ink(), 20);
        assert!(!ink.contains("<rect"));
        assert_eq!(ink.matches(r##"stroke-width="8""##).count(), 2);
        assert!(ink.contains(r##"stroke="#20241F""##));
    }

    #[test]
    fn requested_size_reaches_the_document() {
        for size in [14, 16, 20, 22, 40, 84] {
            let doc = svg(Scheme::Light, size);
            assert!(doc.contains(&format!(r##"width="{size}" height="{size}""##)));
            // The coordinate space never moves with the rendered size.
            assert!(doc.contains(r##"viewBox="0 0 64 64""##));
        }
    }

    #[test]
    fn hex_parses_to_unit_components() {
        assert_eq!(rgb("#000000"), Some((0.0, 0.0, 0.0)));
        assert_eq!(rgb("#FFFFFF"), Some((1.0, 1.0, 1.0)));
        assert_eq!(rgb("315FBA"), None);
        assert_eq!(rgb("#FFF"), None);
    }
}
