//! Account-free local mission draft inbox. Proposal strings are rendered as
//! plain text and never opened, executed, or treated as trusted instructions.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use trace_commons_contributor::mission_draft::{MissionDraftSummary, StoredMissionDraft};
use trace_commons_contributor::mission_draft_service::{
    self as service, MissionDraftOperation as Op, MissionDraftRequest,
    MissionDraftResponse as Response,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct RequestTicket {
    generation: u64,
    requested_id: Option<String>,
    selection_id: Option<String>,
    selected_file: Option<PathBuf>,
}

impl RequestTicket {
    fn accepts(
        &self,
        generation: u64,
        selected_id: Option<&str>,
        selected_file: Option<&PathBuf>,
    ) -> bool {
        self.generation == generation
            && self.selection_id.as_deref() == selected_id
            && self.selected_file.as_ref() == selected_file
    }
}

fn response_id_matches(ticket: &RequestTicket, returned_id: &str) -> bool {
    ticket.requested_id.as_deref() == Some(returned_id)
}

fn selection_survives_delete(selected_id: Option<&str>, deleted_id: &str) -> bool {
    selected_id.is_some_and(|selected| selected != deleted_id)
}

#[derive(Default)]
struct Flight {
    busy: bool,
    cancelled: bool,
    closed: bool,
}

impl Flight {
    fn begin(&mut self) -> bool {
        if self.busy || self.closed {
            return false;
        }
        self.busy = true;
        self.cancelled = false;
        true
    }

    fn finish(&mut self) -> bool {
        self.busy = false;
        !self.cancelled && !self.closed
    }
}

fn copy(key: &str) -> &'static str {
    static COPY: std::sync::OnceLock<std::collections::BTreeMap<String, String>> =
        std::sync::OnceLock::new();
    COPY.get_or_init(service::ui_copy)
        .get(key)
        .map(String::as_str)
        .expect("shared mission draft copy key")
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .use_markup(false)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .build()
}

enum Completion {
    Response(Response),
    Failed,
}

pub struct MissionDraftsView {
    pub root: gtk::Box,
    controls: gtk::Box,
    rows: gtk::Box,
    status: gtk::Label,
    notice: gtk::Label,
    detail: gtk::Label,
    import: gtk::Button,
    selected_file: RefCell<Option<PathBuf>>,
    selected_id: RefCell<Option<String>>,
    generation: Cell<u64>,
    pending_refresh: Cell<bool>,
    flight: RefCell<Flight>,
    store_dir: Option<PathBuf>,
}

impl MissionDraftsView {
    pub fn new(window: &adw::ApplicationWindow) -> Rc<Self> {
        Self::with_store(window, None)
    }

    fn with_store(window: &adw::ApplicationWindow, store_dir: Option<PathBuf>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        for setter in [
            gtk::prelude::WidgetExt::set_margin_top,
            gtk::prelude::WidgetExt::set_margin_bottom,
            gtk::prelude::WidgetExt::set_margin_start,
            gtk::prelude::WidgetExt::set_margin_end,
        ] {
            setter(&root, 20);
        }
        let title = label(copy("title"));
        title.add_css_class("title-1");
        root.append(&title);
        root.append(&label(copy("intro")));
        root.append(&label(copy("review_notice")));
        root.append(&label(copy("authority_notice")));

        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let choose = gtk::Button::with_label(copy("choose_file"));
        let import = gtk::Button::with_label(copy("import"));
        import.set_sensitive(false);
        let refresh = gtk::Button::with_label(copy("refresh"));
        controls.append(&choose);
        controls.append(&import);
        controls.append(&refresh);
        root.append(&controls);

        let status = label(copy("empty"));
        let notice = label("");
        root.append(&status);
        root.append(&notice);

        let rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let detail = label("");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.append(&rows);
        content.append(&detail);
        content.append(&label(copy("display_notice")));
        let scroller = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&content)
            .build();
        root.append(&scroller);

        let view = Rc::new(Self {
            root,
            controls,
            rows,
            status,
            notice,
            detail,
            import,
            selected_file: RefCell::new(None),
            selected_id: RefCell::new(None),
            generation: Cell::new(0),
            pending_refresh: Cell::new(false),
            flight: RefCell::new(Flight::default()),
            store_dir,
        });

        choose.connect_clicked({
            let weak = Rc::downgrade(&view);
            let window = window.clone();
            move |_| {
                let Some(view) = weak.upgrade() else { return };
                view.choose_file(&window);
            }
        });
        view.import.connect_clicked({
            let weak = Rc::downgrade(&view);
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.import_selected();
                }
            }
        });
        refresh.connect_clicked({
            let weak = Rc::downgrade(&view);
            move |_| {
                if let Some(view) = weak.upgrade() {
                    if view.flight.borrow().busy {
                        view.pending_refresh.set(true);
                    } else {
                        view.refresh();
                    }
                }
            }
        });
        view.root.connect_unmap({
            let weak = Rc::downgrade(&view);
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.invalidate();
                    view.flight.borrow_mut().cancelled = true;
                }
            }
        });
        view.root.connect_map({
            let weak = Rc::downgrade(&view);
            move |_| {
                if let Some(view) = weak.upgrade() {
                    if view.flight.borrow().busy {
                        view.pending_refresh.set(true);
                    } else {
                        view.refresh();
                    }
                }
            }
        });
        if let Some(application) = window.application() {
            application.connect_shutdown({
                let weak = Rc::downgrade(&view);
                move |_| {
                    if let Some(view) = weak.upgrade() {
                        view.flight.borrow_mut().closed = true;
                    }
                }
            });
        }
        // The window owns the controller lifetime; worker completions and
        // widget callbacks retain only weak references.
        let retained = view.clone();
        window.connect_destroy(move |_| {
            retained.flight.borrow_mut().closed = true;
        });
        view
    }

    fn advance(&self) -> u64 {
        let next = self.generation.get().wrapping_add(1);
        self.generation.set(next);
        next
    }

    fn invalidate(&self) {
        self.advance();
        *self.selected_id.borrow_mut() = None;
        self.detail.set_text("");
    }

    fn choose_file(self: &Rc<Self>, window: &adw::ApplicationWindow) {
        let generation = self.generation.get();
        let chooser = gtk::FileChooserNative::new(
            Some(copy("choose_file")),
            Some(window),
            gtk::FileChooserAction::Open,
            Some(copy("import")),
            Some(copy("cancel")),
        );
        let weak = Rc::downgrade(self);
        chooser.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept
                && let (Some(view), Some(path)) =
                    (weak.upgrade(), dialog.file().and_then(|file| file.path()))
                && view.generation.get() == generation
                && !view.flight.borrow().closed
            {
                view.advance();
                *view.selected_file.borrow_mut() = Some(path);
                view.import.set_sensitive(true);
                view.status.set_text(copy("file_selected"));
            }
            dialog.destroy();
        });
        chooser.show();
    }

    fn import_selected(self: &Rc<Self>) {
        let Some(file) = self.selected_file.borrow().clone() else {
            return;
        };
        let generation = self.advance();
        let ticket = RequestTicket {
            generation,
            requested_id: None,
            selection_id: self.selected_id.borrow().clone(),
            selected_file: Some(file.clone()),
        };
        self.request(Op::Import { file }, ticket);
    }

    fn refresh(self: &Rc<Self>) {
        if self.flight.borrow().busy || self.flight.borrow().closed {
            return;
        }
        let ticket = RequestTicket {
            generation: self.advance(),
            requested_id: None,
            selection_id: self.selected_id.borrow().clone(),
            selected_file: self.selected_file.borrow().clone(),
        };
        self.request(Op::List {}, ticket);
    }

    fn show(self: &Rc<Self>, id: String) {
        if self.flight.borrow().busy || self.flight.borrow().closed {
            return;
        }
        let ticket = RequestTicket {
            generation: self.advance(),
            requested_id: Some(id.clone()),
            selection_id: self.selected_id.borrow().clone(),
            selected_file: self.selected_file.borrow().clone(),
        };
        self.request(Op::Show { id }, ticket);
    }

    fn confirm_delete(self: &Rc<Self>, window: &adw::ApplicationWindow, id: String) {
        let generation = self.generation.get();
        let dialog = adw::MessageDialog::new(
            Some(window),
            Some(copy("delete")),
            Some(copy("delete_confirm")),
        );
        dialog.add_responses(&[("cancel", copy("cancel")), ("delete", copy("delete"))]);
        dialog.set_close_response("cancel");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            let Some(view) = weak.upgrade() else { return };
            if response == "delete"
                && view.generation.get() == generation
                && !view.flight.borrow().busy
                && !view.flight.borrow().closed
            {
                view.delete(id.clone());
            }
        });
        dialog.present();
    }

    fn delete(self: &Rc<Self>, id: String) {
        let ticket = RequestTicket {
            generation: self.advance(),
            requested_id: Some(id.clone()),
            selection_id: self.selected_id.borrow().clone(),
            selected_file: self.selected_file.borrow().clone(),
        };
        self.request(Op::Delete { id }, ticket);
    }

    fn request(self: &Rc<Self>, operation: Op, ticket: RequestTicket) {
        if !self.flight.borrow_mut().begin() {
            return;
        }
        self.controls.set_sensitive(false);
        self.rows.set_sensitive(false);
        self.status.set_text(copy("working"));
        let store_dir = self.store_dir.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = match std::panic::catch_unwind(|| {
                service::execute(MissionDraftRequest {
                    store_dir,
                    operation,
                })
            }) {
                Ok(Ok(response)) => Completion::Response(response),
                _ => Completion::Failed,
            };
            let _ = tx.send_blocking(result);
        });
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            let Ok(completion) = rx.recv().await else {
                return;
            };
            let Some(view) = weak.upgrade() else { return };
            if !view.flight.borrow_mut().finish() {
                if !view.flight.borrow().closed && view.pending_refresh.replace(false) {
                    view.refresh();
                }
                return;
            }
            view.controls.set_sensitive(true);
            view.rows.set_sensitive(true);
            view.import
                .set_sensitive(view.selected_file.borrow().is_some());
            if !ticket.accepts(
                view.generation.get(),
                view.selected_id.borrow().as_deref(),
                view.selected_file.borrow().as_ref(),
            ) {
                return;
            }
            view.apply(completion, &ticket);
        });
    }

    fn apply(self: &Rc<Self>, completion: Completion, ticket: &RequestTicket) {
        match completion {
            Completion::Response(Response::List { drafts }) => {
                self.render_list(&drafts);
                self.status.set_text(if drafts.is_empty() {
                    copy("empty")
                } else {
                    copy("refreshed")
                });
            }
            Completion::Response(Response::Import { draft }) => {
                self.notice
                    .set_text(copy(if draft.inserted { "added" } else { "duplicate" }));
                *self.selected_file.borrow_mut() = None;
                self.import.set_sensitive(false);
                self.refresh();
            }
            Completion::Response(Response::Show { draft })
                if response_id_matches(ticket, &draft.id) =>
            {
                *self.selected_id.borrow_mut() = Some(draft.id.clone());
                self.detail.set_text(&render_detail(&draft));
                self.status.set_text(copy("needs_curator_review"));
            }
            Completion::Response(Response::Delete { draft })
                if draft.deleted && response_id_matches(ticket, &draft.id) =>
            {
                if !selection_survives_delete(self.selected_id.borrow().as_deref(), &draft.id) {
                    self.invalidate();
                }
                self.notice.set_text(copy("deleted"));
                self.refresh();
            }
            Completion::Response(_) | Completion::Failed => self.status.set_text(copy("error")),
        }
    }

    fn render_list(self: &Rc<Self>, drafts: &[MissionDraftSummary]) {
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }
        for draft in drafts {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let summary = label(&format!(
                "{}: {}\n{}: {}\n{}",
                copy("proposal_sha256"),
                draft.id,
                copy("source_count"),
                draft.source_count,
                copy("needs_curator_review")
            ));
            summary.set_hexpand(true);
            row.append(&summary);
            let show = gtk::Button::with_label(copy("show"));
            let delete = gtk::Button::with_label(copy("delete"));
            row.append(&show);
            row.append(&delete);
            show.connect_clicked({
                let weak = Rc::downgrade(self);
                let id = draft.id.clone();
                move |_| {
                    if let Some(view) = weak.upgrade() {
                        view.show(id.clone());
                    }
                }
            });
            delete.connect_clicked({
                let weak = Rc::downgrade(self);
                let id = draft.id.clone();
                move |_| {
                    if let Some(view) = weak.upgrade()
                        && let Some(window) =
                            view.root.root().and_downcast::<adw::ApplicationWindow>()
                    {
                        view.confirm_delete(&window, id.clone());
                    }
                }
            });
            self.rows.append(&row);
        }
    }
}

fn render_detail(draft: &StoredMissionDraft) -> String {
    let proposal = &draft.proposal;
    let mut lines = vec![
        format!("{}: {}", copy("proposal_sha256"), draft.id),
        format!("{}: {}", copy("proposal_title"), proposal.title),
        format!("{}: {}", copy("author_unverified"), proposal.author_id),
        format!(
            "{}: {}",
            copy("evaluator_unverified"),
            proposal.evaluator_id
        ),
        format!("{}: {}", copy("rubric_version"), proposal.rubric_version),
        format!("{}: {}", copy("source_claim"), proposal.claim_to_test),
        format!("{}: {}", copy("task"), proposal.task),
        format!(
            "{}: {}",
            copy("starting_artifact"),
            proposal.starting_artifact.url
        ),
        format!(
            "{}: {}",
            copy("starting_artifact_digest"),
            proposal.starting_artifact.sha256
        ),
        copy("proposed_budget").to_string(),
        format!(
            "{}: {}",
            copy("duration_seconds"),
            proposal.budget.max_duration_seconds
        ),
        format!(
            "{}: {}",
            copy("input_tokens"),
            proposal.budget.max_input_tokens
        ),
        format!(
            "{}: {}",
            copy("output_tokens"),
            proposal.budget.max_output_tokens
        ),
    ];
    append_values(&mut lines, copy("source_urls"), &proposal.source_urls);
    append_values(
        &mut lines,
        copy("success_criteria"),
        &proposal.success_criteria,
    );
    append_values(
        &mut lines,
        copy("required_evidence"),
        &proposal.required_evidence,
    );
    append_values(&mut lines, copy("allowed_models"), &proposal.allowed_models);
    append_values(&mut lines, copy("allowed_tools"), &proposal.allowed_tools);
    lines.push(copy("review_notice").to_string());
    lines.push(copy("authority_notice").to_string());
    lines.join("\n")
}

fn append_values(lines: &mut Vec<String>, heading: &str, values: &[String]) {
    lines.push(format!("{heading}:"));
    lines.extend(values.iter().map(|value| format!("  {value}")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_tickets_bind_generation_selection_and_file() {
        let file = PathBuf::from("proposal.json");
        let ticket = RequestTicket {
            generation: 7,
            requested_id: Some("draft".into()),
            selection_id: Some("draft".into()),
            selected_file: Some(file.clone()),
        };
        assert!(ticket.accepts(7, Some("draft"), Some(&file)));
        assert!(!ticket.accepts(8, Some("draft"), Some(&file)));
        assert!(!ticket.accepts(7, Some("other"), Some(&file)));
        assert!(!ticket.accepts(7, Some("draft"), None));
    }

    #[test]
    fn cancelled_and_closed_flights_never_publish() {
        let mut flight = Flight::default();
        assert!(flight.begin());
        flight.cancelled = true;
        assert!(!flight.finish());
        flight.closed = true;
        assert!(!flight.begin());
    }

    #[test]
    fn mismatched_responses_are_rejected_and_delete_only_clears_its_selection() {
        let ticket = RequestTicket {
            generation: 1,
            requested_id: Some("requested".into()),
            selection_id: Some("requested".into()),
            selected_file: None,
        };
        assert!(response_id_matches(&ticket, "requested"));
        assert!(!response_id_matches(&ticket, "other"));
        assert!(!selection_survives_delete(Some("requested"), "requested"));
        assert!(selection_survives_delete(Some("other"), "requested"));
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn account_free_view_imports_shows_and_deletes_plain_text_draft() {
        assert_eq!(
            std::env::consts::OS,
            "linux",
            "requires a Linux GTK display"
        );
        let context = gtk::glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        let temp = std::env::temp_dir().join(format!("tc-mission-ui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp).unwrap();
        let source = temp.join("proposal.json");
        let original = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../trace-commons-protocol/tests/fixtures/mission-draft.json"),
        )
        .unwrap();
        std::fs::write(&source, &original).unwrap();
        let store = temp.join("inbox");
        let window = adw::ApplicationWindow::builder().build();
        let view = MissionDraftsView::with_store(&window, Some(store.clone()));
        window.set_content(Some(&view.root));
        window.present();
        let settle = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while view.flight.borrow().busy {
                context.iteration(false);
                assert!(
                    std::time::Instant::now() < deadline,
                    "worker did not complete"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        settle();
        assert!(!store.exists(), "empty list must not initialize the inbox");
        *view.selected_file.borrow_mut() = Some(source.clone());
        view.import.set_sensitive(true);
        view.import_selected();
        settle();
        assert_eq!(view.notice.text(), copy("added"));
        assert!(view.rows.first_child().is_some());
        let id = trace_commons_contributor::mission_draft::MissionDraftInbox::at(&store)
            .list()
            .unwrap()
            .remove(0)
            .id;
        view.show(id.clone());
        settle();
        assert!(view.detail.text().contains("https://example.com/paper"));
        assert!(view.detail.text().contains(copy("author_unverified")));
        let retained_detail = view.detail.text();
        let retained_id = view.selected_id.borrow().clone();
        view.show("0".repeat(64));
        settle();
        assert_eq!(view.status.text(), copy("error"));
        assert_eq!(view.detail.text(), retained_detail);
        assert_eq!(*view.selected_id.borrow(), retained_id);

        let declined_close = window.connect_close_request(|_| gtk::glib::Propagation::Stop);
        window.close();
        assert!(view.root.is_mapped());
        assert!(!view.flight.borrow().cancelled);
        window.disconnect(declined_close);
        view.delete(id);
        settle();
        assert_eq!(view.notice.text(), copy("deleted"));
        assert_eq!(view.detail.text(), "");
        assert!(
            trace_commons_contributor::mission_draft::MissionDraftInbox::at(&store)
                .list()
                .unwrap()
                .is_empty()
        );
        assert_eq!(std::fs::read(source).unwrap(), original);
        window.close();
        std::fs::remove_dir_all(temp).unwrap();
    }
}
