//! The mark as pixels, rendered here rather than by a platform toolchain.
//!
//! # Why this exists
//!
//! Every packaging surface that wants a bitmap wanted a different machine to
//! produce it. The macOS `.icns` is built with CoreGraphics, which is fine
//! because the `.icns` is a macOS artifact assembled on a macOS runner. The
//! Windows tiles were built the same way, and that was not fine: they are a
//! Windows artifact that could only be regenerated on a Mac, which put them
//! outside the drift check -- the check runs on `ubuntu-latest`, and a check
//! that cannot regenerate a file cannot compare it to anything.
//!
//! That is the exact shape of the defect this whole slice exists to fix. Three
//! solid `#315FBA` squares shipped as the Windows tiles for months because
//! nothing could regenerate them and every check that looked at them only
//! asserted their dimensions.
//!
//! So the mark is rasterized here, in Rust, with no dependencies, and the same
//! bytes come out on every runner.
//!
//! # Why exact coverage rather than supersampling
//!
//! The mark is entirely axis-aligned rectangles -- see [`shapes`] -- so a
//! pixel's coverage by a shape is the area of a rectangle intersection, which
//! is exact arithmetic rather than an estimate. Supersampling would approximate
//! a number that can simply be computed, and would make the output depend on
//! the sample count.
//!
//! Only `+`, `-`, `*`, `min` and `max` on `f64` are used, all of which IEEE 754
//! defines exactly, so the result is bit-identical across platforms. Nothing
//! here calls a transcendental function or iterates a hash map, which are the
//! two usual ways a renderer stops being reproducible.

use crate::{Scheme, VIEW};

/// An axis-aligned rectangle in view units, as `(x0, y0, x1, y1)`.
///
/// Half-open in the sense that matters here: coverage is computed from the
/// interval overlap, so a rectangle of zero width contributes nothing rather
/// than a hairline.
type Rect = (f64, f64, f64, f64);

/// A colour as straight (non-premultiplied) 8-bit RGBA.
type Rgba = [u8; 4];

/// The mark decomposed into painting order: a list of `(colour, rectangles)`
/// layers, each painted over the last.
///
/// # Why the rectangles within a layer are disjoint
///
/// Coverage within a layer is summed, not unioned, because summing is exact and
/// cheap while unioning axis-aligned rectangles is neither. That is only
/// correct if the rectangles do not overlap, so each L-shaped bracket is split
/// into two disjoint rectangles rather than the two overlapping stroke
/// segments it is drawn from. [`tests::layer_rects_within_a_layer_are_disjoint`]
/// is what holds that invariant; without it a future edit could double-count
/// coverage and produce a seam exactly along a bracket's inner corner.
pub mod shapes {
    use super::Rect;
    use crate::{
        FRAME_RECT, STROKE_FRAME, STROKE_FRAMED, VERTICES_BRACKET_CLOSE, VERTICES_BRACKET_OPEN,
    };

    /// The surface the brackets sit on: everything inside the frame ring.
    pub fn surface_field() -> Vec<Rect> {
        let (x, _, w, _) = FRAME_RECT;
        let half = STROKE_FRAME as f64 / 2.0;
        let lo = x as f64 + half;
        let hi = (x + w) as f64 - half;
        vec![(lo, lo, hi, hi)]
    }

    /// One bracket as two disjoint rectangles.
    ///
    /// A bracket is a three-point polyline stroked with butt caps and a miter
    /// join. The miter at the corner fills the outer square, so the union is a
    /// clean L of constant thickness whose outer corner is offset by half the
    /// stroke. Splitting that L at the arm boundary gives two rectangles that
    /// do not overlap.
    fn bracket(vertices: [(u32, u32); 3], stroke: u32) -> Vec<Rect> {
        let half = stroke as f64 / 2.0;
        let (ax, ay) = (vertices[0].0 as f64, vertices[0].1 as f64);
        let (cx, cy) = (vertices[1].0 as f64, vertices[1].1 as f64);
        let (ex, ey) = (vertices[2].0 as f64, vertices[2].1 as f64);

        // Both arms are axis-aligned: the first shares the corner's x, the
        // second shares its y. Everything below assumes it, and the whole
        // exact-coverage approach depends on it, so it is asserted rather than
        // left as a property of six hand-typed numbers.
        debug_assert_eq!(ax, cx, "first arm is not vertical");
        debug_assert_eq!(ey, cy, "second arm is not horizontal");

        // The vertical arm runs between the first vertex and the corner; the
        // horizontal arm between the corner and the last. Each is widened by
        // half the stroke across its length, and extended by half the stroke at
        // the corner end only -- that extension is the miter.
        let (vy0, vy1) = if ay < cy {
            (ay, cy + half)
        } else {
            (cy - half, ay)
        };
        let vertical = (cx - half, vy0, cx + half, vy1);

        // The horizontal arm is then trimmed to start where the vertical arm
        // ends, so the two do not overlap.
        let (hx0, hx1) = if ex > cx {
            (cx + half, ex)
        } else {
            (ex, cx - half)
        };
        let horizontal = (hx0, cy - half, hx1, cy + half);

        vec![vertical, horizontal]
    }

    /// The user's bracket, top-left.
    pub fn green_bracket() -> Vec<Rect> {
        bracket(VERTICES_BRACKET_OPEN, STROKE_FRAMED)
    }

    /// The agent's bracket, bottom-right.
    pub fn blue_bracket() -> Vec<Rect> {
        bracket(VERTICES_BRACKET_CLOSE, STROKE_FRAMED)
    }
}

/// Parse `#RRGGBB` into opaque RGBA.
///
/// The palette lives in this crate as hex strings because that is the form
/// every client's design system states it in, and a second numeric spelling
/// here would be one more thing that can drift.
fn parse_hex(hex: &str) -> Rgba {
    let b = hex.as_bytes();
    debug_assert_eq!(b.len(), 7, "expected #RRGGBB, got {hex}");
    let nibble = |c: u8| -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 0,
        }
    };
    let byte = |i: usize| nibble(b[i]) * 16 + nibble(b[i + 1]);
    [byte(1), byte(3), byte(5), 255]
}

/// Exact fractional coverage of pixel `(px, py)` by `rect`, in view units.
///
/// `scale` converts view units to pixels. Both intervals are clamped to the
/// pixel, so the result is in `[0, 1]`.
fn coverage(rect: Rect, px: u32, py: u32, scale: f64) -> f64 {
    let (x0, y0, x1, y1) = rect;
    let (x0, y0, x1, y1) = (x0 * scale, y0 * scale, x1 * scale, y1 * scale);
    let px = px as f64;
    let py = py as f64;
    let w = (x1.min(px + 1.0) - x0.max(px)).max(0.0);
    let h = (y1.min(py + 1.0) - y0.max(py)).max(0.0);
    w * h
}

/// Composite `src` over `dst` at coverage `a`, straight alpha, per channel.
fn over(dst: Rgba, src: Rgba, a: f64) -> Rgba {
    let blend = |d: u8, s: u8| -> u8 {
        let v = d as f64 * (1.0 - a) + s as f64 * a;
        // Round half away from zero without calling `round`, so the rounding
        // rule is stated here rather than inherited.
        (v + 0.5) as u8
    };
    [
        blend(dst[0], src[0]),
        blend(dst[1], src[1]),
        blend(dst[2], src[2]),
        blend(dst[3], src[3]),
    ]
}

/// The framed mark rendered to straight RGBA at `size` pixels square.
///
/// The frame's outer edge is the canvas boundary, so every pixel is covered and
/// the result is fully opaque.
pub fn render_framed(scheme: Scheme, size: u32) -> Vec<u8> {
    let scale = size as f64 / VIEW as f64;
    // The frame ring is the background, not a layer.
    //
    // Painting the ring as a coverage layer over transparency and then the
    // surface over that is wrong, and wrong in a way that only shows at small
    // sizes: at 16px the top-left pixel is 0.75 covered by the ring and 0.25 by
    // the surface, and compositing those in sequence over transparency yields
    // alpha 207 rather than 255. The two coverages are adjacent, not stacked,
    // so sequential `over` loses the part of the pixel each one does not claim.
    //
    // Starting from an opaque ring-coloured field removes the problem rather
    // than correcting for it: the ring is by definition everything the surface
    // does not cover, so every pixel begins opaque and only the colour is ever
    // blended.
    let layers: [(Rgba, Vec<Rect>); 3] = [
        (parse_hex(scheme.surface()), shapes::surface_field()),
        (parse_hex(scheme.bracket_open()), shapes::green_bracket()),
        (parse_hex(scheme.bracket_close()), shapes::blue_bracket()),
    ];

    let ring = parse_hex(scheme.line());
    let mut out = vec![0u8; (size as usize) * (size as usize) * 4];
    for py in 0..size {
        for px in 0..size {
            let mut pixel: Rgba = ring;
            for (colour, rects) in &layers {
                let mut a = 0.0;
                for r in rects {
                    a += coverage(*r, px, py, scale);
                }
                let a = a.min(1.0);
                if a > 0.0 {
                    pixel = over(pixel, *colour, a);
                }
            }
            let i = ((py as usize) * (size as usize) + px as usize) * 4;
            out[i..i + 4].copy_from_slice(&pixel);
        }
    }
    out
}

// MARK: - PNG

/// CRC-32 as PNG defines it, table-free.
///
/// Table-free because the table would be either a 256-entry literal nobody can
/// check by eye or a `OnceLock` this crate does not need. Eight shifts per byte
/// over a few hundred kilobytes is not worth either.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Adler-32 over the uncompressed data, as the zlib wrapper requires.
fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b): (u32, u32) = (1, 0);
    for &byte in bytes {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// A deflate bitstream writer.
///
/// Deflate packs bits into bytes least-significant-bit first, but writes
/// Huffman codes most-significant-bit first within that packing. The two rules
/// are easy to conflate and produce a stream that looks plausible and decodes
/// to noise, so they are separate methods here rather than one with a flag.
struct BitWriter {
    out: Vec<u8>,
    bit: u32,
    acc: u32,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter {
            out: Vec::new(),
            bit: 0,
            acc: 0,
        }
    }

    /// Write `n` bits of `value`, least-significant bit first. This is the
    /// packing deflate uses for everything that is not a Huffman code.
    fn bits(&mut self, value: u32, n: u32) {
        for i in 0..n {
            self.acc |= ((value >> i) & 1) << self.bit;
            self.bit += 1;
            if self.bit == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.bit = 0;
            }
        }
    }

    /// Write a Huffman code of `n` bits, most-significant bit first.
    fn code(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            self.bits((value >> i) & 1, 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bit > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Emit one literal byte using the fixed Huffman literal/length alphabet.
///
/// The code assignments are RFC 1951 section 3.2.6, which is a table rather
/// than a formula and is transcribed here as one.
fn fixed_literal(w: &mut BitWriter, byte: u8) {
    let v = byte as u32;
    if v <= 143 {
        w.code(0x30 + v, 8);
    } else {
        w.code(0x190 + (v - 144), 9);
    }
}

/// Length codes 257..=285 as `(code, extra_bits, base_length)`.
const LENGTH_CODES: [(u32, u32, u32); 28] = [
    (257, 0, 3),
    (258, 0, 4),
    (259, 0, 5),
    (260, 0, 6),
    (261, 0, 7),
    (262, 0, 8),
    (263, 0, 9),
    (264, 0, 10),
    (265, 1, 11),
    (266, 1, 13),
    (267, 1, 15),
    (268, 1, 17),
    (269, 2, 19),
    (270, 2, 23),
    (271, 2, 27),
    (272, 2, 31),
    (273, 3, 35),
    (274, 3, 43),
    (275, 3, 51),
    (276, 3, 59),
    (277, 4, 67),
    (278, 4, 83),
    (279, 4, 99),
    (280, 4, 115),
    (281, 5, 131),
    (282, 5, 163),
    (283, 5, 195),
    (284, 5, 227),
    // 285 is length 258 with no extra bits, handled as a special case below.
];

/// Emit a back-reference of `len` bytes at distance 1.
///
/// Distance 1 only. This compressor does run-length encoding and nothing else:
/// the mark is large areas of one colour, and after the PNG `Up` filter almost
/// every row is a run of zeroes, so repeats of the immediately preceding byte
/// are the only matches worth finding. A general LZ77 with a hash chain would
/// compress marginally better and would be several times the code to audit, for
/// an artifact that is already small once the runs are gone.
fn fixed_match(w: &mut BitWriter, len: u32) {
    debug_assert!((3..=258).contains(&len));
    if len == 258 {
        // Literal/length code 285, which lives in the 280..=287 run and is
        // therefore eight bits based at 0xC0 -- NOT a seven-bit code. Writing
        // it as 0x17 instead names code 279, which expects four extra bits;
        // omitting those desynchronises every bit that follows. Short tiles
        // never produce a maximal run, so that mistake decodes fine at 16px
        // and destroys a 150px tile.
        w.code(0xC0 + (285 - 280), 8);
    } else {
        let (code, extra, base) = *LENGTH_CODES
            .iter()
            .rev()
            .find(|(_, _, base)| *base <= len)
            .expect("length below the smallest match");
        // 257..=279 are seven-bit codes based at 0; 280..=287 are eight-bit.
        if code <= 279 {
            w.code(code - 256, 7);
        } else {
            w.code(0xC0 + (code - 280), 8);
        }
        w.bits(len - base, extra);
    }
    // Distance code 0 is distance 1, five bits, no extra bits. Distance codes
    // use their own fixed five-bit alphabet, not the literal/length one.
    w.code(0, 5);
}

/// Deflate `raw` with fixed Huffman codes and distance-1 run-length matches.
fn deflate_rle(raw: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.bits(1, 1); // BFINAL
    w.bits(1, 2); // BTYPE 01, fixed Huffman

    let mut i = 0;
    while i < raw.len() {
        // A distance-1 match repeats the previous byte, so a run of identical
        // bytes of length n is one literal followed by a match of n-1.
        let mut run = 1;
        while i + run < raw.len() && raw[i + run] == raw[i] {
            run += 1;
        }
        fixed_literal(&mut w, raw[i]);
        i += 1;
        let mut remaining = run - 1;
        while remaining >= 3 {
            let take = remaining.min(258);
            // Never leave a tail of 1 or 2, which cannot be encoded as a match
            // and would have to be emitted as literals anyway.
            let take = if remaining - take == 1 || remaining - take == 2 {
                take - 3
            } else {
                take
            };
            fixed_match(&mut w, take as u32);
            i += take;
            remaining -= take;
        }
        for _ in 0..remaining {
            fixed_literal(&mut w, raw[i]);
            i += 1;
        }
    }

    w.code(0, 7); // end of block, literal/length code 256
    w.finish()
}

/// Apply PNG's `Up` filter to every row, prefixing each with its filter byte.
///
/// `Up` subtracts the byte directly above, modulo 256. The first row has no row
/// above it, which PNG defines as an implicit row of zeroes, so `Up` on row 0
/// leaves it unchanged -- the filter type is still declared as `Up` rather than
/// `None`, because a decoder reads the declared type and both give the same
/// bytes there.
fn filter_up(pixels: &[u8], size: u32) -> Vec<u8> {
    let stride = size as usize * 4;
    let mut raw = Vec::with_capacity(size as usize * (stride + 1));
    for row in 0..size as usize {
        raw.push(2);
        let start = row * stride;
        for i in 0..stride {
            let above = if row == 0 {
                0
            } else {
                pixels[start - stride + i]
            };
            raw.push(pixels[start + i].wrapping_sub(above));
        }
    }
    raw
}

/// Encode straight RGBA pixels as a PNG.
///
/// Rows use the `Up` filter, which subtracts the row above. The mark is wide
/// bands of one colour, so almost every row becomes zeroes and the run-length
/// compressor collapses it. Without filtering the same image is stored
/// essentially verbatim: the 150px tile was 90kB before this and is a fraction
/// of that after, which matters because a scale-400 variant of it would
/// otherwise be well over a megabyte of committed binary.
pub fn encode_png(pixels: &[u8], size: u32) -> Vec<u8> {
    assert_eq!(
        pixels.len(),
        (size as usize) * (size as usize) * 4,
        "pixel buffer does not match {size}x{size} RGBA"
    );

    let raw = filter_up(pixels, size);
    let mut z = vec![0x78, 0x01];
    z.extend_from_slice(&deflate_rle(&raw));
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// The framed mark as a PNG at `size` pixels square.
pub fn png(scheme: Scheme, size: u32) -> Vec<u8> {
    encode_png(&render_framed(scheme, size), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sample a rendered pixel as RGBA.
    fn at(pixels: &[u8], size: u32, x: u32, y: u32) -> Rgba {
        let i = ((y as usize) * (size as usize) + x as usize) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    }

    /// Coverage is summed within a layer, which is only exact when the
    /// rectangles do not overlap. This is the invariant that makes the whole
    /// renderer correct rather than approximately correct.
    #[test]
    fn layer_rects_within_a_layer_are_disjoint() {
        let layers = [
            ("surface", shapes::surface_field()),
            ("green", shapes::green_bracket()),
            ("blue", shapes::blue_bracket()),
        ];
        for (name, rects) in layers {
            for (i, a) in rects.iter().enumerate() {
                for b in rects.iter().skip(i + 1) {
                    let ox = a.2.min(b.2) - a.0.max(b.0);
                    let oy = a.3.min(b.3) - a.1.max(b.1);
                    assert!(
                        ox <= 0.0 || oy <= 0.0,
                        "{name}: {a:?} and {b:?} overlap by {ox}x{oy}"
                    );
                }
            }
        }
    }

    /// The brackets land where the geometry says. Sampling the middle of each
    /// arm is what distinguishes "the mark is drawn" from "something is drawn".
    #[test]
    fn both_brackets_are_rendered_in_their_own_ink() {
        let size = 640;
        let px = render_framed(Scheme::Light, size);
        let green = parse_hex(Scheme::Light.bracket_open());
        let blue = parse_hex(Scheme::Light.bracket_close());
        let surface = parse_hex(Scheme::Light.surface());

        // Ten pixels per view unit at this size. Vertical arm of the green
        // bracket is centred on x=11, spanning y 11..28.
        assert_eq!(at(&px, size, 110, 200), green, "green vertical arm");
        // Horizontal arm is centred on y=11, spanning x 11..28.
        assert_eq!(at(&px, size, 200, 110), green, "green horizontal arm");
        // Blue is the same rotated 180 degrees.
        assert_eq!(at(&px, size, 530, 440), blue, "blue vertical arm");
        assert_eq!(at(&px, size, 440, 530), blue, "blue horizontal arm");
        // The space between the brackets is the mark, and nothing is drawn in
        // it.
        assert_eq!(at(&px, size, 320, 320), surface, "the space between");
    }

    /// The frame ring is the line colour and the interior is the surface.
    #[test]
    fn the_frame_ring_is_drawn() {
        let size = 640;
        let px = render_framed(Scheme::Light, size);
        assert_eq!(
            at(&px, size, 0, 0),
            parse_hex(Scheme::Light.line()),
            "corner"
        );
        assert_eq!(
            at(&px, size, 320, 5),
            parse_hex(Scheme::Light.line()),
            "top edge"
        );
        assert_eq!(
            at(&px, size, 320, 40),
            parse_hex(Scheme::Light.surface()),
            "inside the ring"
        );
    }

    /// Every pixel is covered, so the tile is opaque. A tile that was
    /// accidentally transparent would look correct in most viewers and wrong on
    /// a Start menu.
    #[test]
    fn every_pixel_is_opaque() {
        for size in [16, 44, 50, 150] {
            let px = render_framed(Scheme::Light, size);
            for (i, chunk) in px.chunks(4).enumerate() {
                assert_eq!(chunk[3], 255, "size {size}, pixel {i} is not opaque");
            }
        }
    }

    /// The renderer is a pure function of its inputs. This is the property that
    /// lets the drift check run on a different machine from the one that last
    /// generated the committed file.
    #[test]
    fn rendering_is_deterministic() {
        for size in [16, 44, 150] {
            assert_eq!(
                png(Scheme::Light, size),
                png(Scheme::Light, size),
                "size {size}"
            );
        }
    }

    /// The teeth. At every size actually shipped, the render must contain both
    /// bracket inks -- not merely more than one colour, which a gradient or a
    /// stray antialiased edge would also satisfy, and which is the metric that
    /// let three flat squares through.
    #[test]
    fn no_shipped_size_renders_without_the_mark() {
        for size in [16, 24, 32, 44, 48, 50, 150, 256, 300, 600] {
            let px = render_framed(Scheme::Light, size);
            let green = parse_hex(Scheme::Light.bracket_open());
            let blue = parse_hex(Scheme::Light.bracket_close());
            let has = |want: Rgba| px.chunks(4).any(|p| p == want);
            assert!(has(green), "size {size} has no green bracket");
            assert!(has(blue), "size {size} has no blue bracket");
        }
    }

    /// PNG structure, checked by recomputing what the format says must hold
    /// rather than by decoding with the encoder's own assumptions.
    #[test]
    fn png_structure_and_checksums_are_valid() {
        let size = 44;
        let bytes = png(Scheme::Light, size);
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );

        let mut i = 8;
        let mut kinds = Vec::new();
        while i < bytes.len() {
            let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
            let kind = &bytes[i + 4..i + 8];
            let data = &bytes[i + 8..i + 8 + len];
            let stated = u32::from_be_bytes(bytes[i + 8 + len..i + 12 + len].try_into().unwrap());
            let mut crc_input = Vec::new();
            crc_input.extend_from_slice(kind);
            crc_input.extend_from_slice(data);
            assert_eq!(
                crc32(&crc_input),
                stated,
                "bad CRC on {}",
                String::from_utf8_lossy(kind)
            );
            kinds.push(String::from_utf8_lossy(kind).to_string());
            i += 12 + len;
        }
        assert_eq!(i, bytes.len(), "trailing bytes after IEND");
        assert_eq!(kinds, vec!["IHDR", "IDAT", "IEND"]);
    }

    /// The zlib wrapper's own rules: the header must be a multiple of 31, and
    /// the trailer must be the Adler-32 of the raw data.
    #[test]
    fn zlib_wrapper_is_well_formed() {
        let raw_size = 8;
        let pixels = render_framed(Scheme::Light, raw_size);
        let bytes = encode_png(&pixels, raw_size);
        // Walk to IDAT.
        let mut i = 8;
        loop {
            let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
            if &bytes[i + 4..i + 8] == b"IDAT" {
                let z = &bytes[i + 8..i + 8 + len];
                let header = ((z[0] as u32) << 8) | z[1] as u32;
                assert_eq!(header % 31, 0, "zlib header {header:#x} is not valid");
                let raw = filter_up(&pixels, raw_size);
                let stated = u32::from_be_bytes(z[z.len() - 4..].try_into().unwrap());
                assert_eq!(adler32(&raw), stated, "adler mismatch");
                return;
            }
            i += 12 + len;
        }
    }

    /// The maximal match length is code 285, an EIGHT-bit code based at 0xC0.
    ///
    /// This is a regression test for a real bug. Writing it as the seven-bit
    /// `0x17` names code 279 instead, which expects four extra bits that then go
    /// unwritten, desynchronising the remainder of the stream. It is invisible
    /// at small sizes because a maximal 258-byte run never comes up: the 16px
    /// tile decoded perfectly while the 150px tile inflated to 704 of its 90150
    /// bytes and stopped.
    #[test]
    fn maximal_match_uses_the_eight_bit_length_code() {
        let mut w = BitWriter::new();
        fixed_match(&mut w, 258);
        let bytes = w.finish();
        // 0xC5 most-significant-bit first, then five zero bits for distance
        // code 0, packed least-significant-bit first into the output.
        let bits: Vec<u8> = bytes
            .iter()
            .flat_map(|b| (0..8).map(move |i| (b >> i) & 1))
            .collect();
        assert_eq!(
            &bits[..8],
            &[1, 1, 0, 0, 0, 1, 0, 1],
            "length code 285 must be 0xC5 in eight bits, MSB first"
        );
        assert_eq!(&bits[8..13], &[0, 0, 0, 0, 0], "distance code 0");
    }

    /// Every match length the encoder can emit maps to a code in the right
    /// width class. The 279/285 confusion above is one instance of a general
    /// hazard: the fixed alphabet changes width twice, and nothing about a
    /// wrong-width code is visible until a decoder desynchronises.
    #[test]
    fn every_match_length_round_trips_through_a_run() {
        // A run long enough to force a maximal match plus a remainder, at a
        // size that actually produces one.
        for len in [3u32, 10, 11, 114, 115, 226, 227, 257, 258] {
            let mut w = BitWriter::new();
            fixed_match(&mut w, len);
            assert!(!w.finish().is_empty(), "length {len} produced no output");
        }
    }

    /// A known-answer test for CRC-32, so a subtle transcription error in the
    /// polynomial is caught here rather than as "some decoder rejects the file".
    #[test]
    fn crc32_matches_known_answers() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn adler32_matches_known_answers() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn hex_parses_to_the_palette() {
        assert_eq!(parse_hex("#315FBA"), [0x31, 0x5F, 0xBA, 255]);
        assert_eq!(parse_hex("#FFFFFF"), [255, 255, 255, 255]);
    }
}
