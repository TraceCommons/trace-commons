//! Home's decoration, drawn with cairo alone: the hero cards' grounds and
//! pixel graphics, the Insights hero's bars, and the coins that burst from
//! the Balance card when NEAR AI credit arrives.
//!
//! A port of the Mac's `GlassHeroCard`, `GlassHeroSymbol` and
//! `GlassPixelGlyph` (`macos/Sources/TCDesign/Components/HeroCard.swift`,
//! `PixelGlyph.swift`), the rules of `GlassPixelField` (`PixelField.swift`)
//! the glyph moves by, `HomeInsightsGraphic`
//! (`Views/Monitor/HomeInsightsVisual.swift`), `GlassCoinBurst`
//! (`Components/CoinBurst.swift`) and `BalanceArrival`
//! (`TCShellCore/BalanceArrival.swift`). The numbers are the Mac's, as the
//! flat theme it ships draws them, and the tests below pin them.
//!
//! Everything here is a function of its inputs and the time, with no widget
//! in it, so a test or an offscreen render can draw it without a display.
//! `home.rs` owns the widgets, the clocks and when anything moves.
//!
//! Where the Mac samples an Apple SF Symbol for a graphic (`scope`,
//! `sparkles`, `bolt`, `globe`), Linux cannot: those are drawn here by hand
//! on the same grid, from circles, lines and stars of this file's own, to
//! look like the symbol. `cpu` and `chart.bar.xaxis` the Mac already places
//! by hand, and they are its own runs, cell for cell.

use gtk::cairo;

use super::style::brand_tokens::hero as tokens;

/// An opaque colour from a token's hex, as cairo's three channels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f64, pub f64, pub f64);

impl Rgb {
    pub fn hex(hex: &str) -> Self {
        let hex = hex.trim_start_matches('#');
        let channel = |at: usize| {
            u8::from_str_radix(hex.get(at..at + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0
        };
        Self(channel(0), channel(2), channel(4))
    }

    fn source(self, cr: &cairo::Context, alpha: f64) {
        cr.set_source_rgba(self.0, self.1, self.2, alpha);
    }
}

/// The hero card's measures (`GlassHeroCard`, `GlassHeroClose`,
/// `GlassHeroSymbol`).
pub mod card {
    /// The card's least height, the words' trailing room for the graphic,
    /// and the graphic's own width.
    pub const MIN_HEIGHT: f64 = 156.0;
    pub const GRAPHIC_RESERVE: i32 = 90;
    pub const GRAPHIC_WIDTH: f64 = 210.0;
    /// The ground's corner: the flat theme's card radius.
    pub const RADIUS: f64 = super::super::style::brand_tokens::glass::radius::CARD as f64;
    /// The hover: heroInk at this opacity over the ground, under the graphic.
    pub const HOVER: f64 = 0.06;
    /// The subtitle's ink, and the content lines' (`drafts`, the teaser).
    pub const SUBTITLE_INK: f64 = 0.88;
    pub const CONTENT_INK: f64 = 0.92;
    /// The X: its glyph, its inset from the top and trailing edges, and its
    /// ink at rest and under the pointer.
    pub const CLOSE_GLYPH: i32 = 12;
    pub const CLOSE_INSET: i32 = super::super::style::brand_tokens::glass::space::S4;
    pub const CLOSE_INK: f64 = 0.75;
    pub const CLOSE_INK_HOVER: f64 = 1.0;
    /// The pill: its padding across and down, and the gap over it.
    pub const PILL_PADDING: (i32, i32) = (14, 7);
    pub const PILL_GAP: i32 = super::super::style::brand_tokens::glass::space::S1;
    /// The graphic: the symbol's size, how far it is pushed past the
    /// trailing and bottom edges, and its tilt in degrees (negative turns it
    /// anticlockwise on screen, as SwiftUI's does).
    pub const SYMBOL_SIZE: f64 = 190.0;
    pub const BLEED: (f64, f64) = (46.0, 34.0);
    pub const ANGLE: f64 = -12.0;
    /// The pixel pitch and the cells' strengths.
    pub const CELL: f64 = 6.0;
    pub const BRIGHT_CELLS: f64 = 0.46;
    pub const DIM_CELLS: f64 = 0.32;
    /// How far the glyph's canvas runs past `SYMBOL_SIZE`, as a fraction.
    pub const OVERSCAN: f64 = 0.3;

    /// The glyph's canvas side, a whole number of cells.
    pub fn canvas_side() -> f64 {
        (SYMBOL_SIZE * (1.0 + OVERSCAN) / CELL).ceil() * CELL
    }

    /// Where the glyph's centre sits on a card of `width` by `height`: the
    /// middle of the graphic's column at the trailing edge, then pushed past
    /// the edges by `BLEED`.
    pub fn symbol_centre(width: f64, height: f64) -> (f64, f64) {
        (
            width - GRAPHIC_WIDTH / 2.0 + BLEED.0,
            height / 2.0 + BLEED.1,
        )
    }
}

/// Which ground a hero card takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    Campaign,
    Missions,
    Insights,
}

impl Ground {
    pub fn token(self) -> tokens::Ground {
        match self {
            Self::Campaign => tokens::HERO_CAMPAIGN_CARD,
            Self::Missions => tokens::HERO_MISSIONS,
            Self::Insights => tokens::HERO_INSIGHTS,
        }
    }
}

/// The pixel graphic's three blues in one scheme, bright then dim.
pub fn tints(dark: bool) -> [Rgb; 3] {
    let hexes = if dark {
        tokens::TINTS_DARK
    } else {
        tokens::TINTS_LIGHT
    };
    hexes.map(Rgb::hex)
}

/// A rounded rectangle path, its corners `radius` round.
pub fn rounded(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let r = radius.min(width / 2.0).min(height / 2.0).max(0.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + width - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + width - r, y + height - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + height - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 3.0 * pi / 2.0);
    cr.close_path();
}

/// The start and end of a CSS-angle gradient over a box (`GlassGradient
/// .unitPoints`): 180 runs top to bottom.
pub fn gradient_points(angle: f64, width: f64, height: f64) -> ((f64, f64), (f64, f64)) {
    let radians = angle.to_radians();
    let dx = radians.sin() / 2.0;
    let dy = -radians.cos() / 2.0;
    (
        ((0.5 - dx) * width, (0.5 - dy) * height),
        ((0.5 + dx) * width, (0.5 + dy) * height),
    )
}

/// The card's ground: its gradient, and under the pointer heroInk at
/// `card::HOVER` over it. The caller clips to the card first.
pub fn paint_ground(cr: &cairo::Context, width: f64, height: f64, ground: Ground, hover: bool) {
    let (angle, stops) = ground.token();
    let ((x0, y0), (x1, y1)) = gradient_points(angle as f64, width, height);
    let gradient = cairo::LinearGradient::new(x0, y0, x1, y1);
    for (offset, hex) in stops {
        let Rgb(r, g, b) = Rgb::hex(hex);
        gradient.add_color_stop_rgb(*offset, r, g, b);
    }
    let _ = cr.set_source(&gradient);
    cr.rectangle(0.0, 0.0, width, height);
    let _ = cr.fill();
    if hover {
        Rgb::hex(tokens::HERO_INK).source(cr, card::HOVER);
        cr.rectangle(0.0, 0.0, width, height);
        let _ = cr.fill();
    }
}

/// A whole hero ground with its graphic, clipped to the card: what the
/// card's drawing area paints. `mask` is `None` for a card whose graphic is
/// not the pixel glyph (the Insights hero's bars, which are widgets).
pub fn paint_card(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    ground: Ground,
    hover: bool,
    mask: Option<&glyph::Mask>,
    dark: bool,
    time: f64,
) {
    let _ = cr.save();
    rounded(cr, 0.0, 0.0, width, height, card::RADIUS);
    cr.clip();
    paint_ground(cr, width, height, ground, hover);
    if let Some(mask) = mask {
        glyph::paint_symbol(cr, width, height, mask, dark, time);
    }
    let _ = cr.restore();
}

/// The rules the pixel graphics move by (`GlassPixelField`).
pub mod field {
    /// The pixel graphics run slow: every frame is a redraw, and a stepped
    /// grid reads as stepped anyway.
    pub const FRAMES_PER_SECOND: f64 = 10.0;

    /// The ordered dither's thresholds, 0 to 15 (Bayer, 4x4).
    pub const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

    /// A cell's threshold in (0, 1): lit when its level is above it.
    pub fn threshold(column: usize, row: usize) -> f64 {
        (BAYER[row & 3][column & 3] as f64 + 0.5) / 16.0
    }

    /// How far a point is from the bottom trailing corner, in units of the
    /// field's reach.
    pub fn distance(x: f64, y: f64, size: (f64, f64), reach: (f64, f64)) -> f64 {
        if size.0 <= 0.0 || size.1 <= 0.0 || reach.0 <= 0.0 || reach.1 <= 0.0 {
            return f64::INFINITY;
        }
        let u = (size.0 - x) / (size.0 * reach.0);
        let v = (size.1 - y) / (size.1 * reach.1);
        (u * u + v * v).sqrt()
    }

    /// The light at `distance` and `time`: a smooth falloff with a ripple
    /// travelling outward.
    pub fn level(distance: f64, time: f64) -> f64 {
        if distance >= 1.0 {
            return 0.0;
        }
        let falloff = 1.0 - distance;
        let smooth = falloff * falloff * (3.0 - 2.0 * falloff);
        let ripple = 0.5 + 0.5 * (distance * 14.0 - time * 1.6).sin();
        smooth * (0.72 + 0.28 * ripple)
    }

    /// Which of `count` colours a cell takes at `time`: a slow drift, so
    /// neighbours share a colour and the colours move.
    pub fn shade(column: usize, row: usize, time: f64, count: usize) -> usize {
        if count <= 1 {
            return 0;
        }
        let (c, r) = (column as f64, row as f64);
        let wave = (c * 0.31 + r * 0.23 + time * 0.45).sin() + ((c - r) * 0.17 - time * 0.3).sin();
        let unit = ((wave + 2.0) / 4.0).clamp(0.0, 0.999_999);
        (unit * count as f64) as usize
    }
}

/// A symbol drawn as lit cells on a visible grid (`GlassPixelGlyph`).
pub mod glyph {
    use super::{Rgb, card, field};
    use gtk::cairo;

    /// How much of each cell the source covers, 0 to 1, row by row.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Mask {
        pub columns: usize,
        pub coverage: Vec<f32>,
    }

    /// How much of a cell the source must cover for the cell to light.
    pub const MINIMUM_COVERAGE: f32 = 0.5;
    /// Where along the diagonal the fade starts, and how much it takes.
    pub const FADE_START: f64 = 0.55;
    pub const FADE_DEPTH: f64 = 0.92;

    /// How much of the glyph shows at a cell: all of it toward the bottom
    /// trailing corner, thinning toward the top leading one.
    pub fn presence(across: f64, down: f64) -> f64 {
        let far = 1.0 - (across + down) / 2.0;
        let into = ((far - FADE_START) / (1.0 - FADE_START)).clamp(0.0, 1.0);
        1.0 - FADE_DEPTH * into * into * (3.0 - 2.0 * into)
    }

    /// Whether a cell lights: covered, and present past its dither
    /// threshold.
    pub fn is_lit(coverage: f32, column: usize, row: usize, columns: usize, rows: usize) -> bool {
        if coverage < MINIMUM_COVERAGE || columns <= 1 || rows <= 1 {
            return false;
        }
        let across = column as f64 / (columns - 1) as f64;
        let down = row as f64 / (rows - 1) as f64;
        presence(across, down) > field::threshold(column, row)
    }

    /// A run of cells: x, y, width, height.
    type Run = (i32, i32, i32, i32);

    /// The Mac's two hand-placed symbols (`GlassPixelGlyph.drawnCells`):
    /// the grid's side, how far right of centre it sits, and each layer in
    /// turn, lighting or cutting back out its runs.
    const DRAWN: &[(&str, i32, i32, &[(bool, &[Run])])] = &[
        (
            "cpu",
            38,
            2,
            &[
                (
                    true,
                    &[
                        (11, 0, 2, 7),
                        (18, 0, 2, 7),
                        (25, 0, 2, 7),
                        (11, 31, 2, 7),
                        (18, 31, 2, 7),
                        (25, 31, 2, 7),
                        (0, 11, 7, 2),
                        (0, 18, 7, 2),
                        (0, 25, 7, 2),
                        (31, 11, 7, 2),
                        (31, 18, 7, 2),
                        (31, 25, 7, 2),
                        (6, 6, 26, 26),
                    ],
                ),
                (
                    false,
                    &[
                        (6, 6, 1, 1),
                        (31, 6, 1, 1),
                        (6, 31, 1, 1),
                        (31, 31, 1, 1),
                        (9, 9, 20, 20),
                    ],
                ),
                (true, &[(13, 13, 12, 12)]),
                (false, &[(15, 15, 8, 8)]),
            ],
        ),
        (
            "chart.bar.xaxis",
            32,
            0,
            &[(true, &[(-2, 12, 7, 20), (8, 0, 7, 32), (18, 7, 7, 25)])],
        ),
    ];

    /// A Mac hand-placed symbol centred on a canvas `columns` cells square.
    pub fn drawn_mask(symbol: &str, columns: usize) -> Option<Mask> {
        let (_, grid, shift, layers) = DRAWN.iter().find(|(name, ..)| *name == symbol)?;
        let n = columns as i32;
        let inset = (n - grid) / 2;
        let mut coverage = vec![0.0; columns * columns];
        for (lit, runs) in layers.iter() {
            for &(x, y, w, h) in runs.iter() {
                for row in y..y + h {
                    for column in x..x + w {
                        let (yy, xx) = (row + inset, column + inset + shift);
                        if (0..n).contains(&yy) && (0..n).contains(&xx) {
                            coverage[(yy * n + xx) as usize] = if *lit { 1.0 } else { 0.0 };
                        }
                    }
                }
            }
        }
        Some(Mask { columns, coverage })
    }

    /// The symbols Linux draws by hand where the Mac samples an SF Symbol:
    /// shapes in cell units on the canvas, centred on (21, 21) of the
    /// 42-cell canvas the hero glyph has. Each is a test at a point.
    fn hand_drawn_inside(symbol: &str, x: f64, y: f64) -> Option<bool> {
        let (dx, dy) = (x - 21.0, y - 21.0);
        let r = (dx * dx + dy * dy).sqrt();
        Some(match symbol {
            // A ring, and a crosshair's four arms crossing it.
            "scope" => {
                let ring = (12.5..=16.5).contains(&r);
                let arm_x = dy.abs() <= 1.5 && (6.5..=19.0).contains(&dx.abs());
                let arm_y = dx.abs() <= 1.5 && (6.5..=19.0).contains(&dy.abs());
                ring || arm_x || arm_y
            }
            // A ring with its meridian, an inner meridian ellipse, the
            // equator and two parallels.
            "globe" => {
                if r > 16.5 {
                    false
                } else {
                    let ring = r >= 14.3;
                    let ellipse = |rx: f64| (dx / rx).powi(2) + (dy / 16.0).powi(2);
                    let meridian = ellipse(7.6) <= 1.0 && ellipse(5.4) >= 1.0;
                    let axis = dx.abs() <= 1.1;
                    let equator = dy.abs() <= 1.2;
                    let parallels = (dy.abs() - 8.5).abs() <= 1.0;
                    ring || meridian || axis || equator || parallels
                }
            }
            // A lightning bolt's outline.
            "bolt" => {
                const POINTS: [(f64, f64); 6] = [
                    (27.0, 3.0),
                    (10.0, 24.5),
                    (21.0, 24.5),
                    (15.5, 39.0),
                    (33.0, 16.5),
                    (22.0, 16.5),
                ];
                (0..POINTS.len()).any(|i| {
                    let (a, b) = (POINTS[i], POINTS[(i + 1) % POINTS.len()]);
                    segment_distance((x, y), a, b) <= 1.35
                })
            }
            // Three four-pointed stars, the largest low and to the right.
            "sparkles" => {
                const STARS: [(f64, f64, f64, f64); 3] = [
                    (24.0, 27.0, 12.0, 13.0),
                    (12.0, 13.0, 6.0, 6.5),
                    (21.5, 5.0, 3.5, 4.0),
                ];
                STARS.iter().any(|&(cx, cy, rx, ry)| {
                    ((x - cx).abs() / rx).powf(0.6) + ((y - cy).abs() / ry).powf(0.6) <= 1.0
                })
            }
            _ => return None,
        })
    }

    fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
        let (vx, vy) = (b.0 - a.0, b.1 - a.1);
        let length = vx * vx + vy * vy;
        let t = if length > 0.0 {
            (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (cx, cy) = (a.0 + t * vx, a.1 + t * vy);
        ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
    }

    /// Samples per cell along each side, as the Mac samples a symbol.
    const SAMPLES: usize = 4;

    /// A hand-drawn symbol, sampled onto a canvas `columns` cells square,
    /// scaled from the 42-cell design.
    pub fn hand_drawn_mask(symbol: &str, columns: usize) -> Option<Mask> {
        hand_drawn_inside(symbol, 0.0, 0.0)?;
        let scale = 42.0 / columns as f64;
        let mut coverage = vec![0.0; columns * columns];
        for row in 0..columns {
            for column in 0..columns {
                let mut inside = 0;
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let x = (column as f64 + (sx as f64 + 0.5) / SAMPLES as f64) * scale;
                        let y = (row as f64 + (sy as f64 + 0.5) / SAMPLES as f64) * scale;
                        if hand_drawn_inside(symbol, x, y) == Some(true) {
                            inside += 1;
                        }
                    }
                }
                coverage[row * columns + column] = inside as f32 / (SAMPLES * SAMPLES) as f32;
            }
        }
        Some(Mask { columns, coverage })
    }

    /// The symbols a hero may name: the Mac's two drawn ones and Linux's
    /// four hand-drawn ones.
    pub const SYMBOLS: [&str; 6] = [
        "cpu",
        "chart.bar.xaxis",
        "scope",
        "sparkles",
        "bolt",
        "globe",
    ];

    /// The hero glyph's mask for `symbol`, on the hero canvas; `None` for a
    /// symbol neither the Mac nor this file draws by hand.
    pub fn hero_mask(symbol: &str) -> Option<Mask> {
        let columns = (card::canvas_side() / card::CELL).round() as usize;
        drawn_mask(symbol, columns).or_else(|| hand_drawn_mask(symbol, columns))
    }

    /// One lit cell: where, which colour, and whether it is a bright one.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Lit {
        pub column: usize,
        pub row: usize,
        pub shade: usize,
        pub bright: bool,
    }

    /// The cells lit at `time`, with their colour and brightness.
    pub fn lit(mask: &Mask, time: f64, colours: usize) -> Vec<Lit> {
        let n = mask.columns;
        let area = (n as f64 * card::CELL, n as f64 * card::CELL);
        let mut out = Vec::new();
        for row in 0..n {
            for column in 0..n {
                if !is_lit(mask.coverage[row * n + column], column, row, n, n) {
                    continue;
                }
                let x = column as f64 * card::CELL + card::CELL / 2.0;
                let y = row as f64 * card::CELL + card::CELL / 2.0;
                let distance = field::distance(x, y, area, (1.0, 1.0));
                out.push(Lit {
                    column,
                    row,
                    shade: field::shade(column, row, time, colours),
                    bright: field::level(distance, time) > 0.28,
                });
            }
        }
        out
    }

    /// The glyph's cells at `time`, its canvas's top left at the origin:
    /// one fill per colour and brightness, so a frame is a handful of fills.
    pub fn draw(cr: &cairo::Context, mask: &Mask, palette: &[Rgb], time: f64) {
        if palette.is_empty() {
            return;
        }
        // A one-point gap, as the pixel field's.
        let side = (card::CELL - 1.0).max(1.0);
        let cells = lit(mask, time, palette.len());
        for shade in 0..palette.len() {
            for bright in [false, true] {
                let mut any = false;
                for cell in cells
                    .iter()
                    .filter(|c| c.shade == shade && c.bright == bright)
                {
                    cr.rectangle(
                        cell.column as f64 * card::CELL,
                        cell.row as f64 * card::CELL,
                        side,
                        side,
                    );
                    any = true;
                }
                if any {
                    let alpha = if bright {
                        card::BRIGHT_CELLS
                    } else {
                        card::DIM_CELLS
                    };
                    palette[shade].source(cr, alpha);
                    let _ = cr.fill();
                }
            }
        }
    }

    /// The hero's graphic on a card of `width` by `height`: the glyph's
    /// canvas centred on `card::symbol_centre`, tilted by `card::ANGLE`.
    pub fn paint_symbol(
        cr: &cairo::Context,
        width: f64,
        height: f64,
        mask: &Mask,
        dark: bool,
        time: f64,
    ) {
        let side = mask.columns as f64 * card::CELL;
        let (cx, cy) = card::symbol_centre(width, height);
        let _ = cr.save();
        cr.translate(cx, cy);
        cr.rotate(card::ANGLE.to_radians());
        cr.translate(-side / 2.0, -side / 2.0);
        draw(cr, mask, &super::tints(dark), time);
        let _ = cr.restore();
    }
}

/// The Insights hero's graphic (`HomeInsightsGraphic`): today's tokens per
/// tool as bars, the longest first, cut off by the card's bottom edge.
pub mod bars {
    /// The most tools drawn, each bar's width, the tallest bar, the gap
    /// between bars, and the name's column (`barWidth + 22`).
    pub const TOOL_LIMIT: usize = 4;
    pub const BAR_WIDTH: f64 = 30.0;
    pub const MAX_BAR_HEIGHT: f64 = 190.0;
    pub const SHORTEST: f64 = 8.0;
    pub const SPACING: i32 = 14;
    pub const NAME_WIDTH: i32 = 52;
    /// The name over each bar, and the gap between them.
    pub const NAME_INK: f64 = 0.85;
    pub const NAME_GAP: i32 = super::super::style::brand_tokens::glass::space::S2;
    /// How far the whole graphic is pushed right and down.
    pub const OFFSET: (f64, f64) = (30.0, 40.0);
    /// The bar's corner and its ink, top to bottom.
    pub const RADIUS: f64 = 6.0;
    pub const INK_TOP: f64 = 0.95;
    pub const INK_BOTTOM: f64 = 0.45;

    /// One tool's figure for today, as the daemon's glance gives it.
    #[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
    pub struct GlanceTool {
        pub tool: String,
        #[serde(default)]
        pub tokens: Option<u64>,
    }

    /// `insights_glance`'s answer, the parts the hero reads.
    #[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
    pub struct Glance {
        #[serde(default)]
        pub enabled: bool,
        #[serde(default)]
        pub readable: Option<bool>,
        #[serde(default)]
        pub tools: Option<Vec<GlanceTool>>,
    }

    /// The bars: tools with known, positive tokens, the most first, at most
    /// `TOOL_LIMIT`. None for a disabled or unreadable glance.
    pub fn bars(glance: Option<&Glance>) -> Vec<(String, u64)> {
        let Some(glance) = glance else {
            return Vec::new();
        };
        if !glance.enabled || glance.readable == Some(false) {
            return Vec::new();
        }
        let mut bars: Vec<(String, u64)> = glance
            .tools
            .iter()
            .flatten()
            .filter_map(|tool| Some((tool.tool.clone(), tool.tokens?)))
            .filter(|(_, tokens)| *tokens > 0)
            .collect();
        bars.sort_by(|a, b| b.1.cmp(&a.1));
        bars.truncate(TOOL_LIMIT);
        bars
    }

    /// A bar's height: its share of the longest, never under `SHORTEST`.
    pub fn height(tokens: u64, most: u64) -> f64 {
        if most == 0 {
            return SHORTEST;
        }
        (MAX_BAR_HEIGHT * tokens as f64 / most as f64).max(SHORTEST)
    }

    /// The whole graphic's width for `count` bars.
    pub fn width(count: usize) -> i32 {
        if count == 0 {
            return 0;
        }
        count as i32 * NAME_WIDTH + (count as i32 - 1) * SPACING
    }

    /// One bar: heroInk fading down, its corners round.
    pub fn paint_bar(cr: &gtk::cairo::Context, width: f64, height: f64) {
        let ink = super::Rgb::hex(super::tokens::HERO_INK);
        let gradient = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, height);
        gradient.add_color_stop_rgba(0.0, ink.0, ink.1, ink.2, INK_TOP);
        gradient.add_color_stop_rgba(1.0, ink.0, ink.1, ink.2, INK_BOTTOM);
        super::rounded(cr, 0.0, 0.0, width, height, RADIUS);
        let _ = cr.set_source(&gradient);
        let _ = cr.fill();
    }
}

/// Today's total, as the Mac's compact figure (`.notation(.compactName)` in
/// English): whole below a thousand, one decimal under ten of a unit,
/// rounded half to even, then K, M, B and T.
pub fn compact(value: u64) -> String {
    if value < 1000 {
        return value.to_string();
    }
    const UNITS: [(u64, &str); 4] = [
        (1_000, "K"),
        (1_000_000, "M"),
        (1_000_000_000, "B"),
        (1_000_000_000_000, "T"),
    ];
    let mut index = UNITS
        .iter()
        .rposition(|(size, _)| value >= *size)
        .unwrap_or(0);
    loop {
        let (size, suffix) = UNITS[index];
        let last = index + 1 == UNITS.len();
        if (value as u128) < 10 * size as u128 {
            let tenths = half_even(value as u128 * 10, size as u128);
            if tenths >= 10_000 && !last {
                index += 1;
                continue;
            }
            return if tenths % 10 == 0 {
                format!("{}{suffix}", tenths / 10)
            } else {
                format!("{}.{}{suffix}", tenths / 10, tenths % 10)
            };
        }
        let whole = half_even(value as u128, size as u128);
        if whole >= 1000 && !last {
            index += 1;
            continue;
        }
        return format!("{}{suffix}", grouped(whole));
    }
}

/// Thousands separated by commas, as the compact figure's last unit is.
fn grouped(value: u128) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn half_even(numerator: u128, denominator: u128) -> u128 {
    let (quotient, remainder) = (numerator / denominator, numerator % denominator);
    match (remainder * 2).cmp(&denominator) {
        std::cmp::Ordering::Less => quotient,
        std::cmp::Ordering::Greater => quotient + 1,
        std::cmp::Ordering::Equal => quotient + (quotient & 1),
    }
}

/// NEAR AI credit arriving on the Balance card (`BalanceArrival`).
pub mod arrival {
    /// What the card last drew, or draws now.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub struct Reading {
        pub received_nanos: Option<i64>,
        pub remaining_nanos: Option<i64>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Effect {
        None,
        /// Count the figure up, with the coins.
        CountUp {
            from: i64,
            to: i64,
        },
        /// The coins alone: under reduced motion, or no figure moved.
        Highlight,
    }

    /// Whether to mark an arrival: only when the daemon's received total
    /// grew past what was last shown, never from the balance, which
    /// spending also moves.
    pub fn effect(shown: Option<Reading>, now: Reading, reduce_motion: bool) -> Effect {
        let (Some(before), Some(after)) =
            (shown.and_then(|s| s.received_nanos), now.received_nanos)
        else {
            return Effect::None;
        };
        if after <= before {
            return Effect::None;
        }
        if reduce_motion {
            return Effect::Highlight;
        }
        match (shown.and_then(|s| s.remaining_nanos), now.remaining_nanos) {
            (Some(from), Some(to)) if from != to => Effect::CountUp { from, to },
            _ => Effect::Highlight,
        }
    }

    /// How many frames the count runs, and each frame's length.
    pub const FRAMES: usize = 24;
    pub const FRAME_MS: u64 = 50;
    /// How long the still coins stay under reduced motion.
    pub const HOLD_MS: u64 = 1600;

    /// The figures a count passes through, one per frame, ending exactly on
    /// `to`.
    pub fn count_steps(from: i64, to: i64, frames: usize) -> Vec<i64> {
        if frames == 0 {
            return vec![to];
        }
        let delta = to as f64 - from as f64;
        let mut steps: Vec<i64> = (1..frames)
            .map(|frame| from + (delta * frame as f64 / frames as f64).floor() as i64)
            .collect();
        steps.push(to);
        steps
    }
}

/// Gold coins bursting from the Balance card (`GlassCoinBurst`).
pub mod coins {
    use super::Rgb;
    use gtk::cairo;

    /// How long a burst lasts, how many coins, and how far past the card
    /// they may fly.
    pub const DURATION: f64 = 1.7;
    pub const COUNT: usize = 44;
    pub const SPILL: f64 = 120.0;
    /// The moment drawn still under reduced motion.
    pub const STILL_AT: f64 = 0.45;
    pub const DRAG: f64 = 3.4;
    pub const GRAVITY: f64 = 50.0;
    pub const SQUASH: f64 = 0.72;
    pub const FADE_FROM: f64 = 0.55;
    pub const FLASH_DURATION: f64 = 0.28;
    pub const FLASH_RADIUS: f64 = 46.0;
    /// The room past the card's right side, inside what holds Home.
    pub const EDGE_ROOM: f64 = super::super::style::brand_tokens::glass::space::S3 as f64;

    /// The coin's golds and the sparks it trails: the Mac's own, which no
    /// theme changes.
    pub const GOLD_LIGHT: &str = "#ffe58a";
    pub const GOLD: &str = "#f5b921";
    pub const GOLD_DEEP: &str = "#c58a12";
    pub const GOLD_RIM: &str = "#8f5e0a";
    pub const SPARK: &str = "#ffd45c";

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Coin {
        pub angle: f64,
        pub speed: f64,
        pub diameter: f64,
        pub spin: f64,
        pub phase: f64,
        pub tilt: f64,
        pub delay: f64,
    }

    /// A repeatable number in 0..1 for `index` and `salt`.
    pub fn noise(index: usize, salt: f64) -> f64 {
        let x = (index as f64 * 12.9898 + salt * 78.233).sin() * 43758.5453;
        x - x.floor()
    }

    pub fn coin(index: usize) -> Coin {
        let slot = index as f64 / COUNT as f64;
        Coin {
            angle: (slot + (noise(index, 1.0) - 0.5) / COUNT as f64) * 2.0 * std::f64::consts::PI,
            speed: 220.0 + noise(index, 2.0) * 200.0,
            diameter: 3.5 + noise(index, 3.0) * 2.5,
            spin: 1.4 + noise(index, 4.0) * 2.2,
            phase: noise(index, 5.0),
            tilt: (noise(index, 6.0) - 0.5) * 1.2,
            delay: noise(index, 7.0) * 0.08,
        }
    }

    pub fn coins() -> Vec<Coin> {
        (0..COUNT).map(coin).collect()
    }

    /// Where a coin is `t` seconds in, from the centre.
    pub fn offset(coin: &Coin, t: f64, sideways: f64) -> (f64, f64) {
        let cosine = coin.angle.cos();
        let flown = (t - coin.delay).max(0.0);
        let reach = coin.speed * (1.0 - (-DRAG * flown).exp()) / DRAG;
        let fall = 0.5 * GRAVITY * flown * flown;
        (
            cosine * reach * if cosine > 0.0 { sideways } else { 1.0 },
            coin.angle.sin() * reach * SQUASH + fall,
        )
    }

    /// How much of its reach a coin flies right when the edge that cuts it
    /// off is `room` from the centre.
    pub fn sideways(room: f64) -> f64 {
        let all = coins();
        let farthest = all.iter().map(|c| c.speed).fold(0.0, f64::max) / DRAG;
        let widest = all.iter().map(|c| c.diameter).fold(0.0, f64::max);
        ((room - widest) / farthest).clamp(0.0, 1.0)
    }

    /// How much of the coins shows `t` seconds in.
    pub fn opacity(t: f64) -> f64 {
        let from = DURATION * FADE_FROM;
        if t <= from {
            return 1.0;
        }
        (1.0 - (t - from) / (DURATION - from)).max(0.0)
    }

    /// The burst `t` seconds in, round `centre`, with `room` between the
    /// centre and the edge that would cut the coins off on the right.
    pub fn draw(cr: &cairo::Context, centre: (f64, f64), t: f64, flash: bool, room: f64) {
        if !(0.0..DURATION).contains(&t) {
            return;
        }
        let sideways = sideways(room);
        if flash && t < FLASH_DURATION {
            let left = 1.0 - t / FLASH_DURATION;
            let radius = FLASH_RADIUS * (0.4 + 0.6 * (t / FLASH_DURATION));
            let glow =
                cairo::RadialGradient::new(centre.0, centre.1, 0.0, centre.0, centre.1, radius);
            let (light, gold) = (Rgb::hex(GOLD_LIGHT), Rgb::hex(GOLD));
            glow.add_color_stop_rgba(0.0, light.0, light.1, light.2, 0.75 * left);
            glow.add_color_stop_rgba(0.5, gold.0, gold.1, gold.2, 0.3 * left);
            glow.add_color_stop_rgba(1.0, gold.0, gold.1, gold.2, 0.0);
            let _ = cr.set_source(&glow);
            cr.arc(centre.0, centre.1, radius, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();
        }
        let alpha = opacity(t);
        let _ = cr.push_group();
        let spark = Rgb::hex(SPARK);
        for coin in coins() {
            let flown = t - coin.delay;
            if flown <= 0.0 {
                continue;
            }
            let (dx, dy) = offset(&coin, t, sideways);
            if flown < 0.55 {
                for step in 1..=3 {
                    let (bx, by) = offset(&coin, t - step as f64 * 0.035, sideways);
                    let dot = coin.diameter * (0.42 - step as f64 * 0.08);
                    spark.source(cr, (0.75 - step as f64 * 0.2) * (1.0 - flown / 0.55));
                    cr.arc(
                        centre.0 + bx,
                        centre.1 + by,
                        dot / 2.0,
                        0.0,
                        2.0 * std::f64::consts::PI,
                    );
                    let _ = cr.fill();
                }
            }
            draw_coin(
                cr,
                &coin,
                (centre.0 + dx, centre.1 + dy),
                coin.phase + coin.spin * flown,
            );
        }
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(alpha);
    }

    /// One coin turning end over end.
    fn draw_coin(cr: &cairo::Context, coin: &Coin, at: (f64, f64), turn: f64) {
        let facing = (turn * 2.0 * std::f64::consts::PI).cos();
        let width = coin.diameter * facing.abs().max(0.2);
        let d = coin.diameter;
        let _ = cr.save();
        cr.translate(at.0, at.1);
        cr.rotate(coin.tilt);
        let ellipse = |cr: &cairo::Context| {
            let _ = cr.save();
            cr.scale(width / 2.0, d / 2.0);
            cr.arc(0.0, 0.0, 1.0, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.restore();
        };
        let gradient = cairo::LinearGradient::new(-width / 2.0, -d / 2.0, width / 2.0, d / 2.0);
        let colours: &[&str] = if facing > 0.0 {
            &[GOLD_LIGHT, GOLD, GOLD_DEEP]
        } else {
            &[GOLD, GOLD_DEEP]
        };
        for (index, hex) in colours.iter().enumerate() {
            let Rgb(r, g, b) = Rgb::hex(hex);
            gradient.add_color_stop_rgb(index as f64 / (colours.len() - 1) as f64, r, g, b);
        }
        ellipse(cr);
        let _ = cr.set_source(&gradient);
        let _ = cr.fill_preserve();
        Rgb::hex(GOLD_RIM).source(cr, 1.0);
        cr.set_line_width(0.6);
        let _ = cr.stroke();
        if facing.abs() > 0.92 {
            let glint = d * 0.32;
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            cr.arc(
                -d * 0.28 + glint / 2.0,
                -d * 0.3 + glint / 2.0,
                glint / 2.0,
                0.0,
                2.0 * std::f64::consts::PI,
            );
            let _ = cr.fill();
        }
        let _ = cr.restore();
    }
}

#[cfg(test)]
#[path = "hero_art_tests.rs"]
mod tests;
