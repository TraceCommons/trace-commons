//! The widths of the window's three panes, for one window width.
//!
//! A port of `GlassPaneLayout` in
//! `macos/Sources/TCDesign/Components/ThreePane.swift`, which is the
//! reference behaviour (spec 2026-11-16, "Logic is ported per platform").
//! Pure numbers, so the rules are tested without a display.
//!
//! - The main pane opens at [`MAIN_WIDTH`] and never goes below it.
//! - The inspector is a fixed width, two thirds of the main pane's default.
//! - Without the map, the main pane takes a resize of the window.
//! - With the map, the map takes a resize first, down to [`MAP_MIN_WIDTH`];
//!   past that the main pane gives up room, down to [`MAIN_WIDTH`].
//! - The divider between the main pane and the map moves room between them.
//! - A hidden pane reserves no space; shown panes are [`PANE_GAP`] apart.
//!
//! Deliberate platform differences from the Swift:
//!
//! - There is no `Transition`. The Mac animates the window's edge and lays
//!   the panes out for both compositions while it moves. A Wayland
//!   compositor resizes a window when it chooses to, so the GTK container
//!   lays out for whatever width it was actually given
//!   ([`PaneLayout::new`]), which is also the in-window fallback when the
//!   compositor leaves the window's size alone.
//! - There is no `windowFrame(onScreen:)`. A Wayland client cannot place
//!   its window, so keeping it on screen is the compositor's job.

/// `GlassTokens.Size.paneLeftWidth + 40`.
pub const MAIN_WIDTH: f64 = 440.0;
/// `GlassTokens.Size.mapWidth`.
pub const MAP_WIDTH: f64 = 440.0;
/// `GlassTokens.Size.mapMinWidth`.
pub const MAP_MIN_WIDTH: f64 = 360.0;
/// `round(MAIN_WIDTH * 2 / 3)`. The token `size.inspectorWidth` (300) is not
/// the value the Mac window uses.
pub const INSPECTOR_WIDTH: f64 = 293.0;
/// `GlassTokens.Space.paneGap`.
pub const PANE_GAP: f64 = 10.0;
/// `GlassTokens.Space.windowPadding`.
pub const WINDOW_PADDING: f64 = 0.0;
/// `GlassTokens.Size.mapBreakpoint`.
pub const MAP_BREAKPOINT: f64 = 1100.0;
/// `GlassTokens.Size.inspectorBreakpoint`.
pub const INSPECTOR_BREAKPOINT: f64 = 900.0;
/// `GlassTokens.Size.windowHeight`.
pub const WINDOW_HEIGHT: f64 = 614.0;
/// `GlassTokens.Size.windowMinHeight`.
pub const WINDOW_MIN_HEIGHT: f64 = 560.0;
/// The tallest the notice column above a tab's content grows before it
/// scrolls: a third of the window's least height, so at the window's
/// minimum the toolbar, the tabs and the tab's first lines stay in view
/// under any number of notices. Not a Mac token: the Mac's peeking stack
/// is one card tall (`GlassPopoverStack`).
pub const NOTICES_MAX_HEIGHT: f64 = WINDOW_MIN_HEIGHT / 3.0;
/// How far into the map the divider takes the pointer
/// (`GlassPaneLayout.dividerReach`). Not into the main pane: its scroller
/// runs along the edge beside the gap.
pub const DIVIDER_REACH: f64 = 4.0;

/// Which panes are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visibility {
    pub map: bool,
    pub inspector: bool,
}

impl Visibility {
    pub const fn new(map: bool, inspector: bool) -> Self {
        Self { map, inspector }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneLayout {
    pub main: f64,
    /// `None` when the map is not drawn.
    pub map: Option<f64>,
    /// `None` when the inspector is not drawn.
    pub inspector: Option<f64>,
}

impl PaneLayout {
    /// `main` is the main pane's kept width (the width the person dragged
    /// the divider to), which it keeps beside the map while the window has
    /// room for the map at its minimum.
    pub fn new(window_width: f64, shows: Visibility, main: f64) -> Self {
        let room = window_width - others_width(shows.inspector);
        let inspector = shows.inspector.then_some(INSPECTOR_WIDTH);
        if shows.map {
            let shared = room - PANE_GAP;
            let main = main
                .max(MAIN_WIDTH)
                .min(MAIN_WIDTH.max(shared - MAP_MIN_WIDTH));
            Self {
                main,
                map: Some((shared - main).max(0.0)),
                inspector,
            }
        } else {
            // Without the map, the main pane takes all the room there is.
            Self {
                main: MAIN_WIDTH.max(room),
                map: None,
                inspector,
            }
        }
    }

    pub fn visibility(&self) -> Visibility {
        Visibility::new(self.map.is_some(), self.inspector.is_some())
    }

    /// Where the map starts, for the divider's hit area.
    pub fn map_x(&self) -> f64 {
        WINDOW_PADDING + self.main + PANE_GAP
    }

    /// Where the inspector starts.
    pub fn inspector_x(&self) -> f64 {
        match self.map {
            Some(map) => self.map_x() + map + PANE_GAP,
            None => WINDOW_PADDING + self.main + PANE_GAP,
        }
    }
}

/// The window's width without the main pane, the map and the map's gap.
fn others_width(shows_inspector: bool) -> f64 {
    WINDOW_PADDING * 2.0
        + if shows_inspector {
            PANE_GAP + INSPECTOR_WIDTH
        } else {
            0.0
        }
}

fn fixed_width(shows_inspector: bool) -> f64 {
    others_width(shows_inspector) + MAIN_WIDTH
}

/// The window's width with every shown pane at its default width, the map
/// at `map_width` (never below its minimum) and the main pane at `main`
/// (never below its default).
pub fn window_width(shows: Visibility, map_width: f64, main: f64) -> f64 {
    fixed_width(shows.inspector)
        + (main.max(MAIN_WIDTH) - MAIN_WIDTH)
        + if shows.map {
            PANE_GAP + MAP_MIN_WIDTH.max(map_width)
        } else {
            0.0
        }
}

/// The window's default width: every pane shown at its default width.
pub fn default_window_width() -> f64 {
    window_width(Visibility::new(true, true), MAP_WIDTH, MAIN_WIDTH)
}

/// The narrowest the window may be for these panes: the map at its minimum,
/// every other pane at its fixed width. There is no widest.
pub fn minimum_window_width(shows: Visibility) -> f64 {
    fixed_width(shows.inspector)
        + if shows.map {
            PANE_GAP + MAP_MIN_WIDTH
        } else {
            0.0
        }
}

/// The window's new width when panes open or close: the current width, plus
/// each pane that opened and its gap, less each pane that closed (at the
/// width it had) and its gap. The other panes keep their widths.
pub fn window_width_after(
    current: f64,
    old: Visibility,
    new: Visibility,
    main: f64,
    map_width: f64,
) -> f64 {
    let before = PaneLayout::new(current, old, main);
    let mut width = current;
    if old.map != new.map {
        if new.map {
            width += MAP_MIN_WIDTH.max(map_width) + PANE_GAP;
        } else {
            width -= before.map.unwrap_or(0.0) + PANE_GAP;
        }
    }
    if old.inspector != new.inspector {
        let pane = INSPECTOR_WIDTH + PANE_GAP;
        width += if new.inspector { pane } else { -pane };
    }
    minimum_window_width(new).max(width)
}

/// The main pane's width while the divider is dragged: its width when the
/// drag began plus how far the pointer has moved, never under
/// [`MAIN_WIDTH`] and never so wide that the map goes under
/// [`MAP_MIN_WIDTH`]. The window keeps its width.
pub fn dragged_main(start: f64, dx: f64, window_width: f64, shows_inspector: bool) -> f64 {
    PaneLayout::new(
        window_width,
        Visibility::new(true, shows_inspector),
        start + dx,
    )
    .main
}

/// The panes a window opens with the first time, before the person has
/// chosen: the map when the monitor has 1100px for it, the inspector from
/// 900px.
pub fn first_launch(monitor_width: f64) -> Visibility {
    Visibility::new(
        monitor_width >= MAP_BREAKPOINT,
        monitor_width >= INSPECTOR_BREAKPOINT,
    )
}

#[cfg(test)]
mod tests {
    //! `macos/Tests/TCDesignTests/ThreePaneTests.swift`, case for case,
    //! less the cases about the Mac's animated transition, its on-screen
    //! frame rule and its AppKit divider view.
    use super::*;

    const COMPOSITIONS: [Visibility; 4] = [
        Visibility::new(false, false),
        Visibility::new(true, false),
        Visibility::new(false, true),
        Visibility::new(true, true),
    ];

    fn default_width(v: Visibility) -> f64 {
        window_width(v, MAP_WIDTH, MAIN_WIDTH)
    }

    fn layout(width: f64, v: Visibility) -> PaneLayout {
        PaneLayout::new(width, v, MAIN_WIDTH)
    }

    #[test]
    fn the_default_widths_are_the_mac_tokens() {
        assert_eq!(MAIN_WIDTH, 440.0);
        assert_eq!(MAP_WIDTH, MAIN_WIDTH);
        assert_eq!(INSPECTOR_WIDTH, (MAIN_WIDTH * 2.0 / 3.0).round());
        assert_eq!(INSPECTOR_WIDTH, 293.0);
        assert_eq!(MAP_MIN_WIDTH, 360.0);
        assert_eq!(PANE_GAP, 10.0);
        assert_eq!(default_window_width(), 1193.0);
        assert_eq!(WINDOW_HEIGHT, 614.0);
        assert_eq!(WINDOW_MIN_HEIGHT, 560.0);
    }

    #[test]
    fn the_gap_and_padding_are_the_generated_tokens() {
        use crate::ui::style::brand_tokens::glass::space;
        assert_eq!(PANE_GAP, f64::from(space::PANE_GAP));
        assert_eq!(WINDOW_PADDING, f64::from(space::WINDOW_PADDING));
    }

    #[test]
    fn every_composition_at_its_default_width_is_its_panes() {
        for v in COMPOSITIONS {
            let width = default_width(v);
            let layout = layout(width, v);
            assert_eq!(layout.main, MAIN_WIDTH, "{v:?}");
            assert_eq!(layout.map, v.map.then_some(MAP_WIDTH), "{v:?}");
            assert_eq!(
                layout.inspector,
                v.inspector.then_some(INSPECTOR_WIDTH),
                "{v:?}"
            );
            let panes: Vec<f64> = [Some(layout.main), layout.map, layout.inspector]
                .into_iter()
                .flatten()
                .collect();
            let gaps = PANE_GAP * (panes.len() as f64 - 1.0);
            assert_eq!(
                panes.iter().sum::<f64>() + gaps + WINDOW_PADDING * 2.0,
                width,
                "{v:?}"
            );
            assert_eq!(layout.visibility(), v);
        }
    }

    #[test]
    fn a_hidden_pane_reserves_nothing() {
        let alone = layout(1000.0, Visibility::new(false, false));
        assert_eq!(alone.main, 1000.0);
        assert_eq!(alone.map, None);
        assert_eq!(alone.inspector, None);
        let with_inspector = layout(1000.0, Visibility::new(false, true));
        assert_eq!(with_inspector.main, 1000.0 - PANE_GAP - INSPECTOR_WIDTH);
    }

    #[test]
    fn opening_or_closing_a_pane_resizes_the_window_not_the_other_panes() {
        for old in COMPOSITIONS {
            for new in COMPOSITIONS {
                if new == old {
                    continue;
                }
                let before = default_width(old);
                let after = window_width_after(before, old, new, MAIN_WIDTH, MAP_WIDTH);
                let mut expected = before;
                if old.map != new.map {
                    let pane = MAP_WIDTH + PANE_GAP;
                    expected += if new.map { pane } else { -pane };
                }
                if old.inspector != new.inspector {
                    let pane = INSPECTOR_WIDTH + PANE_GAP;
                    expected += if new.inspector { pane } else { -pane };
                }
                assert_eq!(after, expected, "{old:?} -> {new:?}");
                let a = layout(before, old);
                let b = layout(after, new);
                assert_eq!(a.main, b.main, "the main pane moved: {old:?} -> {new:?}");
                if old.map && new.map {
                    assert_eq!(a.map, b.map, "the map moved: {old:?} -> {new:?}");
                }
            }
        }
    }

    #[test]
    fn a_widened_map_keeps_its_width() {
        let both = Visibility::new(true, true);
        let map_only = Visibility::new(true, false);
        let widened = 1500.0;
        let before = layout(widened, both);
        assert_eq!(before.main, MAIN_WIDTH);
        assert_eq!(before.inspector, Some(INSPECTOR_WIDTH));
        assert_eq!(
            before.map,
            Some(widened - MAIN_WIDTH - PANE_GAP - PANE_GAP - INSPECTOR_WIDTH)
        );
        let closed = window_width_after(widened, both, map_only, MAIN_WIDTH, MAP_WIDTH);
        assert_eq!(layout(closed, map_only).map, before.map);
        let no_map = window_width_after(
            widened,
            both,
            Visibility::new(false, true),
            MAIN_WIDTH,
            MAP_WIDTH,
        );
        assert_eq!(no_map, default_width(Visibility::new(false, true)));
    }

    #[test]
    fn the_map_opens_at_its_kept_width() {
        let kept = 620.0;
        for inspector in [false, true] {
            let v = Visibility::new(true, inspector);
            let width = window_width(v, kept, MAIN_WIDTH);
            let layout = layout(width, v);
            assert_eq!(layout.map, Some(kept), "inspector {inspector}");
            assert_eq!(layout.main, MAIN_WIDTH, "inspector {inspector}");
        }
        // A kept width under the minimum opens at the minimum.
        assert_eq!(
            window_width(Visibility::new(true, false), 100.0, MAIN_WIDTH),
            minimum_window_width(Visibility::new(true, false))
        );
        // No map, no map width.
        let closed_v = Visibility::new(false, true);
        assert_eq!(
            window_width(closed_v, kept, MAIN_WIDTH),
            default_width(closed_v)
        );
        let closed = default_width(closed_v);
        let both = Visibility::new(true, true);
        let reopened = window_width_after(closed, closed_v, both, MAIN_WIDTH, kept);
        assert_eq!(layout(reopened, both).map, Some(kept));
    }

    #[test]
    fn the_windows_minimum_is_the_sum_of_the_shown_panes() {
        assert_eq!(
            minimum_window_width(Visibility::new(false, false)),
            MAIN_WIDTH
        );
        assert_eq!(
            minimum_window_width(Visibility::new(true, false)),
            MAIN_WIDTH + PANE_GAP + MAP_MIN_WIDTH
        );
        assert_eq!(
            minimum_window_width(Visibility::new(false, true)),
            MAIN_WIDTH + PANE_GAP + INSPECTOR_WIDTH
        );
        assert_eq!(
            minimum_window_width(Visibility::new(true, true)),
            MAIN_WIDTH + PANE_GAP + MAP_MIN_WIDTH + PANE_GAP + INSPECTOR_WIDTH
        );
        // Narrower than the minimum, the map is what is squeezed; the main
        // pane never goes under its width.
        assert_eq!(layout(100.0, Visibility::new(true, true)).map, Some(0.0));
        assert_eq!(layout(100.0, Visibility::new(true, true)).main, MAIN_WIDTH);
        assert_eq!(
            layout(100.0, Visibility::new(false, false)).main,
            MAIN_WIDTH
        );
    }

    #[test]
    fn a_widened_main_pane_keeps_its_width() {
        let widened_main = 700.0;
        for old in COMPOSITIONS {
            if old.map {
                continue;
            }
            let start = widened_main
                + if old.inspector {
                    PANE_GAP + INSPECTOR_WIDTH
                } else {
                    0.0
                };
            let before = layout(start, old);
            assert_eq!(before.main, widened_main, "{old:?}");
            for new in COMPOSITIONS {
                if new == old {
                    continue;
                }
                let end = window_width_after(start, old, new, before.main, MAP_WIDTH);
                let mut expected = start;
                if new.map {
                    expected += MAP_WIDTH + PANE_GAP;
                }
                if old.inspector != new.inspector {
                    let pane = INSPECTOR_WIDTH + PANE_GAP;
                    expected += if new.inspector { pane } else { -pane };
                }
                assert_eq!(end, expected, "{old:?} -> {new:?}");
                let after = PaneLayout::new(end, new, before.main);
                assert_eq!(after.main, widened_main, "{old:?} -> {new:?}");
                if new.map {
                    assert_eq!(after.map, Some(MAP_WIDTH), "{old:?} -> {new:?}");
                }
                let back = window_width_after(end, new, old, after.main, MAP_WIDTH);
                assert_eq!(back, start, "{new:?} -> {old:?}");
            }
        }
    }

    #[test]
    fn before_the_main_pane_gives_way_the_map_reaches_its_floor() {
        let widened_main = 700.0;
        let v = Visibility::new(true, false);
        let at = |width: f64| PaneLayout::new(width, v, widened_main);
        let roomy = at(widened_main + PANE_GAP + 500.0);
        assert_eq!(roomy.main, widened_main);
        assert_eq!(roomy.map, Some(500.0));
        let at_floor = at(widened_main + PANE_GAP + MAP_MIN_WIDTH);
        assert_eq!(at_floor.main, widened_main);
        assert_eq!(at_floor.map, Some(MAP_MIN_WIDTH));
        let squeezed = at(600.0 + PANE_GAP + MAP_MIN_WIDTH);
        assert_eq!(squeezed.main, 600.0);
        assert_eq!(squeezed.map, Some(MAP_MIN_WIDTH));
        let narrowest = at(minimum_window_width(v));
        assert_eq!(narrowest.main, MAIN_WIDTH);
        assert_eq!(narrowest.map, Some(MAP_MIN_WIDTH));
        // Never dragged, the main pane stays at its default however wide.
        assert_eq!(layout(2000.0, v).main, MAIN_WIDTH);
    }

    #[test]
    fn the_first_launch_seeds_panes_from_the_monitor_width() {
        assert_eq!(first_launch(1320.0), Visibility::new(true, true));
        assert_eq!(first_launch(1100.0), Visibility::new(true, true));
        assert_eq!(first_launch(1099.0), Visibility::new(false, true));
        assert_eq!(first_launch(900.0), Visibility::new(false, true));
        assert_eq!(first_launch(899.0), Visibility::new(false, false));
    }

    #[test]
    fn resizing_within_a_composition_keeps_the_visibility() {
        let both = Visibility::new(true, true);
        let narrow = layout(1200.0, both);
        let wide = layout(1500.0, both);
        assert_ne!(narrow, wide);
        assert_eq!(narrow.visibility(), wide.visibility());
        let hidden = layout(1500.0, Visibility::new(false, true));
        assert_ne!(hidden.visibility(), wide.visibility());
    }

    #[test]
    fn the_divider_moves_room_between_the_main_pane_and_the_map() {
        for inspector in [false, true] {
            let v = Visibility::new(true, inspector);
            let window = default_width(v) + 200.0;
            let wider = dragged_main(MAIN_WIDTH, 120.0, window, inspector);
            assert_eq!(wider, MAIN_WIDTH + 120.0);
            let layout = PaneLayout::new(window, v, wider);
            assert_eq!(
                layout.main + layout.map.unwrap_or(0.0),
                MAIN_WIDTH + MAP_WIDTH + 200.0,
                "the window keeps its width"
            );
            let widest = dragged_main(MAIN_WIDTH, 10_000.0, window, inspector);
            assert_eq!(PaneLayout::new(window, v, widest).map, Some(MAP_MIN_WIDTH));
            assert_eq!(
                dragged_main(MAIN_WIDTH, -500.0, window, inspector),
                MAIN_WIDTH
            );
        }
    }

    #[test]
    fn the_divider_reaches_into_the_map_but_stays_inside_the_gap() {
        const { assert!(DIVIDER_REACH > 0.0) };
        const { assert!(DIVIDER_REACH < PANE_GAP) };
    }

    #[test]
    fn the_kept_main_width_widens_the_opening_window() {
        let both = Visibility::new(true, true);
        let base = default_width(both);
        assert_eq!(
            window_width(both, MAP_WIDTH, MAIN_WIDTH + 80.0),
            base + 80.0
        );
        assert_eq!(window_width(both, MAP_WIDTH, MAIN_WIDTH - 80.0), base);
    }

    #[test]
    fn the_inspector_starts_after_the_map_or_after_the_main_pane() {
        let both = layout(default_window_width(), Visibility::new(true, true));
        assert_eq!(both.map_x(), MAIN_WIDTH + PANE_GAP);
        assert_eq!(
            both.inspector_x(),
            MAIN_WIDTH + PANE_GAP + MAP_WIDTH + PANE_GAP
        );
        let no_map = layout(
            default_width(Visibility::new(false, true)),
            Visibility::new(false, true),
        );
        assert_eq!(no_map.inspector_x(), MAIN_WIDTH + PANE_GAP);
    }
}
