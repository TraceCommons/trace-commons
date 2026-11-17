//! The Mac's toolbar glyphs, drawn in GTK.
//!
//! `brand_tokens::glyph` holds each glyph as an SVG, exported by
//! `scripts/design-tokens/generate.py` from the path data in
//! `macos/Sources/TCDesign/Components/Glyphs.swift`. This file reads those
//! strings (no SVG loader, no icon theme) and strokes or fills the path with
//! cairo in the widget's own foreground colour, so a glyph follows the
//! button's state and the scheme the way a symbolic icon does.
//!
//! The path data uses only `M L H V A Z` and their relative forms, which is
//! all this parses; any other command makes the glyph not parse, and the
//! tests below fail for it.

use gtk::prelude::*;

use crate::ui::style::brand_tokens::glyph;

/// One segment of a path, in absolute coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Move(f64, f64),
    Line(f64, f64),
    Arc {
        radius: (f64, f64),
        rotation: f64,
        large: bool,
        sweep: bool,
        to: (f64, f64),
    },
    Close,
}

/// A parsed glyph: its outline in viewBox units, and how it is painted.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub segments: Vec<Segment>,
    pub view_box: (f64, f64),
    /// The size the Mac draws it at.
    pub size: (f64, f64),
    /// The stroke width in viewBox units; `None` for a filled glyph.
    pub stroke: Option<f64>,
}

fn attribute<'a>(svg: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = svg.find(&needle)? + needle.len();
    let end = svg[start..].find('"')? + start;
    Some(&svg[start..end])
}

struct Numbers<'a> {
    rest: &'a str,
}

impl Numbers<'_> {
    fn skip(&mut self) {
        self.rest = self
            .rest
            .trim_start_matches(|c: char| c == ',' || c.is_whitespace());
    }

    fn next_command(&mut self) -> Option<char> {
        self.skip();
        let c = self.rest.chars().next()?;
        if c.is_ascii_alphabetic() {
            self.rest = &self.rest[1..];
            Some(c)
        } else {
            None
        }
    }

    fn has_number(&mut self) -> bool {
        self.skip();
        self.rest
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')
    }

    fn number(&mut self) -> Option<f64> {
        self.skip();
        let bytes = self.rest.as_bytes();
        let mut end = 0;
        if matches!(bytes.first(), Some(b'-' | b'+')) {
            end += 1;
        }
        let mut dot = false;
        while let Some(&b) = bytes.get(end) {
            if b.is_ascii_digit() {
                end += 1;
            } else if b == b'.' && !dot {
                dot = true;
                end += 1;
            } else {
                break;
            }
        }
        let value = self.rest[..end].parse().ok()?;
        self.rest = &self.rest[end..];
        Some(value)
    }

    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = self.rest.chars().next()?;
        self.rest = &self.rest[1..];
        match c {
            '0' => Some(false),
            '1' => Some(true),
            _ => None,
        }
    }

    fn pair(&mut self) -> Option<(f64, f64)> {
        Some((self.number()?, self.number()?))
    }
}

/// Parses SVG path data into absolute segments, or `None` for a command
/// outside `M L H V A Z`.
pub fn parse_path(data: &str) -> Option<Vec<Segment>> {
    let mut input = Numbers { rest: data };
    let mut out = Vec::new();
    let (mut x, mut y) = (0.0, 0.0);
    let (mut start_x, mut start_y) = (0.0, 0.0);
    while let Some(command) = input.next_command() {
        let relative = command.is_ascii_lowercase();
        let (dx, dy) = if relative { (x, y) } else { (0.0, 0.0) };
        match command.to_ascii_uppercase() {
            'Z' => {
                out.push(Segment::Close);
                (x, y) = (start_x, start_y);
            }
            'M' | 'L' => {
                let mut first = command.eq_ignore_ascii_case(&'M');
                loop {
                    let (px, py) = input.pair()?;
                    let (dx, dy) = if relative { (x, y) } else { (0.0, 0.0) };
                    (x, y) = (dx + px, dy + py);
                    if first {
                        out.push(Segment::Move(x, y));
                        (start_x, start_y) = (x, y);
                        first = false;
                    } else {
                        out.push(Segment::Line(x, y));
                    }
                    if !input.has_number() {
                        break;
                    }
                }
            }
            'H' => loop {
                x = dx + input.number()?;
                out.push(Segment::Line(x, y));
                if !input.has_number() {
                    break;
                }
            },
            'V' => loop {
                y = dy + input.number()?;
                out.push(Segment::Line(x, y));
                if !input.has_number() {
                    break;
                }
            },
            'A' => loop {
                let radius = input.pair()?;
                let rotation = input.number()?;
                let large = input.flag()?;
                let sweep = input.flag()?;
                let (px, py) = input.pair()?;
                let (dx, dy) = if relative { (x, y) } else { (0.0, 0.0) };
                (x, y) = (dx + px, dy + py);
                out.push(Segment::Arc {
                    radius,
                    rotation,
                    large,
                    sweep,
                    to: (x, y),
                });
                if !input.has_number() {
                    break;
                }
            },
            _ => return None,
        }
    }
    input.skip();
    input.rest.is_empty().then_some(out)
}

impl Glyph {
    /// Reads one of `brand_tokens::glyph`'s SVGs.
    pub fn parse(svg: &str) -> Option<Self> {
        let path_at = svg.find("<path")?;
        let data = attribute(&svg[path_at..], "d")?;
        let mut box_numbers = attribute(svg, "viewBox")?
            .split_whitespace()
            .map(str::parse::<f64>);
        let (_, _, w, h) = (
            box_numbers.next()?.ok()?,
            box_numbers.next()?.ok()?,
            box_numbers.next()?.ok()?,
            box_numbers.next()?.ok()?,
        );
        let size = (
            attribute(svg, "width")?.parse().ok()?,
            attribute(svg, "height")?.parse().ok()?,
        );
        let stroke = attribute(&svg[path_at..], "stroke-width").and_then(|v| v.parse().ok());
        Some(Self {
            segments: parse_path(data)?,
            view_box: (w, h),
            size,
            stroke,
        })
    }

    /// Strokes or fills the glyph into `width` by `height`, aspect kept and
    /// centred, in the colour already set on `cr`.
    pub fn draw(&self, cr: &gtk::cairo::Context, width: f64, height: f64) {
        let scale = (width / self.view_box.0).min(height / self.view_box.1);
        let _ = cr.save();
        cr.translate(
            (width - self.view_box.0 * scale) / 2.0,
            (height - self.view_box.1 * scale) / 2.0,
        );
        cr.scale(scale, scale);
        self.trace(cr);
        match self.stroke {
            Some(line) => {
                cr.set_line_width(line);
                cr.set_line_join(gtk::cairo::LineJoin::Round);
                cr.set_line_cap(gtk::cairo::LineCap::Butt);
                let _ = cr.stroke();
            }
            None => {
                let _ = cr.fill();
            }
        }
        let _ = cr.restore();
    }

    fn trace(&self, cr: &gtk::cairo::Context) {
        cr.new_path();
        let (mut x, mut y) = (0.0, 0.0);
        for segment in &self.segments {
            match *segment {
                Segment::Move(px, py) => {
                    cr.move_to(px, py);
                    (x, y) = (px, py);
                }
                Segment::Line(px, py) => {
                    cr.line_to(px, py);
                    (x, y) = (px, py);
                }
                Segment::Arc {
                    radius,
                    rotation,
                    large,
                    sweep,
                    to,
                } => {
                    arc_to(cr, (x, y), radius, rotation, large, sweep, to);
                    (x, y) = to;
                }
                Segment::Close => cr.close_path(),
            }
        }
    }
}

/// An SVG endpoint arc as a cairo arc (SVG 1.1 implementation notes F.6.5).
fn arc_to(
    cr: &gtk::cairo::Context,
    from: (f64, f64),
    radius: (f64, f64),
    rotation_degrees: f64,
    large: bool,
    sweep: bool,
    to: (f64, f64),
) {
    let (mut rx, mut ry) = (radius.0.abs(), radius.1.abs());
    if from == to {
        return;
    }
    if rx == 0.0 || ry == 0.0 {
        cr.line_to(to.0, to.1);
        return;
    }
    let phi = rotation_degrees.to_radians();
    let (sin, cos) = phi.sin_cos();
    let half = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let x1 = cos * half.0 + sin * half.1;
    let y1 = -sin * half.0 + cos * half.1;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut factor = (numerator / denominator).max(0.0).sqrt();
    if large == sweep {
        factor = -factor;
    }
    let cx1 = factor * (rx * y1 / ry);
    let cy1 = -factor * (ry * x1 / rx);
    let cx = cos * cx1 - sin * cy1 + (from.0 + to.0) / 2.0;
    let cy = sin * cx1 + cos * cy1 + (from.1 + to.1) / 2.0;
    let angle = |ux: f64, uy: f64| uy.atan2(ux);
    let start = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut end = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    if sweep && end < start {
        end += std::f64::consts::TAU;
    } else if !sweep && end > start {
        end -= std::f64::consts::TAU;
    }
    let _ = cr.save();
    cr.translate(cx, cy);
    cr.rotate(phi);
    cr.scale(rx, ry);
    if sweep {
        cr.arc(0.0, 0.0, 1.0, start, end);
    } else {
        cr.arc_negative(0.0, 0.0, 1.0, start, end);
    }
    let _ = cr.restore();
}

/// A glyph as a widget, at the size the Mac draws it, in its parent's
/// foreground colour. Decorative: the control around it carries the name.
pub fn icon(svg: &str) -> gtk::DrawingArea {
    let glyph = Glyph::parse(svg).expect("brand_tokens::glyph holds well-formed glyphs");
    let area = gtk::DrawingArea::builder()
        .content_width(glyph.size.0.round() as i32)
        .content_height(glyph.size.1.round() as i32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    area.set_draw_func(move |area, cr, width, height| {
        // GTK 4.10's `Widget::color` is not in this crate's feature floor.
        #[allow(deprecated)]
        let color = area.style_context().color();
        cr.set_source_rgba(
            f64::from(color.red()),
            f64::from(color.green()),
            f64::from(color.blue()),
            f64::from(color.alpha()),
        );
        glyph.draw(cr, f64::from(width), f64::from(height));
    });
    area
}

/// `brand_tokens::glyph`'s eight glyphs by their Mac name.
pub fn named(name: &str) -> Option<&'static str> {
    glyph::ALL
        .iter()
        .find_map(|(known, svg)| (*known == name).then_some(*svg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::cairo;

    #[test]
    fn the_mac_names_eight_glyphs() {
        let names: Vec<&str> = glyph::ALL.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "viewMenu",
                "graph",
                "map",
                "inspector",
                "gear",
                "chevronDown",
                "folder",
                "kebab"
            ]
        );
        assert_eq!(named("map"), Some(glyph::MAP));
        assert_eq!(named("nothing"), None);
    }

    #[test]
    fn every_glyph_parses_and_stays_inside_its_view_box() {
        for (name, svg) in glyph::ALL {
            let parsed = Glyph::parse(svg).unwrap_or_else(|| panic!("{name} does not parse"));
            assert!(
                matches!(parsed.segments.first(), Some(Segment::Move(..))),
                "{name} must begin with a move"
            );
            for segment in &parsed.segments {
                let points: Vec<(f64, f64)> = match *segment {
                    Segment::Move(x, y) | Segment::Line(x, y) => vec![(x, y)],
                    Segment::Arc { to, .. } => vec![to],
                    Segment::Close => vec![],
                };
                for (x, y) in points {
                    assert!(
                        (0.0..=parsed.view_box.0).contains(&x)
                            && (0.0..=parsed.view_box.1).contains(&y),
                        "{name}: ({x}, {y}) is outside {:?}",
                        parsed.view_box
                    );
                }
            }
        }
    }

    #[test]
    fn strokes_and_fills_follow_the_swift_artwork() {
        let by = |name| Glyph::parse(named(name).unwrap()).unwrap();
        assert_eq!(by("viewMenu").stroke, Some(1.4));
        assert_eq!(by("viewMenu").size, (14.0, 12.0));
        assert_eq!(by("graph").stroke, Some(1.3));
        assert_eq!(by("gear").stroke, Some(1.7));
        assert_eq!(by("gear").size, (15.0, 15.0));
        assert_eq!(by("chevronDown").stroke, Some(1.8));
        assert_eq!(by("folder").stroke, Some(1.8));
        assert_eq!(by("kebab").stroke, None);
        assert_eq!(by("kebab").view_box, (4.0, 14.0));
    }

    #[test]
    fn relative_and_implicit_commands_are_absolute_segments() {
        assert_eq!(
            parse_path("M1,2h12M1,6h12").unwrap(),
            [
                Segment::Move(1.0, 2.0),
                Segment::Line(13.0, 2.0),
                Segment::Move(1.0, 6.0),
                Segment::Line(13.0, 6.0),
            ]
        );
        // Pairs after an `l` are further lines, and a minus sign separates.
        assert_eq!(
            parse_path("M0.5,2.5l4-2,5,2").unwrap(),
            [
                Segment::Move(0.5, 2.5),
                Segment::Line(4.5, 0.5),
                Segment::Line(9.5, 2.5),
            ]
        );
        assert_eq!(parse_path("M1,1C2,2,3,3,4,4"), None);
    }

    /// The number of painted pixels, and whether any lies outside the
    /// image's bounds is impossible to ask of cairo, so the glyph is drawn
    /// into a frame with a margin and the margin is checked instead.
    fn painted(svg: &str, margin: i32) -> (usize, usize) {
        let glyph = Glyph::parse(svg).unwrap();
        let (w, h) = (
            glyph.size.0.round() as i32 * 4,
            glyph.size.1.round() as i32 * 4,
        );
        let (full_w, full_h) = (w + 2 * margin, h + 2 * margin);
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, full_w, full_h)
            .expect("an image surface");
        {
            let cr = cairo::Context::new(&surface).unwrap();
            cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
            cr.translate(f64::from(margin), f64::from(margin));
            glyph.draw(&cr, f64::from(w), f64::from(h));
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let (mut inside, mut in_margin) = (0, 0);
        for y in 0..full_h {
            for x in 0..full_w {
                let alpha = data[y as usize * stride + x as usize * 4 + 3];
                if alpha > 32 {
                    let in_frame =
                        (margin..margin + w).contains(&x) && (margin..margin + h).contains(&y);
                    if in_frame {
                        inside += 1;
                    } else {
                        in_margin += 1;
                    }
                }
            }
        }
        (inside, in_margin)
    }

    #[test]
    fn every_glyph_paints_something_and_stays_in_its_frame() {
        for (name, svg) in glyph::ALL {
            let (inside, outside) = painted(svg, 8);
            assert!(inside > 0, "{name} painted nothing");
            // Half a stroke past the outline is the most it may reach; the
            // frame is four times the Mac's size, so a margin of eight is
            // wider than that for every stroke here.
            assert_eq!(outside, 0, "{name} painted outside its frame");
        }
    }

    #[test]
    fn the_gear_arcs_make_a_hub_that_is_not_filled_in() {
        // The gear is a stroked outline with a ring at its centre: its
        // centre pixel is empty, which a mis-converted arc would fill or
        // move.
        let glyph = Glyph::parse(glyph::GEAR).unwrap();
        let side = 120;
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, side, side).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
            glyph.draw(&cr, f64::from(side), f64::from(side));
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let alpha = |x: usize, y: usize| data[y * stride + x * 4 + 3];
        let middle = side as usize / 2;
        assert_eq!(alpha(middle, middle), 0, "the hub's centre is open");
        // The hub ring is 3.2 viewBox units of 24 from the centre.
        let ring = middle + (3.2 / 24.0 * side as f64).round() as usize;
        assert!(
            alpha(ring, middle) > 128,
            "the hub ring is drawn at its radius"
        );
    }
}
