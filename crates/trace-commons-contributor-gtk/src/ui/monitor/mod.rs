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
//! `notices`, `prefs`, `traces_tree`, `map_scene`, `transfers`) build no
//! widget and are tested without a display. `three_pane`, `graph_footer`,
//! `inspector`, `map`, `map_transfers`, `transfer_card` and `shell` are
//! the widgets and their drawing.

pub mod demand;
pub mod glass_paint;
pub mod glyph;
pub mod graph_footer;
pub mod graph_model;
pub mod inspector;
pub mod map;
pub mod map_scene;
pub mod map_transfers;
pub mod notices;
pub mod pane_layout;
pub mod prefs;
pub mod shell;
pub mod tabs;
pub mod three_pane;
pub mod traces_tree;
pub mod transfer_card;
pub mod transfers;

pub use shell::{Pages, Shell};
