//! The Monitor window: the three-pane shell the screens are hosted in.
//!
//! A native port of the Mac's Monitor window
//! (`macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift`), per
//! `docs/superpowers/specs/2026-11-16-linux-mac-window-and-map-design.md`.
//! The Swift is the reference behaviour; the logic is ported here rather
//! than shared (owner, 2026-11-16), and each deliberate platform difference
//! is recorded beside the code that makes it.
//!
//! The pure rules (`pane_layout`, `tabs`, `demand`, `graph_model`,
//! `notices`, `prefs`, `traces_tree`, `map_scene`, `transfers`,
//! `tool_art`) build no widget and are tested without a display.
//! `three_pane`, `graph_footer`, `inspector`, `map`, `map_transfers`,
//! `transfer_card`, `traces_list` and `shell` are the widgets and their
//! drawing; `glyph`, `station` and `orb` are drawings (the Mac's glyphs and
//! logos, the library's station and orb), tested offscreen with cairo.

pub mod demand;
pub mod field;
pub mod glass_paint;
pub mod glyph;
pub mod graph_footer;
pub mod graph_model;
pub mod inspector;
pub mod map;
pub mod map_scene;
pub mod map_transfers;
pub mod notices;
pub mod orb;
pub mod pane_layout;
pub mod prefs;
pub mod shell;
pub mod station;
pub mod summary;
pub mod tabs;
pub mod three_pane;
pub mod tool_art;
pub mod traces_list;
pub mod traces_tree;
pub mod transfer_card;
pub mod transfers;

pub use shell::{Pages, Shell};
