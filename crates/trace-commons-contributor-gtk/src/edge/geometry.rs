//! The rail's and the panel's measures, from the Mac's.
//!
//! The rail's are `EdgeRailGeometry` (`TCDesign/Components/EdgeRail.swift`),
//! every one a glass token; the panel's are `MenuBarGlassPanel`'s. A
//! throwaway probe compiled the Mac's own `EdgeRailGeometry` against its
//! generated tokens on 2026-11-18 and printed, for a 1920 x 1080 screen,
//! open (1436, 200, 484, 680) and closed (1916, 420, 4, 240); the tests
//! below pin the same numbers.

use crate::ui::style::brand_tokens::glass::{size, space};

/// The pointer may leave the open rail this long, as it does crossing from
/// an icon to its peek, before the rail closes (`EdgeRailView`'s 300 ms).
pub const CLOSE_DELAY_MS: u64 = 300;

/// How long after the pointer leaves the rail it closes: the delay while it
/// is open, and nothing at all -- no timer -- while it is closed, so a
/// closed rail runs nothing.
pub fn close_delay_on_leave(open: bool) -> Option<u64> {
    open.then_some(CLOSE_DELAY_MS)
}

/// Room left of the peek for its edge and shadow to draw into.
pub const SHADOW_ROOM: i32 = space::EDGE_RAIL_GAP + space::S2;

/// The open rail's width: shadow room, the peek, the gap, the icon rail and
/// its inset from the screen's edge.
pub const OPEN_WIDTH: i32 = SHADOW_ROOM
    + size::EDGE_RAIL_PEEK_WIDTH
    + space::EDGE_RAIL_GAP
    + size::EDGE_RAIL_WIDTH
    + space::EDGE_RAIL_INSET;

/// A rectangle in a monitor's logical pixels, y down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The rail's size on a screen whose usable height is `visible_height`:
/// the thin zone while closed, the rail and its peek while open, never
/// taller than the screen.
pub fn rail_size(open: bool, visible_height: i32) -> (i32, i32) {
    let (width, height) = if open {
        (OPEN_WIDTH, size::EDGE_RAIL_OPEN_HEIGHT)
    } else {
        (size::EDGE_RAIL_ZONE_WIDTH, size::EDGE_RAIL_ZONE_HEIGHT)
    };
    (width, height.min(visible_height.max(0)))
}

/// The rail's frame on a screen whose usable area is `visible`: against its
/// right edge, centred vertically (`EdgeRailGeometry.frame`). A layer
/// surface anchored to the right edge alone lands here by itself; the GNOME
/// Shell extension computes the same.
pub fn rail_frame(open: bool, visible: Rect) -> Rect {
    let (width, height) = rail_size(open, visible.height);
    Rect {
        x: visible.x + visible.width - width,
        y: visible.y + (visible.height - height) / 2,
        width,
        height,
    }
}

/// The panel's width: "a quarter narrower than the handoff's 380" (owner,
/// 2026-10-21).
pub const PANEL_WIDTH: i32 = 285;
/// The panel's padding (`space.s5`) and the gap between its rows (`s4`).
pub const PANEL_PADDING: i32 = space::S5;
pub const PANEL_SPACING: i32 = space::S4;
/// The panel's distance from the screen edges and the tray it opens under.
pub const PANEL_MARGIN: i32 = space::S4;

/// A peek's and the panel's graph height: the bars and their day labels.
pub const GRAPH_HEIGHT: i32 = 110;
/// The Private AI peek's map height.
pub const MAP_HEIGHT: i32 = 190;
/// The side of the N mark on the rail's app tile, the tiles' glyph size.
pub const MARK_SIDE: i32 = 20;

/// The Traces peek's folder list: a row's pitch, and at most seven rows
/// before it scrolls (`EdgeRailFolderListHeights`).
pub const FOLDER_ROW_PITCH: i32 = size::LIST_ROW + 2;
pub const FOLDER_LIST_MAX_ROWS: i32 = 7;

/// The folder list's height for `rows` rows: all of them up to seven.
pub fn folder_list_height(rows: usize) -> i32 {
    let rows = i32::try_from(rows).unwrap_or(i32::MAX);
    rows.min(FOLDER_LIST_MAX_ROWS) * FOLDER_ROW_PITCH
}

/// Which edges a layer-shell panel is anchored to, and its margins from
/// them, in the output's usable area (a panel bar's exclusive zone is
/// already outside it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelAnchor {
    pub top: bool,
    pub left: bool,
    /// Distance from the anchored vertical edge (top or bottom).
    pub vertical: i32,
    /// Distance from the anchored horizontal edge (left or right).
    pub horizontal: i32,
}

/// Where the panel goes for a tray click at `click`, on `monitor` (its
/// place and size in desktop coordinates, logical pixels, y down; the
/// click in the same space, as tray hosts send it).
///
/// The Mac's popover hangs under its menu-bar item. A tray host passes the
/// pointer's position with `Activate`: the panel opens on the click's side
/// of the screen -- under a bar at the top, over one at the bottom --
/// centred on the click and kept inside the screen. Many hosts pass (0, 0)
/// or nothing at all; then it opens in the top right corner, where a tray
/// most often is.
pub fn panel_anchor(click: Option<(i32, i32)>, monitor: Rect) -> PanelAnchor {
    let corner = PanelAnchor {
        top: true,
        left: false,
        vertical: PANEL_MARGIN,
        horizontal: PANEL_MARGIN,
    };
    let Some((x, y)) = click.filter(|&(x, y)| (x, y) != (0, 0)) else {
        return corner;
    };
    // The anchor's margins are the monitor's own.
    let (x, y) = (x - monitor.x, y - monitor.y);
    if x < 0 || y < 0 || x >= monitor.width || y >= monitor.height {
        // A click outside this monitor: another output's, or a host that
        // passes something else in these numbers.
        return corner;
    }
    let top = y < monitor.height / 2;
    let widest = (monitor.width - PANEL_WIDTH - PANEL_MARGIN).max(PANEL_MARGIN);
    let left = (x - PANEL_WIDTH / 2).clamp(PANEL_MARGIN, widest);
    PanelAnchor {
        top,
        left: true,
        vertical: PANEL_MARGIN,
        horizontal: left,
    }
}

/// Which of `monitors` (desktop coordinates) a tray click landed on, so the
/// panel opens on that monitor. `None` with no position, a host's (0, 0),
/// or a point on none of them.
pub fn monitor_for_click(monitors: &[Rect], click: Option<(i32, i32)>) -> Option<usize> {
    let (x, y) = click.filter(|&point| point != (0, 0))?;
    monitors.iter().position(|monitor| {
        x >= monitor.x
            && y >= monitor.y
            && x < monitor.x + monitor.width
            && y < monitor.y + monitor.height
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn the_open_rail_is_the_macs_484_by_680() {
        assert_eq!(SHADOW_ROOM, 20);
        assert_eq!(OPEN_WIDTH, 484);
        assert_eq!(
            rail_frame(true, SCREEN),
            Rect {
                x: 1436,
                y: 200,
                width: 484,
                height: 680
            }
        );
    }

    #[test]
    fn the_closed_rail_covers_four_pixels_of_edge() {
        assert_eq!(
            rail_frame(false, SCREEN),
            Rect {
                x: 1916,
                y: 420,
                width: 4,
                height: 240
            }
        );
    }

    #[test]
    fn the_rail_never_outgrows_a_short_screen() {
        // The Mac's own short-screen case: 400 high, 25 down.
        let short = Rect {
            x: 0,
            y: 25,
            width: 1024,
            height: 400,
        };
        let frame = rail_frame(true, short);
        assert_eq!((frame.y, frame.height), (25, 400));
        assert_eq!(frame.x + frame.width, 1024);
    }

    #[test]
    fn the_rail_measures_are_the_tokens() {
        assert_eq!(size::EDGE_RAIL_WIDTH, 56);
        assert_eq!(size::EDGE_RAIL_TILE, 44);
        assert_eq!(size::EDGE_RAIL_RULE_WIDTH, 28);
        assert_eq!(size::EDGE_RAIL_PEEK_WIDTH, 380);
        assert_eq!(size::EDGE_RAIL_HANDLE_WIDTH, 3);
        assert_eq!(size::EDGE_RAIL_HANDLE_HEIGHT, 72);
        assert_eq!(space::EDGE_RAIL_INSET, 12);
        assert_eq!(space::EDGE_RAIL_GAP, 16);
        assert_eq!(crate::ui::style::brand_tokens::glass::motion::SLIDE_MS, 250);
    }

    #[test]
    fn the_panel_is_the_macs_285_wide_with_its_padding() {
        assert_eq!(PANEL_WIDTH, 285);
        assert_eq!((PANEL_PADDING, PANEL_SPACING), (10, 8));
        assert_eq!((GRAPH_HEIGHT, MAP_HEIGHT), (110, 190));
        assert_eq!(CLOSE_DELAY_MS, 300);
    }

    #[test]
    fn only_an_open_rail_starts_a_close_timer() {
        assert_eq!(close_delay_on_leave(true), Some(300));
        assert_eq!(close_delay_on_leave(false), None);
    }

    #[test]
    fn the_folder_list_holds_seven_rows_then_scrolls() {
        assert_eq!(FOLDER_ROW_PITCH, 42);
        assert_eq!(folder_list_height(0), 0);
        assert_eq!(folder_list_height(3), 126);
        assert_eq!(folder_list_height(7), 294);
        assert_eq!(folder_list_height(40), 294);
    }

    #[test]
    fn a_click_on_a_top_bar_opens_the_panel_under_it_centred() {
        let anchor = panel_anchor(Some((1700, 12)), FIRST);
        assert!(anchor.top && anchor.left);
        assert_eq!(anchor.horizontal, 1700 - 142);
        assert_eq!(anchor.vertical, PANEL_MARGIN);
    }

    #[test]
    fn a_click_on_a_bottom_bar_opens_it_above() {
        let anchor = panel_anchor(Some((400, 1070)), FIRST);
        assert!(!anchor.top && anchor.left);
        assert_eq!(anchor.horizontal, 400 - 142);
    }

    #[test]
    fn the_panel_stays_on_screen_at_either_side() {
        let right = panel_anchor(Some((1915, 5)), FIRST);
        assert_eq!(right.horizontal, 1920 - PANEL_WIDTH - PANEL_MARGIN);
        let left = panel_anchor(Some((3, 5)), FIRST);
        assert_eq!(left.horizontal, PANEL_MARGIN);
    }

    #[test]
    fn no_coordinates_open_it_in_the_top_right_corner() {
        let corner = PanelAnchor {
            top: true,
            left: false,
            vertical: PANEL_MARGIN,
            horizontal: PANEL_MARGIN,
        };
        assert_eq!(panel_anchor(None, FIRST), corner);
        // What many hosts send for "no position".
        assert_eq!(panel_anchor(Some((0, 0)), FIRST), corner);
        // Another monitor's coordinates.
        assert_eq!(panel_anchor(Some((2500, 10)), FIRST), corner);
        assert_eq!(panel_anchor(Some((-5, 10)), FIRST), corner);
    }

    /// A second monitor to the right of the first, as GDK lays them out
    /// in desktop coordinates.
    const SECOND: Rect = Rect {
        x: 1920,
        y: 0,
        width: 2560,
        height: 1440,
    };

    #[test]
    fn a_click_on_a_second_monitor_opens_the_panel_on_it() {
        // The tray's coordinates are the desktop's; the anchor is in the
        // monitor's own.
        let anchor = panel_anchor(Some((2100, 10)), SECOND);
        assert!(anchor.top && anchor.left);
        assert_eq!(anchor.horizontal, 2100 - 1920 - 142);
        let bottom = panel_anchor(Some((4470, 1430)), SECOND);
        assert!(!bottom.top);
        assert_eq!(bottom.horizontal, 2560 - PANEL_WIDTH - PANEL_MARGIN);
        // The first monitor's click is not this monitor's.
        assert!(!panel_anchor(Some((1700, 12)), SECOND).left);
    }

    #[test]
    fn the_panel_opens_on_the_monitor_under_the_click() {
        let monitors = [FIRST, SECOND];
        assert_eq!(monitor_for_click(&monitors, Some((2100, 10))), Some(1));
        assert_eq!(monitor_for_click(&monitors, Some((1919, 10))), Some(0));
        assert_eq!(monitor_for_click(&monitors, Some((1920, 10))), Some(1));
        // No position, a host's (0, 0), or a point on no monitor: the
        // caller's own monitor decides.
        assert_eq!(monitor_for_click(&monitors, None), None);
        assert_eq!(monitor_for_click(&monitors, Some((0, 0))), None);
        assert_eq!(monitor_for_click(&monitors, Some((9000, 10))), None);
    }
}
