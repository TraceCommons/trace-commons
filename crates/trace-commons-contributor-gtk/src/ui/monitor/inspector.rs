//! Read-only context for the live queue selection. Full review stays in the sheet.
//!
//! The window-level inspector's Traces column: the selected session's or
//! folder's card, or the summary when nothing is selected. It moved here
//! from the Queue page, where it was the end child of a `gtk::Paned`; the
//! window's inspector pane hosts it now (`monitor::shell`). As on the Mac,
//! it never closes itself: a selection that has gone is the summary.
use crate::model::QueueEntry;
use crate::ui::{App, style};
use adw::prelude::*;
use std::{cell::RefCell, rc::Rc};
use style::brand_tokens::insights as tokens;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    None,
    Folder(String),
    Session(String),
}

impl Selection {
    pub(super) fn resolve(&self, entries: &[QueueEntry]) -> Self {
        let exists =
            entries
                .iter()
                .filter(|entry| entry.state == "pending")
                .any(|entry| match self {
                    Self::Folder(id) => entry.project_id == *id,
                    Self::Session(id) => entry.entry_id == *id,
                    Self::None => false,
                });
        if exists { self.clone() } else { Self::None }
    }

    /// As `resolve`, and a folder the core lists that the folder list
    /// draws (any mode but Ignore) stays selected with nothing waiting.
    pub(super) fn resolve_listed(
        &self,
        entries: &[QueueEntry],
        projects: &[crate::model::Project],
    ) -> Self {
        if let Self::Folder(id) = self
            && projects
                .iter()
                .any(|project| &project.project_id == id && project.mode != "ignore")
        {
            return self.clone();
        }
        self.resolve(entries)
    }
}

pub struct Inspector {
    pub root: gtk::ScrolledWindow,
    content: gtk::Box,
    selected: RefCell<Selection>,
}

impl Default for Inspector {
    fn default() -> Self {
        Self::new()
    }
}

impl Inspector {
    pub fn new() -> Self {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(tokens::SECTION_GAP)
            .margin_top(tokens::BODY_INSET)
            .margin_bottom(tokens::BODY_INSET)
            .margin_start(tokens::BODY_INSET)
            .margin_end(tokens::BODY_INSET)
            .build();
        content.add_css_class("tc-insights");
        let root = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(280)
            .vexpand(true)
            .build();
        Self {
            root,
            content,
            selected: RefCell::new(Selection::None),
        }
    }

    pub fn selection(&self) -> Selection {
        self.selected.borrow().clone()
    }

    /// Moves the selection and redraws. Whether the pane opens is the
    /// window's decision (`Shell::select`), not this column's.
    pub fn select(&self, app: &Rc<App>, selection: Selection) {
        *self.selected.borrow_mut() = selection;
        self.render(app);
    }

    /// Redraw for a read only the summary draws from: only while the
    /// summary is what shows ([`redraws_for_summary_read`]).
    pub fn render_summary(&self, app: &Rc<App>) {
        if redraws_for_summary_read(&self.selection()) {
            self.render(app);
        }
    }

    pub fn render(&self, app: &Rc<App>) {
        let entries = app.entries.borrow();
        let projects = app.projects.borrow();
        let selected = self.selected.borrow().resolve_listed(&entries, &projects);
        *self.selected.borrow_mut() = selected.clone();
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let words = trace_commons_contributor::preview_copy::monitor_traces_copy();
        if selected == Selection::None {
            // Nothing selected is the summary, never an empty pane.
            let inputs = super::summary::Inputs {
                status: app.status.borrow().clone(),
                loaded: app.shell.map.pending_read(),
                pending: entries
                    .iter()
                    .filter(|entry| entry.state == "pending")
                    .cloned()
                    .collect(),
                projects: app.projects.borrow().clone(),
                rollup: app.summary_rollup.borrow().clone(),
                condition: app.credit_condition.borrow().clone(),
                history: app.history_records.borrow().clone(),
                watched: app.shell.map.watched_tools(),
            };
            self.content.append(&super::summary::view(&inputs));
            return;
        }
        let close = gtk::Button::with_label(crate::copy::CLOSE);
        close.set_halign(gtk::Align::End);
        let weak = Rc::downgrade(app);
        close.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                // Close puts the card away and the pane with it.
                app.shell.select(&app, Selection::None);
                app.shell.set_inspector_shown(&app, false);
            }
        });
        self.content.append(&close);
        match selected {
            Selection::Session(id) => {
                let Some(entry) = entries
                    .iter()
                    .find(|entry| entry.entry_id == id && entry.state == "pending")
                else {
                    return;
                };
                self.content.append(&body(
                    words
                        .inspector
                        .session_of
                        .replace("{tool}", entry.agent_label()),
                ));
                // The trace's review, as the folder list's row once drew it
                // (the Mac's review card lives here too): what would leave,
                // what scrubbing took out and left in, its attestation and
                // eligibility in the core's sentences, and Look inside,
                // Not this one and Submit under the same rules. Opened by
                // its place in the flat pending list, which the preview
                // sheet derives the same way.
                let index = entries
                    .iter()
                    .filter(|entry| entry.state == "pending")
                    .position(|entry| entry.entry_id == id)
                    .unwrap_or_default();
                self.content
                    .append(&crate::ui::queue::row(app, entry, index));
            }
            Selection::Folder(id) => {
                let pending: Vec<_> = entries
                    .iter()
                    .filter(|entry| entry.state == "pending")
                    .collect();
                let folders = crate::queue_folders::group(&pending);
                let Some(folder) = folders.iter().find(|folder| folder.project_id == id) else {
                    // A listed folder with nothing waiting.
                    if let Some(project) = projects.iter().find(|p| p.project_id == id) {
                        self.content.append(&heading(&project.project_label));
                        self.content.append(&body(words.inspector.project_details));
                        if !project.project_path.is_empty() {
                            self.content.append(&body(&project.project_path));
                        }
                    }
                    return;
                };
                self.content.append(&heading(&folder.project_label));
                self.content.append(&body(words.inspector.project_details));
                self.content.append(&body(&folder.project_path));
                self.content.append(&body(crate::copy::folder_summary(
                    folder.members.len(),
                    folder.bytes,
                )));
            }
            Selection::None => {}
        }
    }
}

fn body(text: impl AsRef<str>) -> gtk::Label {
    gtk::Label::builder()
        .label(text.as_ref())
        .xalign(0.0)
        .wrap(true)
        .build()
}

fn heading(text: impl AsRef<str>) -> gtk::Label {
    let label = body(text);
    label.add_css_class("heading");
    label
}

/// Whether a read only the summary draws from (`tool_destinations`,
/// `history_rollup`, `commons_credit_summary`) redraws the inspector:
/// only while nothing is selected, so a review card being read is not
/// rebuilt under the reader on every daemon event.
pub fn redraws_for_summary_read(selection: &Selection) -> bool {
    *selection == Selection::None
}

#[cfg(test)]
mod tests {
    use super::{Selection, redraws_for_summary_read};

    use crate::model::QueueEntry;

    fn entry(id: &str, project: &str, state: &str) -> QueueEntry {
        QueueEntry {
            entry_id: id.into(),
            project_id: project.into(),
            project_label: "same label".into(),
            state: state.into(),
            ..Default::default()
        }
    }

    #[test]
    fn selection_survives_reordering_by_identity() {
        let selection = Selection::Session("a".into());
        assert_eq!(
            selection.resolve(&[entry("b", "p", "pending"), entry("a", "p", "pending")]),
            selection
        );
    }

    #[test]
    fn disappeared_or_sent_session_falls_back_to_the_summary() {
        let selection = Selection::Session("a".into());
        assert_eq!(selection.resolve(&[]), Selection::None);
        assert_eq!(
            selection.resolve(&[entry("a", "p", "approved")]),
            Selection::None
        );
    }

    /// The folder list draws every folder the core lists, waiting traces
    /// or not, so selecting an idle one keeps it (the Mac's
    /// `TracesTree.resolve`); an ignored folder, which the list leaves
    /// out, and one the core does not list, fall back to the summary.
    #[test]
    fn an_idle_listed_folder_stays_selected() {
        let project = |id: &str, mode: &str| -> crate::model::Project {
            serde_json::from_value(serde_json::json!({
                "project_id": id, "project_label": id, "mode": mode
            }))
            .unwrap()
        };
        let projects = [project("idle", "notify_only"), project("quiet", "ignore")];
        let idle = Selection::Folder("idle".into());
        assert_eq!(idle.resolve_listed(&[], &projects), idle);
        assert_eq!(
            Selection::Folder("quiet".into()).resolve_listed(&[], &projects),
            Selection::None
        );
        assert_eq!(
            Selection::Folder("gone".into()).resolve_listed(&[], &projects),
            Selection::None
        );
        // A trace still resolves only while it waits.
        let session = Selection::Session("a".into());
        assert_eq!(session.resolve_listed(&[], &projects), Selection::None);
        assert_eq!(
            session.resolve_listed(&[entry("a", "idle", "pending")], &projects),
            session
        );
    }

    #[test]
    fn identical_folder_labels_never_transfer_selection() {
        let selection = Selection::Folder("p1".into());
        assert_eq!(
            selection.resolve(&[entry("a", "p2", "pending")]),
            Selection::None
        );
        assert_eq!(selection.resolve(&[entry("a", "p1", "pending")]), selection);
    }

    /// The summary's reads redraw the inspector only while it shows the
    /// summary; a selected folder or trace keeps its card.
    #[test]
    fn a_summary_read_redraws_only_the_summary() {
        assert!(redraws_for_summary_read(&Selection::None));
        assert!(!redraws_for_summary_read(&Selection::Folder("p".into())));
        assert!(!redraws_for_summary_read(&Selection::Session("e".into())));
        let refresh = include_str!("../mod.rs");
        for read in [
            "\"tool_destinations\"",
            "\"history_rollup\"",
            "\"commons_credit_summary\"",
        ] {
            let at = refresh.find(read).expect(read);
            // The call's statement: up to the next one at its indent.
            let rest = &refresh[at..];
            let end = rest
                .match_indices("\n        ")
                .map(|(i, _)| i)
                .find(|&i| {
                    rest[i + 9..]
                        .chars()
                        .next()
                        .is_some_and(|c| !" })".contains(c))
                })
                .expect(read);
            let call = &rest[..end];
            assert!(call.contains("inspector.render_summary(app)"), "{read}");
            assert!(!call.contains("inspector.render(app)"), "{read}");
        }
    }
}
