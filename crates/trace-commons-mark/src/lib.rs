//! "The Turn" -- the product mark, as pure geometry.
//!
//! Two corner brackets facing each other inside a hairline frame: the user's
//! bracket top-left in green, the agent's answer bottom-right in blue, and the
//! session implied in the space between them. No gradients, no fills other
//! than the frame, no asset file behind any of it.
//!
//! The geometry is transcribed unit for unit on a 64-unit coordinate space:
//!
//! ```text
//! frame     rect x=1 y=1 w=62 h=62, stroke-width 2
//! green     M11 28 V11 H28          stroke-width 7
//! blue      M53 36 v17 H36          stroke-width 7
//! template  the same two paths in one ink, stroke-width 8, no frame
//! ```
//!
//! The frame is inset one unit under a two-unit stroke, so its outer edge
//! lands exactly on the 64x64 boundary. The template variant thickens to 8
//! because it loses the frame that was holding the brackets apart.
//!
//! ## Why this crate exists
//!
//! The mark is drawn live in all three clients -- `BrandMark.swift` on macOS,
//! `ui/mark.rs` on Linux, `Controls/BrandMark.xaml.cs` on Windows -- and that
//! stays true. A `DrawingArea` or a SwiftUI `Shape` is handed the real device
//! scale and redraws into it, so a UI surface needs no file and no size
//! ladder.
//!
//! Packaging is the exception. An operating system that wants a Dock icon, a
//! Start tile or a `hicolor` entry wants a file on disk, and until this crate
//! existed nobody generated one: macOS shipped an empty `Contents/Resources`,
//! the Linux desktop entry named an icon nothing installed, and the three
//! Windows tiles were solid squares of `#315FBA`. This crate is the single
//! description those files are generated from, and the drift tests below plus
//! the per-client transcription tests are what keep the four descriptions of
//! the mark from wandering apart.
//!
//! Nothing here has a dependency, and nothing here rasterizes. SVG out; each
//! platform's own toolchain turns that into whatever it needs.
//!
//! ## Colour
//!
//! Every ink is a palette token, repeated here as a literal because this crate
//! cannot reach any client's palette. If a value below ever drifts from a
//! client's own token, the client's token is right and
//! [`tests::palette_matches_client_tokens`] is what should have caught it.

pub mod raster;

/// The mark's coordinate space. Every number in this module is in these units
/// and is scaled to the requested pixel size at render time.
pub const VIEW: u32 = 64;

/// Stroke width of a bracket inside the frame, in view units.
pub const STROKE_FRAMED: u32 = 7;

/// Stroke width of a bracket without the frame. Thicker, because the frame is
/// no longer there to give the two brackets something to sit against.
pub const STROKE_TEMPLATE: u32 = 8;

/// Stroke width of the frame itself, in view units.
pub const STROKE_FRAME: u32 = 2;

/// The frame rectangle, as `(x, y, width, height)` in view units. Inset one
/// unit so a two-unit stroke lands its outer edge on the view box boundary.
pub const FRAME_RECT: (u32, u32, u32, u32) = (1, 1, 62, 62);

/// The user's bracket, top-left. `M11 28 V11 H28`.
pub const PATH_GREEN: &str = "M11 28V11h17";

/// The agent's answer, bottom-right. `M53 36 v17 H36`.
pub const PATH_BLUE: &str = "M53 36v17H36";

/// The green bracket as vertices, for renderers that draw lines rather than
/// parse a path string -- cairo on Linux, `CGPath` on macOS, Win2D on Windows.
///
/// The SVG path constants above say the same thing in SVG's own notation.
/// [`tests::path_strings_agree_with_vertices`] is what keeps the two spellings
/// from drifting; without it this would be the fifth description of the mark
/// rather than a second view of the first.
pub const VERTICES_GREEN: [(u32, u32); 3] = [(11, 28), (11, 11), (28, 11)];

/// The blue bracket as vertices. See [`VERTICES_GREEN`].
pub const VERTICES_BLUE: [(u32, u32); 3] = [(53, 36), (53, 53), (36, 53)];

/// Which palette the mark is drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    Light,
    Dark,
}

impl Scheme {
    /// Both schemes, in the order the export tool writes them.
    pub const ALL: [Scheme; 2] = [Scheme::Light, Scheme::Dark];

    /// The lowercase name used in generated filenames.
    pub fn name(self) -> &'static str {
        match self {
            Scheme::Light => "light",
            Scheme::Dark => "dark",
        }
    }

    /// Frame fill. `tc_surface`.
    pub fn surface(self) -> &'static str {
        match self {
            Scheme::Light => "#FFFFFF",
            Scheme::Dark => "#21241E",
        }
    }

    /// Frame stroke. `tc_line`.
    pub fn line(self) -> &'static str {
        match self {
            Scheme::Light => "#D9DFDC",
            Scheme::Dark => "#3B4038",
        }
    }

    /// The user's bracket. `tc_green`.
    pub fn green(self) -> &'static str {
        match self {
            Scheme::Light => "#178F70",
            Scheme::Dark => "#3FBE9A",
        }
    }

    /// The agent's bracket. `tc_blue`.
    pub fn blue(self) -> &'static str {
        match self {
            Scheme::Light => "#315FBA",
            Scheme::Dark => "#7FA0EC",
        }
    }

    /// The single ink of the template variant. `tc_ink`.
    ///
    /// A status area recolours a template icon itself where it can, so this is
    /// what it falls back to when it cannot.
    pub fn ink(self) -> &'static str {
        match self {
            Scheme::Light => "#20241F",
            Scheme::Dark => "#E8EAE3",
        }
    }
}

/// The mark as an SVG document, for surfaces that can only take a serialised
/// icon.
///
/// `size` is the rendered edge in pixels; the geometry stays on its 64-unit
/// `viewBox`, so the document is resolution-independent whatever is passed
/// here.
pub fn svg(scheme: Scheme, size: u32) -> String {
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" "#,
            r#"viewBox="0 0 {view} {view}" role="img" aria-label="Trace Commons">"#,
            r#"<rect x="{fx}" y="{fy}" width="{fw}" height="{fh}" fill="{surface}" stroke="{line}" stroke-width="{frame_stroke}"/>"#,
            r#"<path d="{green_path}" fill="none" stroke="{green}" stroke-width="{stroke}"/>"#,
            r#"<path d="{blue_path}" fill="none" stroke="{blue}" stroke-width="{stroke}"/>"#,
            r#"</svg>"#,
        ),
        size = size,
        view = VIEW,
        fx = FRAME_RECT.0,
        fy = FRAME_RECT.1,
        fw = FRAME_RECT.2,
        fh = FRAME_RECT.3,
        frame_stroke = STROKE_FRAME,
        green_path = PATH_GREEN,
        blue_path = PATH_BLUE,
        stroke = STROKE_FRAMED,
        surface = scheme.surface(),
        line = scheme.line(),
        green = scheme.green(),
        blue = scheme.blue(),
    )
}

/// The frameless, single-ink template variant as an SVG document.
///
/// `ink` is a CSS colour written straight into the document -- pass
/// [`Scheme::ink`] unless a status area has told you what it wants.
pub fn template_svg(ink: &str, size: u32) -> String {
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" "#,
            r#"viewBox="0 0 {view} {view}" role="img" aria-label="Trace Commons">"#,
            r#"<path d="{green_path}" fill="none" stroke="{ink}" stroke-width="{stroke}"/>"#,
            r#"<path d="{blue_path}" fill="none" stroke="{ink}" stroke-width="{stroke}"/>"#,
            r#"</svg>"#,
        ),
        size = size,
        view = VIEW,
        green_path = PATH_GREEN,
        blue_path = PATH_BLUE,
        stroke = STROKE_TEMPLATE,
        ink = ink,
    )
}

/// The geometry and palette as JSON, for renderers that cannot read SVG.
///
/// macOS builds its `.icns` with CoreGraphics rather than by rasterizing the
/// SVG, because the rasterizers available on a stock macOS runner get it
/// wrong: `sips` zeroes the blue channel in the bottom-right corner at 16 and
/// 32 pixels, which is both invisible in a build log and exactly the size the
/// Finder and the menu bar use. A renderer that draws the geometry itself has
/// no such failure mode.
///
/// This is emitted rather than hand-written on the Swift side so the numbers
/// still come from here. Written by hand rather than with serde: this crate is
/// dependency-free on purpose, and the document is nine fields.
pub fn geometry_json() -> String {
    let vertices = |v: &[(u32, u32); 3]| {
        v.iter()
            .map(|(x, y)| format!("[{x}, {y}]"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let scheme_object = |s: Scheme| {
        format!(
            concat!(
                "{{\n      \"surface\": \"{surface}\",\n",
                "      \"line\": \"{line}\",\n",
                "      \"green\": \"{green}\",\n",
                "      \"blue\": \"{blue}\",\n",
                "      \"ink\": \"{ink}\"\n    }}",
            ),
            surface = s.surface(),
            line = s.line(),
            green = s.green(),
            blue = s.blue(),
            ink = s.ink(),
        )
    };
    format!(
        concat!(
            "{{\n",
            "  \"view\": {view},\n",
            "  \"frame\": [{fx}, {fy}, {fw}, {fh}],\n",
            "  \"strokeFrame\": {sframe},\n",
            "  \"strokeFramed\": {sframed},\n",
            "  \"strokeTemplate\": {stemplate},\n",
            "  \"green\": [{green}],\n",
            "  \"blue\": [{blue}],\n",
            "  \"schemes\": {{\n    \"light\": {light},\n    \"dark\": {dark}\n  }}\n",
            "}}",
        ),
        view = VIEW,
        fx = FRAME_RECT.0,
        fy = FRAME_RECT.1,
        fw = FRAME_RECT.2,
        fh = FRAME_RECT.3,
        sframe = STROKE_FRAME,
        sframed = STROKE_FRAMED,
        stemplate = STROKE_TEMPLATE,
        green = vertices(&VERTICES_GREEN),
        blue = vertices(&VERTICES_BLUE),
        light = scheme_object(Scheme::Light),
        dark = scheme_object(Scheme::Dark),
    )
}

/// One generated file: the repository-relative path under the export root, and
/// its contents.
pub struct Export {
    pub relative_path: &'static str,
    pub contents: String,
}

/// One generated binary file: a path relative to the repository root, and its
/// bytes.
///
/// Unlike [`Export`], the path is relative to the repository rather than to an
/// export directory, because these files have to land where a packaging
/// manifest already names them. `Package.appxmanifest` references
/// `Assets\StoreLogo.png`, and MSIX resolves that against the package layout --
/// so the tile cannot live under `assets/mark` with the SVGs and be copied
/// later without something doing the copying, which is one more step nothing
/// would verify.
pub struct BinaryExport {
    pub repo_path: &'static str,
    pub bytes: Vec<u8>,
}

/// Every raster asset a packaging manifest names, in a stable order.
///
/// # Why these are generated in Rust rather than by a platform toolchain
///
/// These three files were solid `#315FBA` squares for months. They were the
/// right dimensions with the right names in the right brand colour, so the only
/// checks that looked at them passed. The first fix generated them with
/// CoreGraphics, which put the mark on them but left them regenerable only on a
/// Mac -- and the drift check runs on `ubuntu-latest`, so they stayed outside
/// it. An asset nothing can regenerate is an asset nothing can verify.
///
/// [`raster`] renders them with no dependencies and no platform toolchain, so
/// the drift check can regenerate them on any runner and compare bytes.
///
/// The sizes come from the filenames, which are what `Package.appxmanifest`
/// names and what MSIX resolves against; see the manifest's `Logo` and
/// `uap:VisualElements` elements.
pub fn windows_tiles() -> Vec<BinaryExport> {
    const TILES: [(&str, u32); 3] = [
        ("windows/packaging/Assets/StoreLogo.png", 50),
        ("windows/packaging/Assets/Square150x150Logo.png", 150),
        ("windows/packaging/Assets/Square44x44Logo.png", 44),
    ];
    TILES
        .into_iter()
        .map(|(repo_path, size)| BinaryExport {
            repo_path,
            // Light only. A Start tile is composited on a background the app
            // does not choose, and the manifest sets `BackgroundColor` to
            // transparent, so the tile carries its own light surface rather
            // than following a system appearance it cannot observe.
            bytes: raster::png(Scheme::Light, size),
        })
        .collect()
}

/// Every SVG the packaging surfaces consume, in a stable order.
///
/// The export tool writes exactly this list and the drift check re-runs it, so
/// adding a packaging surface means adding it here and nowhere else.
///
/// Sizes are the nominal `width`/`height` on the document. They do not
/// constrain anything -- the `viewBox` makes every one of these scalable -- but
/// a consumer that ignores the view box gets a sensible default rather than a
/// 64px icon on a 1024px canvas.
pub fn all_exports() -> Vec<Export> {
    let mut out = Vec::new();
    for scheme in Scheme::ALL {
        out.push(Export {
            relative_path: match scheme {
                Scheme::Light => "mark-light.svg",
                Scheme::Dark => "mark-dark.svg",
            },
            contents: svg(scheme, 1024),
        });
        out.push(Export {
            relative_path: match scheme {
                Scheme::Light => "mark-template-light.svg",
                Scheme::Dark => "mark-template-dark.svg",
            },
            contents: template_svg(scheme.ink(), 1024),
        });
    }
    out.push(Export {
        relative_path: "geometry.json",
        contents: geometry_json(),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers themselves. If a change to this crate is meant to change the
    /// mark, this test is the one that has to be edited on purpose -- which is
    /// the point, because everything else generated here follows from it.
    #[test]
    fn geometry_constants_are_the_canonical_numbers() {
        assert_eq!(VIEW, 64);
        assert_eq!(FRAME_RECT, (1, 1, 62, 62));
        assert_eq!(STROKE_FRAME, 2);
        assert_eq!(STROKE_FRAMED, 7);
        assert_eq!(STROKE_TEMPLATE, 8);
        assert_eq!(PATH_GREEN, "M11 28V11h17");
        assert_eq!(PATH_BLUE, "M53 36v17H36");
    }

    /// The frame's outer edge lands exactly on the view box boundary: inset by
    /// half the stroke on each side. A frame that failed this would clip on one
    /// edge and float on the other, and at 16px nobody would be able to say
    /// which.
    #[test]
    fn frame_outer_edge_lands_on_the_view_box() {
        let (x, y, w, h) = FRAME_RECT;
        let half = STROKE_FRAME / 2;
        assert_eq!(x - half, 0);
        assert_eq!(y - half, 0);
        assert_eq!(x + w + half, VIEW);
        assert_eq!(y + h + half, VIEW);
    }

    /// The SVG path strings and the vertex lists are two spellings of the same
    /// two brackets. A renderer that draws lines uses one and a renderer that
    /// parses SVG uses the other, so nothing else in the codebase would notice
    /// them disagreeing.
    #[test]
    fn path_strings_agree_with_vertices() {
        // "M11 28V11h17" -- absolute move, absolute vertical, relative
        // horizontal. Rebuilt here from the vertices in exactly that notation.
        let green = format!(
            "M{} {}V{}h{}",
            VERTICES_GREEN[0].0,
            VERTICES_GREEN[0].1,
            VERTICES_GREEN[1].1,
            VERTICES_GREEN[2].0 - VERTICES_GREEN[1].0,
        );
        assert_eq!(green, PATH_GREEN);

        // "M53 36v17H36" -- absolute move, relative vertical, absolute
        // horizontal. The two brackets are spelled differently in the source
        // this was transcribed from, and that asymmetry is preserved.
        let blue = format!(
            "M{} {}v{}H{}",
            VERTICES_BLUE[0].0,
            VERTICES_BLUE[0].1,
            VERTICES_BLUE[1].1 - VERTICES_BLUE[0].1,
            VERTICES_BLUE[2].0,
        );
        assert_eq!(blue, PATH_BLUE);
    }

    /// The two brackets are rotationally symmetric about the centre of the view
    /// box: the blue bracket is the green one turned 180 degrees. That is the
    /// whole idea of the mark -- the user's corner and the agent's answer
    /// facing each other -- so it is worth asserting rather than trusting six
    /// hand-typed coordinate pairs.
    #[test]
    fn brackets_are_rotationally_symmetric() {
        for (green, blue) in VERTICES_GREEN.iter().zip(VERTICES_BLUE.iter()) {
            assert_eq!(VIEW - green.0, blue.0, "x of {green:?} vs {blue:?}");
            assert_eq!(VIEW - green.1, blue.1, "y of {green:?} vs {blue:?}");
        }
    }

    /// The palette literals here are copies of tokens that live in each
    /// client's own design system. This test pins the copies; the per-client
    /// transcription tests pin the originals against these.
    #[test]
    fn palette_matches_client_tokens() {
        assert_eq!(Scheme::Light.surface(), "#FFFFFF");
        assert_eq!(Scheme::Dark.surface(), "#21241E");
        assert_eq!(Scheme::Light.line(), "#D9DFDC");
        assert_eq!(Scheme::Dark.line(), "#3B4038");
        assert_eq!(Scheme::Light.green(), "#178F70");
        assert_eq!(Scheme::Dark.green(), "#3FBE9A");
        assert_eq!(Scheme::Light.blue(), "#315FBA");
        assert_eq!(Scheme::Dark.blue(), "#7FA0EC");
        assert_eq!(Scheme::Light.ink(), "#20241F");
        assert_eq!(Scheme::Dark.ink(), "#E8EAE3");
    }

    /// The tiles are named by the manifest, at the sizes their filenames
    /// promise. A tile generated at the wrong size still installs; it is just
    /// resampled, which looks like a slightly soft icon rather than a failure.
    #[test]
    fn windows_tiles_are_the_sizes_their_names_promise() {
        let tiles = windows_tiles();
        assert_eq!(tiles.len(), 3);
        for tile in &tiles {
            let name = tile
                .repo_path
                .rsplit('/')
                .next()
                .expect("tile path has a filename");
            let want: u32 = match name {
                "StoreLogo.png" => 50,
                "Square150x150Logo.png" => 150,
                "Square44x44Logo.png" => 44,
                other => panic!("unexpected tile {other}"),
            };
            // IHDR width and height are bytes 16..24 of a PNG.
            let w = u32::from_be_bytes(tile.bytes[16..20].try_into().unwrap());
            let h = u32::from_be_bytes(tile.bytes[20..24].try_into().unwrap());
            assert_eq!((w, h), (want, want), "{}", tile.repo_path);
        }
    }

    /// The teeth, at the level the packaging actually consumes. Not "the file
    /// exists", not "it is the right size", not "it has more than one colour" --
    /// the two bracket inks have to be present in the decoded pixels, because a
    /// flat square satisfies all three of the others and that is precisely what
    /// shipped.
    #[test]
    fn windows_tiles_carry_both_brackets() {
        for tile in windows_tiles() {
            let size = u32::from_be_bytes(tile.bytes[16..20].try_into().unwrap());
            let pixels = raster::render_framed(Scheme::Light, size);
            let ink = |hex: &str| {
                let b = hex.as_bytes();
                let n = |c: u8| match c {
                    b'0'..=b'9' => c - b'0',
                    b'A'..=b'F' => c - b'A' + 10,
                    _ => 0,
                };
                [
                    n(b[1]) * 16 + n(b[2]),
                    n(b[3]) * 16 + n(b[4]),
                    n(b[5]) * 16 + n(b[6]),
                    255,
                ]
            };
            for (name, hex) in [
                ("green", Scheme::Light.green()),
                ("blue", Scheme::Light.blue()),
            ] {
                let want = ink(hex);
                assert!(
                    pixels.chunks(4).any(|p| p == want),
                    "{} has no {name} bracket",
                    tile.repo_path
                );
            }
        }
    }

    /// The emitted document has to carry the geometry, not merely be
    /// well-formed. A generator that emitted an empty `<svg/>` would satisfy
    /// every check that only counts files.
    #[test]
    fn framed_svg_carries_both_brackets_and_the_frame() {
        let doc = svg(Scheme::Light, 512);
        assert!(doc.contains(r#"width="512" height="512""#), "{doc}");
        assert!(doc.contains(r#"viewBox="0 0 64 64""#), "{doc}");
        assert!(doc.contains(PATH_GREEN), "{doc}");
        assert!(doc.contains(PATH_BLUE), "{doc}");
        assert!(doc.contains(r#"stroke-width="7""#), "{doc}");
        assert!(
            doc.contains(r#"<rect x="1" y="1" width="62" height="62""#),
            "{doc}"
        );
        assert!(doc.contains("#178F70"), "{doc}");
        assert!(doc.contains("#315FBA"), "{doc}");
    }

    /// The template variant is the one that has to survive being masked to a
    /// single channel, so its two defining properties -- no frame, thicker
    /// stroke -- are asserted rather than assumed.
    #[test]
    fn template_svg_has_no_frame_and_the_thicker_stroke() {
        let doc = template_svg(Scheme::Light.ink(), 15);
        assert!(
            !doc.contains("<rect"),
            "template must not carry a frame: {doc}"
        );
        assert!(doc.contains(r#"stroke-width="8""#), "{doc}");
        assert!(doc.contains(PATH_GREEN), "{doc}");
        assert!(doc.contains(PATH_BLUE), "{doc}");
        assert!(!doc.contains("#178F70"), "template is single-ink: {doc}");
    }

    /// Four documents, distinct paths, and none of them empty. The list drives
    /// both the export tool and the drift check, so a duplicate path here would
    /// silently drop a packaging surface.
    #[test]
    fn exports_are_five_distinct_non_empty_documents() {
        let exports = all_exports();
        assert_eq!(exports.len(), 5);
        let mut paths: Vec<&str> = exports.iter().map(|e| e.relative_path).collect();
        paths.sort_unstable();
        assert_eq!(
            paths,
            [
                "geometry.json",
                "mark-dark.svg",
                "mark-light.svg",
                "mark-template-dark.svg",
                "mark-template-light.svg",
            ]
        );
        for export in &exports {
            assert!(!export.contents.is_empty(), "{}", export.relative_path);
            if !export.relative_path.ends_with(".svg") {
                continue;
            }
            assert!(
                export.contents.starts_with("<svg") && export.contents.ends_with("</svg>"),
                "{} is not a complete document",
                export.relative_path
            );
            assert!(
                export.contents.contains(PATH_GREEN),
                "{}",
                export.relative_path
            );
            assert!(
                export.contents.contains(PATH_BLUE),
                "{}",
                export.relative_path
            );
        }
    }

    /// The JSON the macOS renderer consumes has to carry every number that
    /// renderer needs, and carry them as numbers rather than strings. A typo
    /// here surfaces as a blank icon at build time, which is the failure this
    /// slice exists to stop shipping.
    #[test]
    fn geometry_json_carries_the_numbers_the_renderer_needs() {
        let doc = geometry_json();
        assert!(doc.contains(r#""view": 64"#), "{doc}");
        assert!(doc.contains(r#""frame": [1, 1, 62, 62]"#), "{doc}");
        assert!(doc.contains(r#""strokeFrame": 2"#), "{doc}");
        assert!(doc.contains(r#""strokeFramed": 7"#), "{doc}");
        assert!(doc.contains(r#""strokeTemplate": 8"#), "{doc}");
        assert!(
            doc.contains(r#""green": [[11, 28], [11, 11], [28, 11]]"#),
            "{doc}"
        );
        assert!(
            doc.contains(r#""blue": [[53, 36], [53, 53], [36, 53]]"#),
            "{doc}"
        );
        assert!(doc.contains("#FFFFFF"), "{doc}");
        assert!(doc.contains("#178F70"), "{doc}");
        assert!(doc.contains("#7FA0EC"), "{doc}");
    }

    /// Light and dark have to actually differ, in both treatments. Emitting the
    /// same document twice under two names is a failure that looks like success
    /// in any file listing.
    #[test]
    fn light_and_dark_differ() {
        assert_ne!(svg(Scheme::Light, 64), svg(Scheme::Dark, 64));
        assert_ne!(
            template_svg(Scheme::Light.ink(), 64),
            template_svg(Scheme::Dark.ink(), 64)
        );
    }
}
