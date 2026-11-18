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
pub const FRAME: f64 = super::map_scene::ORB_FRAME;
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
///
/// Unlit, every drawing operation is faded to 0.6 on its own, as the Mac
/// sets `opacity` on its `GraphicsContext` copy: the sphere shows through
/// the mark, and the lines (one layer there, one group here) through
/// nothing but their own fade. Not the orb faded as one group.
pub fn draw(cr: &gtk::cairo::Context, width: f64, height: f64, angle: f64, lit: bool) {
    let r = radius(width, height);
    let (cx, cy) = (width / 2.0, height / 2.0);
    let [br, bg, bb] = BRAND_BLUE;
    let [dr, dg, db] = DEEP_BLUE;
    let a = if lit { 1.0 } else { 0.6 };
    let _ = cr.save();

    // The glass: deep at the rim, bright toward the light above left.
    let (lx, ly) = (cx - r * 0.35, cy - r * 0.4);
    let glass = gtk::cairo::RadialGradient::new(lx, ly, 0.0, lx, ly, r * 1.5);
    glass.add_color_stop_rgba(0.0, br, bg, bb, 0.95 * a);
    glass.add_color_stop_rgba(0.5, dr, dg, db, 0.95 * a);
    glass.add_color_stop_rgba(1.0, dr, dg, db, a);
    cr.new_path();
    ellipse(cr, cx - r, cy - r, r * 2.0, r * 2.0);
    let _ = cr.set_source(&glass);
    let _ = cr.fill();

    // The lines, inside the sphere and turned about the leaning axis: one
    // layer, faded as a whole (`drawLayer`).
    cr.push_group();
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
    let _ = cr.pop_group_to_source();
    let _ = cr.paint_with_alpha(a);

    // The mark at the centre: its top-left at the square's, scaled by its
    // longer side (`GlassBrandMark.path(at:size:)`).
    let side = r * 0.95;
    let mark = Artwork::brand_mark();
    if let Some(segments) = mark.cached_segments() {
        let [bx, by, bw, bh] = mark.view_box;
        let _ = cr.save();
        cr.translate(cx - side / 2.0, cy - side / 2.0);
        cr.scale(side / bw.max(bh), side / bw.max(bh));
        cr.translate(-bx, -by);
        super::glyph::trace_segments(cr, &segments);
        cr.set_source_rgba(1.0, 1.0, 1.0, a);
        cr.set_fill_rule(gtk::cairo::FillRule::Winding);
        let _ = cr.fill();
        let _ = cr.restore();
    }

    // The highlight and the rim.
    let shine = gtk::cairo::LinearGradient::new(cx, cy - r * 0.78, cx, cy - r * 0.4);
    shine.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.45 * a);
    shine.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    cr.new_path();
    ellipse(cr, cx - r * 0.62, cy - r * 0.78, r * 0.7, r * 0.38);
    let _ = cr.set_source(&shine);
    let _ = cr.fill();
    cr.new_path();
    ellipse(cr, cx - r, cy - r, r * 2.0, r * 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35 * a);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

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

    /// The orb at rest, `lit` or not, as the Mac's comparison render is
    /// made: a 104 frame in 124 at twice the size, on `ground`.
    fn rendered(lit: bool, ground: [u8; 3]) -> gtk::cairo::ImageSurface {
        use gtk::cairo;
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 248, 248).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            cr.set_source_rgb(
                f64::from(ground[0]) / 255.0,
                f64::from(ground[1]) / 255.0,
                f64::from(ground[2]) / 255.0,
            );
            let _ = cr.paint();
            cr.scale(2.0, 2.0);
            node((10.0, 10.0), FRAME, FRAME, REST_ANGLE, lit).draw(&cr);
        }
        surface.flush();
        surface
    }

    fn pixel(surface: &mut gtk::cairo::ImageSurface, x: usize, y: usize) -> [u8; 3] {
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let at = y * stride + x * 4;
        [data[at + 2], data[at + 1], data[at]]
    }

    /// Unlit, the Mac fades each of the orb's drawing operations to 0.6
    /// (`orb.opacity` on a copy of the `GraphicsContext`), not the orb as
    /// one group: the white mark at its centre lets the sphere under it
    /// show through. These are the Mac's own pixels at the mark's centre,
    /// from `GlassLibraryOrb` rendered at rest (date 0) by the Swift probe;
    /// a group fade would give near-white here instead (about 250 on the
    /// light ground).
    #[test]
    fn unlit_the_mark_fades_by_operation_as_on_the_mac() {
        for (ground, mac) in [
            ([0xf2, 0xf3, 0xf6], [191, 217, 241]),
            ([0x0d, 0x11, 0x24], [155, 182, 209]),
        ] {
            let mut surface = rendered(false, ground);
            let got = pixel(&mut surface, 122, 124);
            for k in 0..3 {
                assert!(
                    (i32::from(got[k]) - mac[k]).abs() <= 8,
                    "unlit mark centre on {ground:?}: {got:?}, the Mac's {mac:?}"
                );
            }
            // Lit, the mark is white.
            let mut lit = rendered(true, ground);
            assert!(pixel(&mut lit, 122, 124).iter().all(|&c| c >= 245));
        }
    }

    /// Renders the orb as the Mac's comparison render is made: a 104 frame
    /// in 124, at twice the size, on the pane's ground, at rest, lit and
    /// unlit.
    #[test]
    #[ignore = "writes comparison renders; set TC_STATION_RENDER_DIR"]
    fn render_for_comparison() {
        let Ok(dir) = std::env::var("TC_STATION_RENDER_DIR") else {
            return;
        };
        for (scheme, ground) in [("light", [0xf2, 0xf3, 0xf6]), ("dark", [0x0d, 0x11, 0x24])] {
            for lit in [true, false] {
                let mut surface = rendered(lit, ground);
                let state = if lit { "lit" } else { "unlit" };
                let name = format!("{dir}/orb-{scheme}-{state}-248x248.bgra");
                std::fs::write(name, &*surface.data().unwrap()).unwrap();
            }
        }
    }
}
