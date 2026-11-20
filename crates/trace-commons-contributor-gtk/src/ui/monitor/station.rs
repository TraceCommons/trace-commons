//! The library on the Traces map: the NEAR AI space station.
//!
//! A port of `GlassLibraryStation` in
//! `macos/Sources/TCDesign/Components/LibraryStation.swift` (owner,
//! 2026-10-16): a mesh built round the NEAR AI mark, extruded from the
//! brand kit's path into a slab lying in the station's plane, with a torus
//! ring, four spokes, four modules and two solar arrays round it. Each
//! frame the mesh turns about its own axis, is tilted toward the viewer
//! and leaned, projected in perspective, sorted far to near and
//! flat-shaded from one light. One turn takes `TURN_SECONDS`; with
//! animations off it is drawn once, at rest.
//!
//! The station is drawn as bright whether or not anything has been
//! contributed (owner, 2026-10-19); only its glow is softer before.
//!
//! The arithmetic is the Swift's, step for step, so the mesh and its
//! projection match the Mac's to rounding; the tests pin numbers printed
//! from the Swift.

use std::f64::consts::TAU;
use std::sync::OnceLock;

use super::glyph::{Artwork, Segment, parse_path};

/// One turn of the station, in seconds.
pub const TURN_SECONDS: f64 = 26.0;
/// The angle the station rests at, and starts from.
pub const REST_ANGLE: f64 = 0.6;
/// The station's own frame rate: one turn in 26 seconds moves the solar
/// arrays' tips about a point and a third a frame at 15 frames a second.
/// Each frame rebuilds, sorts and fills the whole mesh, so this rate is
/// most of what the Traces map costs while it is on screen.
pub const FRAME_INTERVAL: f64 = 1.0 / 15.0;
/// The glow's radius on the Mac (`.shadow(radius: 6)`), in points.
pub const GLOW_RADIUS: f64 = 6.0;
/// The same glow as a GSK shadow's blur radius. The two read a radius
/// differently: rendered beside the Mac's at 2x, 16 came closest (mean
/// difference 1.6/255 over the frame, against 2.0 at 6 and 1.7 at 12).
pub const GSK_GLOW_RADIUS: f64 = 16.0;
/// The station's tilt toward the viewer and its lean, and the camera.
pub const TILT: f64 = 42.0 * std::f64::consts::PI / 180.0;
pub const LEAN: f64 = -14.0 * std::f64::consts::PI / 180.0;
pub const CAMERA_DISTANCE: f64 = 240.0;
/// The ring's radius and its tube's, in mesh units.
pub const RING_RADIUS: f64 = 36.0;
pub const RING_TUBE: f64 = 3.6;
/// The NEAR AI brand blue, #0091fd: the campaign gradient's light stop.
pub const BRAND_BLUE: [f64; 3] = [0.0, 145.0 / 255.0, 253.0 / 255.0];

type Vec3 = [f64; 3];

/// One flat face: its corners, its colour (0...1), and whether it is one
/// of the mark's two faces or part of the mark at all.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub points: Vec<Vec3>,
    pub color: Vec3,
    pub mark_cap: bool,
    pub mark: bool,
}

/// A face as one frame draws it: its shading, its fill, and the depth it
/// is sorted by. `index` is its place in `mesh()`; its corners are
/// `indexed().face(index)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    pub index: usize,
    pub shade: f64,
    pub spec: f64,
    pub depth: f64,
    pub fill: Vec3,
}

/// The glow's strength: full once something has been contributed, a
/// little softer before.
pub fn glow(lit: bool) -> f64 {
    if lit { 0.35 } else { 0.25 }
}

/// The station's angle `seconds` into the clock; at rest with animations
/// off.
pub fn angle(seconds: f64, reduce_motion: bool) -> f64 {
    if reduce_motion {
        REST_ANGLE
    } else {
        // Swift's `truncatingRemainder` is Rust's `%` on floats.
        REST_ANGLE + (seconds % TURN_SECONDS) / TURN_SECONDS * TAU
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

/// The light, from above left and in front.
pub fn light() -> Vec3 {
    let l = [-0.42, 0.62, 0.66];
    let n = length(l);
    [l[0] / n, l[1] / n, l[2] / n]
}

/// The point `t` of the way along a Bezier curve given by its control
/// points (de Casteljau).
fn bezier(controls: &[[f64; 2]], t: f64) -> [f64; 2] {
    let mut level = controls.to_vec();
    while level.len() > 1 {
        level = level
            .windows(2)
            .map(|w| {
                [
                    w[0][0] + (w[1][0] - w[0][0]) * t,
                    w[0][1] + (w[1][1] - w[0][1]) * t,
                ]
            })
            .collect();
    }
    level.first().copied().unwrap_or([0.0, 0.0])
}

/// The mark's outline from the brand kit path (`logo::BRAND_MARK`),
/// curves flattened into five steps each, centred, scaled to `height`
/// units and turned y-up.
pub fn mark_outline(height: f64) -> Vec<[f64; 2]> {
    let mark = Artwork::brand_mark();
    let Some(segments) = mark.paths.first().and_then(|data| parse_path(data)) else {
        return Vec::new();
    };
    let mut points: Vec<[f64; 2]> = Vec::new();
    let mut current = [0.0, 0.0];
    for segment in segments {
        match segment {
            Segment::Move(x, y) | Segment::Line(x, y) => {
                current = [x, y];
                points.push(current);
            }
            Segment::Cubic(a, b, to) => {
                let curve = [current, [a.0, a.1], [b.0, b.1], [to.0, to.1]];
                for step in 1..=5 {
                    points.push(bezier(&curve, f64::from(step) / 5.0));
                }
                current = [to.0, to.1];
            }
            // The mark is lines and cubics only; an arc never reaches here.
            Segment::Arc { to, .. } => {
                current = [to.0, to.1];
                points.push(current);
            }
            Segment::Close => {}
        }
    }
    let [bx, by, bw, bh] = mark.view_box;
    let k = height / bh;
    let centre = [bx + bw / 2.0, by + bh / 2.0];
    let distance =
        |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let mut outline: Vec<[f64; 2]> = Vec::new();
    for point in points {
        let offset = [point[0] - centre[0], point[1] - centre[1]];
        let q = [offset[0] * k, -offset[1] * k];
        if outline
            .last()
            .is_some_and(|&last| distance(q, last) <= 0.05)
        {
            continue;
        }
        outline.push(q);
    }
    if outline.len() > 1 && distance(outline[0], outline[outline.len() - 1]) < 0.05 {
        outline.pop();
    }
    outline
}

fn rgb(r: f64, g: f64, b: f64) -> Vec3 {
    [r / 255.0, g / 255.0, b / 255.0]
}

fn build() -> Vec<Face> {
    let mut faces: Vec<Face> = Vec::new();
    let face = |points: Vec<Vec3>, color: Vec3| Face {
        points,
        color,
        mark_cap: false,
        mark: false,
    };
    let boxed = |faces: &mut Vec<Face>, c: Vec3, s: Vec3, color: Vec3| {
        let a = [c[0] - s[0] / 2.0, c[1] - s[1] / 2.0, c[2] - s[2] / 2.0];
        let b = [c[0] + s[0] / 2.0, c[1] + s[1] / 2.0, c[2] + s[2] / 2.0];
        let p = |x: f64, y: f64, z: f64| [x, y, z];
        for points in [
            [
                p(a[0], a[1], b[2]),
                p(b[0], a[1], b[2]),
                p(b[0], b[1], b[2]),
                p(a[0], b[1], b[2]),
            ],
            [
                p(b[0], a[1], a[2]),
                p(a[0], a[1], a[2]),
                p(a[0], b[1], a[2]),
                p(b[0], b[1], a[2]),
            ],
            [
                p(a[0], b[1], a[2]),
                p(a[0], b[1], b[2]),
                p(b[0], b[1], b[2]),
                p(b[0], b[1], a[2]),
            ],
            [
                p(a[0], a[1], b[2]),
                p(a[0], a[1], a[2]),
                p(b[0], a[1], a[2]),
                p(b[0], a[1], b[2]),
            ],
            [
                p(b[0], a[1], b[2]),
                p(b[0], a[1], a[2]),
                p(b[0], b[1], a[2]),
                p(b[0], b[1], b[2]),
            ],
            [
                p(a[0], a[1], a[2]),
                p(a[0], a[1], b[2]),
                p(a[0], b[1], b[2]),
                p(a[0], b[1], a[2]),
            ],
        ] {
            faces.push(face(points.to_vec(), color));
        }
    };

    // The mark, extruded, lying in the station's plane (owner,
    // 2026-10-16): its top toward the plane's far side, so it reads
    // upright once the station is tilted.
    let mark = mark_outline(30.0);
    let h = 3.2;
    let blue = rgb(0.0, 145.0, 253.0);
    faces.push(Face {
        points: mark.iter().map(|p| [p[0], h, -p[1]]).collect(),
        color: blue,
        mark_cap: true,
        mark: true,
    });
    faces.push(Face {
        points: mark.iter().rev().map(|p| [p[0], -h, -p[1]]).collect(),
        color: blue,
        mark_cap: true,
        mark: true,
    });
    for (j, p) in mark.iter().enumerate() {
        let q = mark[(j + 1) % mark.len()];
        faces.push(Face {
            points: vec![
                [p[0], h, -p[1]],
                [q[0], h, -q[1]],
                [q[0], -h, -q[1]],
                [p[0], -h, -p[1]],
            ],
            color: rgb(0.0, 98.0, 176.0),
            mark_cap: false,
            mark: true,
        });
    }

    // The ring: a torus round the mark, in the same plane; metal with a
    // little of the brand blue in it.
    let (ring_radius, tube, around, across) = (RING_RADIUS, RING_TUBE, 40, 8);
    let torus = |u: i32, v: i32| -> Vec3 {
        let a = f64::from(u) / f64::from(around) * 2.0 * std::f64::consts::PI;
        let b = f64::from(v) / f64::from(across) * 2.0 * std::f64::consts::PI;
        let r = ring_radius + tube * b.cos();
        [r * a.cos(), tube * b.sin(), r * a.sin()]
    };
    for u in 0..around {
        for v in 0..across {
            faces.push(face(
                vec![
                    torus(u, v),
                    torus(u + 1, v),
                    torus(u + 1, v + 1),
                    torus(u, v + 1),
                ],
                rgb(172.0, 196.0, 230.0),
            ));
        }
    }

    // Spokes from beside the mark to the ring, and modules on the ring.
    let spoke = rgb(140.0, 164.0, 200.0);
    let hub = 17.0;
    let length = ring_radius - hub;
    let mid = (hub + ring_radius) / 2.0;
    boxed(&mut faces, [0.0, 0.0, mid], [1.8, 1.8, length], spoke);
    boxed(&mut faces, [0.0, 0.0, -mid], [1.8, 1.8, length], spoke);
    boxed(&mut faces, [mid, 0.0, 0.0], [length, 1.8, 1.8], spoke);
    boxed(&mut faces, [-mid, 0.0, 0.0], [length, 1.8, 1.8], spoke);
    for degrees in [45.0, 135.0, 225.0, 315.0_f64] {
        let a = degrees * std::f64::consts::PI / 180.0;
        boxed(
            &mut faces,
            [ring_radius * a.cos(), 0.0, ring_radius * a.sin()],
            [6.0, 5.0, 6.0],
            rgb(226.0, 236.0, 248.0),
        );
    }

    // Solar arrays on trusses beyond the ring, lying in the station's
    // plane like the mark.
    let truss = rgb(120.0, 134.0, 156.0);
    let panel = rgb(44.0, 96.0, 176.0);
    for side in [1.0, -1.0] {
        boxed(
            &mut faces,
            [side * (ring_radius + 9.0), 0.0, 0.0],
            [14.0, 1.6, 1.6],
            truss,
        );
        boxed(
            &mut faces,
            [side * (ring_radius + 20.0), 0.0, 0.0],
            [16.0, 0.9, 20.0],
            panel,
        );
        boxed(
            &mut faces,
            [side * (ring_radius + 20.0), 0.0, 0.0],
            [0.9, 1.1, 20.4],
            truss,
        );
    }
    faces
}

/// The mesh, built once.
pub fn mesh() -> &'static [Face] {
    static MESH: OnceLock<Vec<Face>> = OnceLock::new();
    MESH.get_or_init(build)
}

/// The tilt's and the lean's cosines and sines, worked out once rather
/// than for every corner of every face on every frame, as the Swift keeps
/// them in statics.
static POSE: std::sync::LazyLock<[f64; 4]> =
    std::sync::LazyLock::new(|| [TILT.cos(), TILT.sin(), LEAN.cos(), LEAN.sin()]);

/// `turned` with the angle's cosine and sine given, so a frame works them
/// out once for the whole mesh: the same arithmetic in the same order as
/// the Swift.
fn turned_by(p: Vec3, cy: f64, sy: f64) -> Vec3 {
    let [cx, sx, cz, sz] = *POSE;
    let x = p[0] * cy + p[2] * sy;
    let z = -p[0] * sy + p[2] * cy;
    let y = p[1];
    let y2 = y * cx - z * sx;
    let z2 = y * sx + z * cx;
    [x * cz - y2 * sz, x * sz + y2 * cz, z2]
}

/// A point of the mesh turned by `angle` about the station's axis, then
/// tilted and leaned.
pub fn turned(p: Vec3, angle: f64) -> Vec3 {
    turned_by(p, angle.cos(), angle.sin())
}

/// A turned point projected into a frame `width` by `height`: the mesh's
/// 140 by 120 units fitted to it, in perspective.
pub fn project(q: Vec3, width: f64, height: f64) -> (f64, f64) {
    let scale = (width / 140.0).min(height / 120.0);
    let s = scale * CAMERA_DISTANCE / (CAMERA_DISTANCE - q[2]);
    (width / 2.0 + q[0] * s, height / 2.0 - q[1] * s)
}

/// The mesh as shared corners and the faces that index them, so a frame
/// turns and projects each corner once rather than once per face that
/// meets there (each of the ring's corners is shared by four).
pub struct IndexedMesh {
    /// Every distinct corner of the mesh.
    pub vertices: Vec<Vec3>,
    /// Face `f`'s corners are `corners[starts[f]..starts[f + 1]]`, indices
    /// into `vertices`, in the face's own order.
    pub starts: Vec<usize>,
    pub corners: Vec<usize>,
}

impl IndexedMesh {
    /// Face `f`'s corners, as indices into `vertices`.
    pub fn face(&self, f: usize) -> &[usize] {
        &self.corners[self.starts[f]..self.starts[f + 1]]
    }
}

fn index(mesh: &[Face]) -> IndexedMesh {
    let mut lookup: std::collections::HashMap<[u64; 3], usize> = std::collections::HashMap::new();
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut starts = vec![0];
    let mut corners: Vec<usize> = Vec::new();
    for face in mesh {
        for &point in &face.points {
            let key = point.map(f64::to_bits);
            let at = *lookup.entry(key).or_insert_with(|| {
                vertices.push(point);
                vertices.len() - 1
            });
            corners.push(at);
        }
        starts.push(corners.len());
    }
    IndexedMesh {
        vertices,
        starts,
        corners,
    }
}

/// The mesh indexed, built once.
pub fn indexed() -> &'static IndexedMesh {
    static INDEXED: OnceLock<IndexedMesh> = OnceLock::new();
    INDEXED.get_or_init(|| index(mesh()))
}

/// The mesh's corners turned by `angle`, and its faces at that angle,
/// sorted far to near, each with its fill. Each corner is turned once,
/// and a face is a handful of numbers, not a list of its turned corners:
/// a frame allocates two arrays, not one per face. The faces, their order
/// and their fills are what turning face by face gave.
fn turn(angle: f64) -> (Vec<Vec3>, Vec<Drawn>) {
    let light = light();
    let mesh = mesh();
    let indexed = indexed();
    let (cy, sy) = (angle.cos(), angle.sin());
    let turned: Vec<Vec3> = indexed
        .vertices
        .iter()
        .map(|&p| turned_by(p, cy, sy))
        .collect();
    let mut drawn: Vec<Drawn> = Vec::with_capacity(mesh.len());
    let mut caps: Vec<usize> = Vec::new();
    let (mut mark_near, mut mark_far) = (f64::NEG_INFINITY, f64::INFINITY);
    for (index, face) in mesh.iter().enumerate() {
        let corners = indexed.face(index);
        if corners.len() < 3 {
            continue;
        }
        let q = |k: usize| turned[corners[k]];
        let mut n = cross(sub(q(1), q(0)), sub(q(2), q(0)));
        let l = length(n);
        n = if l > 0.0 {
            [n[0] / l, n[1] / l, n[2] / l]
        } else {
            [0.0, 0.0, 1.0]
        };
        // Two-sided: light the side that faces the viewer.
        if n[2] < 0.0 {
            n = [-n[0], -n[1], -n[2]];
        }
        let lambert = dot(n, light).max(0.0);
        let spec = (n[2] * 0.5 + lambert * 0.5).max(0.0).powf(18.0) * 0.35;
        let depth = corners.iter().fold(0.0, |sum, &c| sum + turned[c][2]) / corners.len() as f64;
        if face.mark {
            for &c in corners {
                mark_near = mark_near.max(turned[c][2]);
                mark_far = mark_far.min(turned[c][2]);
            }
        }
        if face.mark_cap {
            caps.push(drawn.len());
        }
        let shade = 0.3 + 0.7 * lambert;
        let fill = [
            (face.color[0] * shade + spec).min(1.0),
            (face.color[1] * shade + spec).min(1.0),
            (face.color[2] * shade + spec).min(1.0),
        ];
        drawn.push(Drawn {
            index,
            shade,
            spec,
            depth,
            fill,
        });
    }
    // The mark's face toward the viewer is drawn over its own wall, the
    // other under it.
    let middle = (mark_near + mark_far) / 2.0;
    for cap in caps {
        let face = &mut drawn[cap];
        face.depth = if face.depth > middle {
            mark_near + 0.01
        } else {
            mark_far - 0.01
        };
    }
    // Far to near; the sort is stable, so equal depths keep the mesh's
    // order.
    drawn.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    (turned, drawn)
}

/// The faces at `angle`, sorted far to near, each with its fill.
pub fn frame(angle: f64) -> Vec<Drawn> {
    turn(angle).1
}

/// Draws the station at `angle` into a frame `width` by `height` at the
/// context's origin: each face filled, then stroked in the same colour,
/// which closes the seams between faces. Each corner is projected once.
pub fn draw(cr: &gtk::cairo::Context, width: f64, height: f64, angle: f64) {
    let indexed = indexed();
    let (turned, drawn) = turn(angle);
    let projected: Vec<(f64, f64)> = turned.iter().map(|&q| project(q, width, height)).collect();
    let _ = cr.save();
    cr.set_line_width(0.6);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    for face in drawn {
        cr.new_path();
        let corners = indexed.face(face.index);
        let (x, y) = projected[corners[0]];
        cr.move_to(x, y);
        for &c in &corners[1..] {
            let (x, y) = projected[c];
            cr.line_to(x, y);
        }
        cr.close_path();
        cr.set_source_rgb(face.fill[0], face.fill[1], face.fill[2]);
        let _ = cr.fill_preserve();
        let _ = cr.stroke();
    }
    let _ = cr.restore();
}

/// The ring's outline in a frame `width` by `height`: its outer and inner
/// edges, as a path to fill even-odd. The ring lies in the plane the
/// station turns in, so its outline is the same at every angle of the
/// turn, and the glow drawn from it can be drawn once.
pub fn ring_outline(cr: &gtk::cairo::Context, width: f64, height: f64) {
    let steps = 96;
    for radius in [RING_RADIUS + RING_TUBE, RING_RADIUS - RING_TUBE] {
        for step in 0..=steps {
            let a = f64::from(step) / f64::from(steps) * TAU;
            let (x, y) = project(
                turned([radius * a.cos(), 0.0, radius * a.sin()], 0.0),
                width,
                height,
            );
            if step == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        cr.close_path();
    }
}

/// Room round the station's frame, in points, for its glow to fade out
/// in: past the farthest the glow's blur reaches.
pub const GLOW_MARGIN: f64 = GSK_GLOW_RADIUS * 1.5;

/// The station's glow: a soft brand-blue light round its ring, for a
/// frame `width` by `height` drawn at `scale` device pixels a point, as
/// an image `GLOW_MARGIN` larger on each side. Drawn once per size, scale
/// and strength (`LibraryArt`) and then shown as that image. It used to
/// be a shadow on the turning mesh, blurred again on every frame; it
/// keeps that shadow's colour, strength and radius, cast from the ring
/// alone. `None` for an empty frame.
pub fn glow_texture(width: f64, height: f64, scale: f64, lit: bool) -> Option<gtk::gdk::Texture> {
    use gtk::prelude::*;
    use gtk::{gdk, glib};
    let surface = glow_surface(width, height, scale, lit)?;
    let (w, h, stride) = (surface.width(), surface.height(), surface.stride());
    let mut surface = surface;
    let bytes = glib::Bytes::from(&*surface.data().ok()?);
    // Cairo's ARGB32 is premultiplied B, G, R, A in memory on a
    // little-endian machine, A, R, G, B on a big-endian one.
    let format = if cfg!(target_endian = "little") {
        gdk::MemoryFormat::B8g8r8a8Premultiplied
    } else {
        gdk::MemoryFormat::A8r8g8b8Premultiplied
    };
    Some(gdk::MemoryTexture::new(w, h, format, &bytes, stride as usize).upcast())
}

/// `glow_texture`'s pixels, as a cairo surface.
pub fn glow_surface(
    width: f64,
    height: f64,
    scale: f64,
    lit: bool,
) -> Option<gtk::cairo::ImageSurface> {
    use gtk::gsk::prelude::IsRenderNode;
    use gtk::{cairo, graphene, gsk};
    if !(width > 0.0 && height > 0.0 && scale > 0.0) {
        return None;
    }
    let w = ((width + GLOW_MARGIN * 2.0) * scale).ceil() as i32;
    let h = ((height + GLOW_MARGIN * 2.0) * scale).ceil() as i32;
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        cr.scale(scale, scale);
        cr.translate(GLOW_MARGIN, GLOW_MARGIN);
        // Only the shadow is wanted: the ring is filled far off the image
        // and its shadow cast back onto it.
        let away = (width + GLOW_MARGIN * 2.0) * 4.0;
        let ring = gsk::CairoNode::new(&graphene::Rect::new(
            away as f32,
            0.0,
            width as f32,
            height as f32,
        ));
        {
            let ring_cr = ring.draw_context();
            ring_cr.translate(away, 0.0);
            ring_outline(&ring_cr, width, height);
            ring_cr.set_fill_rule(cairo::FillRule::EvenOdd);
            ring_cr.set_source_rgb(0.0, 0.0, 0.0);
            let _ = ring_cr.fill();
        }
        let blue = gtk::gdk::RGBA::new(
            BRAND_BLUE[0] as f32,
            BRAND_BLUE[1] as f32,
            BRAND_BLUE[2] as f32,
            glow(lit) as f32,
        );
        let shadow = gsk::ShadowNode::new(
            &ring,
            &[gsk::Shadow::new(
                blue,
                -away as f32,
                0.0,
                GSK_GLOW_RADIUS as f32,
            )],
        );
        shadow.upcast().draw(&cr);
    }
    surface.flush();
    Some(surface)
}

/// The station at `angle` over its glow, as a render node over the frame
/// `width` by `height` whose top-left is `origin`. `glow` is
/// `glow_texture`'s image for this frame's size.
pub fn node(
    origin: (f64, f64),
    width: f64,
    height: f64,
    angle: f64,
    glow: Option<&gtk::gdk::Texture>,
) -> gtk::gsk::RenderNode {
    use gtk::gsk::prelude::IsRenderNode;
    use gtk::{graphene, gsk};
    let bounds = graphene::Rect::new(
        origin.0 as f32,
        origin.1 as f32,
        width as f32,
        height as f32,
    );
    let mesh = gsk::CairoNode::new(&bounds);
    {
        let cr = mesh.draw_context();
        cr.translate(origin.0, origin.1);
        draw(&cr, width, height, angle);
    }
    let Some(glow) = glow else {
        return mesh.upcast();
    };
    let glowing = gsk::TextureNode::new(
        glow,
        &graphene::Rect::new(
            (origin.0 - GLOW_MARGIN) as f32,
            (origin.1 - GLOW_MARGIN) as f32,
            (width + GLOW_MARGIN * 2.0) as f32,
            (height + GLOW_MARGIN * 2.0) as f32,
        ),
    );
    gsk::ContainerNode::new(&[glowing.upcast(), mesh.upcast()]).upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Within 1e-6 of a number printed from the Swift.
    fn near(actual: f64, mac: f64, what: &str) {
        assert!(
            (actual - mac).abs() <= 1e-6,
            "{what}: {actual} is not the Mac's {mac}"
        );
    }

    #[test]
    fn the_mesh_has_the_macs_faces() {
        let mesh = mesh();
        // 2 caps, 100 walls, 320 ring faces, 4 spokes, 4 modules and 6
        // solar boxes of 6 faces each.
        assert_eq!(mesh.len(), 506);
        let corners: usize = mesh.iter().map(|f| f.points.len()).sum();
        assert_eq!(corners, 2216);
        assert_eq!(mesh.iter().filter(|f| f.mark_cap).count(), 2);
        assert_eq!(mesh.iter().filter(|f| f.mark).count(), 102);
        let mut sum = [0.0; 3];
        for face in mesh {
            for p in &face.points {
                for axis in 0..3 {
                    sum[axis] += p[axis];
                }
            }
        }
        near(sum[0], -0.170982910, "corner sum x");
        near(sum[1], 0.0, "corner sum y");
        near(sum[2], -0.110923874, "corner sum z");
        let colours = [
            (0, [0.0, 0.568627451, 0.992156863]),
            (2, [0.0, 0.384313725, 0.690196078]),
            (102, [0.674509804, 0.768627451, 0.901960784]),
            (422, [0.549019608, 0.643137255, 0.784313725]),
            (446, [0.886274510, 0.925490196, 0.972549020]),
            (505, [0.470588235, 0.525490196, 0.611764706]),
        ];
        for (index, colour) in colours {
            for axis in 0..3 {
                near(mesh[index].color[axis], colour[axis], "face colour");
            }
        }
    }

    #[test]
    fn the_marks_outline_is_the_macs() {
        let outline = mark_outline(30.0);
        assert_eq!(outline.len(), 100);
        near(outline[0][0], 11.924650440186, "outline[0].x");
        near(outline[0][1], 14.999262040394, "outline[0].y");
        near(outline[7][0], 2.747423096841, "outline[7].x");
        near(outline[7][1], 3.971287415847, "outline[7].y");
        near(outline[99][0], 12.944872087002, "outline last x");
        near(outline[99][1], 14.833935784568, "outline last y");
    }

    #[test]
    fn turned_and_projected_points_are_the_macs() {
        // (angle, face, turned first corner, projected in 150 by 130)
        let cases = [
            (
                0.0,
                0,
                [14.573783686548, 9.160930146686, -9.005406030950],
                (90.050052271055, 55.539692332173),
            ),
            (
                0.6,
                400,
                [22.495716659579, 18.940667458113, -21.074957846058],
                (97.156904296930, 46.344601617273),
            ),
            (
                1.7,
                505,
                [-12.130729895652, -36.907308404617, 42.209489200004],
                (59.229119324383, 112.982335928200),
            ),
            (
                3.9,
                100,
                [-2.799823972395, -10.609980126160, 16.968139943983],
                (71.771964617173, 77.232694482235),
            ),
        ];
        for (angle, face, mac, (px, py)) in cases {
            let q = turned(mesh()[face].points[0], angle);
            for axis in 0..3 {
                near(q[axis], mac[axis], "turned corner");
            }
            let (x, y) = project(q, 150.0, 130.0);
            near(x, px, "projected x");
            near(y, py, "projected y");
        }
    }

    #[test]
    fn shading_depth_and_order_are_the_macs() {
        // (angle, face, shade, spec, depth after the caps' override, fill)
        let cases = [
            (
                0.0,
                0,
                0.870254007096,
                0.001622464025,
                13.298005472781,
                [0.001622464025, 0.496472781786, 0.865050949497],
            ),
            (
                0.0,
                1,
                0.870254007096,
                0.001622464025,
                -13.297841911646,
                [0.001622464025, 0.496472781786, 0.865050949497],
            ),
            (
                0.6,
                102,
                0.446033736425,
                0.0,
                -13.568306655435,
                [0.300854128099, 0.342833773880, 0.402304938737],
            ),
            (
                1.7,
                446,
                0.701795195922,
                0.000000000981,
                -21.484349367053,
                [0.621983194230, 0.649504574462, 0.682530230741],
            ),
            (
                3.9,
                505,
                0.659018469122,
                0.000002045865,
                -28.852151728869,
                [0.310128384275, 0.346309790423, 0.403166285798],
            ),
        ];
        for (angle, index, shade, spec, depth, fill) in cases {
            let drawn = frame(angle);
            let face = drawn
                .iter()
                .find(|d| d.index == index)
                .unwrap_or_else(|| panic!("face {index} is drawn at {angle}"));
            near(face.shade, shade, "shade");
            near(face.spec, spec, "spec");
            near(face.depth, depth, "depth");
            for axis in 0..3 {
                near(face.fill[axis], fill[axis], "fill");
            }
        }
        let order = |angle: f64| -> (Vec<usize>, Vec<usize>) {
            let drawn: Vec<usize> = frame(angle).iter().map(|d| d.index).collect();
            (drawn[..6].to_vec(), drawn[drawn.len() - 6..].to_vec())
        };
        assert_eq!(
            order(0.6),
            (
                vec![373, 483, 372, 381, 477, 380],
                vec![215, 494, 214, 207, 500, 206]
            )
        );
        assert_eq!(
            order(3.9),
            (
                vec![500, 494, 499, 221, 229, 220],
                vec![375, 382, 374, 480, 477, 483]
            )
        );
    }

    #[test]
    fn it_turns_once_in_twenty_six_seconds_and_rests_without_animations() {
        assert_eq!(TURN_SECONDS, 26.0);
        assert_eq!(REST_ANGLE, 0.6);
        assert_eq!(FRAME_INTERVAL, 1.0 / 15.0);
        near(angle(13.0, false), 3.741592653589793, "half a turn");
        near(angle(30.0, false), 1.5666438934122442, "past one turn");
        assert_eq!(angle(1234.5, true), REST_ANGLE);
        assert_eq!(angle(0.0, false), REST_ANGLE);
    }

    #[test]
    fn the_glow_is_softer_before_anything_is_contributed() {
        assert_eq!(glow(true), 0.35);
        assert_eq!(glow(false), 0.25);
        assert_eq!(GLOW_RADIUS, 6.0);
        let light = light();
        near(light[0], -0.4207580473429521, "light x");
        near(light[1], 0.6211190222681674, "light y");
        near(light[2], 0.6611912172532104, "light z");
    }

    /// The station as it was drawn before its corners were indexed: each
    /// face's corners turned and projected on their own, one list per
    /// face, sorted as structs. Kept as the oracle the indexed frame must
    /// match.
    fn face_by_face(angle: f64) -> Vec<(Drawn, Vec<Vec3>)> {
        let light = light();
        let (cy, sy) = (angle.cos(), angle.sin());
        let mut drawn: Vec<(Drawn, Vec<Vec3>)> = Vec::new();
        let mut caps: Vec<usize> = Vec::new();
        let (mut mark_near, mut mark_far) = (f64::NEG_INFINITY, f64::INFINITY);
        for (index, face) in mesh().iter().enumerate() {
            let q: Vec<Vec3> = face.points.iter().map(|&p| turned_by(p, cy, sy)).collect();
            if q.len() < 3 {
                continue;
            }
            let mut n = cross(sub(q[1], q[0]), sub(q[2], q[0]));
            let l = length(n);
            n = if l > 0.0 {
                [n[0] / l, n[1] / l, n[2] / l]
            } else {
                [0.0, 0.0, 1.0]
            };
            if n[2] < 0.0 {
                n = [-n[0], -n[1], -n[2]];
            }
            let lambert = dot(n, light).max(0.0);
            let spec = (n[2] * 0.5 + lambert * 0.5).max(0.0).powf(18.0) * 0.35;
            let depth = q.iter().fold(0.0, |sum, p| sum + p[2]) / q.len() as f64;
            if face.mark {
                for p in &q {
                    mark_near = mark_near.max(p[2]);
                    mark_far = mark_far.min(p[2]);
                }
            }
            if face.mark_cap {
                caps.push(drawn.len());
            }
            let shade = 0.3 + 0.7 * lambert;
            let fill = [
                (face.color[0] * shade + spec).min(1.0),
                (face.color[1] * shade + spec).min(1.0),
                (face.color[2] * shade + spec).min(1.0),
            ];
            drawn.push((
                Drawn {
                    index,
                    shade,
                    spec,
                    depth,
                    fill,
                },
                q,
            ));
        }
        let middle = (mark_near + mark_far) / 2.0;
        for cap in caps {
            let face = &mut drawn[cap].0;
            face.depth = if face.depth > middle {
                mark_near + 0.01
            } else {
                mark_far - 0.01
            };
        }
        drawn.sort_by(|a, b| a.0.depth.total_cmp(&b.0.depth));
        drawn
    }

    fn draw_face_by_face(cr: &gtk::cairo::Context, width: f64, height: f64, angle: f64) {
        let _ = cr.save();
        cr.set_line_width(0.6);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        for (face, points) in face_by_face(angle) {
            cr.new_path();
            for (index, &point) in points.iter().enumerate() {
                let (x, y) = project(point, width, height);
                if index == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            cr.close_path();
            cr.set_source_rgb(face.fill[0], face.fill[1], face.fill[2]);
            let _ = cr.fill_preserve();
            let _ = cr.stroke();
        }
        let _ = cr.restore();
    }

    /// Raw pixels of `paint` into a `w` by `h` surface.
    fn pixels(w: i32, h: i32, paint: impl Fn(&gtk::cairo::Context)) -> (usize, Vec<u8>) {
        use gtk::cairo;
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            paint(&cr);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        (stride, surface.data().unwrap().to_vec())
    }

    /// The indexed mesh a frame draws from is the mesh itself: every face
    /// gets back its own corners in its own order, and the corners faces
    /// share are stored once.
    #[test]
    fn the_indexed_mesh_is_the_mesh() {
        let mesh = mesh();
        let indexed = indexed();
        assert_eq!(indexed.starts.len(), mesh.len() + 1);
        for (f, face) in mesh.iter().enumerate() {
            let corners: Vec<Vec3> = indexed
                .face(f)
                .iter()
                .map(|&c| indexed.vertices[c])
                .collect();
            assert_eq!(corners, face.points, "face {f} changed");
        }
        let all: usize = mesh.iter().map(|f| f.points.len()).sum();
        assert!(
            indexed.vertices.len() < all / 2,
            "{} distinct corners of {all}",
            indexed.vertices.len()
        );
    }

    /// Turning each corner once gives the same faces, in the same order,
    /// with the same depths and fills, and the same pixels, as turning
    /// each face's corners on their own.
    #[test]
    fn a_frame_is_the_same_as_drawing_face_by_face() {
        for step in 0..48 {
            let angle = REST_ANGLE + f64::from(step) / 48.0 * TAU;
            let expected: Vec<Drawn> = face_by_face(angle).into_iter().map(|(d, _)| d).collect();
            assert_eq!(frame(angle), expected, "the frame at {angle} changed");
        }
        for angle in [0.0, REST_ANGLE, 1.7, 3.9, 5.2] {
            let paint = |indexed: bool| {
                pixels(300, 260, move |cr| {
                    cr.scale(2.0, 2.0);
                    if indexed {
                        draw(cr, 150.0, 130.0, angle);
                    } else {
                        draw_face_by_face(cr, 150.0, 130.0, angle);
                    }
                })
            };
            assert!(
                paint(true) == paint(false),
                "the station's pixels at {angle} changed"
            );
        }
    }

    /// The ring lies in the plane the station turns in, so every turn of
    /// it projects onto the same outline: what lets its glow be drawn once.
    #[test]
    fn the_rings_outline_is_the_same_at_every_angle() {
        let (w, h) = (150.0, 130.0);
        let mut a = 0.0;
        while a < TAU {
            for radius in [RING_RADIUS + RING_TUBE, RING_RADIUS - RING_TUBE] {
                let p = [radius * f64::cos(a), 0.0, radius * f64::sin(a)];
                let (x, y) = project(turned(p, 1.3), w, h);
                let on_ring = (0..720).any(|step| {
                    let b = f64::from(step) / 720.0 * TAU;
                    let (qx, qy) =
                        project(turned([radius * b.cos(), 0.0, radius * b.sin()], 0.0), w, h);
                    (qx - x).hypot(qy - y) < 0.5
                });
                assert!(on_ring, "a turned point of the ring left its outline");
            }
            a += 0.37;
        }
    }

    /// The glow is an image with room for its blur: lit round the ring,
    /// clear over the mark at the centre and at its own corners.
    #[test]
    fn the_glow_is_drawn_round_the_ring() {
        let (w, h, scale) = (150.0, 130.0, 2.0);
        let mut surface = glow_surface(w, h, scale, true).unwrap();
        assert_eq!(surface.width(), ((w + GLOW_MARGIN * 2.0) * scale) as i32);
        assert_eq!(surface.height(), ((h + GLOW_MARGIN * 2.0) * scale) as i32);
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap().to_vec();
        let alpha = |x: f64, y: f64| {
            let (px, py) = (
                ((x + GLOW_MARGIN) * scale) as usize,
                ((y + GLOW_MARGIN) * scale) as usize,
            );
            data[py * stride + px * 4 + 3]
        };
        // The ring's outer edge at the top of its outline.
        let (_, top) = project(turned([0.0, 0.0, -(RING_RADIUS + RING_TUBE)], 0.0), w, h);
        assert!(alpha(w / 2.0, top - 2.0) > 0, "no glow past the ring");
        assert_eq!(alpha(w / 2.0, h / 2.0), 0, "the glow covers the mark");
        assert_eq!(
            alpha(-GLOW_MARGIN + 0.5, -GLOW_MARGIN + 0.5),
            0,
            "the glow runs off the image"
        );
        // The arrays reach the frame's sides; the glow does not follow them.
        assert_eq!(alpha(4.0, h / 2.0), 0, "the glow follows the arrays");
        assert!(glow_surface(0.0, 130.0, 2.0, true).is_none());
        assert!(glow_surface(150.0, 130.0, 0.0, true).is_none());
    }

    /// The glow keeps the old shadow's colour, strength and radius, cast
    /// from the ring alone: outside the ring it is, pixel for pixel, the
    /// GSK shadow the station used to carry, put on the ring's outline;
    /// and it is blue.
    #[test]
    fn the_glow_is_the_old_shadow_cast_from_the_ring() {
        use gtk::gsk::prelude::IsRenderNode;
        use gtk::{cairo, graphene, gsk};
        let (w, h) = (150.0, 130.0);
        let m = GLOW_MARGIN;
        let (sw, sh) = ((w + m * 2.0) as i32, (h + m * 2.0) as i32);
        for lit in [true, false] {
            let ring = || {
                let node = gsk::CairoNode::new(&graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
                let cr = node.draw_context();
                ring_outline(&cr, w, h);
                cr.set_fill_rule(cairo::FillRule::EvenOdd);
                cr.set_source_rgb(0.0, 0.0, 0.0);
                let _ = cr.fill();
                drop(cr);
                node
            };
            // The old shadow, as the station carried it, on the ring.
            let (stride, old) = pixels(sw, sh, |cr| {
                cr.translate(m, m);
                let blue = gtk::gdk::RGBA::new(
                    BRAND_BLUE[0] as f32,
                    BRAND_BLUE[1] as f32,
                    BRAND_BLUE[2] as f32,
                    glow(lit) as f32,
                );
                gsk::ShadowNode::new(
                    ring(),
                    &[gsk::Shadow::new(blue, 0.0, 0.0, GSK_GLOW_RADIUS as f32)],
                )
                .upcast()
                .draw(cr);
            });
            // The ring itself, to leave out.
            let (_, covered) = pixels(sw, sh, |cr| {
                cr.translate(m, m);
                ring().upcast().draw(cr);
            });
            let mut glow_image = glow_surface(w, h, 1.0, lit).unwrap();
            let new = glow_image.data().unwrap().to_vec();
            let (mut compared, mut lit_pixels, mut worst) = (0, 0, 0);
            for y in 0..sh as usize {
                for x in 0..sw as usize {
                    let i = y * stride + x * 4;
                    if covered[i + 3] > 0 {
                        continue;
                    }
                    compared += 1;
                    worst = worst.max((i32::from(old[i + 3]) - i32::from(new[i + 3])).abs());
                    if new[i + 3] > 0 {
                        lit_pixels += 1;
                        // The brand blue, premultiplied B, G, R: no red.
                        assert!(
                            new[i] >= new[i + 1] && new[i + 2] == 0,
                            "the glow is not blue"
                        );
                    }
                }
            }
            assert!(
                compared > 10_000 && lit_pixels > 2_000,
                "{compared} {lit_pixels}"
            );
            assert!(
                worst <= 1,
                "the glow is {worst}/255 from the old shadow (lit {lit})"
            );
        }
    }

    #[test]
    fn the_glow_spreads_past_the_mesh_in_the_brand_blue() {
        let (w, h) = (150.0, 130.0);
        let m = GLOW_MARGIN;
        let (sw, sh) = ((w + m * 2.0) as i32, (h + m * 2.0) as i32);
        let render = |lit: bool| {
            let texture = glow_texture(w, h, 1.0, lit).unwrap();
            pixels(sw, sh, |cr| {
                node((m, m), w, h, REST_ANGLE, Some(&texture)).draw(cr);
            })
        };
        let (stride, lit) = render(true);
        let (_, dim) = render(false);
        // Round the mesh the frame is only glow: blue, and stronger when
        // lit.
        let mut glow_lit = 0u32;
        let mut glow_dim = 0u32;
        for y in 0..sh as usize {
            for x in 0..sw as usize {
                let i = y * stride + x * 4;
                // Partly covered pixels: the glow, and the mesh's own
                // anti-aliased rim, which is blue-grey too.
                if lit[i + 3] > 0 && lit[i + 3] < 120 {
                    glow_lit += u32::from(lit[i + 3]);
                    glow_dim += u32::from(dim[i + 3]);
                    assert!(lit[i] >= lit[i + 2], "the glow is blue at ({x}, {y})");
                }
            }
        }
        assert!(glow_lit > 0, "the station has a glow");
        assert!(
            glow_dim < glow_lit,
            "the glow is softer before anything is contributed"
        );
        // Without its image the station is the mesh alone.
        let (_, bare) = pixels(sw, sh, |cr| node((m, m), w, h, REST_ANGLE, None).draw(cr));
        let (_, mesh) = pixels(sw, sh, |cr| {
            cr.translate(m, m);
            draw(cr, w, h, REST_ANGLE);
        });
        assert!(bare == mesh, "the station without a glow is its mesh");
    }

    #[test]
    fn the_station_paints_in_its_frame() {
        use gtk::cairo;
        let (w, h) = (150, 130);
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            draw(&cr, f64::from(w), f64::from(h), REST_ANGLE);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let painted = (0..h as usize)
            .flat_map(|y| (0..w as usize).map(move |x| (x, y)))
            .filter(|&(x, y)| data[y * stride + x * 4 + 3] > 128)
            .count();
        // The ring, the arrays and the mark cover a good part of the frame,
        // and its centre is the mark's blue.
        assert!(painted > 3000, "the station painted {painted} pixels");
        let centre = (h as usize / 2) * stride + (w as usize / 2) * 4;
        let (b, g, r) = (data[centre], data[centre + 1], data[centre + 2]);
        assert!(
            b > r + 60 && g > r,
            "the centre is blue, not ({r}, {g}, {b})"
        );
    }

    /// What one frame of the station costs on the CPU, as the cairo
    /// renderer (software rendering) pays it: the mesh alone, and the
    /// station's whole node, glow and all, at twice the size. Prints
    /// microseconds a frame over a full turn.
    #[test]
    #[ignore = "measurement; run with --ignored --nocapture"]
    fn measure_station_cost() {
        use gtk::cairo;
        let (w, h) = (340, 300);
        let frames = 390;
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
        let glow = glow_texture(150.0, 130.0, 2.0, true);
        let time = |surface: &mut cairo::ImageSurface, whole: bool| {
            let start = std::time::Instant::now();
            for i in 0..frames {
                let angle = REST_ANGLE + f64::from(i) / f64::from(frames) * TAU;
                let cr = cairo::Context::new(&*surface).unwrap();
                cr.set_operator(cairo::Operator::Clear);
                let _ = cr.paint();
                cr.set_operator(cairo::Operator::Over);
                cr.scale(2.0, 2.0);
                if whole {
                    node((10.0, 10.0), 150.0, 130.0, angle, glow.as_ref()).draw(&cr);
                } else {
                    cr.translate(10.0, 10.0);
                    draw(&cr, 150.0, 130.0, angle);
                }
            }
            start.elapsed().as_secs_f64() * 1e6 / f64::from(frames)
        };
        // Warm up, then measure.
        time(&mut surface, true);
        let mesh = time(&mut surface, false);
        let whole = time(&mut surface, true);
        let start = std::time::Instant::now();
        for i in 0..frames {
            std::hint::black_box(frame(f64::from(i) * 0.01));
        }
        let geometry = start.elapsed().as_secs_f64() * 1e6 / f64::from(frames);
        eprintln!(
            "station cost: geometry {geometry:.1} us, mesh {mesh:.1} us, whole node {whole:.1} us a frame"
        );
    }

    /// Renders the station as the Mac's comparison renders are made: a
    /// 150 by 130 frame centred in 170 by 150, at twice the size, on the
    /// pane's ground in each scheme, at four angles, lit and not. Writes
    /// raw premultiplied BGRA (`<name>.bgra`, width and height in the name)
    /// into `TC_STATION_RENDER_DIR`, which a script turns into PNG.
    #[test]
    #[ignore = "writes comparison renders; set TC_STATION_RENDER_DIR"]
    fn render_for_comparison() {
        use gtk::cairo;
        let Ok(dir) = std::env::var("TC_STATION_RENDER_DIR") else {
            return;
        };
        let grounds = [("light", [0xf2, 0xf3, 0xf6]), ("dark", [0x0d, 0x11, 0x24])];
        for (scheme, ground) in grounds {
            for angle in [0.6, 1.7, 3.9, 5.2] {
                for lit in [true, false] {
                    let (w, h) = (340, 300);
                    let mut surface =
                        cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
                    {
                        let cr = cairo::Context::new(&surface).unwrap();
                        cr.set_source_rgb(
                            f64::from(ground[0]) / 255.0,
                            f64::from(ground[1]) / 255.0,
                            f64::from(ground[2]) / 255.0,
                        );
                        let _ = cr.paint();
                        cr.scale(2.0, 2.0);
                        let glow = glow_texture(150.0, 130.0, 2.0, lit);
                        node((10.0, 10.0), 150.0, 130.0, angle, glow.as_ref()).draw(&cr);
                    }
                    surface.flush();
                    let name = format!(
                        "{dir}/station-{scheme}-{angle}-{}-{w}x{h}.bgra",
                        if lit { "lit" } else { "unlit" }
                    );
                    std::fs::write(name, &*surface.data().unwrap()).unwrap();
                }
            }
        }
    }
}
