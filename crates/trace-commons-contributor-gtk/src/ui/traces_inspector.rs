//! Read-only context for the live queue selection. Full review stays in the sheet.
use super::{App, style};
use crate::model::{QueueEntry, human_when};
use adw::prelude::*;
use std::{cell::RefCell, rc::Rc};
use style::brand_tokens::insights as tokens;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Selection {
    #[default]
    None,
    Folder(String),
    Session(String),
}

impl Selection {
    fn resolve(&self, entries: &[QueueEntry]) -> Self {
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
}

pub(super) struct Inspector {
    pub root: gtk::ScrolledWindow,
    content: gtk::Box,
    selected: RefCell<Selection>,
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
            .visible(false)
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

    pub fn select(&self, app: &Rc<App>, selection: Selection) {
        let opening = !self.root.is_visible() && selection != Selection::None;
        *self.selected.borrow_mut() = selection;
        self.render(app);
        app.queue
            .map
            .highlight(&self.selection(), &app.entries.borrow());
        if opening
            && self.root.is_visible()
            && let Some(split) = self.root.parent().and_downcast::<gtk::Paned>()
        {
            split.set_position(if split.orientation() == gtk::Orientation::Horizontal {
                (split.width() - 320).max(0)
            } else {
                split.height() / 2
            });
        }
    }

    pub fn render(&self, app: &Rc<App>) {
        let entries = app.entries.borrow();
        let selected = self.selected.borrow().resolve(&entries);
        *self.selected.borrow_mut() = selected.clone();
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        self.root.set_visible(selected != Selection::None);
        if selected == Selection::None {
            return;
        }
        let words = trace_commons_contributor::preview_copy::monitor_traces_copy();
        let close = gtk::Button::with_label(crate::copy::CLOSE);
        close.set_halign(gtk::Align::End);
        let weak = Rc::downgrade(app);
        close.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.queue.inspector.select(&app, Selection::None);
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
                self.content.append(&heading(&entry.project_label));
                self.content.append(&body(
                    words
                        .inspector
                        .session_of
                        .replace("{tool}", entry.agent_label()),
                ));
                self.content
                    .append(&caption(human_when(entry.discovered_at)));
                self.content.append(&body(words.session_review.heading));
                let previews = app.previews.borrow();
                if let Some(preview) = previews.get(&id) {
                    let opening = if preview.opening_prompt.trim().is_empty() {
                        words.session_review.no_opening_prompt
                    } else {
                        &preview.opening_prompt
                    };
                    self.content.append(&body(opening));
                    let facts = gtk::Box::new(gtk::Orientation::Vertical, tokens::LINE_GAP);
                    facts.add_css_class("tc-insights-card");
                    facts.append(&body(
                        words
                            .session_review
                            .events
                            .replace("{count}", &preview.event_count.to_string()),
                    ));
                    facts.append(&body(words.session_review.redacted_payload.replace(
                        "{size}",
                        &crate::model::human_bytes(preview.would_send_bytes),
                    )));
                    facts.append(&caption(crate::copy::REMOVED_BY_PATTERN));
                    facts.append(&body(crate::redaction_labels::line(
                        &preview.redactions,
                        &preview.redactions_distinct,
                    )));
                    self.content.append(&facts);
                } else if app.previews_too_large.borrow().contains_key(&id) {
                    self.content
                        .append(&body(crate::copy::TOO_LARGE_OPENING_LINE));
                } else {
                    self.content.append(&body(crate::copy::CHECKING));
                }
                let look = gtk::Button::with_label(words.session_review.look_inside);
                look.update_property(&[gtk::accessible::Property::Label(&format!(
                    "{} — {}",
                    words.session_review.look_inside, entry.project_label
                ))]);
                let weak = Rc::downgrade(app);
                look.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        super::super::preview::open_entry(&app, &id);
                    }
                });
                self.content.append(&look);
            }
            Selection::Folder(id) => {
                let pending: Vec<_> = entries
                    .iter()
                    .filter(|entry| entry.state == "pending")
                    .collect();
                let folders = crate::queue_folders::group(&pending);
                let Some(folder) = folders.iter().find(|folder| folder.project_id == id) else {
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

fn caption(text: impl AsRef<str>) -> gtk::Label {
    let label = body(text);
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    label
}

/// Keep both scrollable panes usable when the window is narrow.
pub(super) fn orientation(width: i32) -> gtk::Orientation {
    if width >= 1000 {
        gtk::Orientation::Horizontal
    } else {
        gtk::Orientation::Vertical
    }
}

#[cfg(test)]
mod tests {
    use super::Selection;

    #[test]
    fn narrow_windows_stack_the_inspector_below_the_queue() {
        assert_eq!(super::orientation(640), gtk::Orientation::Vertical);
        assert_eq!(super::orientation(980), gtk::Orientation::Vertical);
        assert_eq!(super::orientation(1200), gtk::Orientation::Horizontal);
    }
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
    fn disappeared_or_sent_session_closes_inspector() {
        let selection = Selection::Session("a".into());
        assert_eq!(selection.resolve(&[]), Selection::None);
        assert_eq!(
            selection.resolve(&[entry("a", "p", "approved")]),
            Selection::None
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
}
