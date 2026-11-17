//! Tools, as the Mac draws it (`ToolsScreen`, `ToolChecks`): one check per
//! tool found here, unticked until the person ticks it (an unticked tool is
//! Not used), a row per folder added with "add your tool", the core's line
//! when one tool is watched in two rows, and Continue once anything is
//! watched. Every decision is `first_run::tools`; this file only draws it.
//!
//! The same screen serves both first-run windows: before the daemon runs
//! (`roots::present`, which writes the declaration to the settings file and
//! starts the daemon) and with it running (the coordinator, which sends the
//! planner's `Commit::Roots`).
use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use trace_commons_contributor::first_run_copy::first_run_copy;
use trace_commons_contributor::source::discovery;

use super::onboarding::body_label;
use crate::first_run::tools::{AddOutcome, AddedFolder, FolderKind, Found, Tools, add_outcome};

/// The live Tools screen.
pub(super) struct ToolsScreen {
    pub root: gtk::Box,
    rows: gtk::Box,
    pending: RefCell<Option<(String, Vec<FolderKind>)>>,
    pub continue_button: gtk::Button,
    pub failure: gtk::Label,
    tools: Rc<RefCell<Tools>>,
    parent: glib::WeakRef<gtk::Window>,
    busy: std::cell::Cell<bool>,
    refused: gtk::Label,
}

/// The tool's name, as the Mac's `SourceKind.displayName` gives it.
pub(super) fn tool_name(source: &str) -> &'static str {
    super::roots::source_title(source)
}

fn kind_name(kind: &FolderKind) -> &'static str {
    match kind {
        FolderKind::Source(source) => tool_name(source),
        FolderKind::Trajectory => first_run_copy().tools.trajectory_label,
    }
}

fn fill(template: &str, key: &str, value: &str) -> String {
    template.replace(&format!("{{{key}}}"), value)
}

fn folder_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Build Tools. `on_continue` runs when Continue is pressed with a
/// declaration the daemon would take; the caller commits it and calls
/// [`ToolsScreen::finish`] with the result.
pub(super) fn build(
    tools: Rc<RefCell<Tools>>,
    parent: &gtk::Window,
    on_continue: impl Fn() + 'static,
) -> Rc<ToolsScreen> {
    let copy = first_run_copy();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
    root.set_margin_start(32);
    root.set_margin_end(32);
    root.set_margin_top(24);
    root.set_margin_bottom(24);
    let title = gtk::Label::builder()
        .label(format!(
            "{}{}",
            copy.tools.title_light, copy.tools.title_bold
        ))
        .xalign(0.0)
        .wrap(true)
        .build();
    title.add_css_class("tc-flow-title");
    root.append(&title);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.append(&rows);
    let refused = body_label(copy.tools.add_tool_refused);
    refused.add_css_class("tc-refused");
    refused.set_visible(false);
    let failure = body_label("");
    failure.add_css_class("tc-refused");
    failure.set_visible(false);
    let continue_button = gtk::Button::with_label(copy.frame.continue_button);
    continue_button.add_css_class("suggested-action");
    continue_button.set_halign(gtk::Align::End);
    continue_button.set_sensitive(false);
    let screen = Rc::new(ToolsScreen {
        root,
        rows,
        pending: RefCell::new(None),
        continue_button,
        failure,
        tools,
        parent: parent.downgrade(),
        busy: std::cell::Cell::new(false),
        refused,
    });
    let add = gtk::Button::with_label(copy.tools.add_tool);
    add.add_css_class("flat");
    add.set_halign(gtk::Align::Start);
    add.set_tooltip_text(Some(copy.tools.add_tool_caption));
    add.connect_clicked({
        let weak = Rc::downgrade(&screen);
        move |_| {
            if let Some(screen) = weak.upgrade() {
                screen.choose_folder();
            }
        }
    });
    screen.root.append(&add);
    // "Click to add or drag & drop": a folder dropped anywhere on Tools is
    // added as one picked with the button.
    let drop = gtk::DropTarget::new(gtk::gio::File::static_type(), gtk::gdk::DragAction::COPY);
    drop.connect_drop({
        let weak = Rc::downgrade(&screen);
        move |_, value, _, _| {
            let (Some(screen), Ok(file)) = (weak.upgrade(), value.get::<gtk::gio::File>()) else {
                return false;
            };
            match file.path() {
                Some(path) => {
                    screen.add_path(&path);
                    true
                }
                None => false,
            }
        }
    });
    screen.root.add_controller(drop);
    screen.root.append(&screen.refused);
    screen.root.append(&screen.failure);
    screen.root.append(&screen.continue_button);
    let on_continue = Rc::new(on_continue);
    screen.continue_button.connect_clicked({
        let weak = Rc::downgrade(&screen);
        move |_| {
            let Some(screen) = weak.upgrade() else {
                return;
            };
            if screen.busy.get() || !screen.tools.borrow().can_continue() {
                return;
            }
            screen.set_busy(true);
            screen.failure.set_visible(false);
            on_continue();
        }
    });
    if screen.tools.borrow().discovered.is_none() {
        screen.discover();
    } else {
        screen.render();
    }
    screen
}

impl ToolsScreen {
    fn discover(self: &Rc<Self>) {
        // The core's loading line, for any computer (owner, 2026-11-17).
        let loading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let spinner = gtk::Spinner::new();
        spinner.start();
        loading.append(&spinner);
        let line = gtk::Label::new(Some(first_run_copy().folders.loading_this_computer));
        line.add_css_class("tc-meta");
        loading.append(&line);
        self.rows.append(&loading);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let found: Vec<Found> = discovery::probe_this_machine()
                .iter()
                .map(Found::from)
                .collect();
            let _ = tx.send_blocking(found);
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(found) = rx.recv().await else {
                return;
            };
            if let Some(screen) = weak.upgrade() {
                screen.tools.borrow_mut().record_discovery(found);
                screen.render();
            }
        });
    }

    /// The commit ended: on failure say so with `failure`, and let Continue
    /// be pressed again.
    pub fn finish(&self, failure: Option<&str>) {
        self.set_busy(false);
        match failure {
            Some(line) => {
                self.failure.set_label(line);
                self.failure.set_visible(true);
            }
            None => self.failure.set_visible(false),
        }
    }

    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        self.rows.set_sensitive(!busy);
        self.update();
    }

    fn update(&self) {
        self.continue_button
            .set_sensitive(!self.busy.get() && self.tools.borrow().can_continue());
    }

    fn rerender(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(screen) = weak.upgrade() {
                screen.render();
            }
        });
    }

    fn render(self: &Rc<Self>) {
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }
        let copy = first_run_copy();
        let tools = self.tools.borrow().clone();
        for found in tools.listed() {
            let name = tool_name(&found.source);
            let check = gtk::CheckButton::with_label(name);
            check.set_active(tools.is_watched(&found.source));
            check.set_tooltip_text(Some(&found.path));
            check.update_property(&[gtk::accessible::Property::Label(&fill(
                copy.folders.watch_question,
                "tool",
                name,
            ))]);
            check.connect_toggled({
                let weak = Rc::downgrade(self);
                let found = found.clone();
                move |check| {
                    if let Some(screen) = weak.upgrade() {
                        screen
                            .tools
                            .borrow_mut()
                            .set_watched(&found, check.is_active());
                        screen.update();
                        screen.rerender();
                    }
                }
            });
            self.rows.append(&check);
        }
        let twice = tools.watched_twice();
        for folder in tools.added.iter() {
            let name = match &folder.kind {
                FolderKind::Trajectory => copy.tools.trajectory_label.to_owned(),
                FolderKind::Source(_) => folder_name(&folder.path),
            };
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let check = gtk::CheckButton::with_label(&name);
            check.set_active(folder.watched == Some(true));
            check.set_tooltip_text(Some(&folder.path));
            check.update_property(&[gtk::accessible::Property::Label(&fill(
                copy.folders.watch_question,
                "tool",
                &name,
            ))]);
            check.connect_toggled({
                let weak = Rc::downgrade(self);
                let path = folder.path.clone();
                let trajectory = folder.kind == FolderKind::Trajectory;
                move |check| {
                    if let Some(screen) = weak.upgrade() {
                        {
                            let mut tools = screen.tools.borrow_mut();
                            if trajectory && !check.is_active() {
                                tools.withdraw_trajectory();
                            } else {
                                tools.answer_added(&path, Some(check.is_active()));
                            }
                        }
                        screen.update();
                        screen.rerender();
                    }
                }
            });
            row.append(&check);
            let meta = gtk::Label::builder()
                .label(copy.tools.added_by_you)
                .xalign(0.0)
                .build();
            meta.add_css_class("tc-meta");
            row.append(&meta);
            if let FolderKind::Source(source) = &folder.kind {
                if folder.watched == Some(true) && twice.contains(source) {
                    let line = body_label(&fill(
                        copy.tools.one_folder_per_tool,
                        "tool",
                        tool_name(source),
                    ));
                    line.add_css_class("tc-refused");
                    row.append(&line);
                }
            }
            self.rows.append(&row);
        }
        if let Some((path, kinds)) = self.pending.borrow().clone() {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            row.append(&body_label(&fill(
                copy.tools.which_kind,
                "folder",
                &folder_name(&path),
            )));
            let choices = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            for kind in kinds.iter().cloned().map(Some).chain([None]) {
                let label = kind.as_ref().map_or(copy.tools.neither, kind_name);
                let button = gtk::Button::with_label(label);
                button.connect_clicked({
                    let weak = Rc::downgrade(self);
                    let path = path.clone();
                    move |_| {
                        if let Some(screen) = weak.upgrade() {
                            screen.pending.borrow_mut().take();
                            if let Some(kind) = kind.clone() {
                                screen
                                    .tools
                                    .borrow_mut()
                                    .add(AddedFolder::new(kind, path.clone()));
                            }
                            screen.update();
                            screen.rerender();
                        }
                    }
                });
                choices.append(&button);
            }
            row.append(&choices);
            self.rows.append(&row);
        }
        self.update();
    }

    fn choose_folder(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let parent = self.parent.upgrade();
        let chooser = gtk::FileChooserNative::new(
            Some(first_run_copy().tools.add_tool_caption),
            parent.as_ref(),
            gtk::FileChooserAction::SelectFolder,
            None,
            None,
        );
        let weak = Rc::downgrade(self);
        chooser.connect_response(move |chooser, response| {
            if response == gtk::ResponseType::Accept
                && let Some(path) = chooser.file().and_then(|f| f.path())
                && let Some(screen) = weak.upgrade()
            {
                screen.add_path(&path);
            }
            chooser.destroy();
        });
        chooser.show();
    }

    /// A folder picked or dropped: described by the core, then added, asked
    /// about, or refused (`ToolsScreenLayout.outcome`).
    fn add_path(self: &Rc<Self>, path: &std::path::Path) {
        if self.busy.get() {
            return;
        }
        let matches: Vec<Found> = discovery::describe_folder(path)
            .iter()
            .map(Found::from)
            .collect();
        let path = path.to_string_lossy().into_owned();
        self.refused.set_visible(false);
        match add_outcome(&path, &matches) {
            AddOutcome::Added(folder) => self.tools.borrow_mut().add(folder),
            AddOutcome::Ask { path, kinds } => {
                self.pending.replace(Some((path, kinds)));
            }
            AddOutcome::Refused => self.refused.set_visible(true),
        }
        self.render();
    }
}
