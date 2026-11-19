//! The Traces tab's folder list: the Mac's "Folders on this Mac" column,
//! headed "Folders on this computer" here.
//!
//! A native port of `TracesTreeView`, `TracesFolderRow`, `TracesSessionRow`
//! and `TracesColumnTitles` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/TracesViews.swift`, and of
//! the `GlassListRow` they draw (`macos/Sources/TCDesign/Components/
//! ListRow.swift`). Folders are the top level (no tool level, owner
//! 2026-10-05); each starts closed and opens to its waiting traces, and
//! every trace row shows its own tool, since one folder can hold several
//! tools' traces.
//!
//! The rules are plain functions here, tested without a display; the rows
//! are built by `folder_row` and `session_row` and placed by
//! `ui::queue::render`. Every daemon call goes through the queue's and
//! Settings' existing paths (`queue::submit_group`, `queue::submit_entry`,
//! `queue::dismiss_entry`, `queue::confirm_ignore`,
//! `settings::confirm_arming_folder`, `settings::set_folder_mode`), so
//! the consent rules those carry hold here too.
//!
//! Deliberate platform differences:
//! - The folder column is titled in the core's words for every computer
//!   but a Mac (`column_folders_computer`, owner 2026-11-18).
//! - Turning a folder Automatic asks first, as Linux Settings always has
//!   ("Arming is allowed from the app, but never silently",
//!   `settings::confirm_arming`); the Mac's switch writes at once (owner,
//!   2026-11-11).
//! - The chevron and History button take stock GTK symbolic icons where
//!   the Mac draws SF Symbols (owner, 2026-11-18). Every tile is the shared
//!   `ui::tool_tile`: a tool's logo in its tint, the folder mark, or the
//!   plain trace mark.
//! - The search field, the order menu, "Show ignored folders", dragging
//!   folders into place and the toolbar's Automatic-for-every-folder eye
//!   are not ported yet; ignored folders are not listed (the Mac's
//!   default).

use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use chrono::TimeZone;
use gtk::glib;
use trace_commons_contributor::preview_copy::monitor_traces_copy;

use super::inspector::Selection;
use super::traces_tree::{FolderNode, ProjectMode, SourceKind, TracesTree};
use crate::copy::PrivateInferenceTone;
use crate::model::{QueueEntry, human_bytes};
use crate::ui::App;
use crate::ui::tool_tile::{Kind, ToolTile};

/// The row's measures, the Mac's (`GlassTokens.Size`, `GlassListRow`,
/// `TracesColumnTitles`). Pinned to the Swift by
/// `the_rows_measure_what_the_mac_draws`.
pub mod size {
    /// `GlassTokens.Size.listRow`: a row's least height.
    pub const ROW: i32 = 40;
    /// `GlassTokens.Size.toolTile`: the tile's side.
    pub const TILE: i32 = 22;
    /// The chevron's column (`.frame(width: 16)`).
    pub const CHEVRON: i32 = 16;
    /// The row menu's column (`.frame(width: 22)`).
    pub const KEBAB: i32 = 22;
    /// `GlassTokens.Size.watchSwitchWidth`: the Automatic switch's column.
    pub const SWITCH: i32 = 38;
    /// `GlassTokens.Size.watchSwitchHeight`.
    pub const SWITCH_HEIGHT: i32 = 22;
    /// `GlassTokens.Size.controlLarge`: the round History button's side.
    pub const ROUND_BUTTON: i32 = 28;
    /// `GlassTokens.Size.submitPill`: the Submit pill's height.
    pub const SUBMIT_PILL: i32 = 24;
    /// The Submit column title's least width (`.frame(minWidth: 128)`).
    pub const SUBMIT_TITLE: i32 = 128;
    /// The gap between rows (`LazyVStack(spacing: 2)`).
    pub const ROW_GAP: i32 = 2;
    /// The row's trailing inset (`.padding(.trailing, 6)`).
    pub const TRAILING: i32 = 6;
    /// `GlassTokens.Space.s4`: the gap between a row's columns.
    pub const COLUMN_GAP: i32 = 8;
    /// `GlassTokens.Space.s3`: Submit to the switch.
    pub const ACCESSORY_GAP: i32 = 6;
    /// The lines under a folder row (`TracesTreeView.noteInset`).
    pub const NOTE_INSET: i32 = 38;
}

/// A row's level. Folders are the tree's top level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    Folder,
    Session,
}

/// `GlassListRow.leadingInset` with the tree rooted at folders: 8, and 18
/// for each level below.
pub fn leading_inset(depth: Depth) -> i32 {
    match depth {
        Depth::Folder => 8,
        Depth::Session => 8 + 18,
    }
}

/// The three column titles (`TracesColumnTitles`): the folders, in the
/// core's words for every computer but a Mac, Submit, and Auto.
pub fn column_titles() -> [&'static str; 3] {
    let tree = monitor_traces_copy().tree;
    [
        tree.column_folders_computer,
        tree.column_submit,
        tree.column_automatic,
    ]
}

// --- The rows and the selection --------------------------------------

/// The selectable rows in drawing order (`visibleRows`): every folder, and
/// the traces of the folders in `expanded`.
pub fn visible_rows(tree: &TracesTree, expanded: &HashSet<String>) -> Vec<Selection> {
    let mut rows = Vec::new();
    for folder in &tree.folders {
        rows.push(Selection::Folder(folder.id.clone()));
        if expanded.contains(&folder.id) {
            rows.extend(
                folder
                    .sessions
                    .iter()
                    .map(|entry| Selection::Session(entry.entry_id.clone())),
            );
        }
    }
    rows
}

/// The selection after a click (`toggled`): the clicked row, or nothing
/// when it was the one selected.
pub fn toggled(clicked: Selection, current: &Selection) -> Selection {
    if &clicked == current {
        Selection::None
    } else {
        clicked
    }
}

/// The selection after an arrow key (`moved`): the next or previous row as
/// drawn; the first row from nothing or from a row that has gone; the ends
/// hold.
pub fn moved(current: &Selection, down: bool, rows: &[Selection]) -> Option<Selection> {
    let Some(at) = rows.iter().position(|row| row == current) else {
        return rows.first().cloned();
    };
    let next = if down {
        (at + 1).min(rows.len() - 1)
    } else {
        at.saturating_sub(1)
    };
    rows.get(next).cloned()
}

/// The folder a trace sits in.
pub fn folder_of(entry_id: &str, tree: &TracesTree) -> Option<String> {
    tree.folders
        .iter()
        .find(|folder| folder.sessions.iter().any(|s| s.entry_id == entry_id))
        .map(|folder| folder.id.clone())
}

/// The open folders once `selected` is shown (`revealing`): a trace's
/// folder opens; a folder, or nothing, changes nothing.
pub fn revealing(
    selected: &Selection,
    tree: &TracesTree,
    expanded: &HashSet<String>,
) -> HashSet<String> {
    let mut open = expanded.clone();
    if let Selection::Session(id) = selected
        && let Some(folder) = folder_of(id, tree)
    {
        open.insert(folder);
    }
    open
}

// --- A folder row's words and controls -------------------------------

/// A folder row's sub-line (`folderSub`): its mode's name and the count
/// waiting, or the core's line for an ignored folder. A folder the core
/// does not list has no mode name.
pub fn folder_sub(folder: &FolderNode) -> String {
    let words = monitor_traces_copy();
    if folder.mode == Some(ProjectMode::Ignore) {
        return words.tree.ignored_folder.to_string();
    }
    let count = if folder.sessions.len() == 1 {
        words.counts.sessions_waiting_one.to_string()
    } else {
        words
            .counts
            .sessions_waiting
            .replace("{count}", &folder.sessions.len().to_string())
    };
    folder
        .mode
        .map(ProjectMode::label)
        .filter(|label| !label.is_empty())
        .into_iter()
        .map(str::to_string)
        .chain([count])
        .collect::<Vec<_>>()
        .join(" \u{00b7} ")
}

/// The folder's Submit pill, with the count it sends (`submitTitle`).
pub fn submit_title(count: u64) -> String {
    monitor_traces_copy()
        .tree
        .submit_count
        .replace("{count}", &count.to_string())
}

/// The line under a folder whose Submit leaves traces behind
/// (`withheldNote`): how many it sends, then the core's line for the rest.
pub fn withheld_note(withheld: &str, eligible: u64) -> String {
    format!(
        "{} \u{00b7} {withheld}",
        monitor_traces_copy()
            .tree
            .eligible_count
            .replace("{count}", &eligible.to_string())
    )
}

/// How many traces a folder's Submit leaves behind, for its withheld line
/// (`GroupSubmitOffer.withheldLine`).
/// With no count from the daemon there is no eligibility question and no
/// gap to explain, whatever Submit's own count (`sends`) says.
pub fn withheld_count(waiting: u64, contributable: Option<u64>) -> u64 {
    contributable.map_or(0, |eligible| waiting.saturating_sub(eligible))
}

/// The folder's Automatic switch (`automaticSwitch`): on for Automatic, off
/// for Ask, none for a folder it cannot switch (Never, or a mode the core
/// does not list). Traces with no project draw a switch that stays off.
pub fn automatic_switch(folder: &FolderNode) -> Option<bool> {
    if folder.is_bucket {
        return folder.mode.map(|_| false);
    }
    match folder.mode {
        Some(ProjectMode::AutoUpload) => Some(true),
        Some(ProjectMode::Ask) => Some(false),
        Some(ProjectMode::Ignore) | None => None,
    }
}

/// The modes a folder can be set to (`offerableModes`): every mode, but
/// never Automatic for Traces with no project, which the daemon refuses
/// to arm (`settings::mode_choices`).
pub fn offerable_modes(folder: &FolderNode) -> Vec<ProjectMode> {
    [
        ProjectMode::Ask,
        ProjectMode::AutoUpload,
        ProjectMode::Ignore,
    ]
    .into_iter()
    .filter(|mode| !(folder.is_bucket && *mode == ProjectMode::AutoUpload))
    .collect()
}

/// What asking for a folder mode does (`modeChange`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeChange {
    /// The mode it has.
    Noop,
    /// A direct write.
    Apply,
    /// Ignoring a folder with traces waiting asks first, with their count.
    ConfirmIgnore,
    /// Automatic asks first on Linux (`settings::confirm_arming`).
    ConfirmArming,
}

pub fn mode_change(folder: &FolderNode, mode: ProjectMode) -> ModeChange {
    if folder.mode == Some(mode) {
        return ModeChange::Noop;
    }
    match mode {
        ProjectMode::Ignore if !folder.sessions.is_empty() => ModeChange::ConfirmIgnore,
        ProjectMode::AutoUpload => ModeChange::ConfirmArming,
        _ => ModeChange::Apply,
    }
}

/// What a folder says under its row (`folderNotes`): why Traces with no
/// project can never be armed. A refused write is said as a toast here.
pub fn folder_notes(folder: &FolderNode) -> Vec<String> {
    if folder.is_bucket {
        vec![
            monitor_traces_copy()
                .tree
                .unresolved_bucket_note
                .to_string(),
        ]
    } else {
        Vec::new()
    }
}

// --- A trace row's words ----------------------------------------------

/// A trace's title (`when`): the short weekday and time it was found, or
/// when it started if that was not recorded ("Mon 3:04 PM").
pub fn when<Tz: TimeZone>(entry: &QueueEntry, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Some(at) = entry.discovered_at.or(entry.started_at) else {
        return "\u{2014}".to_string();
    };
    let local = at.with_timezone(zone);
    // The weekday, the time and the period, as the Mac's en-US
    // `Date.FormatStyle` draws them ("Mon 3:04 PM").
    [
        local.format("%a").to_string(),
        local.format("%-I:%M").to_string(),
        local.format("%p").to_string(),
    ]
    .join(" ")
}

/// The trace's size, when the daemon reported one.
pub fn size(entry: &QueueEntry) -> Option<String> {
    (entry.size_bytes > 0).then(|| human_bytes(entry.size_bytes))
}

/// A trace row's sub-line (`sub`): its size, then its state in the core's
/// words: held, then why it cannot be sent, then trimmed to fit, then the
/// attestation sentence, else waiting.
pub fn session_sub(
    entry: &QueueEntry,
    held: Option<&str>,
    ineligible: Option<&str>,
    attestation: Option<&str>,
) -> String {
    let tree = monitor_traces_copy().tree;
    let state = held
        .or(ineligible)
        .or((entry.subagents_dropped > 0).then_some(tree.session_trimmed))
        .or(attestation.filter(|line| !line.is_empty()))
        .unwrap_or(tree.session_waiting);
    size(entry)
        .into_iter()
        .chain([state.to_string()])
        .collect::<Vec<_>>()
        .join(" \u{00b7} ")
}

/// The sub-line's colour (`GlassListRow.Flag`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    /// Amber.
    Ask,
    /// Green.
    On,
}

/// Amber when held, when the core says the trace cannot go as it stands,
/// or when its attestation asks for attention or was refused; green for a
/// clear attestation; nothing for any other tone (`flag`).
pub fn flag(
    held: bool,
    ineligible: bool,
    attestation: Option<PrivateInferenceTone>,
) -> Option<Flag> {
    if held || ineligible {
        return Some(Flag::Ask);
    }
    match attestation {
        Some(PrivateInferenceTone::Attention | PrivateInferenceTone::Refused) => Some(Flag::Ask),
        Some(PrivateInferenceTone::Clear) => Some(Flag::On),
        _ => None,
    }
}

/// Why a trace cannot be sent as it stands, in the core's sentence, or
/// nothing for one that can (`ineligibleLine`).
pub fn ineligible_line(entry: &QueueEntry) -> Option<&'static str> {
    crate::eligibility::view(entry)
        .filter(|view| view.control != crate::copy::ContributionControl::Contribute)
        .map(|view| view.state_line)
}

/// The tool a trace came from (`tool`): its declared source, then the
/// adapter's.
pub fn tool(entry: &QueueEntry) -> Option<SourceKind> {
    entry
        .declared_source
        .as_deref()
        .and_then(SourceKind::from_wire)
        .or_else(|| SourceKind::from_wire(&entry.source))
}

/// The core's held word for a trace held for a person (the Mac's amber
/// flag), or nothing: the same word the review card says
/// (`queue::held_fact_texts`).
pub fn held_word(entry: &QueueEntry) -> Option<&'static str> {
    (crate::eligibility::is_held(entry) || crate::eligibility::held_for_review(entry))
        .then(|| monitor_traces_copy().held)
}

/// Whether a trace row offers Submit (`maySubmitSession`): never under an
/// ignored folder, never for a held trace, and only while the shared table
/// offers Contribute (`eligibility::offers_row_send`).
pub fn row_offers_send(entry: &QueueEntry, folder_ignored: bool) -> bool {
    !folder_ignored && crate::eligibility::offers_row_send(entry)
}

// --- The widgets -----------------------------------------------------

/// The titles over the columns (`TracesColumnTitles`). Decorative: every
/// control carries its own name, so they are hidden from assistive tech.
pub fn column_titles_widget() -> gtk::Box {
    let [folders, submit, automatic] = column_titles();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("tc-tree-columns");
    row.set_margin_start(8);
    row.set_margin_end(size::TRAILING);
    row.set_margin_bottom(4);
    row.set_accessible_role(gtk::AccessibleRole::Presentation);
    let title = |text: &str| {
        let label = gtk::Label::new(Some(&text.to_uppercase()));
        label.add_css_class("tc-type-eyebrow");
        label.add_css_class("tc-tree-column");
        label.set_single_line_mode(true);
        label
    };
    let folders = title(folders);
    folders.set_hexpand(true);
    folders.set_xalign(0.0);
    folders.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&folders);
    let trailing = gtk::Box::new(gtk::Orientation::Horizontal, size::ACCESSORY_GAP);
    let submit = title(submit);
    submit.set_size_request(size::SUBMIT_TITLE, -1);
    trailing.append(&submit);
    let automatic = title(automatic);
    automatic.set_size_request(size::SWITCH, -1);
    trailing.append(&automatic);
    row.append(&trailing);
    // The row menu's column.
    let kebab = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    kebab.set_size_request(size::KEBAB, -1);
    row.append(&kebab);
    row
}

/// The row frame every row shares (`GlassListRow`): the chevron column,
/// the tile, the title over its sub-line, then the trailing controls.
struct RowFrame {
    root: gtk::Box,
    trailing: gtk::Box,
}

fn row_frame(
    depth: Depth,
    chevron: Option<gtk::Widget>,
    tile: gtk::Widget,
    title: &str,
    sub: Option<(&str, Option<Flag>)>,
    selected: bool,
) -> RowFrame {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, size::COLUMN_GAP);
    root.add_css_class("tc-tree-row");
    if selected {
        root.add_css_class("tc-tree-row-selected");
    }
    root.set_size_request(-1, size::ROW);
    // The leading inset by depth and the 6 trailing are the row's own
    // padding (`.tc-tree-row-folder`, `.tc-tree-row-session`), inside its
    // hover and selection fill, as the Mac pads inside its background.
    root.add_css_class(match depth {
        Depth::Folder => "tc-tree-row-folder",
        Depth::Session => "tc-tree-row-session",
    });
    // The chevron's column. The Mac's target reaches into the inset and
    // the gap after it; GTK has no negative margin, so the target here is
    // the column, as tall as the row.
    let column = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    column.set_size_request(size::CHEVRON, -1);
    if let Some(chevron) = chevron {
        column.append(&chevron);
    }
    root.append(&column);
    tile.set_valign(gtk::Align::Center);
    root.append(&tile);

    let naming = gtk::Box::new(gtk::Orientation::Vertical, 0);
    naming.set_hexpand(true);
    naming.set_valign(gtk::Align::Center);
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
    heading.set_single_line_mode(true);
    heading.add_css_class(match depth {
        Depth::Folder => "tc-tree-title-folder",
        Depth::Session => "tc-tree-title-session",
    });
    naming.append(&heading);
    if let Some((line, flag)) = sub {
        let label = gtk::Label::new(Some(line));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("tc-type-caption");
        label.add_css_class(match flag {
            Some(Flag::Ask) => "tc-tree-sub-ask",
            Some(Flag::On) => "tc-tree-sub-on",
            None => "tc-tree-sub",
        });
        naming.append(&label);
    }
    root.append(&naming);
    let trailing = gtk::Box::new(gtk::Orientation::Horizontal, size::ACCESSORY_GAP);
    trailing.set_valign(gtk::Align::Center);
    root.append(&trailing);
    root.update_property(&[gtk::accessible::Property::Label(title)]);
    RowFrame { root, trailing }
}

/// The disclosure chevron: pointing right while closed, down while open.
fn chevron(open: bool, name: &str, on_toggle: impl Fn() + 'static) -> gtk::Widget {
    let button = gtk::Button::from_icon_name(if open {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    });
    button.add_css_class("tc-tree-chevron");
    button.set_size_request(size::CHEVRON, size::ROW);
    button.set_focus_on_click(false);
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
    button.connect_clicked(move |_| on_toggle());
    button.upcast()
}

/// A tile (`GlassToolTile`), the shared `ui::tool_tile`: the folder's, a
/// tool's logo, or the plain trace mark. Decorative. The class names the
/// tile for the layout tests; the tile paints itself.
fn tile(kind: Kind) -> gtk::Widget {
    let class = if kind == Kind::Folder {
        "tc-tree-tile-folder"
    } else {
        "tc-tree-tile"
    };
    let tile = ToolTile::new(kind, false);
    // A fixed square: never a share of the row's spare width.
    tile.set_hexpand(false);
    tile.add_css_class(class);
    tile.upcast()
}

/// A trace row's tile (`sessionTile`): its tool's, the declared source
/// first, else the plain trace mark.
pub fn session_tile(entry: &QueueEntry) -> Kind {
    Kind::session(entry.declared_source.as_deref(), &entry.source)
}

/// The Submit pill (`GlassButtonStyle(.submit)`).
fn submit_pill(title: &str, tip: &str) -> gtk::Button {
    let pill = gtk::Button::with_label(title);
    pill.add_css_class("tc-tree-submit");
    pill.set_size_request(-1, size::SUBMIT_PILL);
    pill.set_valign(gtk::Align::Center);
    if !tip.is_empty() {
        pill.set_tooltip_text(Some(tip));
    }
    pill
}

/// The row menu's button (`GlassKebab`), named in the core's words.
fn kebab(name: &str, popover: &gtk::Popover) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    // The stock symbolic for the Mac's `GlassKebab` dots: GTK 4.6's
    // `MenuButton::set_child` is past this crate's feature floor.
    button.set_icon_name("view-more-symbolic");
    button.add_css_class("tc-tree-kebab");
    button.set_size_request(size::KEBAB, size::KEBAB);
    button.set_valign(gtk::Align::Center);
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button.set_tooltip_text(Some(name));
    button.set_popover(Some(popover));
    button
}

fn menu_item(label: &str, on_pick: impl Fn() + 'static) -> gtk::Button {
    let item = gtk::Button::with_label(label);
    item.add_css_class("flat");
    if let Some(child) = item.child().and_downcast::<gtk::Label>() {
        child.set_xalign(0.0);
    }
    item.connect_clicked(move |_| on_pick());
    item
}

fn select_on_click(row: &gtk::Box, app: &Rc<App>, clicked: Selection) {
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(app);
    click.connect_released(move |gesture, presses, _, _| {
        if presses != 1 {
            return;
        }
        let Some(app) = weak.upgrade() else { return };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let next = toggled(clicked.clone(), &app.shell.inspector.selection());
        app.shell.select(&app, next);
        crate::ui::queue::focus_list(&app);
    });
    row.add_controller(click);
}

/// One folder row and the lines under it (`TracesFolderRow`).
pub fn folder_row(app: &Rc<App>, folder: &FolderNode, open: bool, selected: bool) -> gtk::Widget {
    let words = monitor_traces_copy();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, size::ROW_GAP);
    let id = folder.id.clone();
    let chevron = (!folder.sessions.is_empty()).then(|| {
        let weak = Rc::downgrade(app);
        let id = id.clone();
        chevron(open, &folder.label, move || {
            if let Some(app) = weak.upgrade() {
                crate::ui::queue::toggle_folder(&app, &id);
            }
        })
    });
    let sub = folder_sub(folder);
    let frame = row_frame(
        Depth::Folder,
        chevron,
        tile(Kind::Folder),
        &folder.label,
        Some((&sub, None)),
        selected,
    );
    frame
        .root
        .update_property(&[gtk::accessible::Property::Description(&sub)]);
    let ignored = folder.mode == Some(ProjectMode::Ignore);

    // Submit all: a consent action, drawn only when the shared table
    // offers Contribute for the daemon's counts, never for an ignored
    // folder, and removed rather than disabled (`groupOffer`). Its count is
    // the daemon's, else the traces here it would send.
    let waiting = folder.sessions.len() as u64;
    let contributable = crate::eligibility::group_contributable(&app.projects.borrow(), &folder.id);
    let sends = contributable
        .unwrap_or_else(|| crate::eligibility::sendable_ids(folder.sessions.iter()).len() as u64);
    let submits =
        !ignored && waiting > 0 && crate::eligibility::group_offers_send(waiting, contributable);
    let busy = app.submitting_folder.borrow().as_deref() == Some(folder.id.as_str());
    let tip = words.tree.submit_tip;
    let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    split.add_css_class("tc-tree-split");
    if submits {
        let title = if busy {
            words.tree.submitting.to_string()
        } else {
            submit_title(sends)
        };
        let pill = submit_pill(&title, tip);
        pill.set_sensitive(!busy);
        let weak = Rc::downgrade(app);
        let (pid, label) = (folder.id.clone(), folder.label.clone());
        pill.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                crate::ui::queue::submit_group(&app, &pid, &label, None);
            }
        });
        split.append(&pill);
        // The split's chevron: Submit all as, the verdict asked once for
        // the whole folder (`SubmitAllAsModal`), through the same call.
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let popover = gtk::Popover::builder().child(&menu).build();
        for (label, verdict) in [
            (crate::copy::VERDICT_WORKED, "worked"),
            (crate::copy::VERDICT_PARTLY, "partly"),
            (crate::copy::VERDICT_FAILED, "failed"),
        ] {
            let weak = Rc::downgrade(app);
            let (pid, name) = (folder.id.clone(), folder.label.clone());
            let popover_for_item = popover.clone();
            menu.append(&menu_item(label, move || {
                popover_for_item.popdown();
                if let Some(app) = weak.upgrade() {
                    crate::ui::queue::submit_group(&app, &pid, &name, Some(verdict));
                }
            }));
        }
        let more = gtk::MenuButton::new();
        more.set_icon_name("pan-down-symbolic");
        more.add_css_class("tc-tree-split-menu");
        more.set_size_request(-1, size::SUBMIT_PILL);
        more.set_valign(gtk::Align::Center);
        more.set_sensitive(!busy);
        more.update_property(&[gtk::accessible::Property::Label(crate::copy::SUBMIT_ALL_AS)]);
        more.set_tooltip_text(Some(crate::copy::SUBMIT_ALL_AS_TOOLTIP));
        more.set_popover(Some(&popover));
        split.append(&more);
        frame.trailing.append(&split);
    }

    // The Automatic switch, or its empty column so every Submit lines up.
    match automatic_switch(folder) {
        Some(on) => {
            let switch = gtk::Switch::new();
            switch.set_active(on);
            switch.add_css_class("tc-tree-switch");
            switch.set_valign(gtk::Align::Center);
            let name = words
                .tree
                .automatic_folder
                .replace("{label}", &folder.label);
            switch.update_property(&[gtk::accessible::Property::Label(&name)]);
            switch.set_tooltip_text(Some(&name));
            let weak = Rc::downgrade(app);
            let node = folder.clone();
            switch.connect_state_set(move |switch, wanted| {
                let Some(app) = weak.upgrade() else {
                    return glib::Propagation::Stop;
                };
                // Traces with no project are never armed: the switch goes
                // back, and the line under the row already says why.
                if node.is_bucket {
                    if wanted {
                        switch.set_active(false);
                    }
                    return glib::Propagation::Stop;
                }
                request(
                    &app,
                    &node,
                    if wanted {
                        ProjectMode::AutoUpload
                    } else {
                        ProjectMode::Ask
                    },
                );
                // The switch's state (its trough) stays the daemon's: it
                // is set only by the read that follows the write and
                // redraws the row. GTK moves the knob with the press, so
                // while a confirmation is open the knob leads the trough.
                glib::Propagation::Stop
            });
            frame.trailing.append(&switch);
        }
        None => {
            let room = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            room.set_size_request(size::SWITCH, 1);
            room.set_accessible_role(gtk::AccessibleRole::Presentation);
            frame.trailing.append(&room);
        }
    }

    // The row menu: the contribution rule, and Ignore (owner, 2026-10-19).
    if folder.mode.is_some() {
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let popover = gtk::Popover::builder().child(&menu).build();
        let heading = gtk::Label::new(Some(words.inspector.contribution_rule));
        heading.add_css_class("tc-type-eyebrow");
        heading.set_xalign(0.0);
        menu.append(&heading);
        for mode in offerable_modes(folder) {
            if mode == ProjectMode::Ignore {
                continue;
            }
            let item = menu_item(mode.label(), {
                let weak = Rc::downgrade(app);
                let node = folder.clone();
                let popover = popover.clone();
                move || {
                    popover.popdown();
                    if let Some(app) = weak.upgrade() {
                        request(&app, &node, mode);
                    }
                }
            });
            if folder.mode == Some(mode) {
                item.add_css_class("tc-tree-menu-current");
                item.update_state(&[gtk::accessible::State::Checked(
                    gtk::AccessibleTristate::True,
                )]);
            }
            menu.append(&item);
        }
        if !ignored && offerable_modes(folder).contains(&ProjectMode::Ignore) {
            let weak = Rc::downgrade(app);
            let node = folder.clone();
            let popover_for_item = popover.clone();
            menu.append(&menu_item(words.tree.ignore_folder, move || {
                popover_for_item.popdown();
                if let Some(app) = weak.upgrade() {
                    request(&app, &node, ProjectMode::Ignore);
                }
            }));
        }
        frame.root.append(&kebab(words.tree.folder_menu, &popover));
    } else {
        let room = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        room.set_size_request(size::KEBAB, 1);
        frame.root.append(&room);
    }

    select_on_click(&frame.root, app, Selection::Folder(id));
    outer.append(&frame.root);

    // The lines under it: why the bucket is never armed, and what Submit
    // leaves behind, drawn once.
    let mut notes = folder_notes(folder);
    if !ignored {
        let withheld = crate::copy::group_withheld_line(withheld_count(waiting, contributable));
        if !withheld.is_empty() {
            notes.push(withheld_note(&withheld, sends));
        }
    }
    for line in notes {
        let label = gtk::Label::new(Some(&line));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("tc-type-caption");
        label.add_css_class("tc-tree-sub");
        label.set_margin_start(size::NOTE_INSET);
        label.set_margin_end(size::NOTE_INSET);
        label.set_margin_bottom(4);
        outer.append(&label);
    }
    outer.upcast()
}

/// The route every folder mode change takes from the tree (`request`).
fn request(app: &Rc<App>, folder: &FolderNode, mode: ProjectMode) {
    match mode_change(folder, mode) {
        ModeChange::Noop => {}
        ModeChange::Apply => crate::ui::settings::set_folder_mode(app, &folder.id, mode.wire()),
        ModeChange::ConfirmIgnore => {
            crate::ui::queue::confirm_ignore(app, &folder.id, &folder.label, folder.sessions.len())
        }
        ModeChange::ConfirmArming => {
            let weak = Rc::downgrade(app);
            crate::ui::settings::confirm_arming_folder(app, &folder.id, &folder.label, move || {
                // Not armed: the switch goes back to what the daemon holds.
                if let Some(app) = weak.upgrade() {
                    crate::ui::queue::render(&app);
                }
            });
        }
    }
}

/// One trace row (`TracesSessionRow`).
pub fn session_row(app: &Rc<App>, entry: &QueueEntry, off: bool, selected: bool) -> gtk::Widget {
    let words = monitor_traces_copy();
    let held = held_word(entry);
    let ineligible = ineligible_line(entry);
    let mark = crate::attestation::view(entry);
    let sub = session_sub(entry, held, ineligible, Some(mark.state_line));
    let flag = flag(held.is_some(), ineligible.is_some(), Some(mark.tone));
    let title = when(entry, &chrono::Local);
    let frame = row_frame(
        Depth::Session,
        None,
        tile(session_tile(entry)),
        &title,
        Some((&sub, flag)),
        selected,
    );
    frame
        .root
        .update_property(&[gtk::accessible::Property::Description(&sub)]);
    if off {
        frame.root.add_css_class("tc-tree-row-off");
    }
    // Submit sends the trace at once (owner, 2026-10-22), drawn turned off
    // while it cannot be sent.
    let pill = submit_pill(words.tree.submit, words.tree.submit_trace_tip);
    pill.set_sensitive(row_offers_send(entry, off));
    let weak = Rc::downgrade(app);
    let live = entry.clone();
    pill.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            crate::ui::queue::submit_entry(&app, &live);
        }
    });
    frame.trailing.append(&pill);
    // The row menu's one item: Dismiss trace.
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let popover = gtk::Popover::builder().child(&menu).build();
    let weak = Rc::downgrade(app);
    let live = entry.clone();
    let popover_for_item = popover.clone();
    menu.append(&menu_item(words.tree.dismiss_session, move || {
        popover_for_item.popdown();
        if let Some(app) = weak.upgrade() {
            crate::ui::queue::dismiss_entry(&app, &live);
        }
    }));
    frame
        .root
        .append(&kebab(words.tree.dismiss_session, &popover));
    select_on_click(&frame.root, app, Selection::Session(entry.entry_id.clone()));
    frame.root.upcast()
}

/// `GlassMotion.fast`: how long the list takes to bring a row into view
/// (`TracesTreeView.scroll(to:)`, `withAnimation(GlassMotion.fast)`).
pub const SCROLL_SECONDS: f64 = 0.15;

/// Where the list scrolls to show a row from `top` to `bottom` (in the
/// list's own coordinates) in a view `page` tall now at `value`, of
/// `upper` in all: SwiftUI's `scrollTo(_:)` with no anchor, the least
/// movement that shows the whole row, its top when it is taller than the
/// view. `None` when it is already shown.
pub fn scroll_target(value: f64, page: f64, upper: f64, top: f64, bottom: f64) -> Option<f64> {
    let wanted = if top < value || bottom - top > page {
        top
    } else if bottom > value + page {
        bottom - page
    } else {
        return None;
    };
    let wanted = wanted.clamp(0.0, (upper - page).max(0.0));
    ((wanted - value).abs() > 0.5).then_some(wanted)
}

/// The list's scroll `t` seconds into a move from `from` to `to`, on the
/// token curve over [`SCROLL_SECONDS`].
pub fn scrolled(from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * crate::ui::glass_motion::curve(t / SCROLL_SECONDS)
}

/// Brings `row` into `scroller`'s view once it is laid out, moving on the
/// token curve at display rate, or at once under reduced motion.
pub fn scroll_into_view(
    scroller: &gtk::ScrolledWindow,
    row: &gtk::Widget,
    driver: &Rc<crate::ui::glass_motion::Driver>,
) {
    let row = row.downgrade();
    let driver = Rc::clone(driver);
    let frames = std::cell::Cell::new(0u32);
    scroller.add_tick_callback(move |scroller, _| {
        // The first tick runs before the rebuilt rows' layout.
        frames.set(frames.get() + 1);
        if frames.get() < 2 {
            return glib::ControlFlow::Continue;
        }
        let Some(row) = row.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let Some(content) = scroller.child() else {
            return glib::ControlFlow::Break;
        };
        let Some(bounds) = row.compute_bounds(&content) else {
            return glib::ControlFlow::Break;
        };
        let adjustment = scroller.vadjustment();
        let from = adjustment.value();
        let top = f64::from(bounds.y());
        let Some(to) = scroll_target(
            from,
            adjustment.page_size(),
            adjustment.upper(),
            top,
            top + f64::from(bounds.height()),
        ) else {
            return glib::ControlFlow::Break;
        };
        let moving = adjustment.clone();
        driver.start(
            scroller,
            SCROLL_SECONDS,
            0.0,
            move |t| {
                let now = if crate::ui::glass_motion::animations_enabled() {
                    scrolled(from, to, t)
                } else {
                    to
                };
                moving.set_value(now);
            },
            || {},
        );
        glib::ControlFlow::Break
    });
}

#[cfg(all(test, target_os = "linux"))]
#[path = "../../../tests/support/traces_list_tests.rs"]
mod mounted_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Project;

    fn entry(id: &str, project: &str) -> QueueEntry {
        QueueEntry {
            entry_id: id.into(),
            project_id: project.into(),
            project_label: project.into(),
            source: "codex".into(),
            state: "pending".into(),
            ..QueueEntry::default()
        }
    }

    fn project(id: &str, mode: &str) -> Project {
        serde_json::from_value(serde_json::json!({
            "project_id": id, "project_label": id, "project_path": format!("/x/{id}"), "mode": mode,
        }))
        .unwrap()
    }

    fn node(id: &str, mode: Option<ProjectMode>, sessions: usize) -> FolderNode {
        FolderNode {
            sessions: (0..sessions)
                .map(|n| entry(&format!("{id}-{n}"), id))
                .collect(),
            ..FolderNode::new(id, id, mode)
        }
    }

    /// A number the Mac's Swift states, found after `needle`.
    fn mac_number(source: &str, needle: &str) -> i32 {
        let at = source
            .find(needle)
            .unwrap_or_else(|| panic!("the Swift no longer says {needle}"));
        source[at + needle.len()..]
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap()
    }

    const TOKENS: &str =
        include_str!("../../../../../macos/Sources/TCDesign/Generated/GlassTokens.swift");
    const LIST_ROW: &str =
        include_str!("../../../../../macos/Sources/TCDesign/Components/ListRow.swift");
    const TRACES_VIEWS: &str = include_str!(
        "../../../../../macos/Sources/TraceCommonsApp/Views/Monitor/TracesViews.swift"
    );

    /// Every measure is the Mac's, read from its Swift, so a wrong port
    /// fails here rather than in a photograph.
    #[test]
    fn the_rows_measure_what_the_mac_draws() {
        assert_eq!(
            size::ROW,
            mac_number(TOKENS, "static let listRow: CGFloat =")
        );
        assert_eq!(
            size::TILE,
            mac_number(TOKENS, "static let toolTile: CGFloat =")
        );
        assert_eq!(
            size::SWITCH,
            mac_number(TOKENS, "static let watchSwitchWidth: CGFloat =")
        );
        assert_eq!(
            size::SWITCH_HEIGHT,
            mac_number(TOKENS, "static let watchSwitchHeight: CGFloat =")
        );
        assert_eq!(
            size::ROUND_BUTTON,
            mac_number(TOKENS, "static let controlLarge: CGFloat =")
        );
        assert_eq!(
            size::SUBMIT_PILL,
            mac_number(TOKENS, "static let submitPill: CGFloat =")
        );
        assert_eq!(
            size::COLUMN_GAP,
            mac_number(TOKENS, "static let s4: CGFloat =")
        );
        assert_eq!(
            size::ACCESSORY_GAP,
            mac_number(TOKENS, "static let s3: CGFloat =")
        );
        // The chevron's column, then the row menu's, each the first frame
        // after its control.
        // The chevron's column is the frame on the `Group` that holds the
        // chevron, or `Color.clear` on a row without one: read past
        // `Color.clear` on purpose, never off the chevron's own wider tap
        // target (`chevronTarget`), which comes first.
        let chevron = &LIST_ROW[LIST_ROW.find("chevron.right").unwrap()..];
        let column = &chevron[chevron.find("Color.clear").unwrap()..];
        assert_eq!(size::CHEVRON, mac_number(column, ".frame(width:"));
        let kebab = &LIST_ROW[LIST_ROW.find("GlassKebab(menuLabel").unwrap()..];
        assert_eq!(size::KEBAB, mac_number(kebab, ".frame(width:"));
        assert_eq!(size::TRAILING, mac_number(LIST_ROW, ".padding(.trailing,"));
        assert_eq!(
            leading_inset(Depth::Folder),
            mac_number(
                LIST_ROW,
                "static func leadingInset(_ depth: Depth, root: Depth) -> CGFloat {"
            )
        );
        assert!(LIST_ROW.contains("8 + CGFloat(max(0, depth.rawValue - root.rawValue)) * 18"));
        assert_eq!(
            leading_inset(Depth::Session) - leading_inset(Depth::Folder),
            18
        );
        assert_eq!(
            size::SUBMIT_TITLE,
            mac_number(TRACES_VIEWS, ".frame(minWidth:")
        );
        assert_eq!(
            size::ROW_GAP,
            mac_number(TRACES_VIEWS, "LazyVStack(spacing:")
        );
        assert_eq!(
            size::NOTE_INSET,
            mac_number(TRACES_VIEWS, "static let noteInset: CGFloat =")
        );
    }

    /// The rows' paddings in the stylesheet are the insets above.
    #[test]
    fn the_stylesheet_pads_rows_at_the_macs_insets() {
        let css = include_str!("../style.css");
        for (class, inset) in [
            ("tc-tree-row-folder", leading_inset(Depth::Folder)),
            ("tc-tree-row-session", leading_inset(Depth::Session)),
        ] {
            let rule = &css[css
                .find(&format!(".{class} {{"))
                .unwrap_or_else(|| panic!("{class}"))..];
            let rule = &rule[..rule.find('}').unwrap()];
            assert!(
                rule.contains(&format!("padding-left: {inset}px;")),
                "{rule}"
            );
            assert!(
                rule.contains(&format!("padding-right: {}px;", size::TRAILING)),
                "{rule}"
            );
        }
    }

    /// The column titles are the core's, the folders' in the words for
    /// every computer but a Mac (owner, 2026-11-18).
    #[test]
    fn the_columns_are_titled_for_this_computer() {
        let tree = monitor_traces_copy().tree;
        assert_eq!(
            column_titles(),
            [
                "Folders on this computer",
                tree.column_submit,
                tree.column_automatic
            ]
        );
        assert!(column_titles().iter().all(|title| !title.contains("Mac")));
    }

    /// `test_theKeyboardWalksFoldersAndSessionsAsDrawn` and
    /// `test_foldersStartCollapsedAndASelectedSessionIsRevealed`.
    #[test]
    fn the_rows_are_folders_then_the_open_folders_traces() {
        let entries = vec![entry("a", "p1"), entry("b", "p1"), entry("c", "p2")];
        let tree = TracesTree::build(&entries, &[project("idle", "notify_only")], None);
        let ids: Vec<_> = tree.folders.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids.len(), 3, "an idle folder is listed: {ids:?}");
        // Closed: one row per folder.
        assert_eq!(
            visible_rows(&tree, &HashSet::new()),
            tree.folders
                .iter()
                .map(|f| Selection::Folder(f.id.clone()))
                .collect::<Vec<_>>()
        );
        let all: HashSet<String> = tree.folders.iter().map(|f| f.id.clone()).collect();
        let rows = visible_rows(&tree, &all);
        let mut expected = Vec::new();
        for folder in &tree.folders {
            expected.push(Selection::Folder(folder.id.clone()));
            expected.extend(
                folder
                    .sessions
                    .iter()
                    .map(|s| Selection::Session(s.entry_id.clone())),
            );
        }
        assert_eq!(rows, expected);
        // Down from a folder goes to its own first trace; up comes back.
        let p1 = tree.folders.iter().find(|f| f.id == "p1").unwrap();
        let first = Selection::Session(p1.sessions[0].entry_id.clone());
        assert_eq!(
            moved(&Selection::Folder("p1".into()), true, &rows),
            Some(first.clone())
        );
        assert_eq!(
            moved(&first, false, &rows),
            Some(Selection::Folder("p1".into()))
        );
        // Nothing, or something gone: the top. The ends hold.
        assert_eq!(moved(&Selection::None, true, &rows), rows.first().cloned());
        assert_eq!(
            moved(&Selection::Session("gone".into()), false, &rows),
            rows.first().cloned()
        );
        assert_eq!(
            moved(rows.last().unwrap(), true, &rows),
            rows.last().cloned()
        );
        assert_eq!(moved(&rows[0], false, &rows), Some(rows[0].clone()));
        assert_eq!(moved(&Selection::None, true, &[]), None);
        // Selecting a trace from elsewhere opens its folder; a folder opens none.
        assert_eq!(
            revealing(&first, &tree, &HashSet::new()),
            HashSet::from(["p1".to_string()])
        );
        assert!(revealing(&Selection::Folder("p1".into()), &tree, &HashSet::new()).is_empty());
    }

    /// `test_selectingTheSelectedRowAgainClearsIt`.
    #[test]
    fn a_second_click_clears_the_selection() {
        let folder = Selection::Folder("p1".into());
        let session = Selection::Session("e1".into());
        assert_eq!(toggled(folder.clone(), &Selection::None), folder);
        assert_eq!(toggled(folder.clone(), &folder), Selection::None);
        assert_eq!(toggled(folder.clone(), &session), folder);
        assert_eq!(toggled(session.clone(), &session), Selection::None);
        // Same id, other kind: not the selected row.
        assert_eq!(
            toggled(Selection::Session("p1".into()), &folder),
            Selection::Session("p1".into())
        );
    }

    /// `test_ignoredFoldersStayInTheTree`'s sub-lines: the mode's name and
    /// the count, or the core's ignored line.
    #[test]
    fn a_folder_says_its_mode_and_what_waits() {
        let words = monitor_traces_copy();
        let ask =
            trace_commons_contributor::project_copy::folder_mode_label("notify_only").unwrap();
        assert_eq!(
            folder_sub(&node("p", Some(ProjectMode::Ask), 1)),
            format!("{ask} \u{00b7} {}", words.counts.sessions_waiting_one)
        );
        assert_eq!(
            folder_sub(&node("p", Some(ProjectMode::Ask), 3)),
            format!("{ask} \u{00b7} 3 traces waiting")
        );
        assert_eq!(folder_sub(&node("p", None, 2)), "2 traces waiting");
        assert_eq!(
            folder_sub(&node("p", Some(ProjectMode::Ignore), 2)),
            words.tree.ignored_folder
        );
        assert_eq!(submit_title(3), "Submit \u{00b7} 3");
        // `test_theWithheldLineSaysWhatSubmitSends`.
        let note = withheld_note("1 is not eligible.", 2);
        assert!(note.starts_with("2 eligible"), "{note}");
        assert!(note.ends_with("1 is not eligible."), "{note}");
    }

    /// `automaticSwitch` and `test_theBucketIsNamedAndExplained`.
    #[test]
    fn the_automatic_switch_follows_the_mode() {
        assert_eq!(
            automatic_switch(&node("p", Some(ProjectMode::AutoUpload), 0)),
            Some(true)
        );
        assert_eq!(
            automatic_switch(&node("p", Some(ProjectMode::Ask), 0)),
            Some(false)
        );
        assert_eq!(
            automatic_switch(&node("p", Some(ProjectMode::Ignore), 0)),
            None
        );
        assert_eq!(automatic_switch(&node("p", None, 0)), None);
        let mut bucket = node("b", Some(ProjectMode::Ask), 0);
        bucket.is_bucket = true;
        assert_eq!(
            automatic_switch(&bucket),
            Some(false),
            "a switch that stays off"
        );
        assert!(!offerable_modes(&bucket).contains(&ProjectMode::AutoUpload));
        assert_eq!(
            folder_notes(&bucket),
            [monitor_traces_copy().tree.unresolved_bucket_note]
        );
        assert!(folder_notes(&node("p", Some(ProjectMode::Ask), 0)).is_empty());
        let built = TracesTree::build(
            &[],
            &[{
                let mut p = project("b", "notify_only");
                p.is_unresolved_bucket = true;
                p
            }],
            None,
        );
        assert!(
            built.folders[0].is_bucket,
            "the bucket is known from the core's flag"
        );
    }

    /// `modeChange`: ignoring with traces waiting asks first; Automatic
    /// asks first on Linux; anything else is a direct write.
    #[test]
    fn a_mode_change_asks_only_where_it_must() {
        let busy = node("p", Some(ProjectMode::Ask), 2);
        assert_eq!(mode_change(&busy, ProjectMode::Ask), ModeChange::Noop);
        assert_eq!(
            mode_change(&busy, ProjectMode::Ignore),
            ModeChange::ConfirmIgnore
        );
        assert_eq!(
            mode_change(&node("p", Some(ProjectMode::Ask), 0), ProjectMode::Ignore),
            ModeChange::Apply
        );
        assert_eq!(
            mode_change(&busy, ProjectMode::AutoUpload),
            ModeChange::ConfirmArming
        );
        assert_eq!(
            mode_change(&node("p", Some(ProjectMode::Ignore), 0), ProjectMode::Ask),
            ModeChange::Apply
        );
    }

    /// `test_aSessionIsTitledByWeekdayAndTime`: weekday and time, no month.
    #[test]
    fn a_trace_is_titled_by_weekday_and_time() {
        let mut e = entry("a", "p");
        e.discovered_at = Some("2026-10-05T15:04:00Z".parse().unwrap());
        e.started_at = Some("2026-10-01T09:00:00Z".parse().unwrap());
        assert_eq!(when(&e, &chrono::Utc), "Mon 3:04 PM");
        e.discovered_at = None;
        assert_eq!(when(&e, &chrono::Utc), "Thu 9:00 AM");
        e.started_at = None;
        assert_eq!(when(&e, &chrono::Utc), "\u{2014}");
    }

    /// `test_aHeldSessionSaysSoInWords`, `test_aSessionRowSaysItsAttestation`
    /// and `test_sessionRowsShowTheirTool`: the size, then the state.
    #[test]
    fn a_trace_says_its_size_then_its_state() {
        let tree = monitor_traces_copy().tree;
        let mut plain = entry("a", "p");
        plain.size_bytes = 2048;
        assert_eq!(
            session_sub(&plain, None, None, None),
            format!("2 KB \u{00b7} {}", tree.session_waiting)
        );
        assert_eq!(
            session_sub(&plain, None, None, Some("Attested.")),
            "2 KB \u{00b7} Attested."
        );
        assert_eq!(
            session_sub(&plain, None, None, Some("")),
            format!("2 KB \u{00b7} {}", tree.session_waiting)
        );
        assert_eq!(
            session_sub(&plain, None, Some("Can't go."), Some("Attested.")),
            "2 KB \u{00b7} Can't go."
        );
        assert_eq!(
            session_sub(&plain, Some("Held"), Some("Can't go."), None),
            "2 KB \u{00b7} Held"
        );
        let mut trimmed = plain.clone();
        trimmed.subagents_dropped = 2;
        assert_eq!(
            session_sub(&trimmed, None, None, Some("Attested.")),
            format!("2 KB \u{00b7} {}", tree.session_trimmed)
        );
        // No size reported: the state alone. The tool is the tile, not
        // the sub-line.
        let bare = entry("b", "p");
        assert_eq!(session_sub(&bare, None, None, None), tree.session_waiting);
        assert!(!session_sub(&bare, None, None, None).contains("Codex"));
    }

    /// `test_aSessionRowSaysItsAttestation`'s flags.
    #[test]
    fn the_sub_line_is_amber_or_green_as_the_mac_flags_it() {
        use PrivateInferenceTone as T;
        assert_eq!(flag(false, false, Some(T::Clear)), Some(Flag::On));
        assert_eq!(flag(false, false, Some(T::Attention)), Some(Flag::Ask));
        assert_eq!(flag(false, false, Some(T::Refused)), Some(Flag::Ask));
        assert_eq!(
            flag(false, false, Some(T::Neutral)),
            None,
            "an unread mark is never green"
        );
        assert_eq!(flag(false, false, Some(T::Held)), None);
        assert_eq!(
            flag(true, false, Some(T::Clear)),
            Some(Flag::Ask),
            "held outranks a clear mark"
        );
        assert_eq!(flag(false, true, Some(T::Clear)), Some(Flag::Ask));
        assert_eq!(flag(false, false, None), None);
    }

    /// `test_anIneligibleSessionLooksDifferent`.
    #[test]
    fn an_ineligible_trace_says_the_cores_sentence() {
        let mut e = entry("a", "p");
        e.eligibility = Some("ineligible_configuration".into());
        let line = ineligible_line(&e).expect("a sentence");
        assert!(!line.is_empty());
        assert!(!row_offers_send(&e, false));
        e.eligibility = Some("eligible".into());
        assert_eq!(ineligible_line(&e), None);
        assert!(row_offers_send(&e, false));
        assert!(!row_offers_send(&e, true), "never under an ignored folder");
        assert_eq!(ineligible_line(&entry("b", "p")), None);
    }

    /// The withheld line is the Mac's (`groupSubmit`): with no count from
    /// the daemon there is no eligibility question and no gap to explain,
    /// even where held traces keep Submit's own count below the waiting
    /// one; with a count, the gap is the daemon's.
    #[test]
    fn the_withheld_line_needs_the_daemons_count() {
        assert_eq!(withheld_count(3, None), 0);
        assert_eq!(withheld_count(3, Some(1)), 2);
        assert_eq!(withheld_count(3, Some(3)), 0);
        assert_eq!(withheld_count(2, Some(5)), 0);
    }

    /// `test_sessionRowsShowTheirTool`: each trace's own tool's tile, the
    /// declared source first; the plain trace mark for one this build does
    /// not know.
    #[test]
    fn a_trace_row_shows_its_own_tool() {
        let mut claude = entry("a", "p");
        claude.source = "claude-code".into();
        assert_eq!(session_tile(&claude), Kind::source(SourceKind::ClaudeCode));
        assert_eq!(
            session_tile(&entry("b", "p")),
            Kind::source(SourceKind::Codex)
        );
        let mut declared = entry("c", "p");
        declared.declared_source = Some("cline".into());
        assert_eq!(session_tile(&declared), Kind::source(SourceKind::Cline));
        let mut unknown = entry("d", "p");
        unknown.source = "not-a-tool".into();
        assert_eq!(session_tile(&unknown), Kind::Session);
    }

    /// A trace held for a person says the core's held word, flagged amber,
    /// and its row offers no Submit, as the Mac's `maySubmitSession`.
    #[test]
    fn a_held_trace_says_so_and_offers_no_submit() {
        let traces = monitor_traces_copy();
        let held: QueueEntry = serde_json::from_value(serde_json::json!({
            "entry_id": "e", "project_id": "p", "project_label": "p",
            "state": "pending", "reason_label": "second-look-review-required",
            "second_look": ["looks-unsure"],
        }))
        .unwrap();
        assert_eq!(held_word(&held), Some(traces.held));
        assert!(!row_offers_send(&held, false));
        assert_eq!(held_word(&entry("a", "p")), None);
        assert!(row_offers_send(&entry("a", "p"), false));
    }

    /// `TracesTreeView.scroll(to:)`: `GlassMotion.fast`, 0.15 s on the
    /// token curve, read from the Swift and the generated tokens.
    #[test]
    fn the_list_scrolls_a_row_into_view_as_the_macs_does() {
        let read = |path: &str| {
            std::fs::read_to_string(format!(
                "{}/../../macos/Sources/{path}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap()
        };
        assert!(read("TraceCommonsApp/Views/Monitor/TracesViews.swift").contains(
            "withAnimation(scrollAnimates ? GlassMotion.fast(GlassMotion.systemReducesMotion) : nil) {\n            proxy.scrollTo(id)"
        ));
        assert!(
            read("TCDesign/Generated/GlassTokens.swift")
                .contains("public static let fast: Double = 0.15")
        );
        assert_eq!(SCROLL_SECONDS, 0.15);
        // Shown already: no movement.
        assert_eq!(scroll_target(100.0, 300.0, 1000.0, 150.0, 190.0), None);
        // Below the view: the least movement, the row's foot at the view's.
        assert_eq!(
            scroll_target(100.0, 300.0, 1000.0, 420.0, 460.0),
            Some(160.0)
        );
        // Above: its top at the view's top.
        assert_eq!(scroll_target(100.0, 300.0, 1000.0, 40.0, 80.0), Some(40.0));
        // Taller than the view: its top.
        assert_eq!(scroll_target(0.0, 100.0, 1000.0, 300.0, 520.0), Some(300.0));
        // Never past the end.
        assert_eq!(scroll_target(0.0, 300.0, 400.0, 380.0, 420.0), Some(100.0));
        // On the token curve: SwiftUI's bezier(0.2, 0.8, 0.2, 1) at half
        // way is 0.946079 of the distance.
        assert!((scrolled(100.0, 200.0, 0.075) - 194.607_925).abs() < 1e-3);
        assert!((scrolled(100.0, 200.0, 0.0) - 100.0).abs() < 1e-9);
        assert!((scrolled(100.0, 200.0, 0.15) - 200.0).abs() < 1e-9);
    }
}
