//! The NEAR AI "N", for the Linux app icon and the Windows MSIX tiles.
//!
//! The Linux desktop shows the NEAR AI mark, not "The Turn" (owner,
//! 2026-11-18). Its path data is the brand kit's, as the Mac carries it in
//! `macos/Sources/TCDesign/Logos/GlassBrandMark.swift`; the test below reads
//! that file, so the two cannot drift apart.
//!
//! The N is NEAR AI's mark. It is used with the owner's approval to use the
//! logos, and the path data comes from the brand kit's
//! `Tertiary/02_White & Color/tile-white-color.svg`. It is a trademark and is
//! not covered by this crate's code licence.
//!
//! The app icon is built as the Mac's is (`macos/icons/README.md`): the
//! kit's tile, a `brand-tile` #0B8FF6 ground with the N in `brand-paper`
//! #F3F3F2 at the kit's own proportion, 58.5% of the tile's width, centred.
//! Neither ink is recoloured. A freedesktop icon is one file for light and
//! dark desktops alike, so it keeps the coloured ground rather than a bare
//! single-ink N that would vanish on one of them.
//!
//! The Mac's icon is a full-bleed square the system masks. No Linux desktop
//! masks an icon, so the tile carries its own rounded corners: the radius
//! macOS masks app icons with, 22.5% of the side.

/// The N's own bounds inside the kit's tile: x, y, width, height.
pub const VIEW_BOX: [f64; 4] = [82.96, 82.13, 234.08, 231.72];

/// The kit's path, spaces written as commas, exactly as `GlassBrandMark.data`.
pub const PATH: &str = "M292.106,82.1357C283.445,82.1357,275.379,86.5848,270.847,93.9047L221.912,165.811C220.318,168.188,220.971,171.382,223.353,172.96C225.292,174.234,227.866,174.082,229.633,172.58L277.799,131.227C278.605,130.504,279.834,130.581,280.545,131.379C280.872,131.74,281.044,132.216,281.044,132.691V262.169C281.044,263.233,280.18,264.089,279.085,264.089C278.509,264.089,277.952,263.842,277.587,263.404L132.014,90.8817C127.271,85.3489,120.318,82.1548,112.982,82.1357H107.893C94.1229,82.1357,82.9648,93.1822,82.9648,106.814V289.167C82.9648,302.799,94.1229,313.846,107.893,313.846C116.554,313.846,124.62,309.397,129.153,302.077L178.087,230.17C179.681,227.793,179.028,224.599,176.646,223.021C174.707,221.747,172.133,221.899,170.366,223.401L122.2,264.754C121.394,265.477,120.165,265.401,119.454,264.602C119.128,264.241,118.955,263.766,118.955,263.29V133.775C118.955,132.71,119.819,131.854,120.914,131.854C121.49,131.854,122.047,132.102,122.412,132.539L267.966,305.1C272.709,310.632,279.662,313.827,286.998,313.846H292.087C305.857,313.846,317.034,302.799,317.034,289.167V106.833C317.034,93.2012,305.876,82.1548,292.106,82.1548V82.1357Z";

/// The app icon's ground, `brand-tile`, as the Mac's `AppIcon.icon` fills it.
pub const TILE: &str = "#0B8FF6";

/// The ground in the Mac icon's dark appearance: the NEAR blue at the brand
/// hue, stepped down. The window's own drawing of the tile follows the
/// desktop's dark style with it; the installed icon is one file and stays
/// on [`TILE`].
pub const TILE_DARK: &str = "#024175";

/// The N on the tile, `brand-paper`.
pub const PAPER: &str = "#F3F3F2";

/// The kit's proportion: the N is this share of the tile's width.
pub const N_SHARE: f64 = 0.585;

/// The tile's corner radius as a share of its side.
pub const CORNER_SHARE: f64 = 0.225;

/// The tile's side in the document's own units, the kit's 400.
pub const SIDE: f64 = 400.0;

/// Where the N sits on a tile [`SIDE`] across: the scale from the path's
/// units, then the x and y offsets, so it spans [`N_SHARE`] of the width,
/// centred. The SVG below and the window's cairo drawing both use this.
pub fn n_on_tile() -> (f64, f64, f64) {
    let [bx, by, bw, bh] = VIEW_BOX;
    let scale = SIDE * N_SHARE / bw;
    let tx = (SIDE - bw * scale) / 2.0 - bx * scale;
    let ty = (SIDE - bh * scale) / 2.0 - by * scale;
    (scale, tx, ty)
}

/// The app icon: the N in paper on the brand tile, `size` pixels across.
pub fn app_icon_svg(size: u32) -> String {
    let (scale, tx, ty) = n_on_tile();
    let r = SIDE * CORNER_SHARE;
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 400 400">"#,
            r#"<rect width="400" height="400" rx="{r}" fill="{tile}"/>"#,
            r#"<path transform="matrix({s} 0 0 {s} {tx} {ty})" d="{path}" fill="{paper}"/>"#,
            r#"</svg>"#,
        ),
        size = size,
        r = fmt(r),
        tile = TILE,
        s = fmt(scale),
        tx = fmt(tx),
        ty = fmt(ty),
        path = PATH,
        paper = PAPER,
    )
}

/// The N alone in one ink, `size` pixels across, for a surface that wants
/// the mark without a tile: the logo kit's black on light, white on dark.
pub fn template_svg(ink: &str, size: u32) -> String {
    let [bx, by, bw, bh] = VIEW_BOX;
    let side = bw.max(bh);
    let x = bx - (side - bw) / 2.0;
    let y = by - (side - bh) / 2.0;
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" "#,
            r#"viewBox="{x} {y} {side} {side}"><path d="{path}" fill="{ink}"/></svg>"#,
        ),
        size = size,
        x = fmt(x),
        y = fmt(y),
        side = fmt(side),
        path = PATH,
        ink = ink,
    )
}

/// A point in the tile's own units.
type Pt = (f64, f64);

/// Line segments per cubic when the N is flattened. Fixed, so the polygon
/// and therefore every pixel is the same on every runner.
const CUBIC_STEPS: u32 = 24;

/// Sub-scanlines per pixel row. Horizontal coverage along each one is exact
/// (a span's overlap with the pixel); vertical coverage is this many samples.
const SUB_ROWS: u32 = 16;

/// The quarter-circle's Bezier handle: 4/3 (sqrt 2 - 1), written out, so no
/// transcendental is called and the corners are the same everywhere.
const KAPPA: f64 = 0.552_284_749_830_793_4;

/// Parses [`PATH`]'s absolute `M L H V C Z` commands into closed polygons,
/// cubics flattened at [`CUBIC_STEPS`]. Only `+`, `-`, `*` and `/` are used.
fn path_polygons(data: &str) -> Vec<Vec<Pt>> {
    let mut polys: Vec<Vec<Pt>> = Vec::new();
    let mut current: Vec<Pt> = Vec::new();
    let bytes = data.as_bytes();
    let mut i = 0usize;
    let mut cmd = b'M';
    let mut at: Pt = (0.0, 0.0);
    let number = |i: &mut usize| -> f64 {
        while *i < bytes.len() && (bytes[*i] == b',' || bytes[*i] == b' ') {
            *i += 1;
        }
        let start = *i;
        while *i < bytes.len()
            && (bytes[*i].is_ascii_digit() || bytes[*i] == b'.' || bytes[*i] == b'-')
        {
            *i += 1;
        }
        data[start..*i].parse().expect("path number")
    };
    loop {
        while i < bytes.len() && (bytes[i] == b',' || bytes[i] == b' ') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i].is_ascii_alphabetic() {
            cmd = bytes[i];
            i += 1;
        }
        match cmd {
            b'M' => {
                if current.len() > 2 {
                    polys.push(std::mem::take(&mut current));
                }
                current.clear();
                at = (number(&mut i), number(&mut i));
                current.push(at);
                cmd = b'L';
            }
            b'L' => {
                at = (number(&mut i), number(&mut i));
                current.push(at);
            }
            b'H' => {
                at = (number(&mut i), at.1);
                current.push(at);
            }
            b'V' => {
                at = (at.0, number(&mut i));
                current.push(at);
            }
            b'C' => {
                let c1 = (number(&mut i), number(&mut i));
                let c2 = (number(&mut i), number(&mut i));
                let end = (number(&mut i), number(&mut i));
                flatten_cubic(&mut current, at, c1, c2, end);
                at = end;
            }
            b'Z' => {
                if current.len() > 2 {
                    polys.push(std::mem::take(&mut current));
                }
            }
            other => panic!("unsupported path command {}", other as char),
        }
    }
    if current.len() > 2 {
        polys.push(current);
    }
    polys
}

fn flatten_cubic(out: &mut Vec<Pt>, p0: Pt, p1: Pt, p2: Pt, p3: Pt) {
    for k in 1..=CUBIC_STEPS {
        let t = k as f64 / CUBIC_STEPS as f64;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push((
            a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
            a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
        ));
    }
}

/// The tile's rounded square, [`SIDE`] across with corners of
/// [`CORNER_SHARE`], each corner a quarter circle drawn as one cubic.
fn tile_polygon() -> Vec<Pt> {
    let r = SIDE * CORNER_SHARE;
    let k = r * KAPPA;
    let mut out = vec![(r, 0.0)];
    let mut at = (SIDE - r, 0.0);
    out.push(at);
    for (c1, c2, end, next) in [
        (
            (SIDE - r + k, 0.0),
            (SIDE, r - k),
            (SIDE, r),
            (SIDE, SIDE - r),
        ),
        (
            (SIDE, SIDE - r + k),
            (SIDE - r + k, SIDE),
            (SIDE - r, SIDE),
            (r, SIDE),
        ),
        (
            (r - k, SIDE),
            (0.0, SIDE - r + k),
            (0.0, SIDE - r),
            (0.0, r),
        ),
        ((0.0, r - k), (r - k, 0.0), (r, 0.0), (r, 0.0)),
    ] {
        flatten_cubic(&mut out, at, c1, c2, end);
        out.push(next);
        at = next;
    }
    out
}

/// Each pixel's coverage by `polys` (nonzero winding), `size` pixels across,
/// with `scale` and the offsets taking the polygons' units to pixels.
fn coverage(polys: &[Vec<Pt>], size: u32, scale: f64, tx: f64, ty: f64) -> Vec<f64> {
    let n = size as usize;
    let mut cover = vec![0.0f64; n * n];
    let edges: Vec<(Pt, Pt)> = polys
        .iter()
        .flat_map(|poly| {
            (0..poly.len()).map(move |i| {
                let a = poly[i];
                let b = poly[(i + 1) % poly.len()];
                (
                    (a.0 * scale + tx, a.1 * scale + ty),
                    (b.0 * scale + tx, b.1 * scale + ty),
                )
            })
        })
        .collect();
    let weight = 1.0 / SUB_ROWS as f64;
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    for py in 0..n {
        for sub in 0..SUB_ROWS {
            let y = py as f64 + (sub as f64 + 0.5) / SUB_ROWS as f64;
            crossings.clear();
            for &((x0, y0), (x1, y1)) in &edges {
                let (dir, lo, hi) = if y0 < y1 { (1, y0, y1) } else { (-1, y1, y0) };
                if y0 == y1 || y < lo || y >= hi {
                    continue;
                }
                let x = x0 + (y - y0) * (x1 - x0) / (y1 - y0);
                crossings.push((x, dir));
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            // A span runs from where the winding leaves zero to where it
            // returns to it (nonzero fill).
            let mut open: Option<f64> = None;
            let mut wind = 0;
            for &(x, dir) in crossings.iter() {
                let before = wind;
                wind += dir;
                if before == 0 && wind != 0 {
                    open = Some(x);
                } else if before != 0 && wind == 0 {
                    if let Some(xa) = open.take() {
                        add_span(&mut cover[py * n..(py + 1) * n], xa, x, weight);
                    }
                }
            }
        }
    }
    for c in &mut cover {
        *c = c.clamp(0.0, 1.0);
    }
    cover
}

/// Adds `weight` times each pixel's overlap with the span `[xa, xb)` to `row`.
fn add_span(row: &mut [f64], xa: f64, xb: f64, weight: f64) {
    let n = row.len() as f64;
    let (xa, xb) = (xa.max(0.0), xb.min(n));
    if xb <= xa {
        return;
    }
    let first = xa as usize;
    let last = (xb as usize).min(row.len() - 1);
    for (px, cell) in row.iter_mut().enumerate().take(last + 1).skip(first) {
        let p = px as f64;
        let overlap = xb.min(p + 1.0) - xa.max(p);
        if overlap > 0.0 {
            *cell += overlap * weight;
        }
    }
}

/// The app icon as straight RGBA, `size` pixels square: the brand tile with
/// rounded corners in [`TILE`], the N in [`PAPER`] at the kit's proportion,
/// over transparency. The same tile [`app_icon_svg`] describes, so the
/// Windows tiles and the Linux icon are one drawing (owner decision G-3 (a),
/// 2026-11-18: "the NEAR AI N everywhere ... and the MSIX tiles").
pub fn render_tile(size: u32) -> Vec<u8> {
    let px = size as f64 / SIDE;
    let ground = coverage(&[tile_polygon()], size, px, 0.0, 0.0);
    let (scale, tx, ty) = n_on_tile();
    let mark = coverage(&path_polygons(PATH), size, scale * px, tx * px, ty * px);
    let tile = crate::raster::parse_hex(TILE);
    let paper = crate::raster::parse_hex(PAPER);
    let mut out = Vec::with_capacity(ground.len() * 4);
    for (g, m) in ground.iter().zip(mark.iter()) {
        let mut pixel = crate::raster::over([0, 0, 0, 0], tile, *g);
        // Straight alpha: the colour of a partly covered ground pixel is the
        // tile's, not the tile's darkened toward transparent black.
        if *g > 0.0 {
            pixel = [tile[0], tile[1], tile[2], pixel[3]];
        }
        let inked = crate::raster::over(pixel, paper, *m);
        out.extend_from_slice(&[inked[0], inked[1], inked[2], pixel[3]]);
    }
    out
}

/// [`render_tile`] as a PNG.
pub fn tile_png(size: u32) -> Vec<u8> {
    crate::raster::encode_png(&render_tile(size), size)
}

/// Four decimals at most, trailing zeros dropped: the same bytes on every
/// runner, so the drift check can compare them.
fn fmt(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swift() -> String {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../macos/Sources/TCDesign/Logos/GlassBrandMark.swift"
        );
        std::fs::read_to_string(path).expect("GlassBrandMark.swift")
    }

    /// The path and its bounds are the Mac's, character for character.
    #[test]
    fn the_n_is_the_macs_path() {
        let source = swift();
        assert!(source.contains(&format!("\"{PATH}\"")), "path differs");
        assert!(
            source.contains("CGRect(x: 82.96, y: 82.13, width: 234.08, height: 231.72)"),
            "viewBox differs"
        );
        assert_eq!(VIEW_BOX, [82.96, 82.13, 234.08, 231.72]);
    }

    /// The tile's inks and proportion are the Mac app icon's.
    #[test]
    fn the_app_icon_is_the_macs_tile() {
        let icon = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../macos/icons/production/AppIcon.icon"
        );
        let json = std::fs::read_to_string(format!("{icon}/icon.json")).expect("icon.json");
        // 0x0B/255, 0x8F/255, 0xF6/255, as Icon Composer writes the fill.
        assert!(
            json.contains("srgb:0.04314,0.56078,0.96471,1.00000"),
            "tile ground moved"
        );
        // 0x02/255, 0x41/255, 0x75/255: the dark appearance's fill.
        assert!(
            json.contains("srgb:0.00784,0.25490,0.45882,1.00000"),
            "dark ground moved"
        );
        assert_eq!(TILE_DARK, "#024175");
        let n = std::fs::read_to_string(format!("{icon}/Assets/near-n.svg")).expect("near-n.svg");
        assert!(n.contains(r##"fill="#F3F3F2""##), "paper moved");
        // The Mac's N layer: the same path, spaces for commas.
        assert!(
            n.contains(&PATH.replace(',', " ")),
            "the Mac icon's N moved"
        );
        assert_eq!((TILE, PAPER, N_SHARE), ("#0B8FF6", "#F3F3F2", 0.585));

        let doc = app_icon_svg(512);
        assert!(doc.contains(r#"width="512" height="512""#), "{doc}");
        assert!(doc.contains(r##"fill="#0B8FF6""##), "{doc}");
        assert!(doc.contains(r##"fill="#F3F3F2""##), "{doc}");
        assert!(doc.contains(PATH), "{doc}");
        // 400 * 0.585 / 234.08: the N spans 58.5% of the tile.
        assert!(doc.contains("matrix(0.9997 0 0 0.9997 "), "{doc}");
        assert!(doc.contains(r#"rx="90""#), "{doc}");
    }

    /// The N, transformed, sits centred on the tile.
    #[test]
    fn the_n_is_centred_on_the_tile() {
        let [bx, by, bw, bh] = VIEW_BOX;
        let (s, tx, ty) = n_on_tile();
        let left = bx * s + tx;
        let right = SIDE - ((bx + bw) * s + tx);
        let top = by * s + ty;
        let bottom = SIDE - ((by + bh) * s + ty);
        assert!((left - right).abs() < 1e-9 && (top - bottom).abs() < 1e-9);
        assert!((bw * s / SIDE - 0.585).abs() < 1e-12);
    }

    /// The single-ink N: one ink, no tile, never the product's old name.
    #[test]
    fn the_template_is_one_ink_without_a_tile() {
        let doc = template_svg("#000000", 16);
        assert!(!doc.contains("<rect"), "{doc}");
        assert!(doc.contains(r##"fill="#000000""##), "{doc}");
        assert!(!doc.contains(TILE), "{doc}");
        assert!(
            doc.contains(r#"viewBox="82.96 80.95 234.08 234.08""#),
            "{doc}"
        );
        for doc in [template_svg("#FFFFFF", 16), app_icon_svg(64)] {
            assert!(!doc.contains("Trace Commons"), "{doc}");
            assert!(!doc.contains("aria-label"), "{doc}");
        }
    }
}
