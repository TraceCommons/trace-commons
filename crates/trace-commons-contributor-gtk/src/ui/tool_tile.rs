//! The Mac's tool tile (`GlassToolTile` in
//! `macos/Sources/TCDesign/Components/Indicators.swift`): the 22-point
//! square in a list row, or the 30-point one in a header, holding a tool's
//! logo in its tint (or its initials when it has none), a folder, a
//! folder's first letter, a "+" or a session mark.
//!
//! Sizes, radii and colours are the flat theme's (`brand_tokens::tile`, the
//! tool tints from `brand_tokens::map`). The folder, "+" and session marks
//! are SF Symbols and a text glyph on the Mac; here they are drawn by hand
//! to look like them (owner, 2026-11-18), sized from the Mac's renders.
//! Decorative, as on the Mac: hidden from assistive tech, the row carries
//! the words.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::monitor::glyph::Artwork;
use super::monitor::tool_art::{self, Tint};
use super::monitor::traces_tree::SourceKind;
use super::style::brand_tokens::{map, tile};

/// What a tile shows (`GlassToolTile.Kind`).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Kind {
    /// A tool: its logo in its tint, or its two letters when it has none.
    Tool {
        logo: Option<Artwork>,
        tint: Tint,
        initials: String,
    },
    /// A folder: a folder mark in the primary ink on the folder ground.
    Folder,
    /// A folder marked with its name's first letter, upper-cased.
    FolderInitial(String),
    /// The "+" of an add box.
    Add,
    /// A session from a tool this build does not know.
    #[default]
    Session,
}

impl Kind {
    /// A tool this build knows (`SourceKind.glassTool`).
    pub fn source(kind: SourceKind) -> Self {
        let art = tool_art::tool_art(kind);
        Kind::Tool {
            logo: art.logo,
            tint: art.tint,
            initials: art.initials.to_owned(),
        }
    }

    /// A session row's tile (`TracesWords.sessionTile`): its tool's, from
    /// the declared source first, else the plain session tile.
    pub fn session(declared: Option<&str>, source: &str) -> Self {
        declared
            .and_then(SourceKind::from_wire)
            .or_else(|| SourceKind::from_wire(source))
            .map_or(Kind::Session, Kind::source)
    }

    /// A managed session's tool (`ManagedWords.glassTool`): Claude Code,
    /// Codex and OpenCode by their marks, any other by its first two
    /// letters capitalised, in the primary ink.
    pub fn managed(tool: &str) -> Self {
        match tool {
            "claude" => Kind::source(SourceKind::ClaudeCode),
            "codex" => Kind::source(SourceKind::Codex),
            "opencode" => Kind::source(SourceKind::Opencode),
            other => {
                let mut letters = other.chars().take(2);
                let initials = letters
                    .next()
                    .map(|first| {
                        first
                            .to_uppercase()
                            .chain(letters.flat_map(char::to_lowercase))
                    })
                    .into_iter()
                    .flatten()
                    .collect();
                Kind::Tool {
                    logo: None,
                    tint: Tint::Primary,
                    initials,
                }
            }
        }
    }

    /// An Insights harness by its wire name (`claude_code`); `None` for one
    /// this build does not know (`InsightsHarnessTile` draws nothing).
    pub fn harness(source: &str) -> Option<Self> {
        SourceKind::from_wire(&source.replace('_', "-")).map(Kind::source)
    }

    /// The text the tile draws, if it draws text: a tool's initials or a
    /// folder's first letter (`GlassToolTile.initial`), with `mark` for a
    /// folder with no name.
    pub fn text(&self, mark: &str) -> Option<String> {
        match self {
            Kind::Tool {
                logo: None,
                initials,
                ..
            } => Some(initials.clone()),
            Kind::FolderInitial(name) => Some(
                name.chars()
                    .next()
                    .map_or_else(|| mark.to_owned(), |c| c.to_uppercase().collect()),
            ),
            _ => None,
        }
    }
}

/// A tile's square side, corner radius and logo side, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub side: f64,
    pub radius: f64,
    pub logo: f64,
}

/// The small tile rounds like a tile, the large like a control (#1146
/// `.tc-tool-tile--lg`).
pub fn metrics(large: bool) -> Metrics {
    if large {
        Metrics {
            side: tile::TOOL_TILE_LARGE,
            radius: tile::RADIUS_CONTROL,
            logo: tile::TOOL_LOGO_LARGE,
        }
    } else {
        Metrics {
            side: tile::TOOL_TILE,
            radius: tile::RADIUS_TILE,
            logo: tile::TOOL_LOGO,
        }
    }
}

/// The point size of the tile's text or drawn mark (`.glassGlyph(_:)`).
pub fn glyph_size(kind: &Kind, large: bool) -> f64 {
    match kind {
        Kind::Tool { .. } | Kind::FolderInitial(_) => {
            if large {
                11.0
            } else {
                9.0
            }
        }
        Kind::Folder => {
            if large {
                13.0
            } else {
                11.0
            }
        }
        Kind::Add => {
            if large {
                13.0
            } else {
                10.0
            }
        }
        Kind::Session => 11.0,
    }
}

/// An RGBA colour as cairo takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colour {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Colour {
    pub fn token((hex, alpha): (&str, f64)) -> Self {
        let channel = |at: usize| {
            f64::from(u8::from_str_radix(hex.get(at..at + 2).unwrap_or("00"), 16).unwrap_or(0))
                / 255.0
        };
        Colour {
            r: channel(1),
            g: channel(3),
            b: channel(5),
            a: alpha,
        }
    }

    pub fn alpha(self, alpha: f64) -> Self {
        Colour {
            a: self.a * alpha,
            ..self
        }
    }

    pub fn set(self, cr: &gtk::cairo::Context) {
        cr.set_source_rgba(self.r, self.g, self.b, self.a);
    }

    pub fn gdk(self) -> gtk::gdk::RGBA {
        gtk::gdk::RGBA::new(self.r as f32, self.g as f32, self.b as f32, self.a as f32)
    }
}

/// A tool's tint in the scheme (`GlassTool.tint`).
pub fn tint(tint: Tint, dark: bool) -> Colour {
    macro_rules! pick {
        ($name:ident) => {
            Colour::token(if dark {
                map::dark::$name
            } else {
                map::light::$name
            })
        };
    }
    match tint {
        Tint::Claude => pick!(TOOL_CLAUDE),
        Tint::Codex => pick!(TOOL_CODEX),
        Tint::Antigravity => pick!(TOOL_ANTIGRAVITY),
        Tint::OpenCode => pick!(TOOL_OPEN_CODE),
        Tint::Theia => pick!(TOOL_THEIA),
        Tint::Primary => pick!(TEXT_PRIMARY),
    }
}

/// `GlassColor.ink(alpha)`: white on dark, black on light, at `alpha`.
pub fn ink(alpha: f64, dark: bool) -> Colour {
    Colour::token(if dark {
        tile::dark::INK
    } else {
        tile::light::INK
    })
    .alpha(alpha)
}

/// The tile's ground and the ink its mark is drawn in.
pub fn colours(kind: &Kind, dark: bool) -> (Colour, Colour) {
    let folder = Colour::token(if dark {
        tile::dark::TILE_FOLDER
    } else {
        tile::light::TILE_FOLDER
    });
    let scheme = |light: map::Rgba, dark_value: map::Rgba| {
        Colour::token(if dark { dark_value } else { light })
    };
    match kind {
        Kind::Tool { tint: t, .. } => (ink(0.1, dark), tint(*t, dark)),
        Kind::Folder => (
            folder,
            scheme(map::light::TEXT_PRIMARY, map::dark::TEXT_PRIMARY),
        ),
        Kind::FolderInitial(_) => (
            folder,
            Colour::token(if dark {
                tile::dark::TILE_FOLDER_INK
            } else {
                tile::light::TILE_FOLDER_INK
            }),
        ),
        Kind::Add => (
            ink(0.1, dark),
            scheme(map::light::TEXT_SECONDARY, map::dark::TEXT_SECONDARY),
        ),
        Kind::Session => (
            folder,
            scheme(map::light::STATUS_OFF, map::dark::STATUS_OFF),
        ),
    }
}

/// A rounded rectangle path.
pub fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 1.5 * pi);
    cr.close_path();
}

/// SF Symbols' `folder.fill` at `size` points, drawn by hand, centred on
/// `(cx, cy)`: a tab on the top-left over a body, with the slit under the
/// flap. 1.115 by 0.885 of the point size, as the Mac renders it.
pub fn folder_mark(cr: &gtk::cairo::Context, cx: f64, cy: f64, size: f64, colour: Colour) {
    let (w, h) = (1.115 * size, 0.885 * size);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    let r = 0.11 * h;
    colour.set(cr);
    cr.new_path();
    // The tab and the back.
    rounded(cr, x, y, 0.42 * w, 0.3 * h, r);
    rounded(cr, x, y + 0.09 * h, w, 0.3 * h, r);
    // The front, over the slit.
    rounded(cr, x, y + 0.3 * h, w, 0.7 * h, r * 1.3);
    cr.set_fill_rule(gtk::cairo::FillRule::Winding);
    let _ = cr.fill();
    // The slit between the flap and the body, inside the edges.
    let _ = cr.save();
    cr.set_operator(gtk::cairo::Operator::Clear);
    cr.rectangle(x + 0.1 * w, y + 0.24 * h, 0.8 * w, 0.06 * h);
    let _ = cr.fill();
    let _ = cr.restore();
}

/// SF Symbols' `plus` at `size` points, semibold, centred on `(cx, cy)`.
pub fn plus_mark(cr: &gtk::cairo::Context, cx: f64, cy: f64, size: f64, colour: Colour) {
    // With round caps the stroke reaches half its width past each end.
    let half = 0.3475 * size;
    colour.set(cr);
    cr.set_line_width(0.13 * size);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.new_path();
    cr.move_to(cx - half, cy);
    cr.line_to(cx + half, cy);
    cr.move_to(cx, cy - half);
    cr.line_to(cx, cy + half);
    let _ = cr.stroke();
}

/// The session mark, "▤" at `size` points: a square of eight hairlines,
/// 0.75 of the point size, as the Mac's font draws it.
pub fn session_mark(cr: &gtk::cairo::Context, cx: f64, cy: f64, size: f64, colour: Colour) {
    let side = 0.75 * size;
    let (x, y) = (cx - side / 2.0, cy - side / 2.0);
    let line = 0.05 * size;
    colour.set(cr);
    cr.set_line_width(line);
    cr.new_path();
    cr.rectangle(x + line / 2.0, y + line / 2.0, side - line, side - line);
    for i in 1..7 {
        let at = y + line / 2.0 + (side - line) * f64::from(i) / 7.0;
        cr.move_to(x, at);
        cr.line_to(x + side, at);
    }
    let _ = cr.stroke();
}

/// Draws the tile's ground and any mark that is not text into the square
/// at the origin. Text (initials, a folder's letter) is the widget's.
pub fn draw(cr: &gtk::cairo::Context, kind: &Kind, large: bool, dark: bool) {
    let m = metrics(large);
    let (ground, mark) = colours(kind, dark);
    ground.set(cr);
    rounded(cr, 0.0, 0.0, m.side, m.side, m.radius);
    let _ = cr.fill();
    let centre = m.side / 2.0;
    let size = glyph_size(kind, large);
    match kind {
        Kind::Tool {
            logo: Some(logo), ..
        } => {
            mark.set(cr);
            let inset = (m.side - m.logo) / 2.0;
            logo.fill(cr, (inset, inset), m.logo);
        }
        Kind::Folder => {
            // The slit clears to the ground, so the folder is drawn on its
            // own layer over it.
            cr.push_group();
            folder_mark(cr, centre, centre, size, mark);
            let _ = cr.pop_group_to_source();
            let _ = cr.paint();
        }
        Kind::Add => plus_mark(cr, centre, centre, size, mark),
        Kind::Session => session_mark(cr, centre, centre, size, mark),
        Kind::Tool { logo: None, .. } | Kind::FolderInitial(_) => {}
    }
}

/// Whether the shell is drawn dark.
pub fn dark() -> bool {
    adw::StyleManager::default().is_dark()
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ToolTile {
        pub kind: RefCell<Kind>,
        pub large: Cell<bool>,
        /// The core's folder mark, for a folder with no name.
        pub mark: RefCell<String>,
        /// Redraws when the scheme flips: the tile picks its colours when
        /// it draws.
        pub scheme: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ToolTile {
        const NAME: &'static str = "TcToolTile";
        type Type = super::ToolTile;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ToolTile {
        fn constructed(&self) {
            self.parent_constructed();
            let weak = self.obj().downgrade();
            let id = adw::StyleManager::default().connect_dark_notify(move |_| {
                if let Some(tile) = weak.upgrade() {
                    tile.queue_draw();
                }
            });
            *self.scheme.borrow_mut() = Some(id);
        }

        fn dispose(&self) {
            if let Some(id) = self.scheme.borrow_mut().take() {
                adw::StyleManager::default().disconnect(id);
            }
        }
    }

    impl WidgetImpl for ToolTile {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let side = metrics(self.large.get()).side as i32;
            (side, side, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let large = self.large.get();
            let kind = self.kind.borrow();
            let m = metrics(large);
            let dark = super::dark();
            let bounds = gtk::graphene::Rect::new(0.0, 0.0, m.side as f32, m.side as f32);
            let cr = snapshot.append_cairo(&bounds);
            draw(&cr, &kind, large, dark);
            drop(cr);
            if let Some(text) = kind.text(&self.mark.borrow()) {
                let layout = self.obj().create_pango_layout(Some(&text));
                let mut font = gtk::pango::FontDescription::new();
                font.set_absolute_size(glyph_size(&kind, large) * f64::from(gtk::pango::SCALE));
                font.set_weight(gtk::pango::Weight::Bold);
                layout.set_font_description(Some(&font));
                let (w, h) = layout.pixel_size();
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new(
                    ((m.side - f64::from(w)) / 2.0) as f32,
                    ((m.side - f64::from(h)) / 2.0) as f32,
                ));
                snapshot.append_layout(&layout, &colours(&kind, dark).1.gdk());
                snapshot.restore();
            }
        }
    }
}

glib::wrapper! {
    /// A tile as a widget, at the Mac's size, valign centre.
    pub struct ToolTile(ObjectSubclass<imp::ToolTile>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ToolTile {
    pub fn new(kind: Kind, large: bool) -> Self {
        let tile: Self = glib::Object::builder()
            .property("accessible-role", gtk::AccessibleRole::Presentation)
            .build();
        tile.imp().large.set(large);
        *tile.imp().kind.borrow_mut() = kind;
        *tile.imp().mark.borrow_mut() =
            trace_commons_contributor::preview_copy::monitor_traces_copy()
                .tree
                .folder_mark
                .to_owned();
        tile.set_valign(gtk::Align::Center);
        tile.set_halign(gtk::Align::Start);
        tile.add_css_class("tc-tool-tile");
        tile
    }

    pub fn kind(&self) -> Kind {
        self.imp().kind.borrow().clone()
    }

    pub fn set_kind(&self, kind: Kind) {
        if *self.imp().kind.borrow() != kind {
            *self.imp().kind.borrow_mut() = kind;
            self.queue_draw();
        }
    }

    pub fn is_large(&self) -> bool {
        self.imp().large.get()
    }
}

#[cfg(test)]
#[path = "tool_tile_tests.rs"]
mod tests;

#[cfg(test)]
mod mounted {
    use super::*;

    /// A tile is the Mac's size whatever it holds, and hidden from assistive
    /// tech.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_a_tile_is_the_macs_size_and_says_nothing() {
        gtk::init().unwrap();
        for (kind, large, side) in [
            (Kind::source(SourceKind::ClaudeCode), false, 22),
            (Kind::FolderInitial("web".into()), true, 30),
            (Kind::Session, false, 22),
        ] {
            let tile = ToolTile::new(kind, large);
            let (_, natural, _, _) = tile.measure(gtk::Orientation::Horizontal, -1);
            assert_eq!(natural, side);
            let (_, natural, _, _) = tile.measure(gtk::Orientation::Vertical, -1);
            assert_eq!(natural, side);
            assert_eq!(tile.accessible_role(), gtk::AccessibleRole::Presentation);
        }
    }
}
