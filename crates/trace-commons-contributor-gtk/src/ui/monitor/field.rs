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
//! What moves on it (`map_motion` holds the numbers): the shimmer, a
//! slanted band of light that crosses the marks a second after the map is
//! shown and every 40 seconds after (`GlassMapShimmer`, `draw_shimmer`),
//! and the light under the pointer, the marks within 140 points lit and a
//! soft white bloom over them (`GlassMapMarkGlow`, `GlassMapLight`,
//! `draw_light`). The marks are cached once per size as an alpha mask with
//! the edge fade in it (`Field::tile_mask`), so a shimmer frame paints a
//! gradient through it and builds no path.

use std::cell::RefCell;

use super::glyph::Artwork;
use super::map::MapColours;
use super::map_motion;

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
    append_marks(cr, &origins(width, height));
    append_dots(cr, &dots(width, height));
    cr.set_fill_rule(gtk::cairo::FillRule::Winding);
    rgba(cr, &ink(dark));
    let _ = cr.fill();
    let _ = cr.pop_group_to_source();
    let _ = cr.mask(&fade(width, height));
    let _ = cr.restore();
}

/// The tile's fade toward the field's edges (`GlassMapMarks.fade`): full
/// in the middle, 85% halfway, gone at the edge.
fn fade(width: f64, height: f64) -> gtk::cairo::RadialGradient {
    let white = gtk::gdk::RGBA::WHITE;
    elliptical(
        width * 0.5,
        height * 0.52,
        width * 0.62,
        height * 0.62,
        &[(0.0, white, 1.0), (0.55, white, 0.85), (1.0, white, 0.0)],
    )
}

/// Adds a mark at each of `origins` (top-left corners) to the path.
fn append_marks(cr: &gtk::cairo::Context, origins: &[(f64, f64)]) {
    let mark = Artwork::brand_mark();
    let Some(segments) = mark.cached_segments() else {
        return;
    };
    let [bx, by, bw, bh] = mark.view_box;
    let scale = MARK / bw.max(bh);
    for (x, y) in origins {
        let _ = cr.save();
        cr.translate(*x, *y);
        cr.scale(scale, scale);
        cr.translate(-bx, -by);
        super::glyph::append_segments(cr, &segments);
        let _ = cr.restore();
    }
}

/// Adds a dot at each of `centres` to the path.
fn append_dots(cr: &gtk::cairo::Context, centres: &[(f64, f64)]) {
    for (x, y) in centres {
        cr.new_sub_path();
        cr.arc(*x, *y, DOT / 2.0, 0.0, std::f64::consts::TAU);
    }
}

/// The marks and dots of a field `width` by `height` as an alpha mask,
/// faded toward the edges as the Mac masks its tile, its shimmer and its
/// glow (`.mask(GlassMapMarks.fade)`), at `scale` pixels a point.
pub fn tile_mask(width: f64, height: f64, scale: f64) -> Option<gtk::cairo::ImageSurface> {
    let pixels = |points: f64| (points * scale).ceil() as i32;
    if pixels(width) <= 0 || pixels(height) <= 0 {
        return None;
    }
    let surface =
        gtk::cairo::ImageSurface::create(gtk::cairo::Format::A8, pixels(width), pixels(height))
            .ok()?;
    surface.set_device_scale(scale, scale);
    {
        let cr = gtk::cairo::Context::new(&surface).ok()?;
        cr.push_group();
        append_marks(&cr, &origins(width, height));
        append_dots(&cr, &dots(width, height));
        cr.set_fill_rule(gtk::cairo::FillRule::Winding);
        cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        let _ = cr.fill();
        let _ = cr.pop_group_to_source();
        let _ = cr.mask(&fade(width, height));
    }
    surface.flush();
    Some(surface)
}

/// The shimmer `progress` of the way across (`GlassMapShimmer`): the
/// marks lit along a slanted band, a gradient clear but for the band
/// painted through `mask` (`tile_mask`). Nothing at 0.
pub fn draw_shimmer(
    cr: &gtk::cairo::Context,
    mask: &gtk::cairo::ImageSurface,
    width: f64,
    height: f64,
    progress: f64,
    dark: bool,
) {
    if progress <= 0.0 {
        return;
    }
    let ((sx, sy), (ex, ey)) = map_motion::shimmer_band(progress, width, height);
    let crest = map_motion::shimmer_ink(dark);
    let band = gtk::cairo::LinearGradient::new(sx, sy, ex, ey);
    band.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.0);
    band.add_color_stop_rgba(0.5, 1.0, 1.0, 1.0, crest);
    band.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    let _ = cr.save();
    let _ = cr.set_source(&band);
    let _ = cr.mask_surface(mask, 0.0, 0.0);
    let _ = cr.restore();
}

/// The light under `pointer` at `opacity` (it fades in and out): the
/// marks and dots within `REACH` lit by `light(distance)` in `lit_ink`,
/// faded with the tile (`GlassMapMarkGlow`), then the bloom over them
/// (`GlassMapLight`), which the fade does not touch.
pub fn draw_light(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    pointer: (f64, f64),
    opacity: f64,
    dark: bool,
) {
    if opacity <= 0.0 {
        return;
    }
    let (px, py) = pointer;
    let reach = map_motion::REACH;
    let ink = map_motion::lit_ink(dark) * opacity;
    let _ = cr.save();
    // The marks and dots near the pointer, each as lit as its distance,
    // faded toward the field's edges with the tile.
    cr.push_group();
    let half = MARK / 2.0;
    let near = |x: f64, y: f64| (x - px).abs() < reach && (y - py).abs() < reach;
    for origin in origins(width, height) {
        let lit = map_motion::light((origin.0 + half - px).hypot(origin.1 + half - py));
        if lit <= 0.0 || !near(origin.0 + half, origin.1 + half) {
            continue;
        }
        cr.new_path();
        append_marks(cr, &[origin]);
        cr.set_fill_rule(gtk::cairo::FillRule::Winding);
        cr.set_source_rgba(1.0, 1.0, 1.0, ink * lit);
        let _ = cr.fill();
    }
    for centre in dots(width, height) {
        let lit = map_motion::light((centre.0 - px).hypot(centre.1 - py));
        if lit <= 0.0 {
            continue;
        }
        cr.new_path();
        append_dots(cr, &[centre]);
        cr.set_source_rgba(1.0, 1.0, 1.0, ink * lit);
        let _ = cr.fill();
    }
    let _ = cr.pop_group_to_source();
    let _ = cr.mask(&fade(width, height));
    // The bloom, over the marks and the veil.
    let bloom = gtk::cairo::RadialGradient::new(px, py, 0.0, px, py, reach);
    let alpha = map_motion::bloom(dark) * opacity;
    bloom.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, alpha);
    bloom.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    cr.new_path();
    cr.arc(px, py, reach, 0.0, std::f64::consts::TAU);
    let _ = cr.set_source(&bloom);
    let _ = cr.fill();
    let _ = cr.restore();
}

/// The field's drawing, kept until its size or scheme changes.
/// What the field was last drawn for: its size, scheme and veil.
type FieldKey = (i32, i32, bool, gtk::gdk::RGBA, gtk::gdk::RGBA);

#[derive(Default)]
pub struct Field {
    kept: RefCell<Option<(FieldKey, gtk::gsk::RenderNode)>>,
    /// The marks' alpha mask for the shimmer, by size and scale.
    mask: RefCell<Option<((i32, i32, i32), gtk::cairo::ImageSurface)>>,
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

    /// The marks' mask for a field `width` by `height` at `scale`, drawn
    /// once and kept until its size or scale changes.
    pub fn tile_mask(
        &self,
        width: i32,
        height: i32,
        scale: i32,
    ) -> Option<gtk::cairo::ImageSurface> {
        let key = (width, height, scale);
        if let Some((kept, mask)) = self.mask.borrow().as_ref()
            && *kept == key
        {
            return Some(mask.clone());
        }
        let mask = tile_mask(f64::from(width), f64::from(height), f64::from(scale.max(1)))?;
        self.mask.replace(Some((key, mask.clone())));
        Some(mask)
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

    /// The field over the dark pane, its marks and ground, with what
    /// `paint` adds over it, as the brightness of a few pixels.
    fn dark_field(
        w: i32,
        h: i32,
        paint: impl Fn(&gtk::cairo::Context),
    ) -> impl Fn(f64, f64) -> u32 {
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h).unwrap();
        let colours = MapColours::scheme(true);
        {
            let cr = gtk::cairo::Context::new(&surface).unwrap();
            crate::ui::monitor::map::source(&cr, &colours.pane_opaque, 1.0);
            let _ = cr.paint();
            draw(&cr, f64::from(w), f64::from(h), &colours, true);
            paint(&cr);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap().to_vec();
        move |x: f64, y: f64| {
            let i = y as usize * stride + x as usize * 4;
            u32::from(data[i]) + u32::from(data[i + 1]) + u32::from(data[i + 2])
        }
    }

    /// The brightest pixel of the mark whose top-left corner is `origin`.
    fn mark_peak(px: &impl Fn(f64, f64) -> u32, origin: (f64, f64)) -> u32 {
        (0..6)
            .flat_map(|dy| (0..6).map(move |dx| (dx, dy)))
            .map(|(dx, dy)| px(origin.0 + f64::from(dx), origin.1 + f64::from(dy)))
            .max()
            .unwrap()
    }

    /// The shimmer lights the marks on its band, slanted 20 degrees and
    /// crossing left to right, and leaves the rest as they were; at 0 it
    /// draws nothing (`GlassMapShimmer`).
    #[test]
    fn the_shimmer_lights_the_marks_on_its_band_only() {
        let (w, h) = (400, 400);
        let mask = tile_mask(f64::from(w), f64::from(h), 1.0).unwrap();
        let rest = dark_field(w, h, |_| {});
        let none = dark_field(w, h, |cr| {
            draw_shimmer(cr, &mask, f64::from(w), f64::from(h), 0.0, true)
        });
        let half = dark_field(w, h, |cr| {
            draw_shimmer(cr, &mask, f64::from(w), f64::from(h), 0.5, true)
        });
        let marks = origins(f64::from(w), f64::from(h));
        let ((sx, sy), (ex, ey)) = map_motion::shimmer_band(0.5, f64::from(w), f64::from(h));
        let (ux, uy) = ((ex - sx), (ey - sy));
        let length = ux.hypot(uy);
        // How far along the band's normal a point is, 0 at its crest.
        let off_crest = |(x, y): (f64, f64)| {
            let along = ((x - sx) * ux + (y - sy) * uy) / length;
            (along - length / 2.0).abs()
        };
        let centre = |&(x, y): &(f64, f64)| (x + MARK / 2.0, y + MARK / 2.0);
        let on = marks
            .iter()
            .filter(|o| {
                let c = centre(o);
                off_crest(c) < 3.0 && (c.0 - 200.0).abs() < 60.0 && (c.1 - 200.0).abs() < 60.0
            })
            .copied()
            .collect::<Vec<_>>();
        assert!(!on.is_empty(), "no mark on the crest near the middle");
        for origin in &on {
            assert!(
                mark_peak(&half, *origin) > mark_peak(&rest, *origin) + 60,
                "a mark on the crest at {origin:?} is not lit"
            );
            assert_eq!(mark_peak(&none, *origin), mark_peak(&rest, *origin));
        }
        let off = marks
            .iter()
            .find(|o| off_crest(centre(o)) > length && (centre(o).0 - 200.0).abs() < 80.0)
            .copied()
            .unwrap();
        assert_eq!(mark_peak(&half, off), mark_peak(&rest, off), "off the band");
    }

    /// Under the pointer the marks are lit to `litInk`, easing to nothing
    /// at 140 points, and a bloom lightens the ground; the light at half
    /// opacity is half as strong (`GlassMapMarkGlow`, `GlassMapLight`).
    #[test]
    fn the_light_under_the_pointer_reaches_140_points() {
        let (w, h) = (400, 400);
        let marks = origins(f64::from(w), f64::from(h));
        let pointer = (marks[0].0 + MARK / 2.0, marks[0].1 + MARK / 2.0);
        let pointer = marks
            .iter()
            .map(|&(x, y)| (x + MARK / 2.0, y + MARK / 2.0))
            .min_by(|a, b| {
                (a.0 - 200.0)
                    .hypot(a.1 - 200.0)
                    .total_cmp(&(b.0 - 200.0).hypot(b.1 - 200.0))
            })
            .unwrap_or(pointer);
        let rest = dark_field(w, h, |_| {});
        let lit = dark_field(w, h, |cr| {
            draw_light(cr, f64::from(w), f64::from(h), pointer, 1.0, true)
        });
        let dim = dark_field(w, h, |cr| {
            draw_light(cr, f64::from(w), f64::from(h), pointer, 0.5, true)
        });
        let under = (pointer.0 - MARK / 2.0, pointer.1 - MARK / 2.0);
        let gain = mark_peak(&lit, under) - mark_peak(&rest, under);
        let half_gain = mark_peak(&dim, under) - mark_peak(&rest, under);
        assert!(gain > 40, "the mark under the pointer gains {gain}");
        assert!(
            half_gain < gain && half_gain * 2 + 12 > gain,
            "half the light gains {half_gain} of {gain}"
        );
        // Past the reach nothing changes, the bloom included.
        let far = marks
            .iter()
            .find(|&&(x, y)| (x + MARK / 2.0 - pointer.0).hypot(y + MARK / 2.0 - pointer.1) > 150.0)
            .copied()
            .unwrap();
        assert_eq!(mark_peak(&lit, far), mark_peak(&rest, far));
        // The ground beside the pointer is lighter by the bloom.
        let ground = (pointer.0 + PITCH / 2.0, pointer.1);
        assert!(lit(ground.0, ground.1) > rest(ground.0, ground.1));
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
