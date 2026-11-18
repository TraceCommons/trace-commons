//! The map's field under both maps: a port of `GlassMapField` in
//! `macos/Sources/TCDesign/Components/Indicators.swift` (#1146
//! `--tc-map-fill`).
//!
//! An elliptical gradient from `mapFieldInner` at its centre to
//! `mapFieldOuter`, the ellipse 80% of the field's width and 60% of its
//! height across each radius, its centre a little below the middle. Over
//! it, the NEAR AI mark tiled small and faint with a dot between every
//! other pair, fading out toward the edges (`GlassMapMarks`). The flat
//! theme's colours are veils over the pane, so the light field is near
//! white with the marks barely there, and the dark one navy with the marks
//! clearly seen, as on the Mac.
//!
//! Deliberate platform differences, each until a later slice: the Mac's
//! shimmer that crosses the field every 40 seconds and the light under the
//! pointer are not drawn here.

use std::cell::RefCell;

use super::glyph::Artwork;
use super::map::MapColours;

/// The tile's cell: two marks to a cell, on its diagonal, so rows stagger.
pub const PITCH: f64 = 28.0;
/// A mark's width and height.
pub const MARK: f64 = 5.6;
/// The dot set between every other pair of marks along a row.
pub const DOT: f64 = 1.6;

/// The cells of a field `width` by `height`: every column and row index
/// whose cell touches it.
fn cells(width: f64, height: f64) -> Vec<(i64, i64)> {
    if width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let columns = (width / PITCH).floor() as i64;
    let rows = (height / PITCH).floor() as i64;
    (0..=rows)
        .flat_map(|row| (0..=columns).map(move |column| (column, row)))
        .collect()
}

/// Every mark's top-left corner in a field `width` by `height`
/// (`GlassMapMarks.origins`).
pub fn origins(width: f64, height: f64) -> Vec<(f64, f64)> {
    let offset = |quarter: f64| PITCH * quarter - MARK / 2.0;
    cells(width, height)
        .into_iter()
        .flat_map(|(column, row)| {
            let cell = (column as f64 * PITCH, row as f64 * PITCH);
            [0.25, 0.75].map(|q| (cell.0 + offset(q), cell.1 + offset(q)))
        })
        .collect()
}

/// The dots' centres in a field `width` by `height`: one between every
/// other pair of marks along each row, the rows taking turns
/// (`GlassMapMarks.dots`).
pub fn dots(width: f64, height: f64) -> Vec<(f64, f64)> {
    cells(width, height)
        .into_iter()
        .map(|(column, row)| {
            let (x, y) = (column as f64 * PITCH, row as f64 * PITCH);
            if (row + column) % 2 == 0 {
                (x + PITCH * 0.75, y + PITCH * 0.25)
            } else {
                (x + PITCH * 0.25, y + PITCH * 0.75)
            }
        })
        .collect()
}

/// The marks' ink at rest in the flat theme (`GlassMapMarks.ink`): white
/// on both fields, faint over the dark one and stronger over the light,
/// where white on white leaves a light grid.
pub fn ink(dark: bool) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(1.0, 1.0, 1.0, if dark { 0.096 } else { 0.4 })
}

fn rgba(cr: &gtk::cairo::Context, colour: &gtk::gdk::RGBA) {
    crate::ui::monitor::map::source(cr, colour, 1.0);
}

/// A radial gradient stretched to the ellipse with radii `rx`, `ry`
/// round `(cx, cy)`, from `stops` (offset, colour, alpha).
fn elliptical(
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    stops: &[(f64, gtk::gdk::RGBA, f64)],
) -> gtk::cairo::RadialGradient {
    let gradient = gtk::cairo::RadialGradient::new(0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
    for (offset, colour, alpha) in stops {
        gradient.add_color_stop_rgba(
            *offset,
            f64::from(colour.red()),
            f64::from(colour.green()),
            f64::from(colour.blue()),
            f64::from(colour.alpha()) * alpha,
        );
    }
    let mut matrix = gtk::cairo::Matrix::identity();
    matrix.scale(1.0 / rx.max(1e-6), 1.0 / ry.max(1e-6));
    matrix.translate(-cx, -cy);
    gradient.set_matrix(matrix);
    gradient
}

/// Draws the field into `width` by `height` at the context's origin.
pub fn draw(cr: &gtk::cairo::Context, width: f64, height: f64, colours: &MapColours, dark: bool) {
    let _ = cr.save();
    // The veil: mapFieldOuter under it all, then the ellipse.
    rgba(cr, &colours.field_outer);
    let _ = cr.paint();
    let veil = elliptical(
        width * 0.5,
        height * 0.55,
        width * 0.8,
        height * 0.6,
        &[
            (0.0, colours.field_inner, 1.0),
            (1.0, colours.field_outer, 1.0),
        ],
    );
    // Over the outer colour, as the Mac lays its gradient on its
    // background: past the ellipse it is the outer colour again.
    let _ = cr.set_source(&veil);
    let _ = cr.paint();
    let _ = cr.restore();
    let _ = cr.save();

    // The marks and dots, faded out toward the edges.
    cr.push_group();
    cr.new_path();
    let mark = Artwork::brand_mark();
    if let Some(segments) = mark.cached_segments() {
        let [bx, by, bw, bh] = mark.view_box;
        let scale = MARK / bw.max(bh);
        for (x, y) in origins(width, height) {
            let _ = cr.save();
            cr.translate(x, y);
            cr.scale(scale, scale);
            cr.translate(-bx, -by);
            super::glyph::append_segments(cr, &segments);
            let _ = cr.restore();
        }
    }
    for (x, y) in dots(width, height) {
        cr.new_sub_path();
        cr.arc(x, y, DOT / 2.0, 0.0, std::f64::consts::TAU);
    }
    cr.set_fill_rule(gtk::cairo::FillRule::Winding);
    rgba(cr, &ink(dark));
    let _ = cr.fill();
    let _ = cr.pop_group_to_source();
    let white = gtk::gdk::RGBA::WHITE;
    let fade = elliptical(
        width * 0.5,
        height * 0.52,
        width * 0.62,
        height * 0.62,
        &[(0.0, white, 1.0), (0.55, white, 0.85), (1.0, white, 0.0)],
    );
    let _ = cr.mask(&fade);
    let _ = cr.restore();
}

/// The field's drawing, kept until its size or scheme changes.
/// What the field was last drawn for: its size, scheme and veil.
type FieldKey = (i32, i32, bool, gtk::gdk::RGBA, gtk::gdk::RGBA);

#[derive(Default)]
pub struct Field {
    kept: RefCell<Option<(FieldKey, gtk::gsk::RenderNode)>>,
}

impl Field {
    /// The field for `colours`, the map's resolved colours (the skin's in
    /// Omarchy), kept until its size, scheme or veil changes.
    pub fn node(
        &self,
        width: i32,
        height: i32,
        dark: bool,
        colours: &MapColours,
    ) -> gtk::gsk::RenderNode {
        let key = (
            width,
            height,
            dark,
            colours.field_inner,
            colours.field_outer,
        );
        if let Some((kept, node)) = self.kept.borrow().as_ref()
            && *kept == key
        {
            return node.clone();
        }
        let bounds = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
        let node = gtk::gsk::CairoNode::new(&bounds);
        {
            let cr = node.draw_context();
            draw(&cr, f64::from(width), f64::from(height), colours, dark);
        }
        let node: gtk::gsk::RenderNode = gtk::gsk::prelude::IsRenderNode::upcast(node);
        self.kept.replace(Some((key, node.clone())));
        node
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GlassMapMarks`' measures.
    #[test]
    fn the_tiles_measures_are_the_macs() {
        assert_eq!((PITCH, MARK, DOT), (28.0, 5.6, 1.6));
        assert_eq!(ink(true), gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 0.096));
        assert_eq!(ink(false), gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 0.4));
    }

    /// `test_theTileCoversTheField`: two marks to a cell, on its diagonal,
    /// every cell of the field drawn.
    #[test]
    fn the_tile_covers_the_field() {
        let origins = origins(28.0 * 2.0 - 1.0, 28.0 * 2.0 - 1.0);
        assert_eq!(origins.len(), 2 * 2 * 2);
        let half = 5.6 / 2.0;
        assert!(origins.contains(&(28.0 / 4.0 - half, 28.0 / 4.0 - half)));
        assert!(origins.contains(&(28.0 * 3.0 / 4.0 - half, 28.0 * 3.0 / 4.0 - half)));
        assert!(super::origins(0.0, 0.0).is_empty());
    }

    /// `test_aDotSitsBetweenEveryOtherPairOfMarks`: one dot to a cell,
    /// halfway between two marks of a row, every other gap, the rows
    /// taking turns.
    #[test]
    fn a_dot_sits_between_every_other_pair_of_marks() {
        let pitch = 28.0;
        let (width, height) = (pitch * 4.0 - 1.0, pitch * 2.0 - 1.0);
        let dots = dots(width, height);
        assert_eq!(dots.len(), 4 * 2);
        let marks: Vec<(f64, f64)> = origins(width, height)
            .into_iter()
            .map(|(x, y)| (x + 5.6 / 2.0, y + 5.6 / 2.0))
            .collect();
        for dot in &dots {
            let row: Vec<_> = marks
                .iter()
                .filter(|m| (m.1 - dot.1).abs() < 0.001)
                .collect();
            let left = row
                .iter()
                .any(|m| (m.0 - (dot.0 - pitch / 2.0)).abs() < 0.001);
            let right = row
                .iter()
                .any(|m| (m.0 - (dot.0 + pitch / 2.0)).abs() < 0.001)
                || dot.0 + pitch / 2.0 > width;
            assert!(left || dot.0 - pitch / 2.0 < 0.0, "no mark left of {dot:?}");
            assert!(right, "no mark right of {dot:?}");
            assert!(
                !marks
                    .iter()
                    .any(|m| (m.0 - dot.0).hypot(m.1 - dot.1) < pitch / 4.0),
                "a dot sits on a mark"
            );
        }
        let mut first: Vec<f64> = dots
            .iter()
            .filter(|d| (d.1 - pitch / 4.0).abs() < 0.001)
            .map(|d| d.0)
            .collect();
        first.sort_by(f64::total_cmp);
        assert_eq!(first.len(), 2);
        assert!((first[1] - first[0] - pitch * 2.0).abs() < 0.001);
    }

    /// The light field over the light pane is the Mac's own near-white:
    /// (247, 248, 249) at the map's edge in the Mac's
    /// `heldSessions-traces-main+map-light` render, whose pane is opaque
    /// in the same flat theme.
    #[test]
    fn the_light_field_is_the_macs_near_white() {
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 400, 400).unwrap();
        let colours = MapColours::scheme(false);
        {
            let cr = gtk::cairo::Context::new(&surface).unwrap();
            crate::ui::monitor::map::source(&cr, &colours.pane_opaque, 1.0);
            let _ = cr.paint();
            draw(&cr, 400.0, 400.0, &colours, false);
        }
        surface.flush();
        let data = surface.data().unwrap();
        let corner = [data[2], data[1], data[0]];
        for (k, mac) in [247, 248, 249].into_iter().enumerate() {
            assert!((i32::from(corner[k]) - mac).abs() <= 2, "corner {corner:?}");
        }
    }

    /// The field over the dark pane is navy and its marks are seen: the
    /// centre lighter than the corner by the veil's gradient, and a mark
    /// near the centre lighter than the ground beside it. The corner is
    /// mapFieldOuter (#070a1a at 0.32) over the pane.
    #[test]
    fn the_dark_field_is_navy_with_its_marks_seen() {
        let (w, h) = (400, 400);
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h).unwrap();
        let colours = MapColours::scheme(true);
        {
            let cr = gtk::cairo::Context::new(&surface).unwrap();
            crate::ui::monitor::map::source(&cr, &colours.pane_opaque, 1.0);
            let _ = cr.paint();
            draw(&cr, f64::from(w), f64::from(h), &colours, true);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let px = |x: usize, y: usize| {
            let i = y * stride + x * 4;
            [data[i + 2], data[i + 1], data[i]]
        };
        // Past the ellipse the Mac's gradient is mapFieldOuter again, over
        // its background of the same: #070a1a at 0.32 twice over #0d1124.
        let corner = px(1, 1);
        let expect = |pane: f64, veil: f64| {
            let once = pane * 0.68 + veil * 0.32;
            (once * 0.68 + veil * 0.32).round() as i32
        };
        let mac = [expect(13.0, 7.0), expect(17.0, 10.0), expect(36.0, 26.0)];
        for k in 0..3 {
            assert!(
                (i32::from(corner[k]) - mac[k]).abs() <= 2,
                "corner {corner:?}"
            );
        }
        // The first mark of the cell at the centre: its middle is ink.
        let cell = (200.0 / PITCH).floor() * PITCH;
        let mark = (cell + PITCH * 0.25, cell + PITCH * 0.25);
        let lit = (0..3)
            .flat_map(|dy| (0..3).map(move |dx| (dx, dy)))
            .map(|(dx, dy)| {
                px(mark.0 as usize - 1 + dx, mark.1 as usize - 1 + dy)
                    .iter()
                    .map(|c| u32::from(*c))
                    .sum::<u32>()
            })
            .max()
            .unwrap();
        let ground: u32 = px(cell as usize + 14, cell as usize + 3)
            .iter()
            .map(|c| u32::from(*c))
            .sum();
        assert!(lit > ground + 30, "mark {lit}, ground {ground}");
    }
}
