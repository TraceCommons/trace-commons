use super::*;

fn swift(path: &str) -> String {
    std::fs::read_to_string(format!("{}/../../macos/{path}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The lengths are the Mac's, read from `GlassMenuBarStrip` itself.
#[test]
fn the_lengths_are_the_macs() {
    let source = swift("Sources/TCDesign/Components/MenuBarPanel.swift");
    for (line, value) in [
        ("static let height: CGFloat = ", HEIGHT),
        ("static let markSize: CGFloat = ", MARK_SIZE),
        ("static let badgeSize: CGFloat = ", BADGE_SIZE),
        ("static let badgeGlyph: CGFloat = ", BADGE_GLYPH),
        ("static let badgeInset: CGFloat = ", BADGE_INSET),
        ("static let haloInset: CGFloat = ", HALO_INSET),
        ("static let haloLine: CGFloat = ", HALO_LINE),
        ("static let markOnlyGap: CGFloat = ", MARK_GAP),
    ] {
        let at = source.find(line).unwrap_or_else(|| panic!("{line} moved"));
        let rest = &source[at + line.len()..];
        let number: f64 = rest[..rest.find('\n').unwrap()].trim().parse().unwrap();
        assert_eq!(value, number, "{line}");
    }
    assert!(source.contains("public static let haloReach: CGFloat = haloInset + haloLine / 2"));
    assert_eq!(HALO_REACH, 3.5);
    // The news ring: a 9-point circle stroked 1.5 wide.
    assert!(source.contains("lineWidth: 1.5)\n                .frame(width: 9, height: 9)"));
    assert_eq!((RING_SIZE, RING_LINE), (9.0, 1.5));
}

/// `MenuBarGlassPanelTests.test_theStripIsDownWhenTheCoreIs`.
#[test]
fn the_strip_is_down_when_the_core_is() {
    use Condition::*;
    assert_eq!(
        condition(Some(2), false, false, false, false, true),
        Unavailable
    );
    assert_eq!(
        condition(Some(2), false, false, true, true, true),
        Unavailable
    );
    assert_eq!(
        condition(Some(2), true, false, true, false, true),
        Attention
    );
    assert_eq!(condition(None, false, false, true, false, true), Attention);
    assert_eq!(condition(Some(0), false, true, true, false, true), Paused);
    assert_eq!(condition(Some(0), false, false, true, false, true), Live);
    assert_eq!(condition(None, false, false, true, false, false), Loading);
    assert_eq!(condition(None, true, false, true, false, false), Attention);
    assert_eq!(
        condition(None, false, false, false, false, false),
        Unavailable
    );
}

/// `MenuBarPanelTests.test_theBadgeStopsAtNinetyNine` and the badge's
/// visibility rules.
#[test]
fn the_badge_stops_at_ninety_nine_and_never_guesses() {
    assert_eq!(badge_text(1), "1");
    assert_eq!(badge_text(99), "99");
    assert_eq!(badge_text(100), "99+");
    assert_eq!(badge_text(1_200), "99+");
    assert_eq!(badge(Some(0)), None);
    assert_eq!(badge(None), None);
    assert_eq!(badge(Some(3)), Some(3));
    assert_eq!(shown_badge(Some(4), Condition::Unavailable), None);
    assert_eq!(shown_badge(Some(4), Condition::Loading), None);
    assert_eq!(shown_badge(Some(4), Condition::Live), Some(4));
    assert_eq!(shown_badge(Some(4), Condition::Paused), Some(4));
    assert_eq!(shown_badge(Some(4), Condition::Attention), Some(4));
}

/// `MenuBarPanelTests.test_theMarkIsDrawnOnlyWhereItsSlotIs`.
#[test]
fn the_mark_is_drawn_only_where_its_slot_is() {
    use Condition::*;
    assert_eq!(shown_mark(Mark::News, None, Live), Mark::News);
    assert_eq!(shown_mark(Mark::News, Some(3), Live), Mark::None);
    assert_eq!(shown_mark(Mark::News, None, Paused), Mark::None);
    assert_eq!(shown_mark(Mark::News, None, Attention), Mark::None);
    assert_eq!(shown_mark(Mark::News, None, Unavailable), Mark::None);
    assert_eq!(shown_mark(Mark::Ready, Some(3), Live), Mark::Ready);
    assert_eq!(shown_mark(Mark::Ready, Some(3), Paused), Mark::Ready);
    assert_eq!(shown_mark(Mark::Ready, None, Live), Mark::None);
    assert_eq!(shown_mark(Mark::Ready, Some(0), Live), Mark::None);
    assert_eq!(shown_mark(Mark::Ready, Some(3), Unavailable), Mark::None);
    assert_eq!(shown_mark(Mark::None, Some(3), Live), Mark::None);
}

fn nudge(json: &str) -> Nudge {
    serde_json::from_str(json).expect("nudge")
}

/// `NudgeSurfaceTests.testTheMarkIsOnlyNewsOrReadyAndOnlyWhileTheStripIsAvailable`,
/// read through `status` so the wire field is the one the daemon writes.
#[test]
fn the_mark_is_only_news_or_ready_and_only_while_available() {
    let status: crate::model::Status = serde_json::from_str(
        r#"{"nudge":{"state":"armed","lead":"verdicts_landed","mark":"news",
            "mark_kinds":["verdicts_landed"],
            "mark_text":{"accessibility":"a","tooltip":"Verdicts are in."}}}"#,
    )
    .expect("status");
    let news = status.nudge.as_ref();
    assert_eq!(lit_mark(news, true), Mark::News);
    assert_eq!(lit_mark(news, false), Mark::None);
    assert_eq!(
        mark_words(news, true, None, Condition::Live).map(|w| w.tooltip.as_str()),
        Some("Verdicts are in.")
    );
    assert_eq!(mark_words(news, false, None, Condition::Live), None);
    // A ring the badge hides is never spoken.
    assert_eq!(mark_words(news, true, Some(2), Condition::Live), None);

    let ready = nudge(r#"{"mark":"ready","mark_text":{"accessibility":"a","tooltip":"t"}}"#);
    assert_eq!(lit_mark(Some(&ready), true), Mark::Ready);
    for json in [
        r#"{"state":"none","mark":"none"}"#,
        r#"{"state":"unknown","mark":"unknown"}"#,
        r#"{"state":"none"}"#,
        r#"{"state":"none","mark":"sparkle"}"#,
    ] {
        assert_eq!(lit_mark(Some(&nudge(json)), true), Mark::None, "{json}");
    }
    assert_eq!(lit_mark(None, true), Mark::None);
    let older: crate::model::Status = serde_json::from_str("{}").expect("status");
    assert_eq!(older.nudge, None);
}

fn strip(condition: Condition, badge: Option<u64>, mark: Mark) -> Strip {
    Strip {
        condition,
        badge,
        mark,
    }
}

/// One pixel of a 22px pixmap as (a, r, g, b), straight alpha.
fn pixel(strip: &Strip, scheme: Scheme, x: usize, y: usize) -> [u8; 4] {
    let (w, h, data) = pixmap(strip, scheme, 22).expect("pixmap");
    assert_eq!((w, h), (22, 22));
    assert_eq!(data.len(), 22 * 22 * 4);
    let i = (y * 22 + x) * 4;
    [data[i], data[i + 1], data[i + 2], data[i + 3]]
}

/// The N alone: in the kit's black on light and white on dark, centred,
/// with nothing in the slot.
#[test]
fn the_plain_icon_is_the_n_in_the_kits_ink() {
    let plain = strip(Condition::Live, None, Mark::None);
    // Kit (100..117, 198..215): the N's left stem.
    assert_eq!(pixel(&plain, Scheme::Light, 5, 11), [255, 0, 0, 0]);
    assert_eq!(pixel(&plain, Scheme::Dark, 5, 11), [255, 255, 255, 255]);
    // The right stem, which a badge clears.
    assert_eq!(pixel(&plain, Scheme::Light, 17, 10), [255, 0, 0, 0]);
    assert_eq!(pixel(&plain, Scheme::Light, 20, 20)[0], 0);
}

/// The badge: the mark's ink with the figure cut out, the mark cleared
/// around it, on the cell's corner.
#[test]
fn the_badge_is_the_marks_ink_with_its_figure_cut_out() {
    let four = strip(Condition::Live, Some(4), Mark::None);
    let s = slot(&four, 1.0);
    assert_eq!(s.badge, Some((11.0, 11.0, 11.0, 11.0)));
    // "4": its top row is set, clear, set from (15, 14).
    assert_eq!(pixel(&four, Scheme::Light, 15, 14)[0], 0);
    assert_eq!(pixel(&four, Scheme::Light, 16, 14), [255, 0, 0, 0]);
    assert_eq!(pixel(&four, Scheme::Dark, 16, 14), [255, 255, 255, 255]);
    assert_eq!(pixel(&four, Scheme::Light, 17, 14)[0], 0);
    // The mark is cleared within the gap of the badge.
    assert_eq!(pixel(&four, Scheme::Light, 17, 10)[0], 0);
    // Past 99 the badge widens to hold three characters.
    let many = slot(&strip(Condition::Live, Some(150), Mark::None), 1.0);
    assert_eq!(many.badge, Some((6.0, 11.0, 16.0, 11.0)));
}

/// The halo: the accent line outside the badge, never clipped by the cell.
#[test]
fn the_halo_rings_the_badge_inside_the_cell() {
    let ready = strip(Condition::Live, Some(4), Mark::Ready);
    let s = slot(&ready, 1.0);
    assert!(s.halo);
    assert_eq!(s.badge, Some((7.5, 7.5, 11.0, 11.0)));
    let (_, _, x0, y0) = (0, 0, 7.5 - HALO_REACH, 7.5 - HALO_REACH);
    assert!(x0 >= 0.0 && y0 >= 0.0 && 7.5 + 11.0 + HALO_REACH <= HEIGHT);
    assert_eq!(
        pixel(&ready, Scheme::Light, 20, 13),
        [255, 0x02, 0x3E, 0x83]
    );
    assert_eq!(
        pixel(&ready, Scheme::Light, 13, 20),
        [255, 0x02, 0x3E, 0x83]
    );
    assert_eq!(pixel(&ready, Scheme::Dark, 20, 13), [255, 0xC6, 0xE1, 0xFF]);
    // No halo without a badge, and none while paused hides nothing.
    assert!(!slot(&strip(Condition::Live, None, Mark::Ready), 1.0).halo);
    assert!(slot(&strip(Condition::Paused, Some(4), Mark::Ready), 1.0).halo);
}

/// The news ring sits in the empty slot on a live strip only.
#[test]
fn the_news_ring_sits_in_the_empty_slot() {
    let news = strip(Condition::Live, None, Mark::News);
    let s = slot(&news, 1.0);
    assert_eq!(s.ring, Some((16.75, 16.75)));
    assert_eq!(s.badge, None);
    // Its centre is hollow and the mark under it cleared.
    assert_eq!(pixel(&news, Scheme::Light, 16, 16)[0], 0);
    // On the ring: the accent.
    let [a, r, g, b] = pixel(&news, Scheme::Light, 16, 12);
    assert!(a > 200, "alpha {a}");
    assert!(
        (i32::from(r) - 0x02).abs() <= 2
            && (i32::from(g) - 0x3E).abs() <= 2
            && (i32::from(b) - 0x83).abs() <= 2,
        "({r}, {g}, {b})"
    );
    for hidden in [
        strip(Condition::Live, Some(1), Mark::News),
        strip(Condition::Paused, None, Mark::News),
        strip(Condition::Attention, None, Mark::News),
        strip(Condition::Unavailable, None, Mark::News),
        strip(Condition::Loading, None, Mark::News),
    ] {
        assert_eq!(slot(&hidden, 1.0).ring, None, "{hidden:?}");
    }
}

/// Every size the host may ask for, each square and whole.
#[test]
fn every_pixmap_is_square_and_whole() {
    let all = pixmaps(
        &strip(Condition::Live, Some(12), Mark::Ready),
        Scheme::Light,
    );
    assert_eq!(all.len(), PIXMAP_SIZES.len());
    for ((w, h, data), px) in all.iter().zip(PIXMAP_SIZES) {
        assert_eq!((*w, *h), (px, px));
        assert_eq!(data.len(), (px * px * 4) as usize);
    }
}

/// Straight alpha: half-covered black stays black, not grey.
#[test]
fn pixels_are_not_premultiplied() {
    assert_eq!(unpremultiply(0x8000_0000), 0x8000_0000);
    assert_eq!(unpremultiply(0x8080_8080), 0x80FF_FFFF);
    assert_eq!(unpremultiply(0), 0);
}
