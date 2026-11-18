//! The tool tile against the Mac's numbers: `GlassToolTile`'s sizes, its
//! grounds and inks per scheme, which tools get which mark, and the hand
//! drawn marks' extents measured on the Mac's own renders (ImageRenderer,
//! TCDesign at this base, 4x).

use super::*;

fn surface(side: f64, scale: f64) -> gtk::cairo::ImageSurface {
    let px = (side * scale).round() as i32;
    gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, px, px).unwrap()
}

/// The pixel at (x, y) in points, as 0-255 RGB, over an opaque ground.
fn pixel(s: &mut gtk::cairo::ImageSurface, x: f64, y: f64, scale: f64) -> [u8; 3] {
    let stride = s.stride() as usize;
    let (px, py) = ((x * scale) as usize, (y * scale) as usize);
    let data = s.data().unwrap();
    let at = py * stride + px * 4;
    // ARGB32 little-endian: B, G, R, A.
    [data[at + 2], data[at + 1], data[at]]
}

/// Draws `kind` at 4x over the Mac pane's colour (`paneOpaque`), as the
/// Mac's render was taken.
fn render(kind: &Kind, large: bool, dark: bool) -> gtk::cairo::ImageSurface {
    let side = metrics(large).side;
    let s = surface(side, 4.0);
    {
        let cr = gtk::cairo::Context::new(&s).unwrap();
        cr.scale(4.0, 4.0);
        Colour::token(if dark {
            map::dark::PANE_OPAQUE
        } else {
            map::light::PANE_OPAQUE
        })
        .set(&cr);
        let _ = cr.paint();
        draw(&cr, kind, large, dark);
    }
    s
}

/// The extent, in points, of the pixels that differ from the corner's.
fn extent(s: &mut gtk::cairo::ImageSurface, scale: f64) -> (f64, f64) {
    let (w, h) = (s.width() as usize, s.height() as usize);
    let stride = s.stride() as usize;
    let data = s.data().unwrap();
    let alpha = |x: usize, y: usize| data[y * stride + x * 4 + 3];
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if alpha(x, y) > 127 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    ((x1 - x0) as f64 / scale, (y1 - y0) as f64 / scale)
}

#[test]
fn the_tiles_are_the_macs_sizes() {
    assert_eq!(
        metrics(false),
        Metrics {
            side: 22.0,
            radius: 6.0,
            logo: 15.0
        }
    );
    // The flat theme's control radius, 11 (classic: 8).
    assert_eq!(
        metrics(true),
        Metrics {
            side: 30.0,
            radius: 11.0,
            logo: 20.0
        }
    );
    let tool = Kind::source(SourceKind::GeminiCli);
    assert_eq!(
        (glyph_size(&tool, false), glyph_size(&tool, true)),
        (9.0, 11.0)
    );
    assert_eq!(
        (
            glyph_size(&Kind::Folder, false),
            glyph_size(&Kind::Folder, true)
        ),
        (11.0, 13.0)
    );
    assert_eq!(
        (glyph_size(&Kind::Add, false), glyph_size(&Kind::Add, true)),
        (10.0, 13.0)
    );
    assert_eq!(glyph_size(&Kind::Session, true), 11.0);
}

#[test]
fn each_source_gets_the_macs_mark() {
    // TracesWords.sessionTile: the declared source first, then the adapter's.
    assert_eq!(
        Kind::session(Some("codex"), "claude-code"),
        Kind::source(SourceKind::Codex)
    );
    assert_eq!(
        Kind::session(Some("nonsense"), "claude-code"),
        Kind::source(SourceKind::ClaudeCode)
    );
    assert_eq!(Kind::session(None, "aider"), Kind::Session);
    // ManagedWords.glassTool.
    assert_eq!(
        Kind::managed("claude"),
        Kind::source(SourceKind::ClaudeCode)
    );
    assert_eq!(
        Kind::managed("opencode"),
        Kind::source(SourceKind::Opencode)
    );
    assert_eq!(
        Kind::managed("gemini"),
        Kind::Tool {
            logo: None,
            tint: Tint::Primary,
            initials: "Ge".into()
        }
    );
    // InsightsHarnessTile.
    assert_eq!(
        Kind::harness("claude_code"),
        Some(Kind::source(SourceKind::ClaudeCode))
    );
    assert_eq!(Kind::harness("aider"), None);
    // GlassToolTile.initial: the first letter, upper-cased; the core's
    // folder mark for none.
    assert_eq!(
        Kind::FolderInitial("web".into()).text("dir"),
        Some("W".into())
    );
    assert_eq!(
        Kind::FolderInitial(String::new()).text("dir"),
        Some("dir".into())
    );
    assert_eq!(
        Kind::source(SourceKind::GeminiCli).text("dir"),
        Some("Gm".into())
    );
    assert_eq!(Kind::source(SourceKind::Codex).text("dir"), None);
}

/// Pixels the Mac's render shows: the tile ground at the edge's middle,
/// the mark at the centre. Light: a tool's ground is black at 0.1 over
/// the pane (#f2f3f6), so (218, 219, 221); Claude's logo is #c2603e; the
/// folder ground #e6e6ea with its mark in #1d1d1f.
#[test]
fn the_grounds_and_marks_are_the_macs_pixels() {
    let cases: [(Kind, bool, [u8; 3], [u8; 3]); 4] = [
        (
            Kind::source(SourceKind::ClaudeCode),
            false,
            [218, 219, 221],
            [194, 96, 62],
        ),
        (Kind::Folder, false, [230, 230, 234], [29, 29, 31]),
        // Dark: white at 0.1 over #0d1124, Claude's dark tint #e08a6a.
        (
            Kind::source(SourceKind::ClaudeCode),
            true,
            [37, 41, 58],
            [224, 138, 106],
        ),
        (Kind::Folder, true, [42, 42, 44], [242, 243, 248]),
    ];
    for (kind, dark, edge, centre) in cases {
        let mut s = render(&kind, false, dark);
        let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2);
        let got = pixel(&mut s, 0.5, 11.0, 4.0);
        assert!(near(got, edge), "{kind:?} dark={dark}: edge {got:?}");
        let got = pixel(&mut s, 11.0, 11.0, 4.0);
        assert!(near(got, centre), "{kind:?} dark={dark}: centre {got:?}");
    }
}

/// The hand-drawn marks are the size of the SF Symbols they stand in for,
/// within half a point: `folder.fill` 12.25 x 9.75 at 11 and 14.5 x 11.5 at
/// 13, `plus` 8.25 at 10 and 10.75 at 13, the session glyph 8.25 x 8 at 11.
#[test]
fn the_drawn_marks_are_the_symbols_sizes() {
    let measure = |draw: &dyn Fn(&gtk::cairo::Context)| {
        let mut s = surface(30.0, 4.0);
        {
            let cr = gtk::cairo::Context::new(&s).unwrap();
            cr.scale(4.0, 4.0);
            draw(&cr);
        }
        extent(&mut s, 4.0)
    };
    let black = Colour {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    let cases: [(&str, (f64, f64), Box<dyn Fn(&gtk::cairo::Context)>); 5] = [
        (
            "folder 11",
            (12.25, 9.75),
            Box::new(move |cr| folder_mark(cr, 15.0, 15.0, 11.0, black)),
        ),
        (
            "folder 13",
            (14.5, 11.5),
            Box::new(move |cr| folder_mark(cr, 15.0, 15.0, 13.0, black)),
        ),
        (
            "plus 10",
            (8.25, 8.25),
            Box::new(move |cr| plus_mark(cr, 15.0, 15.0, 10.0, black)),
        ),
        (
            "plus 13",
            (10.75, 10.75),
            Box::new(move |cr| plus_mark(cr, 15.0, 15.0, 13.0, black)),
        ),
        (
            "session 11",
            (8.25, 8.0),
            Box::new(move |cr| session_mark(cr, 15.0, 15.0, 11.0, black)),
        ),
    ];
    for (name, (w, h), draw) in cases {
        let (got_w, got_h) = measure(&*draw);
        assert!(
            (got_w - w).abs() <= 0.5 && (got_h - h).abs() <= 0.5,
            "{name}: {got_w} x {got_h}, the Mac's {w} x {h}"
        );
    }
}
