//! The tray icon's states: the Mac's menu-bar item, drawn for a
//! `StatusNotifierItem`.
//!
//! A port of `GlassMenuBarStrip` (`macos/Sources/TCDesign/Components/MenuBarPanel.swift`)
//! and the parts of `MenuPanelStatus` and `NudgeSurface.mark` that decide
//! what it draws (`Views/Monitor/MenuBarGlassPanel.swift`,
//! `TCShellCore/NudgeSurface.swift`): the NEAR AI N in the logo kit's single
//! ink, the decisions-owed badge cut out of that ink (99+ past 99), the news
//! ring in the badge's empty slot on a live strip, and the halo around a
//! drawn badge while some of the decisions owed are idle sessions. The ring
//! and halo are the core's: it lights `status.nudge.mark` only after its own
//! mute (`menu_bar_mark_enabled`), cooldowns and news window, so this module
//! never decides to light one, only whether a lit one has its slot.
//!
//! Not drawn here: the 24-hour bars (the Mac leaves them out until a
//! machine has a few days of data, and nothing on Linux reads the columns
//! yet) and the attention capsule, which the owner's decision for the tray
//! does not name. The condition still keeps the ring off the slot whenever
//! the Mac would draw that capsule there.
//!
//! ## Layout
//!
//! Every length is in the Mac's points on its 22-point item, scaled to the
//! pixmap. The Mac's item is a row: the 14-point mark, then the slot. A
//! tray icon is a square cell, so the mark keeps its 14 points centred in a
//! 22-point square and the slot moves onto its bottom-right corner, the
//! badge or the ring touching the cell's edges. The Mac pads the item by
//! the halo's reach so its halo is never clipped; in a 22-point square
//! that padding would push the badge over the middle of the N, so here the
//! badge stays on the corner and the cell's edge cuts the halo on its two
//! outer sides. The mark is cut away under the slot with the Mac's
//! `markOnlyGap` (2) to spare, as the Mac's badge clears the mark rather
//! than touching it, or to the halo's outer edge while a halo is drawn. The badge's figure is a hand-drawn pixel face
//! (no font is guaranteed in a tray host), cut out of the badge as the Mac
//! cuts its figure out.

use gtk::cairo;

use crate::model::{Nudge, NudgeMarkText};
use crate::ui::mark::{self, Scheme};

/// What the strip can say about the data behind it
/// (`GlassMenuBarStrip.Condition`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Live,
    Paused,
    Attention,
    Unavailable,
    Loading,
}

/// The re-engagement mark (`GlassMenuBarStrip.Mark`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    None,
    /// Something new to look at: a small hollow ring in the badge's slot.
    News,
    /// Some decisions owed are idle sessions: a halo around the badge.
    Ready,
}

/// Everything the icon draws. Equal inputs draw the same pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strip {
    pub condition: Condition,
    pub badge: Option<u64>,
    pub mark: Mark,
}

impl Strip {
    /// Before the first status reply: nothing known yet.
    pub const LOADING: Strip = Strip {
        condition: Condition::Loading,
        badge: None,
        mark: Mark::None,
    };
}

/// The item's height, and the side of the square the tray draws in.
pub const HEIGHT: f64 = 22.0;
/// The mark's side.
pub const MARK_SIZE: f64 = 14.0;
/// The badge's height and least width, its figure's size and inset.
pub const BADGE_SIZE: f64 = 11.0;
pub const BADGE_GLYPH: f64 = 7.0;
pub const BADGE_INSET: f64 = 2.5;
/// The halo: inset past the badge's edge, line width, and so its reach.
pub const HALO_INSET: f64 = 2.5;
pub const HALO_LINE: f64 = 2.0;
pub const HALO_REACH: f64 = HALO_INSET + HALO_LINE / 2.0;
/// The news ring: diameter and line width.
pub const RING_SIZE: f64 = 9.0;
pub const RING_LINE: f64 = 1.5;
/// How far the slot clears the mark (`markOnlyGap`).
pub const MARK_GAP: f64 = 2.0;
/// The badge figure's pixel face: cells across and down for one character,
/// and the cell gap between characters. Five cells down is the cap height
/// of the Mac's 7-point bold figure.
pub const FACE_COLUMNS: usize = 3;
pub const FACE_ROWS: usize = 5;

/// The pixel sizes handed to the host. It picks the nearest.
pub const PIXMAP_SIZES: [i32; 7] = [16, 22, 24, 32, 44, 48, 64];

/// `MenuPanelStatus.condition`: never live unless everything is.
pub fn condition(
    decisions_owed: Option<u64>,
    unhealthy: bool,
    paused: bool,
    available: bool,
    stale: bool,
    status_answered: bool,
) -> Condition {
    if !available || stale {
        return Condition::Unavailable;
    }
    if unhealthy {
        return Condition::Attention;
    }
    if !status_answered {
        return Condition::Loading;
    }
    if decisions_owed.is_none() {
        return Condition::Attention;
    }
    if paused {
        Condition::Paused
    } else {
        Condition::Live
    }
}

/// `MenuPanelStatus.badge`: decisions owed when there are any.
pub fn badge(decisions_owed: Option<u64>) -> Option<u64> {
    decisions_owed.filter(|n| *n > 0)
}

/// `NudgeSurface.mark`: only `news` and `ready`, and neither while the icon
/// cannot vouch for what it shows.
pub fn lit_mark(nudge: Option<&Nudge>, available: bool) -> Mark {
    if !available {
        return Mark::None;
    }
    match nudge.and_then(|n| n.mark.as_deref()) {
        Some("news") => Mark::News,
        Some("ready") => Mark::Ready,
        _ => Mark::None,
    }
}

/// `GlassMenuBarStrip.shownBadge`: none while unavailable or loading.
pub fn shown_badge(badge: Option<u64>, condition: Condition) -> Option<u64> {
    if matches!(condition, Condition::Unavailable | Condition::Loading) {
        return None;
    }
    badge.filter(|n| *n > 0)
}

/// `GlassMenuBarStrip.shownMark`: the ring only with no badge on a live
/// strip, the halo only around a badge that is drawn.
pub fn shown_mark(mark: Mark, badge: Option<u64>, condition: Condition) -> Mark {
    let badge_shown = shown_badge(badge, condition).is_some();
    match mark {
        Mark::News if !badge_shown && condition == Condition::Live => Mark::News,
        Mark::Ready if badge_shown => Mark::Ready,
        _ => Mark::None,
    }
}

/// `GlassMenuBarStrip.badgeText`: two digits at most, 99+ past them.
pub fn badge_text(badge: u64) -> String {
    if badge > 99 {
        "99+".to_string()
    } else {
        badge.to_string()
    }
}

/// `MenuPanelStatus.markWords`: the core's words for the mark the icon
/// draws, or none while it draws none.
pub fn mark_words<'a>(
    nudge: Option<&'a Nudge>,
    available: bool,
    badge: Option<u64>,
    condition: Condition,
) -> Option<&'a NudgeMarkText> {
    let lit = lit_mark(nudge, available);
    if shown_mark(lit, badge, condition) == Mark::None {
        return None;
    }
    nudge?.mark_text.as_ref()
}

/// The pixel face of one badge character, rows top to bottom, a set bit
/// per filled cell from the left.
fn face(c: char) -> Option<[u8; FACE_ROWS]> {
    Some(match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        _ => return None,
    })
}

/// The figure's width in cells: three per character, one between them.
fn face_width(text: &str) -> usize {
    let n = text.chars().count();
    if n == 0 {
        0
    } else {
        n * FACE_COLUMNS + (n - 1)
    }
}

/// The badge's width for `text`: the figure and its inset on each side,
/// never narrower than it is tall.
pub fn badge_width(text: &str, cell: f64) -> f64 {
    (face_width(text) as f64 * cell + 2.0 * BADGE_INSET).max(BADGE_SIZE)
}

/// The slot's geometry, in points: the badge's rectangle (x, y, w, h), and
/// for the ring its centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub badge: Option<(f64, f64, f64, f64)>,
    pub halo: bool,
    pub ring: Option<(f64, f64)>,
}

/// Where the slot's contents sit for `strip`, with a figure cell of `cell`
/// points: the badge, or the ring, on the square's bottom-right corner. A
/// halo is drawn at its own inset around the badge and the cell's edge
/// cuts it on the two outer sides.
pub fn slot(strip: &Strip, cell: f64) -> Slot {
    let shown = shown_mark(strip.mark, strip.badge, strip.condition);
    if let Some(n) = shown_badge(strip.badge, strip.condition) {
        let w = badge_width(&badge_text(n), cell);
        return Slot {
            badge: Some((HEIGHT - w, HEIGHT - BADGE_SIZE, w, BADGE_SIZE)),
            halo: shown == Mark::Ready,
            ring: None,
        };
    }
    if shown == Mark::News {
        let outer = RING_SIZE / 2.0 + RING_LINE / 2.0;
        return Slot {
            badge: None,
            halo: false,
            ring: Some((HEIGHT - outer, HEIGHT - outer)),
        };
    }
    Slot {
        badge: None,
        halo: false,
        ring: None,
    }
}

fn capsule(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = (h / 2.0).min(w / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, FRAC_PI_2);
    cr.arc(x + r, y + r, r, FRAC_PI_2, PI + FRAC_PI_2);
    cr.close_path();
}

/// The accent the ring and halo are stroked in: `purpleText`, as the flat
/// theme resolves it.
pub fn accent(scheme: Scheme) -> &'static str {
    use crate::ui::style::brand_tokens::map;
    match scheme {
        Scheme::Light => map::light::PURPLE_TEXT.0,
        Scheme::Dark => map::dark::PURPLE_TEXT.0,
    }
}

/// Draws `strip` into a square `px` pixels across.
pub fn draw(cr: &cairo::Context, strip: &Strip, scheme: Scheme, px: f64) {
    let unit = px / HEIGHT;
    // The figure's cell: one point, snapped to whole pixels so the face is
    // crisp at every size the host asks for.
    let cell_px = unit.round().max(1.0);
    let cell = cell_px / unit;
    let slot = slot(strip, cell);
    let ink = mark::ink(scheme);

    let _ = cr.save();
    cr.scale(unit, unit);
    let inset = (HEIGHT - MARK_SIZE) / 2.0;
    mark::set_source(cr, ink);
    mark::fill_n(cr, (inset, inset), MARK_SIZE);

    // Clear the mark under the slot, with the gap to spare.
    cr.set_operator(cairo::Operator::DestOut);
    if let Some((x, y, w, h)) = slot.badge {
        // The halo's own outer edge, or the gap past the badge.
        let out = if slot.halo { HALO_REACH } else { MARK_GAP };
        capsule(cr, x - out, y - out, w + 2.0 * out, h + 2.0 * out);
        let _ = cr.fill();
    }
    if let Some((cx, cy)) = slot.ring {
        cr.arc(
            cx,
            cy,
            RING_SIZE / 2.0 + RING_LINE / 2.0 + MARK_GAP,
            0.0,
            std::f64::consts::TAU,
        );
        let _ = cr.fill();
    }
    cr.set_operator(cairo::Operator::Over);

    if let Some((x, y, w, h)) = slot.badge {
        mark::set_source(cr, ink);
        capsule(cr, x, y, w, h);
        let _ = cr.fill();
        // The figure, cut out so whatever is behind the icon shows through.
        let n = shown_badge(strip.badge, strip.condition).unwrap_or(0);
        let text = badge_text(n);
        let fw = face_width(&text) as f64 * cell;
        // Whole pixels, so every cell lands on the device grid.
        let left = ((x + (w - fw) / 2.0) * unit).round() / unit;
        let top = ((y + (h - FACE_ROWS as f64 * cell) / 2.0) * unit).round() / unit;
        cr.set_operator(cairo::Operator::DestOut);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        for (i, c) in text.chars().enumerate() {
            let Some(rows) = face(c) else { continue };
            let cx = left + (i * (FACE_COLUMNS + 1)) as f64 * cell;
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..FACE_COLUMNS {
                    if bits & (1 << (FACE_COLUMNS - 1 - col)) != 0 {
                        cr.rectangle(cx + col as f64 * cell, top + row as f64 * cell, cell, cell);
                    }
                }
            }
        }
        let _ = cr.fill();
        cr.set_operator(cairo::Operator::Over);
        if slot.halo {
            mark::set_source(cr, accent(scheme));
            cr.set_line_width(HALO_LINE);
            capsule(
                cr,
                x - HALO_INSET,
                y - HALO_INSET,
                w + 2.0 * HALO_INSET,
                h + 2.0 * HALO_INSET,
            );
            let _ = cr.stroke();
        }
    }
    if let Some((cx, cy)) = slot.ring {
        mark::set_source(cr, accent(scheme));
        cr.set_line_width(RING_LINE);
        cr.arc(cx, cy, RING_SIZE / 2.0, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
    }
    let _ = cr.restore();
}

/// `strip` as one `StatusNotifierItem` pixmap `px` pixels square: ARGB32
/// in network byte order, not premultiplied, as the protocol carries it.
pub fn pixmap(strip: &Strip, scheme: Scheme, px: i32) -> Option<(i32, i32, Vec<u8>)> {
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        draw(&cr, strip, scheme, f64::from(px));
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().ok()?;
    let mut out = Vec::with_capacity((px * px * 4) as usize);
    for y in 0..px as usize {
        for x in 0..px as usize {
            let i = y * stride + x * 4;
            let word = u32::from_ne_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
            out.extend_from_slice(&unpremultiply(word).to_be_bytes());
        }
    }
    Some((px, px, out))
}

/// Cairo's premultiplied ARGB word as a straight-alpha one.
fn unpremultiply(word: u32) -> u32 {
    let a = word >> 24;
    if a == 0 {
        return 0;
    }
    let channel = |shift: u32| {
        let c = (word >> shift) & 0xFF;
        ((c * 255 + a / 2) / a).min(255)
    };
    (a << 24) | (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

/// Every size the host may pick from.
pub fn pixmaps(strip: &Strip, scheme: Scheme) -> Vec<(i32, i32, Vec<u8>)> {
    PIXMAP_SIZES
        .iter()
        .filter_map(|px| pixmap(strip, scheme, *px))
        .collect()
}

#[cfg(test)]
#[path = "tray_icon_tests.rs"]
mod tests;
