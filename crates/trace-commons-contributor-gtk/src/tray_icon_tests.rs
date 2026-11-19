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
            "mark_text":{"accessibility":"a","tooltip":"news-tooltip"}}}"#,
    )
    .expect("status");
    let news = status.nudge.as_ref();
    assert_eq!(lit_mark(news, true), Mark::News);
    assert_eq!(lit_mark(news, false), Mark::None);
    assert_eq!(
        mark_words(news, true, None, Condition::Live).map(|w| w.tooltip.as_str()),
        Some("news-tooltip")
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
/// A pixel of the N just inside the halo's gap, off the badge's own gap.
const PROBE: (usize, usize) = (5, 14);

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

/// The halo: the accent line outside the badge on the corner, the badge
/// where it is without one.
#[test]
fn the_halo_rings_the_badge_on_the_corner() {
    let ready = strip(Condition::Live, Some(4), Mark::Ready);
    let s = slot(&ready, 1.0);
    assert!(s.halo);
    assert_eq!(
        s.badge,
        slot(&strip(Condition::Live, Some(4), Mark::None), 1.0).badge
    );
    // Centre (16.5, 16.5), radius 5.5 + 2.5: the line covers 7..9 out.
    assert_eq!(pixel(&ready, Scheme::Light, 16, 8), [255, 0x02, 0x3E, 0x83]);
    assert_eq!(pixel(&ready, Scheme::Light, 8, 16), [255, 0x02, 0x3E, 0x83]);
    assert_eq!(pixel(&ready, Scheme::Dark, 16, 8), [255, 0xC6, 0xE1, 0xFF]);
    // Between the badge and the halo, the mark is cleared.
    assert_eq!(pixel(&ready, Scheme::Light, 16, 10)[0], 0);
    // The N's left stem survives.
    assert_eq!(pixel(&ready, Scheme::Light, 5, 11), [255, 0, 0, 0]);
    // The halo's outer edge keeps the Mac's 2-point gap from the N
    // (`markOnlyGap + haloReach` past the badge), so it never touches it.
    let plain = strip(Condition::Live, Some(4), Mark::None);
    assert_eq!(pixel(&plain, Scheme::Light, PROBE.0, PROBE.1)[0], 255);
    assert!(pixel(&ready, Scheme::Light, PROBE.0, PROBE.1)[0] < 200);
    // No halo without a badge; pausing keeps it.
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
    assert!(a > 200, "{a}");
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

// --- The credit arrival on the icon (`GlassMenuBarStrip.creditBurst`) ---

/// The burst's numbers, from a Swift probe of `stripCoins`, `creditGleam`,
/// `markHop` and `stripCoinOffset` copied verbatim from
/// `MenuBarPanel.swift` at origin/main 1c2fdcdd3 (#2197; this branch's base
/// predates it, so the source is not in the tree to read): 10 coins, drag
/// 3.8, gravity 10; the gleam up over 0.15 s, held to 0.7 s, gone by 1.7 s;
/// the mark hops 2 points over 0.35 s.
#[test]
fn the_credit_burst_is_the_macs() {
    use credit::*;
    // Since the base took main, the Swift is in the tree too.
    let strip = swift("Sources/TCDesign/Components/MenuBarPanel.swift");
    for line in [
        "static let stripCoinCount = 10",
        "static let stripDrag: Double = 3.8",
        "static let stripGravity: Double = 10",
        "static let markHopDuration: Double = 0.35",
        "static let markHopPeak: CGFloat = 2.0",
        "let rise = 0.15",
        "let holdUntil = 0.7",
        "let angle = slot * 0.88 * .pi + (GlassCoinBurst.noise(index, 21) - 0.5) * 0.14",
        "speed: 44 + GlassCoinBurst.noise(index, 22) * 58,",
        "return CGVector(dx: cos(coin.angle) * reach, dy: sin(coin.angle) * reach * 0.45 + fall)",
        "let radius = (Self.height / 2) * CGFloat(0.45 + 0.55 * (t / GlassCoinBurst.flashDuration))",
        "let flash = (1 - t / 0.3) * 0.55",
    ] {
        assert!(strip.contains(line), "{line}");
    }
    let arrival = swift("Sources/TCShellCore/BalanceArrival.swift");
    for line in [
        "public static let framesPerSecond: Double = 24",
        "public static let stillAt: Double = 0.45",
        "public static let reduceMotionHold: Duration = .milliseconds(1600)",
        "public static let minimumReceivedNanos: Int64 = 1_000_000_000",
        "public static let minimumCreditFinal: Double = 1.0",
    ] {
        assert!(arrival.contains(line), "{line}");
    }
    assert_eq!(
        (COIN_COUNT, DRAG, GRAVITY, HOP_DURATION, HOP_PEAK),
        (10, 3.8, 10.0, 0.35, 2.0)
    );
    let coins = coins();
    assert_eq!(coins.len(), 10);
    for (index, swift) in [
        (
            0,
            [
                -1.352_744_386_354_115_8,
                61.958_669_023_537_37,
                2.956_287_735_211_662_8,
                2.642_961_844_085_948_4,
                0.301_787_141_863_314_9,
                -0.143_937_901_513_709_22,
                0.074_621_610_629_837_96,
            ],
        ),
        (
            3,
            [
                -0.516_324_395_310_487_9,
                66.940_622_926_049_4,
                3.559_186_278_301_058,
                1.980_451_103_748_055_2,
                0.253_495_479_370_030_8,
                -0.265_978_713_399_817_93,
                0.060_171_298_783_097_88,
            ],
        ),
        (
            9,
            [
                1.413_435_964_035_961_7,
                69.020_084_421_779_76,
                2.928_921_054_751_117_3,
                2.007_901_681_532_257_8,
                0.742_162_587_492_202_9,
                0.272_076_943_983_847_74,
                0.038_632_573_023_787_7,
            ],
        ),
    ] {
        let c = coins[index];
        let ours = [
            c.angle, c.speed, c.diameter, c.spin, c.phase, c.tilt, c.delay,
        ];
        for (o, s) in ours.iter().zip(swift) {
            assert!((o - s).abs() < 1e-9, "coin {index}: {ours:?}");
        }
    }
    for (t, swift) in [
        (0.05, 0.333_333_333_333_333_37),
        (0.15, 1.0),
        (0.4, 1.0),
        (0.7, 1.0),
        (1.2, 0.5),
        (1.69, 0.010_000_000_000_000_009),
        (1.7, 0.0),
        (-0.1, 0.0),
    ] {
        assert!((gleam(t) - swift).abs() < 1e-9, "gleam {t}: {}", gleam(t));
    }
    for (t, swift) in [
        (0.05, -0.867_767_478_235_116_4),
        (0.175, -2.0),
        (0.3, -0.867_767_478_235_116_5),
        (0.35, 0.0),
        (0.0, 0.0),
        (0.45, 0.0),
    ] {
        assert!((hop(t) - swift).abs() < 1e-9, "hop {t}: {}", hop(t));
    }
    for (t, swift) in [
        (0.1, (2.119_013_570_357_049_4, -0.139_686_462_378_042_4)),
        (0.5, (9.877_761_713_735_616, 0.305_015_947_794_752_6)),
        (1.0, (11.724_868_918_972_904, 3.676_449_507_965_519)),
    ] {
        let (dx, dy) = offset(&coins[4], t);
        assert!(
            (dx - swift.0).abs() < 1e-9 && (dy - swift.1).abs() < 1e-9,
            "{t}: {dx} {dy}"
        );
    }
}

fn pixels_at(strip: &Strip, scheme: Scheme, burst: Option<f64>) -> Vec<u8> {
    let (w, h, data) = pixmap_at(strip, scheme, 22, burst).expect("pixmap");
    assert_eq!((w, h), (22, 22));
    data
}

/// The topmost row any pixel of the mark's column covers.
fn top_row(data: &[u8]) -> usize {
    (0..22)
        .find(|y| (4..18).any(|x| data[(y * 22 + x) * 4] > 128))
        .expect("drawn")
}

/// At rest, and once the burst has run its 1.7 s, the icon is the plain
/// strip; between, it draws only inside the mark's 14-point column, as the
/// Mac's strip without its graph draws only in its 14-point canvas.
#[test]
fn the_burst_stays_in_the_marks_column_and_ends_on_the_rest_frame() {
    let plain = strip(Condition::Live, None, Mark::None);
    for scheme in [Scheme::Light, Scheme::Dark] {
        let rest = pixels_at(&plain, scheme, None);
        assert_eq!(rest, pixmap(&plain, scheme, 22).unwrap().2);
        assert_eq!(pixels_at(&plain, scheme, Some(1.7)), rest);
        for t in [0.0, 0.05, 0.2, 0.45, 0.9, 1.4] {
            let frame = pixels_at(&plain, scheme, Some(t));
            assert!(frame != rest, "{t}");
            for y in 0..22 {
                for x in (0..4).chain(18..22) {
                    let i = (y * 22 + x) * 4;
                    assert_eq!(frame[i..i + 4], rest[i..i + 4], "{t}: ({x}, {y})");
                }
            }
        }
    }
}

/// The mark rises two points at the hop's peak and is back by its end.
#[test]
fn the_mark_hops_two_points() {
    let plain = strip(Condition::Live, None, Mark::None);
    let rest = top_row(&pixels_at(&plain, Scheme::Light, None));
    assert_eq!(
        top_row(&pixels_at(&plain, Scheme::Light, Some(0.175))),
        rest - 2
    );
    assert_eq!(top_row(&pixels_at(&plain, Scheme::Light, Some(0.36))), rest);
}

/// Held at full gleam, the N reads gold (`GlassCoinBurst.gold` #F5B921
/// over the ink) on both grounds.
#[test]
fn the_mark_gleams_gold() {
    let plain = strip(Condition::Live, None, Mark::None);
    for scheme in [Scheme::Light, Scheme::Dark] {
        let data = pixels_at(&plain, scheme, Some(0.5));
        let i = (PROBE.1 * 22 + PROBE.0) * 4;
        let [a, r, g, b] = [data[i], data[i + 1], data[i + 2], data[i + 3]];
        assert!(a > 200, "{a}");
        assert!(
            (i32::from(r) - 0xF5).abs() <= 3
                && (i32::from(g) - 0xB9).abs() <= 3
                && (i32::from(b) - 0x21).abs() <= 3,
            "{scheme:?}: ({r}, {g}, {b})"
        );
    }
}

/// A badge keeps its slot and its figure through the burst: the coins and
/// the flash are cut away under it with the mark. (A pixel the mark's
/// cut edge only partly covers shows the gleaming mark, so only whole
/// pixels, clear or covered, are compared.)
#[test]
fn the_badge_is_untouched_by_the_burst() {
    let badged = strip(Condition::Live, Some(3), Mark::None);
    let rest = pixels_at(&badged, Scheme::Light, None);
    let slot = slot(&badged, 1.0).badge.unwrap();
    for t in [0.05, 0.3, 0.8] {
        let frame = pixels_at(&badged, Scheme::Light, Some(t));
        for y in (slot.1 as usize)..22 {
            for x in (slot.0 as usize)..22 {
                let i = (y * 22 + x) * 4;
                if rest[i] == 0 || rest[i] == 255 {
                    assert_eq!(frame[i..i + 4], rest[i..i + 4], "{t}: ({x}, {y})");
                }
            }
        }
    }
}

/// The burst drawn offscreen at 64 px, light and dark, at the Mac's frame
/// moments, into `TC_TRAY_RENDER_DIR` for review; writes nothing without it.
#[test]
fn render_the_burst_for_review() {
    let Some(dir) = std::env::var_os("TC_TRAY_RENDER_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("render dir");
    for (name, strip) in [
        ("plain", strip(Condition::Live, None, Mark::None)),
        ("badge", strip(Condition::Live, Some(3), Mark::None)),
    ] {
        for scheme in [Scheme::Light, Scheme::Dark] {
            for frame in [0usize, 1, 4, 7, 10, 17, 24, 33, 40] {
                let t = frame as f64 / 24.0;
                let px = 64;
                let mut surface =
                    cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).expect("surface");
                {
                    let cr = cairo::Context::new(&surface).expect("context");
                    let ground = if scheme == Scheme::Light { 0.93 } else { 0.16 };
                    cr.set_source_rgb(ground, ground, ground);
                    let _ = cr.paint();
                    draw_at(&cr, &strip, scheme, f64::from(px), Some(t));
                }
                let path = dir.join(format!("tray-{name}-{scheme:?}-f{frame:02}.png"));
                surface.flush();
                let (w, h, stride) = (surface.width(), surface.height(), surface.stride());
                let bytes = surface.data().expect("surface data").to_vec();
                let texture = gtk::gdk::MemoryTexture::new(
                    w,
                    h,
                    gtk::gdk::MemoryFormat::B8g8r8a8Premultiplied,
                    &gtk::glib::Bytes::from_owned(bytes),
                    stride as usize,
                );
                gtk::gdk::prelude::TextureExt::save_to_png(&texture, &path).expect("PNG");
            }
        }
    }
}

/// The premultiplied bytes of a 0..`px` square: the strip drawn whole, or
/// a celebration's frame from the mark and the kept slot layers.
fn frame_bytes(strip: &Strip, scheme: Scheme, px: i32, t: f64, kept: bool) -> Vec<u8> {
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).expect("surface");
    {
        let cr = cairo::Context::new(&surface).expect("context");
        if kept {
            draw_mark(&cr, scheme, f64::from(px), Some(t));
            base::compose(&cr, strip, scheme, px).expect("layers");
        } else {
            draw_at(&cr, strip, scheme, f64::from(px), Some(t));
        }
    }
    surface.flush();
    surface.data().expect("surface data").to_vec()
}

/// A celebration's frame drawn from the kept layers is the strip drawn
/// whole, at every size, for a plain mark, a badge, a halo and a ring.
#[test]
fn a_celebrations_frames_are_the_strip_drawn_whole() {
    for strip in [
        strip(Condition::Live, None, Mark::None),
        strip(Condition::Live, Some(3), Mark::None),
        strip(Condition::Live, Some(12), Mark::Ready),
        strip(Condition::Live, None, Mark::News),
    ] {
        for scheme in [Scheme::Light, Scheme::Dark] {
            for px in PIXMAP_SIZES {
                for t in [0.0, 0.1, 0.45, 1.0, 1.6] {
                    let whole = frame_bytes(&strip, scheme, px, t, false);
                    let kept = frame_bytes(&strip, scheme, px, t, true);
                    let worst = whole
                        .iter()
                        .zip(&kept)
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap_or(0);
                    assert!(worst <= 1, "{:?}", (strip, scheme, px, t, worst));
                }
            }
        }
    }
}

/// The 41 frames of one celebration draw the slot's layers once per
/// size, not once per frame.
#[test]
fn a_celebration_draws_the_slot_once_per_size() {
    let strip = strip(Condition::Live, Some(97), Mark::Ready);
    let before = base::drawn();
    for frame in 0..41 {
        let t = frame as f64 / 24.0;
        assert_eq!(
            pixmaps_at(&strip, Scheme::Light, Some(t)).len(),
            PIXMAP_SIZES.len()
        );
    }
    assert_eq!(base::drawn() - before, PIXMAP_SIZES.len());
    // At rest nothing is kept or drawn from them.
    let _ = pixmaps_at(&strip, Scheme::Light, None);
    assert_eq!(base::drawn() - before, PIXMAP_SIZES.len());
}
