//! The Mac's numbers for Home's decoration, pinned. Values marked "probe"
//! were printed by a throwaway Swift probe that runs the Mac's own pure
//! functions (copied from `PixelGlyph.swift`, `PixelField.swift`,
//! `CoinBurst.swift`, `BalanceArrival.swift`) and Foundation's
//! `.compactName`; the masks below are its output for the Mac's two
//! hand-placed symbols.

use super::*;

const MAC_CPU: &str = "\
..........................................\
..........................................\
...............##.....##.....##...........\
...............##.....##.....##...........\
...............##.....##.....##...........\
...............##.....##.....##...........\
...............##.....##.....##...........\
...............##.....##.....##...........\
...........########################.......\
..........##########################......\
..........##########################......\
..........###....................###......\
..........###....................###......\
....#########....................#########\
....#########....................#########\
..........###....############....###......\
..........###....############....###......\
..........###....##........##....###......\
..........###....##........##....###......\
..........###....##........##....###......\
....#########....##........##....#########\
....#########....##........##....#########\
..........###....##........##....###......\
..........###....##........##....###......\
..........###....##........##....###......\
..........###....############....###......\
..........###....############....###......\
....#########....................#########\
....#########....................#########\
..........###....................###......\
..........###....................###......\
..........##########################......\
..........##########################......\
...........########################.......\
...............##.....##.....##...........\
...............##.....##.....##...........\
...............##.....##.....##...........\
...............##.....##.....##...........\
...............##.....##.....##...........\
...............##.....##.....##...........\
..........................................\
..........................................\
";
const MAC_CHART: &str = "\
..........................................\
..........................................\
..........................................\
..........................................\
..........................................\
.............#######......................\
.............#######......................\
.............#######......................\
.............#######......................\
.............#######......................\
.............#######......................\
.............#######......................\
.............#######...#######............\
.............#######...#######............\
.............#######...#######............\
.............#######...#######............\
.............#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
...#######...#######...#######............\
..........................................\
..........................................\
..........................................\
..........................................\
..........................................\
";

fn rows(mask: &glyph::Mask) -> Vec<String> {
    mask.coverage
        .chunks(mask.columns)
        .map(|row| {
            row.iter()
                .map(|c| {
                    if *c >= glyph::MINIMUM_COVERAGE {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect()
        })
        .collect()
}

fn fixture(text: &str) -> Vec<String> {
    text.as_bytes()
        .chunks(42)
        .map(|row| String::from_utf8(row.to_vec()).unwrap())
        .collect()
}

#[test]
fn the_hero_card_takes_the_macs_measures() {
    assert_eq!(card::MIN_HEIGHT, 156.0);
    assert_eq!(card::GRAPHIC_RESERVE, 90);
    assert_eq!(card::GRAPHIC_WIDTH, 210.0);
    assert_eq!(card::PADDING, 14);
    // The flat theme's card radius.
    assert_eq!(card::RADIUS, 16.0);
    assert_eq!(card::HOVER, 0.06);
    assert_eq!((card::CLOSE_GLYPH, card::CLOSE_INSET), (12, 8));
    assert_eq!((card::CLOSE_INK, card::CLOSE_INK_HOVER), (0.75, 1.0));
    assert_eq!(card::PILL_PADDING, (14, 7));
    assert_eq!(card::SYMBOL_SIZE, 190.0);
    assert_eq!(card::BLEED, (46.0, 34.0));
    assert_eq!(card::ANGLE, -12.0);
    assert_eq!(
        (card::CELL, card::BRIGHT_CELLS, card::DIM_CELLS),
        (6.0, 0.46, 0.32)
    );
    // probe: side 252, 42 columns.
    assert_eq!(card::canvas_side(), 252.0);
    assert_eq!(
        card::symbol_centre(600.0, 156.0),
        (600.0 - 105.0 + 46.0, 78.0 + 34.0)
    );
    assert_eq!(field::FRAMES_PER_SECOND, 10.0);
}

#[test]
fn the_grounds_are_the_macs_flat_card_tokens() {
    // HeroCard draws `heroCampaignCard`, `heroMissions` and `heroInsights`
    // (GlassTokens.swift), solid navies since 2026-11-07 (fc4ecc1c6), not
    // the brand-blue `heroCampaign` the map takes.
    assert_eq!(
        Ground::Campaign.token(),
        (120, &[(0.0, "#081538"), (1.0, "#081538")][..])
    );
    assert_eq!(
        Ground::Missions.token(),
        (120, &[(0.0, "#07152f"), (1.0, "#07152f")][..])
    );
    assert_eq!(
        Ground::Insights.token(),
        (120, &[(0.0, "#0a2546"), (1.0, "#0a2546")][..])
    );
    assert_eq!(
        tints(true),
        [
            Rgb::hex("#3a7bd5"),
            Rgb::hex("#0091fd"),
            Rgb::hex("#e2f6ff")
        ]
    );
    assert_eq!(
        tints(false),
        [
            Rgb::hex("#2f6bc4"),
            Rgb::hex("#2a78d6"),
            Rgb::hex("#9fdcff")
        ]
    );
    // GlassGradient.unitPoints: 120 degrees runs from the upper left
    // toward the lower right, as CSS does.
    let ((x0, y0), (x1, y1)) = gradient_points(120.0, 1.0, 1.0);
    assert!((x0 - 0.0669872981).abs() < 1e-9 && (y0 - 0.25).abs() < 1e-9);
    assert!((x1 - 0.9330127019).abs() < 1e-9 && (y1 - 0.75).abs() < 1e-9);
}

#[test]
fn the_field_rules_match_the_mac() {
    // probe
    for (distance, time, want) in [
        (0.0, 0.0, 0.86),
        (0.3, 0.0, 0.5785758432198844),
        (0.3, 1.5, 0.7811295159651906),
        (0.7, 2.0, 0.19518101083264455),
    ] {
        assert!(
            (field::level(distance, time) - want).abs() < 1e-12,
            "{distance} {time}"
        );
    }
    assert_eq!(field::level(1.0, 0.0), 0.0);
    assert_eq!(field::shade(3, 5, 0.0, 3), 1);
    assert_eq!(field::shade(10, 2, 7.5, 3), 1);
    assert_eq!(field::shade(20, 30, 100.0, 3), 2);
    assert_eq!(field::threshold(0, 0), 0.5 / 16.0);
    assert_eq!(field::threshold(1, 2), 11.5 / 16.0);
    assert!((glyph::presence(0.0, 0.0) - 0.07999999999999996).abs() < 1e-12);
    assert!((glyph::presence(0.2, 0.3) - 0.6163511659807956).abs() < 1e-12);
    assert_eq!(glyph::presence(1.0, 1.0), 1.0);
}

#[test]
fn the_macs_drawn_symbols_are_its_own_cells() {
    let cpu = glyph::hero_mask("cpu").unwrap();
    assert_eq!(cpu.columns, 42);
    assert_eq!(rows(&cpu), fixture(MAC_CPU));
    let chart = glyph::hero_mask("chart.bar.xaxis").unwrap();
    assert_eq!(rows(&chart), fixture(MAC_CHART));
    // probe: the lit cells at t = 0, with their colour and brightness.
    for (mask, count, checksum, bright, shades) in [
        (&cpu, 466, 10_909_347, 183, [129, 222, 115]),
        (&chart, 511, 8_782_518, 142, [120, 224, 167]),
    ] {
        let lit = glyph::lit(mask, 0.0, 3);
        assert_eq!(lit.len(), count);
        let sum: usize = lit
            .iter()
            .map(|c| c.column * 1000 + c.row * 10 + c.shade * 2 + c.bright as usize)
            .sum();
        assert_eq!(sum, checksum);
        assert_eq!(lit.iter().filter(|c| c.bright).count(), bright);
        let by_shade = [0, 1, 2].map(|s| lit.iter().filter(|c| c.shade == s).count());
        assert_eq!(by_shade, shades);
    }
}

#[test]
fn every_symbol_a_hero_names_is_drawn() {
    // The campaign feed may name cpu, sparkles, bolt or globe
    // (PrivateAICampaign); Missions draws scope and Insights the chart.
    for symbol in glyph::SYMBOLS {
        let mask = glyph::hero_mask(symbol).unwrap_or_else(|| panic!("{symbol}"));
        let lit = glyph::lit(&mask, 0.0, 3).len();
        assert!(lit > 120, "{symbol} lights {lit} cells");
        // Linux's own drawings keep off the canvas's outermost ring, so the
        // tilt keeps their corners. (The Mac's cpu, shifted right to clear
        // the words, runs to its right edge, and is its own.)
        if matches!(symbol, "cpu" | "chart.bar.xaxis") {
            continue;
        }
        let n = mask.columns;
        for i in 0..n {
            for (c, r) in [(i, 0), (i, n - 1), (0, i), (n - 1, i)] {
                assert!(
                    mask.coverage[r * n + c] < glyph::MINIMUM_COVERAGE,
                    "{symbol}"
                );
            }
        }
    }
    assert!(glyph::hero_mask("person").is_none());
    // The ring symbols are drawn as rings: mirror images left to right.
    for symbol in ["scope", "globe"] {
        let mask = glyph::hero_mask(symbol).unwrap();
        let rows = rows(&mask);
        for row in &rows {
            let mirrored: String = row.chars().rev().collect();
            // Centred on the line between columns 20 and 21.
            assert_eq!(&mirrored[..], &row[..], "{symbol}");
        }
    }
}

#[test]
fn a_glyph_frame_is_a_handful_of_fills_and_moves_with_time() {
    let mask = glyph::hero_mask("scope").unwrap();
    let still = glyph::lit(&mask, 0.0, 3);
    let later = glyph::lit(&mask, 2.0, 3);
    // The same cells light; their colours and brightness move.
    assert_eq!(
        still.iter().map(|c| (c.column, c.row)).collect::<Vec<_>>(),
        later.iter().map(|c| (c.column, c.row)).collect::<Vec<_>>()
    );
    assert_ne!(still, later);
}

#[test]
fn the_insights_bars_follow_the_glance() {
    let glance: bars::Glance = serde_json::from_value(serde_json::json!({
        "enabled": true, "readable": true,
        "tools": [
            {"tool": "codex", "tokens": 20},
            {"tool": "claude-code", "tokens": 900},
            {"tool": "cline", "tokens": null},
            {"tool": "opencode", "tokens": 0},
            {"tool": "gemini-cli", "tokens": 5},
            {"tool": "unknown", "tokens": 40},
            {"tool": "theia", "tokens": 1}
        ]
    }))
    .unwrap();
    assert_eq!(
        bars::bars(Some(&glance)),
        vec![
            ("claude-code".into(), 900),
            ("unknown".into(), 40),
            ("codex".into(), 20),
            ("gemini-cli".into(), 5)
        ]
    );
    let unreadable = bars::Glance {
        readable: Some(false),
        ..glance.clone()
    };
    assert!(bars::bars(Some(&unreadable)).is_empty());
    let off = bars::Glance {
        enabled: false,
        ..glance
    };
    assert!(bars::bars(Some(&off)).is_empty());
    assert!(bars::bars(None).is_empty());
    assert_eq!(bars::height(900, 900), 190.0);
    assert_eq!(bars::height(1, 900), 8.0);
    assert_eq!(bars::width(4), 4 * 52 + 3 * 14);
    assert_eq!(bars::OFFSET, (30.0, 40.0));
    assert_eq!(
        (bars::INK_TOP, bars::INK_BOTTOM, bars::NAME_INK),
        (0.95, 0.45, 0.85)
    );
}

#[test]
fn todays_total_is_the_macs_compact_figure() {
    // probe: Foundation's `.notation(.compactName)`, en_US.
    for (value, want) in [
        (0, "0"),
        (7, "7"),
        (999, "999"),
        (1000, "1K"),
        (1234, "1.2K"),
        (9950, "10K"),
        (12_345, "12K"),
        (99_999, "100K"),
        (123_456, "123K"),
        (999_999, "1M"),
        (1_000_000, "1M"),
        (1_250_000, "1.2M"),
        (12_500_000, "12M"),
        (1_234_567_890, "1.2B"),
        (u64::MAX, "18,446,744T"),
    ] {
        assert_eq!(compact(value), want, "{value}");
    }
}

#[test]
fn an_arrival_is_marked_only_when_credit_grew() {
    use arrival::{Effect, Reading, effect};
    let shown = Reading {
        received_nanos: Some(10),
        remaining_nanos: Some(100),
    };
    let grew = Reading {
        received_nanos: Some(15),
        remaining_nanos: Some(105),
    };
    assert_eq!(
        effect(Some(shown), grew, false),
        Effect::CountUp { from: 100, to: 105 }
    );
    assert_eq!(effect(Some(shown), grew, true), Effect::Highlight);
    // Spending moves the balance and not the received total: nothing.
    let spent = Reading {
        received_nanos: Some(10),
        remaining_nanos: Some(90),
    };
    assert_eq!(effect(Some(shown), spent, false), Effect::None);
    // A first reading has nothing to compare with.
    assert_eq!(effect(None, grew, false), Effect::None);
    let same_figure = Reading {
        received_nanos: Some(15),
        remaining_nanos: Some(100),
    };
    assert_eq!(effect(Some(shown), same_figure, false), Effect::Highlight);
    // probe
    let steps = arrival::count_steps(1_000_000_000, 3_500_000_001, 24);
    assert_eq!(steps.len(), 24);
    assert_eq!(&steps[..3], &[1_104_166_666, 1_208_333_333, 1_312_500_000]);
    assert_eq!(*steps.last().unwrap(), 3_500_000_001);
    assert_eq!(
        (arrival::FRAMES, arrival::FRAME_MS, arrival::HOLD_MS),
        (24, 50, 1600)
    );
}

#[test]
fn the_coins_fly_as_the_macs_do() {
    assert_eq!(coins::COUNT, 44);
    assert_eq!(
        (coins::DURATION, coins::SPILL, coins::STILL_AT),
        (1.7, 120.0, 0.45)
    );
    assert_eq!(
        (coins::DRAG, coins::GRAVITY, coins::SQUASH, coins::FADE_FROM),
        (3.4, 50.0, 0.72, 0.55)
    );
    assert_eq!((coins::FLASH_DURATION, coins::FLASH_RADIUS), (0.28, 46.0));
    // probe
    let (dx, dy) = coins::offset(&coins::coin(5), 0.9, 1.0);
    assert!((dx - 66.1925581873046).abs() < 1e-9, "{dx}");
    assert!((dy - 64.2232490524118).abs() < 1e-9, "{dy}");
    assert_eq!(coins::opacity(0.5), 1.0);
    assert!((coins::opacity(1.7 * 0.55 + 0.3825) - 0.5).abs() < 1e-9);
    assert_eq!(coins::opacity(1.7), 0.0);
    // No room on the right: no coin flies right at all.
    assert_eq!(coins::sideways(0.0), 0.0);
    assert_eq!(coins::sideways(1000.0), 1.0);
}

/// Writes an ARGB32 surface as a PNG through GDK (cairo's PNG writer is not
/// built into this crate's cairo).
pub(crate) fn save_png(surface: &mut cairo::ImageSurface, path: &std::path::Path) {
    surface.flush();
    let (width, height, stride) = (surface.width(), surface.height(), surface.stride());
    let bytes = surface.data().expect("surface data").to_vec();
    let texture = gtk::gdk::MemoryTexture::new(
        width,
        height,
        gtk::gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &gtk::glib::Bytes::from_owned(bytes),
        stride as usize,
    );
    gtk::gdk::prelude::TextureExt::save_to_png(&texture, path).expect("PNG");
}

/// Draws each element offscreen, light and dark, into `TC_HERO_RENDER_DIR`
/// when it is set: the renders the PR compares with the Mac's.
#[test]
fn renders_for_comparison() {
    let Some(dir) = std::env::var_os("TC_HERO_RENDER_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (w, h) = (600, 156);
    for dark in [false, true] {
        let scheme = if dark { "dark" } else { "light" };
        for (name, ground, symbol) in [
            ("campaign-cpu", Ground::Campaign, Some("cpu")),
            ("missions-scope", Ground::Missions, Some("scope")),
            ("insights-chart", Ground::Insights, Some("chart.bar.xaxis")),
            ("campaign-sparkles", Ground::Campaign, Some("sparkles")),
            ("campaign-bolt", Ground::Campaign, Some("bolt")),
            ("campaign-globe", Ground::Campaign, Some("globe")),
        ] {
            for hover in [false, true] {
                let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
                let cr = cairo::Context::new(&surface).unwrap();
                let mask = symbol.and_then(glyph::hero_mask);
                paint_card(
                    &cr,
                    (w as f64, h as f64),
                    ground,
                    hover,
                    mask.as_ref(),
                    dark,
                    0.0,
                );
                drop(cr);
                let suffix = if hover { "-hover" } else { "" };
                let mut surface = surface;
                save_png(
                    &mut surface,
                    &dir.join(format!("{name}{suffix}-{scheme}.png")),
                );
            }
        }
        // The Insights hero's bars (their names are labels, not drawn
        // here), placed as `hero_card::place_bars` places them.
        {
            let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
            let cr = cairo::Context::new(&surface).unwrap();
            paint_card(
                &cr,
                (w as f64, h as f64),
                Ground::Insights,
                false,
                None,
                dark,
                0.0,
            );
            let tokens = [900u64, 400, 120, 40];
            let names = 13.0 + bars::NAME_GAP as f64;
            let tall = bars::height(tokens[0], tokens[0]) + names;
            let (x0, y0) = super::super::hero_card::place_bars(
                w,
                h,
                bars::width(tokens.len()),
                tall.round() as i32,
            );
            let _ = cr.save();
            super::rounded(&cr, 0.0, 0.0, w as f64, h as f64, card::RADIUS);
            cr.clip();
            for (index, value) in tokens.iter().enumerate() {
                let height = bars::height(*value, tokens[0]);
                let x = x0 as f64
                    + index as f64 * (bars::NAME_WIDTH + bars::SPACING) as f64
                    + (bars::NAME_WIDTH as f64 - bars::BAR_WIDTH) / 2.0;
                let y = y0 as f64 + tall - height;
                let _ = cr.save();
                cr.translate(x, y);
                bars::paint_bar(&cr, bars::BAR_WIDTH, height);
                let _ = cr.restore();
            }
            let _ = cr.restore();
            drop(cr);
            let mut surface = surface;
            save_png(
                &mut surface,
                &dir.join(format!("insights-bars-{scheme}.png")),
            );
        }
        // The coin burst at four moments over a 200 by 90 card.
        for t in [0.1, 0.45, 0.9, 1.4] {
            let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 440, 330).unwrap();
            let cr = cairo::Context::new(&surface).unwrap();
            cr.set_source_rgb(
                if dark { 0.05 } else { 0.95 },
                if dark { 0.06 } else { 0.95 },
                if dark { 0.1 } else { 0.97 },
            );
            let _ = cr.paint();
            coins::draw(&cr, (220.0, 165.0), t, true, 100.0 + coins::EDGE_ROOM);
            drop(cr);
            let mut surface = surface;
            save_png(&mut surface, &dir.join(format!("coins-t{t}-{scheme}.png")));
        }
    }
}
