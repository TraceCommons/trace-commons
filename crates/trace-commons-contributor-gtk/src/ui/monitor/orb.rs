//! The library on the Mac's standard Traces map: a glass orb with the
//! NEAR AI mark at its centre, its meridians turning about a leaning axis.
//!
//! A port of `GlassLibraryOrb` in
//! `macos/Sources/TCDesign/Components/LibraryOrb.swift` (owner,
//! 2026-10-19). The Mac draws it only on its standard map, which a
//! Settings switch picks over the rocket map; this shell has no such
//! switch yet and draws the station, so nothing here is on screen until it
//! does.

use std::f64::consts::{PI, TAU};

use super::glyph::Artwork;
use super::station::BRAND_BLUE;

/// One turn of the meridians, in seconds.
pub const TURN_SECONDS: f64 = 18.0;
/// The angle the orb rests at, and starts from.
pub const REST_ANGLE: f64 = 0.4;
/// The share of the frame's half-size left round the sphere, for the glow.
pub const INSET: f64 = 0.29;
/// The meridians drawn, and the axis's lean.
pub const MERIDIANS: usize = 6;
pub const LEAN: f64 = -14.0 * PI / 180.0;
/// The orb's frame on the standard map, in design units
/// (`MapTransferDrawing.orbFrame`).
pub const FRAME: f64 = 104.0;
/// The campaign gradient's deep stop, #0057b0.
pub const DEEP_BLUE: [f64; 3] = [0.0, 87.0 / 255.0, 176.0 / 255.0];
/// The glow's radius on the Mac (`.shadow(radius: 10)`), and as a GSK
/// shadow, scaled as the station's is (`station::GSK_GLOW_RADIUS`).
pub const GLOW_RADIUS: f64 = 10.0;
pub const GSK_GLOW_RADIUS: f64 =
    GLOW_RADIUS * super::station::GSK_GLOW_RADIUS / super::station::GLOW_RADIUS;

/// The orb's angle `seconds` into the clock; at rest with animations off.
pub fn angle(seconds: f64, reduce_motion: bool) -> f64 {
    if reduce_motion {
        REST_ANGLE
    } else {
        REST_ANGLE + (seconds % TURN_SECONDS) / TURN_SECONDS * TAU
    }
}

/// The glow's strength: full once something has been contributed, faint
/// before.
pub fn glow(lit: bool) -> f64 {
    if lit { 0.5 } else { 0.15 }
}

/// Each meridian's half-width as a share of the radius at `angle`, and
/// whether that meridian is on the near side.
pub fn meridian_widths(angle: f64) -> Vec<(f64, bool)> {
    (0..MERIDIANS)
        .map(|index| {
            let phase = angle + index as f64 * PI / MERIDIANS as f64;
            (phase.cos().abs(), phase.sin() >= 0.0)
        })
        .collect()
}

/// The sphere's radius in a frame `width` by `height`.
pub fn radius(width: f64, height: f64) -> f64 {
    width.min(height) / 2.0 * (1.0 - INSET)
}

fn ellipse(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64) {
    let _ = cr.save();
    cr.translate(x + w / 2.0, y + h / 2.0);
    cr.scale(w / 2.0, h / 2.0);
    cr.new_sub_path();
    cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
    let _ = cr.restore();
}

/// Draws the orb at `angle` into a frame `width` by `height` at the
/// context's origin; dimmer while nothing has been contributed.
pub fn draw(cr: &gtk::cairo::Context, width: f64, height: f64, angle: f64, lit: bool) {
    let r = radius(width, height);
    let (cx, cy) = (width / 2.0, height / 2.0);
    let [br, bg, bb] = BRAND_BLUE;
    let [dr, dg, db] = DEEP_BLUE;
    let _ = cr.save();
    cr.push_group();

    // The glass: deep at the rim, bright toward the light above left.
    let (lx, ly) = (cx - r * 0.35, cy - r * 0.4);
    let glass = gtk::cairo::RadialGradient::new(lx, ly, 0.0, lx, ly, r * 1.5);
    glass.add_color_stop_rgba(0.0, br, bg, bb, 0.95);
    glass.add_color_stop_rgba(0.5, dr, dg, db, 0.95);
    glass.add_color_stop_rgba(1.0, dr, dg, db, 1.0);
    cr.new_path();
    ellipse(cr, cx - r, cy - r, r * 2.0, r * 2.0);
    let _ = cr.set_source(&glass);
    let _ = cr.fill();

    // The lines, inside the sphere and turned about the leaning axis.
    let _ = cr.save();
    cr.new_path();
    ellipse(cr, cx - r, cy - r, r * 2.0, r * 2.0);
    cr.clip();
    cr.translate(cx, cy);
    cr.rotate(LEAN);
    for latitude in [-0.5_f64, 0.0, 0.5] {
        let y = r * latitude;
        let half = r * (1.0 - latitude * latitude).sqrt();
        cr.new_path();
        ellipse(cr, -half, y - half * 0.22, half * 2.0, half * 0.44);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    }
    for (width, near) in meridian_widths(angle) {
        let w = r * width;
        cr.new_path();
        ellipse(cr, -w, -r, w * 2.0, r * 2.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, if near { 0.32 } else { 0.12 });
        cr.set_line_width(if near { 1.2 } else { 0.8 });
        let _ = cr.stroke();
    }
    let _ = cr.restore();

    // The mark at the centre: its top-left at the square's, scaled by its
    // longer side (`GlassBrandMark.path(at:size:)`).
    let side = r * 0.95;
    let mark = Artwork::brand_mark();
    if let Some(segments) = mark.segments() {
        let [bx, by, bw, bh] = mark.view_box;
        let _ = cr.save();
        cr.translate(cx - side / 2.0, cy - side / 2.0);
        cr.scale(side / bw.max(bh), side / bw.max(bh));
        cr.translate(-bx, -by);
        super::glyph::trace_segments(cr, &segments);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_fill_rule(gtk::cairo::FillRule::Winding);
        let _ = cr.fill();
        let _ = cr.restore();
    }

    // The highlight and the rim.
    let shine = gtk::cairo::LinearGradient::new(cx, cy - r * 0.78, cx, cy - r * 0.4);
    shine.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.45);
    shine.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    cr.new_path();
    ellipse(cr, cx - r * 0.62, cy - r * 0.78, r * 0.7, r * 0.38);
    let _ = cr.set_source(&shine);
    let _ = cr.fill();
    cr.new_path();
    ellipse(cr, cx - r, cy - r, r * 2.0, r * 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    let _ = cr.pop_group_to_source();
    let _ = cr.paint_with_alpha(if lit { 1.0 } else { 0.6 });
    let _ = cr.restore();
}

/// The orb at `angle` with its glow, as a render node over the frame
/// `width` by `height` whose top-left is `origin`.
pub fn node(
    origin: (f64, f64),
    width: f64,
    height: f64,
    angle: f64,
    lit: bool,
) -> gtk::gsk::RenderNode {
    use gtk::gsk::prelude::IsRenderNode;
    use gtk::{graphene, gsk};
    let bounds = graphene::Rect::new(
        origin.0 as f32,
        origin.1 as f32,
        width as f32,
        height as f32,
    );
    let orb = gsk::CairoNode::new(&bounds);
    {
        let cr = orb.draw_context();
        cr.translate(origin.0, origin.1);
        draw(&cr, width, height, angle, lit);
    }
    let blue = gtk::gdk::RGBA::new(
        BRAND_BLUE[0] as f32,
        BRAND_BLUE[1] as f32,
        BRAND_BLUE[2] as f32,
        glow(lit) as f32,
    );
    gsk::ShadowNode::new(
        &orb,
        &[gsk::Shadow::new(blue, 0.0, 0.0, GSK_GLOW_RADIUS as f32)],
    )
    .upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_orbs_numbers_are_the_macs() {
        assert_eq!(TURN_SECONDS, 18.0);
        assert_eq!(REST_ANGLE, 0.4);
        assert_eq!(MERIDIANS, 6);
        assert_eq!(angle(99.0, true), REST_ANGLE);
        assert!((angle(9.0, false) - (0.4 + PI)).abs() < 1e-12);
        // LibraryStationTests: the radius leaves the inset round it, and
        // the frame's shorter side sets it.
        assert!((radius(104.0, 104.0) - 52.0 * (1.0 - INSET)).abs() < 1e-12);
        assert!(radius(104.0, 80.0) < 40.0);
        assert_eq!(glow(true), 0.5);
        assert_eq!(glow(false), 0.15);
    }

    #[test]
    fn the_meridians_turn() {
        let rest = meridian_widths(0.0);
        let turned = meridian_widths(0.5);
        assert_eq!(rest.len(), 6);
        assert_ne!(rest, turned);
        assert!((rest[0].0 - 1.0).abs() < 1e-12);
        assert!(rest[0].1, "the first meridian is near at rest");
        assert!((turned[0].0 - 0.5_f64.cos()).abs() < 1e-12);
    }

    /// Renders the orb as the Mac's comparison render is made: a 104 frame
    /// in 124, at twice the size, on the pane's ground, at rest, lit.
    #[test]
    #[ignore = "writes comparison renders; set TC_STATION_RENDER_DIR"]
    fn render_for_comparison() {
        use gtk::cairo;
        let Ok(dir) = std::env::var("TC_STATION_RENDER_DIR") else {
            return;
        };
        for (scheme, ground) in [("light", [0xf2, 0xf3, 0xf6]), ("dark", [0x0d, 0x11, 0x24])] {
            let (w, h) = (248, 248);
            let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
            {
                let cr = cairo::Context::new(&surface).unwrap();
                cr.set_source_rgb(
                    f64::from(ground[0]) / 255.0,
                    f64::from(ground[1]) / 255.0,
                    f64::from(ground[2]) / 255.0,
                );
                let _ = cr.paint();
                cr.scale(2.0, 2.0);
                node((10.0, 10.0), FRAME, FRAME, REST_ANGLE, true).draw(&cr);
            }
            surface.flush();
            let name = format!("{dir}/orb-{scheme}-lit-{w}x{h}.bgra");
            std::fs::write(name, &*surface.data().unwrap()).unwrap();
        }
    }
}
