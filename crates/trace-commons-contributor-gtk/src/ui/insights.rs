//! Account-free selected-file Insights. No worker, configuration or discovery.
//! A single bounded IO request runs on an OS thread. Cancellation suppresses
//! presentation; already-started writes finish. Weak callbacks never own a view.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
use trace_commons_contributor::insights::card_presentation::question_copy;
use trace_commons_contributor::insights::service::{
    self, LocalInsightsOperation as Op, LocalInsightsRequest, LocalInsightsResponse as Response,
};
use trace_commons_contributor::insights::summary::{
    CoverageUnit, SavedInsightsSummary, SnapshotEvidence, SummaryLimitation,
};
use trace_commons_contributor::insights::{LocalInsight, SourceFormat, TaskCategory, TaskOutcome};
use trace_commons_contributor::insights::{
    episode_store::EpisodeStoreError,
    episodes::{
        EpisodeDetail, EpisodeListEntry, EpisodeOverlap, EpisodeValidationError, LocalEpisode,
    },
};
use trace_commons_protocol::insights_cards::{InsightCardResult, InsightQuestionId};

#[path = "insights_evidence.rs"]
mod evidence;

/// A chooser belongs to the selection and view lifetime that opened it.
#[derive(Clone)]
struct ChooserTicket {
    generation: u64,
    snapshot: Option<String>,
}
impl ChooserTicket {
    fn matches(&self, generation: u64, snapshot: Option<&str>) -> bool {
        self.generation == generation && self.snapshot.as_deref() == snapshot
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EpisodeDraftTicket {
    generation: u64,
    id: Option<String>,
    revision: Option<u64>,
}
impl EpisodeDraftTicket {
    fn matches(&self, generation: u64, episode: Option<(&str, u64)>) -> bool {
        self.generation == generation
            && self.id.as_deref() == episode.map(|(id, _)| id)
            && self.revision == episode.map(|(_, revision)| revision)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EpisodeReadTicket {
    generation: u64,
    requested_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CardRequestTicket {
    generation: u64,
    snapshot_ids: Vec<String>,
    episode_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardPanelState {
    Empty,
    Loading,
    Presented,
    Failed,
}

fn accept_card_completion(
    current_generation: u64,
    snapshot_ids: &[String],
    episode_ids: &[String],
    ticket: &CardRequestTicket,
) -> bool {
    current_generation == ticket.generation
        && snapshot_ids == ticket.snapshot_ids
        && episode_ids == ticket.episode_ids
}
impl EpisodeReadTicket {
    fn accepts(&self, generation: u64, returned_id: Option<&str>) -> bool {
        self.generation == generation && self.requested_id.as_deref() == returned_id
    }
}

fn copy(key: &str) -> &'static str {
    static COPY: std::sync::OnceLock<std::collections::BTreeMap<String, String>> =
        std::sync::OnceLock::new();
    COPY.get_or_init(service::ui_copy)
        .get(key)
        .map(String::as_str)
        .expect("shared Insights copy key")
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

enum EpisodeResult {
    Response(Response),
    Conflict(Vec<EpisodeListEntry>),
    Failed(&'static str),
}

pub struct InsightsView {
    pub root: gtk::Box,
    controls: gtk::Box,
    source: gtk::DropDown,
    selected: RefCell<Option<PathBuf>>,
    selected_label: gtk::Label,
    status: gtk::Label,
    mutation_notice: gtk::Label,
    detail: gtk::Label,
    summary: gtk::Label,
    summary_expander: gtk::Expander,
    scroller: gtk::ScrolledWindow,
    summary_evidence: gtk::Box,
    pending_refresh: Cell<bool>,
    saved: gtk::Box,
    saved_heading: gtk::Label,
    save: gtk::Button,
    assessment: gtk::Box,
    evidence_expander: gtk::Expander,
    evidence_body: gtk::Box,
    evidence_controls: gtk::Box,
    commit: gtk::Entry,
    chooser_generation: Cell<u64>,
    category: gtk::DropDown,
    outcome: gtk::DropDown,
    current_id: RefCell<Option<String>>,
    episode_generation: Cell<u64>,
    current_episode: RefCell<Option<(String, u64)>>,
    current_membership_revision: Cell<Option<u64>>,
    episode_choices: RefCell<Vec<(String, gtk::CheckButton)>>,
    episode_choices_box: gtk::Box,
    episodes: gtk::Box,
    episode_detail: gtk::Label,
    episode_edit: gtk::Box,
    episode_category: gtk::DropDown,
    episode_outcome: gtk::DropDown,
    episode_success_notice: RefCell<Option<String>>,
    card_generation: Cell<u64>,
    card_questions: Vec<(InsightQuestionId, gtk::CheckButton)>,
    card_snapshot_choices: RefCell<Vec<(String, gtk::CheckButton)>>,
    card_snapshot_choices_box: gtk::Box,
    card_episode_choices: RefCell<Vec<(String, gtk::CheckButton)>>,
    card_episode_choices_box: gtk::Box,
    card_status: gtk::Label,
    card_result: gtk::Label,
    card_evidence: gtk::Box,
    card_state: Cell<CardPanelState>,
    flight: RefCell<Flight>,
    store_dir: Option<PathBuf>,
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(100)
        .selectable(true)
        .xalign(0.0)
        .build()
}

impl InsightsView {
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
        root.append(&label(copy("snapshot_notice")));
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let source = gtk::DropDown::from_strings(&[
            copy("codex"),
            copy("claude_code"),
            copy("trajectory"),
        ]);
        source.set_tooltip_text(Some(copy("source")));
        let choose = gtk::Button::with_label(copy("choose_file"));
        let save = gtk::Button::with_label(copy("save"));
        save.set_sensitive(false);
        let refresh = gtk::Button::with_label(copy("refresh"));
        controls.append(&label(copy("source")));
        controls.append(&source);
        controls.append(&choose);
        controls.append(&save);
        controls.append(&refresh);
        root.append(&controls);
        root.append(&label(copy("save_notice")));
        let selected_label = label(copy("no_file"));
        root.append(&selected_label);
        let cancel = gtk::Button::with_label(copy("cancel"));
        root.append(&cancel);
        root.append(&label(copy("cancellation_notice")));
        let status = label(copy("empty"));
        root.append(&status);
        let mutation_notice = label("");
        root.append(&mutation_notice);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let summary_body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let summary = label("");
        summary_body.append(&summary);
        let summary_evidence = gtk::Box::new(gtk::Orientation::Vertical, 4);
        summary_body.append(&summary_evidence);
        let summary_expander = gtk::Expander::builder()
            .label(copy("summary_title"))
            .expanded(true)
            .child(&summary_body)
            .build();
        content.append(&summary_expander);
        let episode_body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        episode_body.append(&label(copy("episode_scope")));
        episode_body.append(&label(copy("episode_overlap_notice")));
        let episode_choices_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        episode_body.append(&label(copy("episode_select_members")));
        episode_body.append(&episode_choices_box);
        let create_episode = gtk::Button::with_label(copy("episode_create"));
        episode_body.append(&create_episode);
        let refresh_episodes = gtk::Button::with_label(copy("refresh"));
        episode_body.append(&refresh_episodes);
        let episodes = gtk::Box::new(gtk::Orientation::Vertical, 6);
        episode_body.append(&episodes);
        let episode_detail = label("");
        episode_body.append(&episode_detail);
        let episode_edit = gtk::Box::new(gtk::Orientation::Vertical, 6);
        episode_edit.append(&label(copy("episode_assessment_notice_short")));
        let episode_category = gtk::DropDown::from_strings(&[
            copy("category_unknown"),
            copy("category_refactor"),
            copy("category_tests"),
            copy("category_docs"),
            copy("category_debugging"),
            copy("category_other"),
        ]);
        let episode_outcome = gtk::DropDown::from_strings(&[
            copy("outcome_unknown"),
            copy("outcome_accepted"),
            copy("outcome_partial"),
            copy("outcome_rejected"),
        ]);
        episode_edit.append(&episode_category);
        episode_edit.append(&episode_outcome);
        let save_episode_assessment = gtk::Button::with_label(copy("episode_save_assessment"));
        let clear_episode_assessment = gtk::Button::with_label(copy("episode_clear_assessment"));
        let save_episode_members = gtk::Button::with_label(copy("episode_save_members"));
        let delete_episode = gtk::Button::with_label(copy("episode_delete"));
        for button in [
            &save_episode_assessment,
            &clear_episode_assessment,
            &save_episode_members,
            &delete_episode,
        ] {
            episode_edit.append(button);
        }
        episode_edit.set_visible(false);
        episode_body.append(&episode_edit);
        let episode_expander = gtk::Expander::builder()
            .label(copy("episode_title"))
            .child(&episode_body)
            .build();
        content.append(&episode_expander);
        let card_body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        card_body.append(&label(copy("card_selection_notice")));
        let card_questions_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let card_questions = InsightQuestionId::ALL
            .into_iter()
            .map(|question| {
                let choice = gtk::CheckButton::with_label(question_copy(question).1);
                choice.set_active(true);
                card_questions_box.append(&choice);
                (question, choice)
            })
            .collect::<Vec<_>>();
        card_body.append(&card_questions_box);
        card_body.append(&label(copy("card_choose_evidence")));
        card_body.append(&label(copy("saved")));
        let card_snapshot_choices_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card_body.append(&card_snapshot_choices_box);
        card_body.append(&label(copy("episode_title")));
        let card_episode_choices_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card_body.append(&card_episode_choices_box);
        let run_cards = gtk::Button::with_label(copy("card_update"));
        card_body.append(&run_cards);
        let card_status = label(copy("card_selection_notice"));
        card_body.append(&card_status);
        let card_result = label("");
        card_body.append(&card_result);
        let card_evidence = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card_body.append(&card_evidence);
        let card_expander = gtk::Expander::builder()
            .label(copy("card_title"))
            .child(&card_body)
            .build();
        content.append(&card_expander);
        let detail = label("");
        content.append(&detail);
        let evidence_body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let evidence_controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let commit = gtk::Entry::new();
        commit.set_placeholder_text(Some(copy("link_commit")));
        evidence_controls.append(&label(copy("link_commit")));
        evidence_controls.append(&commit);
        let link_git = gtk::Button::with_label(copy("link_git"));
        let link_test = gtk::Button::with_label(copy("choose_test_report"));
        evidence_controls.append(&link_git);
        evidence_controls.append(&link_test);
        evidence_controls.append(&label(copy("test_report_format")));
        let evidence_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        evidence_content.append(&evidence_controls);
        evidence_content.append(&evidence_body);
        let evidence_expander = gtk::Expander::builder()
            .label(copy("evidence"))
            .child(&evidence_content)
            .visible(false)
            .build();
        content.append(&evidence_expander);
        let assessment = gtk::Box::new(gtk::Orientation::Vertical, 8);
        assessment.append(&label(copy("assessment_notice")));
        let category = gtk::DropDown::from_strings(&[
            copy("category_unknown"),
            copy("category_refactor"),
            copy("category_tests"),
            copy("category_docs"),
            copy("category_debugging"),
            copy("category_other"),
        ]);
        category.set_tooltip_text(Some(copy("category")));
        let outcome = gtk::DropDown::from_strings(&[
            copy("outcome_unknown"),
            copy("outcome_accepted"),
            copy("outcome_partial"),
            copy("outcome_rejected"),
        ]);
        outcome.set_tooltip_text(Some(copy("outcome")));
        let annotate = gtk::Button::with_label(copy("save_assessment"));
        let clear = gtk::Button::with_label(copy("clear_assessment"));
        assessment.append(&label(copy("category")));
        assessment.append(&category);
        assessment.append(&label(copy("outcome")));
        assessment.append(&outcome);
        assessment.append(&annotate);
        assessment.append(&clear);
        assessment.set_visible(false);
        content.append(&assessment);
        let saved_heading = label(copy("saved"));
        content.append(&saved_heading);
        let saved = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&saved);
        let scroller = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        root.append(&scroller);
        let view = Rc::new(Self {
            root,
            controls,
            source,
            selected: RefCell::new(None),
            selected_label,
            status,
            mutation_notice,
            detail,
            summary,
            summary_expander,
            scroller,
            summary_evidence,
            pending_refresh: Cell::new(false),
            saved,
            saved_heading,
            save: save.clone(),
            assessment,
            evidence_expander,
            evidence_body,
            evidence_controls,
            commit,
            chooser_generation: Cell::new(0),
            category,
            outcome,
            current_id: RefCell::new(None),
            episode_generation: Cell::new(0),
            current_episode: RefCell::new(None),
            current_membership_revision: Cell::new(None),
            episode_choices: RefCell::new(Vec::new()),
            episode_choices_box,
            episodes,
            episode_detail,
            episode_edit,
            episode_category,
            episode_outcome,
            episode_success_notice: RefCell::new(None),
            card_generation: Cell::new(0),
            card_questions,
            card_snapshot_choices: RefCell::new(Vec::new()),
            card_snapshot_choices_box,
            card_episode_choices: RefCell::new(Vec::new()),
            card_episode_choices_box,
            card_status,
            card_result,
            card_evidence,
            card_state: Cell::new(CardPanelState::Empty),
            flight: RefCell::new(Flight::default()),
            store_dir,
        });
        view.rebuild_episode_choices(&[], &view.episode_choices_box);
        view.rebuild_card_snapshot_choices(&[]);
        view.rebuild_card_episode_choices(&[]);
        for (_, choice) in &view.card_questions {
            let weak = Rc::downgrade(&view);
            choice.connect_toggled(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.invalidate_cards();
                }
            });
        }
        let weak = Rc::downgrade(&view);
        run_cards.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.request_cards();
            }
        });
        let weak = Rc::downgrade(&view);
        create_episode.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.create_episode();
            }
        });
        let weak = Rc::downgrade(&view);
        refresh_episodes.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.mutation_notice.set_text("");
                view.episode_success_notice.borrow_mut().take();
                view.invalidate_episode_draft();
                view.invalidate_cards();
                view.refresh_episodes();
            }
        });
        let weak = Rc::downgrade(&view);
        save_episode_assessment.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.annotate_episode();
            }
        });
        let weak = Rc::downgrade(&view);
        let parent = window.downgrade();
        clear_episode_assessment.connect_clicked(move |_| {
            let (Some(view), Some(parent)) = (weak.upgrade(), parent.upgrade()) else {
                return;
            };
            let ticket = view.episode_ticket();
            if ticket.id.is_none() || !view.accepts_episode_ticket(&ticket) {
                return;
            }
            let dialog = adw::MessageDialog::new(
                Some(&parent),
                Some(copy("episode_clear_assessment")),
                Some(copy("episode_clear_assessment_confirm")),
            );
            dialog.add_responses(&[
                ("cancel", copy("cancel")),
                ("confirm", copy("episode_clear_assessment")),
            ]);
            dialog.set_close_response("cancel");
            let weak = Rc::downgrade(&view);
            dialog.connect_response(None, move |dialog, response| {
                dialog.close();
                if response == "confirm" {
                    if let Some(view) = weak.upgrade() {
                        view.clear_episode_assessment_with(ticket.clone());
                    }
                }
            });
            dialog.present();
        });
        let weak = Rc::downgrade(&view);
        save_episode_members.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.replace_episode_members();
            }
        });
        let weak = Rc::downgrade(&view);
        let parent = window.downgrade();
        delete_episode.connect_clicked(move |_| {
            let (Some(view), Some(parent)) = (weak.upgrade(), parent.upgrade()) else {
                return;
            };
            let ticket = view.episode_ticket();
            if ticket.id.is_none() || !view.accepts_episode_ticket(&ticket) {
                return;
            }
            let dialog = adw::MessageDialog::new(
                Some(&parent),
                Some(copy("episode_delete")),
                Some(copy("episode_delete_confirm")),
            );
            dialog.add_responses(&[
                ("cancel", copy("cancel")),
                ("confirm", copy("episode_delete")),
            ]);
            dialog.set_close_response("cancel");
            let weak = Rc::downgrade(&view);
            dialog.connect_response(None, move |dialog, response| {
                dialog.close();
                if response == "confirm" {
                    if let Some(view) = weak.upgrade() {
                        view.delete_episode_with(ticket.clone());
                    }
                }
            });
            dialog.present();
        });
        let weak = Rc::downgrade(&view);
        let parent = window.downgrade();
        choose.connect_clicked(move |_| {
            let (Some(view), Some(parent)) = (weak.upgrade(), parent.upgrade()) else {
                return;
            };
            let chooser = gtk::FileChooserNative::new(
                Some(copy("choose_file")),
                Some(&parent),
                gtk::FileChooserAction::Open,
                Some(copy("analyze")),
                Some(copy("cancel")),
            );
            let ticket = view.chooser_ticket();
            let weak = Rc::downgrade(&view);
            chooser.connect_response(move |dialog, response| {
                if response == gtk::ResponseType::Accept {
                    if let Some(view) = weak.upgrade() {
                        if !view.accepts_chooser(&ticket) {
                            dialog.destroy();
                            return;
                        }
                        if let Some(path) = dialog.file().and_then(|f| f.path()) {
                            view.selected_label
                                .set_text(&path.file_name().unwrap_or_default().to_string_lossy());
                            *view.selected.borrow_mut() = Some(path);
                            view.analyze(false);
                        }
                    }
                }
                dialog.destroy();
            });
            chooser.show();
        });
        for (button, git) in [(link_git, true), (link_test, false)] {
            let weak = Rc::downgrade(&view);
            let parent = window.downgrade();
            button.connect_clicked(move |_| {
                if let (Some(view), Some(parent)) = (weak.upgrade(), parent.upgrade()) {
                    view.choose_evidence(&parent, git);
                }
            });
        }
        let weak = Rc::downgrade(&view);
        save.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.analyze(true);
            }
        });
        let weak = Rc::downgrade(&view);
        refresh.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.mutation_notice.set_text("");
                v.episode_success_notice.borrow_mut().take();
                v.summary_expander.set_expanded(true);
                v.refresh_history();
            }
        });
        let weak = Rc::downgrade(&view);
        cancel.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.flight.borrow_mut().cancelled = true;
                v.mutation_notice.set_text("");
                v.episode_success_notice.borrow_mut().take();
                v.invalidate_choosers();
                v.invalidate_episode_draft();
                v.status.set_text(copy("cancelled"));
            }
        });
        let weak = Rc::downgrade(&view);
        annotate.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                let id = v.current_id.borrow().clone();
                if let Some(id) = id {
                    v.request(
                        Op::Annotate {
                            id,
                            category: category_at(v.category.selected()),
                            outcome: outcome_at(v.outcome.selected()),
                        },
                        true,
                    );
                }
            }
        });
        let weak = Rc::downgrade(&view);
        clear.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                let id = v.current_id.borrow().clone();
                if let Some(id) = id {
                    v.request(Op::ClearAnnotation { id }, true);
                }
            }
        });
        let weak = Rc::downgrade(&view);
        if let Some(application) = window.application() {
            let weak = Rc::downgrade(&view);
            application.connect_shutdown(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.flight.borrow_mut().closed = true;
                }
            });
        }
        // GtkWindow::close can destroy its native surface without emitting
        // hide/destroy before a queued local completion. Invalidate now, but
        // do not mark the view terminal: another close handler may ask the
        // user to confirm quitting and decline it.
        window.connect_close_request(move |_| {
            if let Some(view) = weak.upgrade() {
                view.flight.borrow_mut().cancelled = true;
                view.mutation_notice.set_text("");
                view.episode_success_notice.borrow_mut().take();
                view.invalidate_choosers();
                view.invalidate_episode_draft();
                view.invalidate_cards();
            }
            gtk::glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&view);
        window.connect_hide(move |_| {
            if let Some(view) = weak.upgrade() {
                view.flight.borrow_mut().cancelled = true;
                view.mutation_notice.set_text("");
                view.episode_success_notice.borrow_mut().take();
                view.invalidate_choosers();
                view.invalidate_episode_draft();
                view.invalidate_cards();
            }
        });
        let weak = Rc::downgrade(&view);
        window.connect_show(move |_| {
            if let Some(view) = weak.upgrade() {
                if !view.flight.borrow().busy && !view.flight.borrow().closed {
                    view.controls.set_sensitive(true);
                    view.assessment.set_sensitive(true);
                    view.saved.set_sensitive(true);
                    view.evidence_expander.set_sensitive(true);
                    view.episodes.set_sensitive(true);
                    view.episode_edit.set_sensitive(true);
                }
            }
        });
        let weak = Rc::downgrade(&view);
        view.root.connect_unmap(move |_| {
            if let Some(view) = weak.upgrade() {
                view.invalidate_choosers();
                view.invalidate_episode_draft();
                view.invalidate_cards();
                view.flight.borrow_mut().cancelled = true;
            }
        });
        let weak = Rc::downgrade(&view);
        view.root.connect_map(move |_| {
            if let Some(view) = weak.upgrade() {
                view.mutation_notice.set_text("");
                view.episode_success_notice.borrow_mut().take();
                view.invalidate_episode_draft();
                if !view.flight.borrow().busy && !view.flight.borrow().closed {
                    view.controls.set_sensitive(true);
                    view.assessment.set_sensitive(true);
                    view.saved.set_sensitive(true);
                    view.evidence_expander.set_sensitive(true);
                    view.episodes.set_sensitive(true);
                    view.episode_edit.set_sensitive(true);
                }
                view.summary_expander.set_expanded(true);
                view.refresh_history();
            }
        });
        // The window retains the controller without a view/root reference cycle.
        // Worker completions retain only a Weak reference.
        let retained = view.clone();
        window.connect_destroy(move |_| {
            retained.flight.borrow_mut().closed = true;
        });
        view
    }

    fn analyze(self: &Rc<Self>, save: bool) {
        self.episode_success_notice.borrow_mut().take();
        let file = self.selected.borrow().clone();
        if let Some(file) = file {
            if self.flight.borrow().busy || self.flight.borrow().closed {
                return;
            }
            self.save.set_sensitive(false);
            self.clear_detail();
            self.request(
                Op::Analyze {
                    source: match self.source.selected() {
                        0 => SourceFormat::Codex,
                        1 => SourceFormat::ClaudeCode,
                        _ => SourceFormat::Trajectory,
                    },
                    file,
                    save,
                },
                save,
            );
        }
    }

    fn clear_detail(&self) {
        self.mutation_notice.set_text("");
        self.invalidate_choosers();
        self.detail.set_text("");
        self.assessment.set_visible(false);
        self.evidence_expander.set_visible(false);
        self.evidence_expander.set_expanded(false);
        self.commit.set_text("");
        while let Some(child) = self.evidence_body.first_child() {
            self.evidence_body.remove(&child);
        }
        *self.current_id.borrow_mut() = None;
    }

    fn invalidate_choosers(&self) {
        self.chooser_generation
            .set(self.chooser_generation.get().wrapping_add(1));
    }

    fn invalidate_episode_draft(&self) {
        self.episode_generation
            .set(self.episode_generation.get().wrapping_add(1));
        *self.current_episode.borrow_mut() = None;
        self.current_membership_revision.set(None);
        self.episode_detail.set_text("");
        self.episode_edit.set_visible(false);
    }

    fn invalidate_cards(&self) {
        self.card_generation
            .set(self.card_generation.get().wrapping_add(1));
        self.card_result.set_text("");
        while let Some(child) = self.card_evidence.first_child() {
            self.card_evidence.remove(&child);
        }
        self.card_state.set(CardPanelState::Empty);
        self.card_status.set_text(copy("card_selection_notice"));
    }

    fn selected_card_ids(values: &RefCell<Vec<(String, gtk::CheckButton)>>) -> Vec<String> {
        values
            .borrow()
            .iter()
            .filter(|(_, choice)| choice.is_active())
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn rebuild_card_snapshot_choices(self: &Rc<Self>, ids: &[String]) {
        let selected: std::collections::BTreeSet<_> =
            Self::selected_card_ids(&self.card_snapshot_choices)
                .into_iter()
                .collect();
        while let Some(child) = self.card_snapshot_choices_box.first_child() {
            self.card_snapshot_choices_box.remove(&child);
        }
        let mut choices = Vec::new();
        for id in ids {
            let choice = gtk::CheckButton::with_label(id);
            choice.set_active(selected.contains(id));
            choice.set_tooltip_text(Some(copy("summary_open_snapshot")));
            let weak = Rc::downgrade(self);
            choice.connect_toggled(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.invalidate_cards();
                }
            });
            self.card_snapshot_choices_box.append(&choice);
            choices.push((id.clone(), choice));
        }
        *self.card_snapshot_choices.borrow_mut() = choices;
    }

    fn rebuild_card_episode_choices(self: &Rc<Self>, ids: &[String]) {
        let selected: std::collections::BTreeSet<_> =
            Self::selected_card_ids(&self.card_episode_choices)
                .into_iter()
                .collect();
        while let Some(child) = self.card_episode_choices_box.first_child() {
            self.card_episode_choices_box.remove(&child);
        }
        let mut choices = Vec::new();
        for id in ids {
            let choice = gtk::CheckButton::with_label(id);
            choice.set_active(selected.contains(id));
            choice.set_tooltip_text(Some(copy("episode_id")));
            let weak = Rc::downgrade(self);
            choice.connect_toggled(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.invalidate_cards();
                }
            });
            self.card_episode_choices_box.append(&choice);
            choices.push((id.clone(), choice));
        }
        *self.card_episode_choices.borrow_mut() = choices;
    }

    fn request_cards(self: &Rc<Self>) {
        let questions = self
            .card_questions
            .iter()
            .filter(|(_, choice)| choice.is_active())
            .map(|(question, _)| *question)
            .collect::<Vec<_>>();
        let snapshot_ids = Self::selected_card_ids(&self.card_snapshot_choices);
        let episode_ids = Self::selected_card_ids(&self.card_episode_choices);
        if questions.is_empty() {
            self.card_state.set(CardPanelState::Failed);
            self.card_status.set_text(copy("card_selection_notice"));
            return;
        }
        if !self.flight.borrow_mut().begin() {
            return;
        }
        let ticket = CardRequestTicket {
            generation: self.card_generation.get(),
            snapshot_ids: snapshot_ids.clone(),
            episode_ids: episode_ids.clone(),
        };
        self.card_state.set(CardPanelState::Loading);
        self.card_status.set_text(copy("working"));
        self.card_result.set_text("");
        let (tx, rx) = async_channel::bounded(1);
        let store_dir = self.store_dir.clone();
        std::thread::spawn(move || {
            let response = std::panic::catch_unwind(|| {
                service::execute(LocalInsightsRequest {
                    store_dir,
                    operation: Op::QuestionCards {
                        questions,
                        snapshot_ids,
                        episode_ids,
                    },
                })
            })
            .ok()
            .and_then(Result::ok);
            let _ = tx.send_blocking(response);
        });
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            let response = rx.recv().await.ok().flatten();
            let Some(view) = weak.upgrade() else { return };
            if !view.flight.borrow_mut().finish()
                || !accept_card_completion(
                    view.card_generation.get(),
                    &Self::selected_card_ids(&view.card_snapshot_choices),
                    &Self::selected_card_ids(&view.card_episode_choices),
                    &ticket,
                )
            {
                return;
            }
            match response {
                Some(Response::QuestionCards { result, text }) => {
                    view.present_cards(&result, &text);
                    view.card_state.set(CardPanelState::Presented);
                    view.card_status.set_text(copy("refreshed"));
                }
                _ => {
                    view.card_state.set(CardPanelState::Failed);
                    view.card_result.set_text("");
                    view.card_status.set_text(copy("summary_unavailable"));
                }
            }
        });
    }

    fn present_cards(self: &Rc<Self>, result: &InsightCardResult, text: &str) {
        self.card_result.set_text(text);
        while let Some(child) = self.card_evidence.first_child() {
            self.card_evidence.remove(&child);
        }
        let evidence_ids = result
            .cards
            .iter()
            .flat_map(|card| card.evidence_ids.iter())
            .collect::<std::collections::BTreeSet<_>>();
        for id in evidence_ids {
            let button =
                gtk::Button::with_label(&format!("{}: {id}", copy("summary_open_snapshot")));
            let weak = Rc::downgrade(self);
            let id = id.clone();
            button.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.request(Op::Explain { id: id.clone() }, true);
                }
            });
            self.card_evidence.append(&button);
        }
        let episode_ids = result
            .cards
            .iter()
            .flat_map(|card| card.episode_ids.iter())
            .collect::<std::collections::BTreeSet<_>>();
        for id in episode_ids {
            let button = gtk::Button::with_label(&format!("{}: {id}", copy("episode_open")));
            let weak = Rc::downgrade(self);
            let id = id.clone();
            button.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.open_episode(id.clone());
                }
            });
            self.card_evidence.append(&button);
        }
    }

    fn episode_ticket(&self) -> EpisodeDraftTicket {
        let episode = self.current_episode.borrow();
        EpisodeDraftTicket {
            generation: self.episode_generation.get(),
            id: episode.as_ref().map(|(id, _)| id.clone()),
            revision: episode.as_ref().map(|(_, revision)| *revision),
        }
    }

    fn accepts_episode_ticket(&self, ticket: &EpisodeDraftTicket) -> bool {
        let episode = self.current_episode.borrow();
        ticket.matches(
            self.episode_generation.get(),
            episode
                .as_ref()
                .map(|(id, revision)| (id.as_str(), *revision)),
        ) && !self.flight.borrow().closed
    }

    fn selected_episode_members(&self) -> Vec<String> {
        self.episode_choices
            .borrow()
            .iter()
            .filter(|(_, choice)| choice.is_active())
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn rebuild_episode_choices(&self, ids: &[String], container: &gtk::Box) {
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }
        let selected: std::collections::BTreeSet<_> = self
            .episode_choices
            .borrow()
            .iter()
            .filter(|(_, choice)| choice.is_active())
            .map(|(id, _)| id.clone())
            .collect();
        let mut choices = Vec::new();
        for id in ids {
            let choice = gtk::CheckButton::with_label(id);
            choice.set_active(selected.contains(id));
            choice.set_tooltip_text(Some(copy("episode_member_id")));
            container.append(&choice);
            choices.push((id.clone(), choice));
        }
        *self.episode_choices.borrow_mut() = choices;
    }

    fn select_episode_members(&self, members: &[String]) {
        let members: std::collections::BTreeSet<_> = members.iter().collect();
        for (id, choice) in self.episode_choices.borrow().iter() {
            choice.set_active(members.contains(id));
        }
    }

    fn create_episode(self: &Rc<Self>) {
        let snapshot_ids = self.selected_episode_members();
        if snapshot_ids.is_empty() {
            self.status.set_text(copy("episode_selection_empty"));
            return;
        }
        self.request_episode(Op::EpisodeCreate { snapshot_ids }, None);
    }

    fn refresh_episodes(self: &Rc<Self>) {
        self.request_episode(Op::EpisodeList {}, None);
    }

    fn open_episode(self: &Rc<Self>, id: String) {
        self.mutation_notice.set_text("");
        self.episode_success_notice.borrow_mut().take();
        self.invalidate_episode_draft();
        self.request_episode(Op::EpisodeExplain { id }, None);
    }

    fn replace_episode_members(self: &Rc<Self>) {
        let ticket = self.episode_ticket();
        if !self.accepts_episode_ticket(&ticket) {
            return;
        }
        let (Some(id), Some(expected_revision)) = (ticket.id.clone(), ticket.revision) else {
            return;
        };
        let snapshot_ids = self.selected_episode_members();
        if snapshot_ids.is_empty() {
            self.status.set_text(copy("episode_selection_empty"));
            return;
        }
        self.request_episode(
            Op::EpisodeReplaceMembers {
                id,
                expected_revision,
                snapshot_ids,
            },
            Some(ticket),
        );
    }

    fn annotate_episode(self: &Rc<Self>) {
        let ticket = self.episode_ticket();
        let (Some(id), Some(expected_revision)) = (ticket.id.clone(), ticket.revision) else {
            return;
        };
        self.request_episode(
            Op::EpisodeAnnotate {
                id,
                expected_revision,
                category: category_at(self.episode_category.selected()),
                outcome: outcome_at(self.episode_outcome.selected()),
            },
            Some(ticket),
        );
    }

    fn clear_episode_assessment_with(self: &Rc<Self>, ticket: EpisodeDraftTicket) {
        if !self.accepts_episode_ticket(&ticket) {
            return;
        }
        let (Some(id), Some(expected_revision)) = (ticket.id.clone(), ticket.revision) else {
            return;
        };
        self.request_episode(
            Op::EpisodeClearAssessment {
                id,
                expected_revision,
            },
            Some(ticket),
        );
    }

    fn delete_episode_with(self: &Rc<Self>, ticket: EpisodeDraftTicket) {
        if !self.accepts_episode_ticket(&ticket) {
            return;
        }
        let (Some(id), Some(expected_revision)) = (ticket.id.clone(), ticket.revision) else {
            return;
        };
        self.request_episode(
            Op::EpisodeDelete {
                id,
                expected_revision,
            },
            Some(ticket),
        );
    }

    fn request_episode(self: &Rc<Self>, operation: Op, ticket: Option<EpisodeDraftTicket>) {
        if !self.flight.borrow_mut().begin() {
            return;
        }
        let mutation = matches!(
            &operation,
            Op::EpisodeCreate { .. }
                | Op::EpisodeReplaceMembers { .. }
                | Op::EpisodeAnnotate { .. }
                | Op::EpisodeClearAssessment { .. }
                | Op::EpisodeDelete { .. }
        );
        if mutation {
            self.invalidate_cards();
        }
        let read_ticket = (!mutation).then(|| EpisodeReadTicket {
            generation: self.episode_generation.get(),
            requested_id: match &operation {
                Op::EpisodeExplain { id } => Some(id.clone()),
                _ => None,
            },
        });
        if mutation {
            self.mutation_notice.set_text("");
            self.episode_success_notice.borrow_mut().take();
        }
        self.controls.set_sensitive(false);
        self.assessment.set_sensitive(false);
        self.saved.set_sensitive(false);
        self.summary_evidence.set_sensitive(false);
        self.evidence_expander.set_sensitive(false);
        self.episodes.set_sensitive(false);
        self.episode_edit.set_sensitive(false);
        if self.episode_success_notice.borrow().is_none() {
            self.status.set_text(copy("working"));
        }
        let (tx, rx) = async_channel::bounded(1);
        let store_dir = self.store_dir.clone();
        std::thread::spawn(move || {
            let request = LocalInsightsRequest {
                store_dir: store_dir.clone(),
                operation,
            };
            let result = match std::panic::catch_unwind(|| service::execute(request)) {
                Ok(Ok(response)) => EpisodeResult::Response(response),
                Ok(Err(error))
                    if error.downcast_ref::<EpisodeStoreError>()
                        == Some(&EpisodeStoreError::RevisionConflict) =>
                {
                    let refreshed = service::execute(LocalInsightsRequest {
                        store_dir,
                        operation: Op::EpisodeList {},
                    });
                    match refreshed {
                        Ok(Response::EpisodeList { episodes }) => EpisodeResult::Conflict(episodes),
                        _ => EpisodeResult::Failed("episode_list_unavailable"),
                    }
                }
                Ok(Err(error)) => EpisodeResult::Failed(episode_error_copy_key(&error)),
                _ => EpisodeResult::Failed("episode_detail_unavailable"),
            };
            let _ = tx.send_blocking(result);
        });
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            let Ok(result) = rx.recv().await else {
                return;
            };
            let Some(view) = weak.upgrade() else {
                return;
            };
            if !view.flight.borrow_mut().finish() {
                return;
            }
            view.controls.set_sensitive(true);
            view.assessment.set_sensitive(true);
            view.saved.set_sensitive(true);
            view.summary_evidence.set_sensitive(true);
            view.evidence_expander.set_sensitive(true);
            view.episodes.set_sensitive(true);
            view.episode_edit.set_sensitive(true);
            if ticket
                .as_ref()
                .is_some_and(|ticket| !view.accepts_episode_ticket(ticket))
            {
                if mutation {
                    view.refresh_episodes();
                }
                return;
            }
            if let Some(read_ticket) = &read_ticket {
                let returned_id = match &result {
                    EpisodeResult::Response(Response::EpisodeExplain { detail }) => {
                        Some(detail.episode.id.as_str())
                    }
                    EpisodeResult::Response(Response::EpisodeList { .. }) => None,
                    _ => read_ticket.requested_id.as_deref(),
                };
                if !read_ticket.accepts(view.episode_generation.get(), returned_id) {
                    return;
                }
            }
            match result {
                EpisodeResult::Response(Response::EpisodeList { episodes }) => {
                    view.render_episode_list(&episodes);
                    view.render_episode_status(copy("refreshed"));
                    if let Some((id, _)) = view.current_episode.borrow().clone() {
                        view.request_episode(Op::EpisodeExplain { id }, None);
                    }
                }
                EpisodeResult::Response(Response::EpisodeExplain { detail }) => {
                    view.render_episode_detail(&detail);
                    view.render_episode_status(copy("refreshed"));
                }
                EpisodeResult::Response(Response::EpisodeDelete { .. }) => {
                    view.invalidate_episode_draft();
                    view.set_episode_success(copy("episode_deleted"));
                    view.refresh_episodes();
                }
                EpisodeResult::Response(Response::EpisodeCreate { episode }) => {
                    view.present_episode(&episode);
                    view.set_episode_success(copy("episode_create_success"));
                    view.refresh_episodes();
                }
                EpisodeResult::Response(Response::EpisodeReplaceMembers { episode, .. }) => {
                    let membership_changed =
                        view.current_membership_revision.get() != Some(episode.membership_revision);
                    view.present_episode(&episode);
                    if membership_changed {
                        view.set_episode_success(&format!(
                            "{}\n{}",
                            copy("episode_members_saved"),
                            copy("episode_membership_changed")
                        ));
                    } else {
                        view.set_episode_success(copy("episode_members_saved"));
                    }
                    view.refresh_episodes();
                }
                EpisodeResult::Response(Response::EpisodeAnnotate { episode, .. }) => {
                    view.present_episode(&episode);
                    view.set_episode_success(copy("episode_assessment_saved"));
                    view.refresh_episodes();
                }
                EpisodeResult::Response(Response::EpisodeClearAssessment { episode, .. }) => {
                    view.present_episode(&episode);
                    view.set_episode_success(copy("episode_assessment_cleared"));
                    view.refresh_episodes();
                }
                EpisodeResult::Conflict(episodes) => {
                    view.invalidate_episode_draft();
                    view.episode_success_notice.borrow_mut().take();
                    view.render_episode_list(&episodes);
                    view.status.set_text(copy("episode_revision_conflict"));
                }
                EpisodeResult::Failed(key) => {
                    view.invalidate_episode_draft();
                    let error = copy(key);
                    let status = episode_failure_status(
                        view.episode_success_notice.borrow().as_deref(),
                        error,
                    );
                    view.status.set_text(&status);
                }
                _ => {
                    view.invalidate_episode_draft();
                    let error = copy("episode_detail_unavailable");
                    let status = episode_failure_status(
                        view.episode_success_notice.borrow().as_deref(),
                        error,
                    );
                    view.status.set_text(&status);
                }
            }
        });
    }

    fn set_episode_success(&self, notice: &str) {
        *self.episode_success_notice.borrow_mut() = Some(notice.to_owned());
        self.status.set_text(notice);
    }

    fn render_episode_status(&self, fallback: &str) {
        self.status.set_text(
            self.episode_success_notice
                .borrow()
                .as_deref()
                .unwrap_or(fallback),
        );
    }

    fn render_episode_list(self: &Rc<Self>, entries: &[EpisodeListEntry]) {
        while let Some(child) = self.episodes.first_child() {
            self.episodes.remove(&child);
        }
        self.episodes.append(&label(&format!(
            "{}: {}",
            copy("episode_count"),
            entries.len()
        )));
        self.rebuild_card_episode_choices(
            &entries
                .iter()
                .map(|entry| entry.episode.id.clone())
                .collect::<Vec<_>>(),
        );
        if entries.is_empty() {
            self.episodes.append(&label(copy("episode_empty")));
            return;
        }
        for entry in entries {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 3);
            row.append(&label(&format!(
                "{}: {}\n{}: {}\n{}: {}",
                copy("episode_id"),
                entry.episode.id,
                copy("episode_members"),
                entry.episode.members.len(),
                copy("episode_overlaps"),
                entry.overlapping_episode_ids.len(),
            )));
            let open = gtk::Button::with_label(copy("episode_open"));
            let weak = Rc::downgrade(self);
            let id = entry.episode.id.clone();
            open.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.open_episode(id.clone());
                }
            });
            row.append(&open);
            self.episodes.append(&row);
        }
    }

    fn present_episode(&self, episode: &LocalEpisode) {
        self.episode_generation
            .set(self.episode_generation.get().wrapping_add(1));
        *self.current_episode.borrow_mut() = Some((episode.id.clone(), episode.revision));
        self.current_membership_revision
            .set(Some(episode.membership_revision));
        let members = episode
            .members
            .iter()
            .map(|member| member.snapshot_id.clone())
            .collect::<Vec<_>>();
        self.select_episode_members(&members);
        self.episode_detail.set_text(&render_episode(episode, &[]));
        self.episode_edit.set_visible(true);
        if let Some(assessment) = &episode.manual_assessment {
            self.episode_category
                .set_selected(category_index(assessment.category));
            self.episode_outcome
                .set_selected(outcome_index(assessment.outcome));
        } else {
            self.episode_category.set_selected(0);
            self.episode_outcome.set_selected(0);
        }
    }

    fn render_episode_detail(&self, detail: &EpisodeDetail) {
        self.present_episode(&detail.episode);
        let mut text = render_episode(&detail.episode, &detail.overlap);
        text.push_str(&format!("\n{}\n", copy("episode_member_evidence")));
        for member in &detail.members {
            text.push_str(&render(member));
        }
        self.episode_detail.set_text(&text);
    }

    fn chooser_ticket(&self) -> ChooserTicket {
        ChooserTicket {
            generation: self.chooser_generation.get(),
            snapshot: self.current_id.borrow().clone(),
        }
    }

    fn accepts_chooser(&self, ticket: &ChooserTicket) -> bool {
        let flight = self.flight.borrow();
        !flight.busy
            && !flight.closed
            && self.root.is_mapped()
            && ticket.matches(
                self.chooser_generation.get(),
                self.current_id.borrow().as_deref(),
            )
    }

    fn choose_evidence(self: &Rc<Self>, parent: &adw::ApplicationWindow, git: bool) {
        let ticket = self.chooser_ticket();
        if ticket.snapshot.is_none() || !self.accepts_chooser(&ticket) {
            return;
        }
        // Capture before opening the picker; later edits cannot change this request.
        let commit = self.commit.text().to_string();
        let chooser = gtk::FileChooserNative::new(
            Some(copy(if git {
                "choose_repository"
            } else {
                "choose_test_report"
            })),
            Some(parent),
            if git {
                gtk::FileChooserAction::SelectFolder
            } else {
                gtk::FileChooserAction::Open
            },
            Some(copy(if git { "link_git" } else { "link_test_report" })),
            Some(copy("cancel")),
        );
        let weak = Rc::downgrade(self);
        chooser.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                if let (Some(view), Some(path)) =
                    (weak.upgrade(), dialog.file().and_then(|f| f.path()))
                {
                    view.link_chosen_evidence(
                        &ticket,
                        path,
                        if git { Some(commit.clone()) } else { None },
                    );
                }
            }
            dialog.destroy();
        });
        chooser.show();
    }

    fn link_chosen_evidence(
        self: &Rc<Self>,
        ticket: &ChooserTicket,
        path: PathBuf,
        commit: Option<String>,
    ) {
        if !self.accepts_chooser(ticket) {
            return;
        }
        let Some(id) = ticket.snapshot.clone() else {
            return;
        };
        let operation = match commit {
            Some(commit) => Op::LinkGit {
                id,
                repository: path,
                commit,
            },
            None => Op::LinkTestReport { id, file: path },
        };
        self.request(operation, true);
    }

    fn render_evidence(self: &Rc<Self>, insight: &LocalInsight, persisted: bool) {
        while let Some(child) = self.evidence_body.first_child() {
            self.evidence_body.remove(&child);
        }
        self.evidence_body.append(&label(&evidence::render_models(
            insight.model_observations.as_ref(),
        )));
        self.evidence_body
            .append(&label(copy("linked_evidence_title")));
        self.evidence_body.append(&label(copy("link_notice")));
        self.evidence_body.append(&label(copy("link_git_notice")));
        self.evidence_body.append(&label(copy("link_test_notice")));
        if !persisted {
            self.evidence_body
                .append(&label(copy("link_saved_required")));
        }
        if insight.outcome_links.is_empty() {
            self.evidence_body.append(&label(copy("link_empty")));
        }
        for link in &insight.outcome_links {
            self.evidence_body
                .append(&label(&evidence::render_link(link)));
            if persisted {
                let unlink = gtk::Button::with_label(copy("unlink_evidence"));
                let weak = Rc::downgrade(self);
                let id = insight.id.clone();
                let evidence_id = link.id.clone();
                unlink.connect_clicked(move |_| {
                    if let Some(view) = weak.upgrade() {
                        if view.current_id.borrow().as_deref() == Some(id.as_str()) {
                            view.request(
                                Op::UnlinkEvidence {
                                    id: id.clone(),
                                    evidence_id: evidence_id.clone(),
                                },
                                true,
                            );
                        }
                    }
                });
                self.evidence_body.append(&unlink);
            }
        }
        self.evidence_controls.set_visible(persisted);
        self.evidence_expander.set_visible(true);
    }

    fn reveal_evidence(&self) {
        self.summary_expander.set_expanded(false);
        let adjustment = self.scroller.vadjustment();
        adjustment.set_value(adjustment.lower());
    }

    fn explain(self: &Rc<Self>, id: String) {
        if self.flight.borrow().busy || self.flight.borrow().closed {
            return;
        }
        self.episode_success_notice.borrow_mut().take();
        // An explicit lookup replaces the previous selection immediately,
        // including an unsaved preview. Failed/deleted evidence cannot leave
        // that earlier successful result looking like the requested snapshot.
        self.clear_detail();
        self.reveal_evidence();
        self.request(Op::Explain { id }, true);
    }

    fn refresh_history(self: &Rc<Self>) {
        if self.flight.borrow().closed {
            return;
        }
        if self.flight.borrow().busy {
            self.pending_refresh.set(true);
        } else {
            self.pending_refresh.set(false);
            self.request(Op::Summary {}, false);
        }
    }

    fn clear_summary(&self) {
        self.summary.set_text("");
        while let Some(child) = self.summary_evidence.first_child() {
            self.summary_evidence.remove(&child);
        }
    }

    fn request(self: &Rc<Self>, operation: Op, persisted: bool) {
        if !self.flight.borrow_mut().begin() {
            return;
        }
        self.invalidate_choosers();
        // Summary/Explain here include automatic reconciliation after a write.
        // Explicit selection/refresh clears the notice at its user entry point.
        let retains_committed_notice = matches!(
            &operation,
            Op::Summary {} | Op::List {} | Op::Explain { .. }
        );
        if !retains_committed_notice {
            self.mutation_notice.set_text("");
        }
        let evidence_mutation = matches!(
            &operation,
            Op::LinkGit { .. } | Op::LinkTestReport { .. } | Op::UnlinkEvidence { .. }
        );
        if evidence_mutation {
            self.clear_detail();
        }
        self.controls.set_sensitive(false);
        self.assessment.set_sensitive(false);
        self.saved.set_sensitive(false);
        self.summary_evidence.set_sensitive(false);
        self.evidence_expander.set_sensitive(false);
        self.episodes.set_sensitive(false);
        self.episode_edit.set_sensitive(false);
        self.status.set_text(copy("working"));
        let refresh_saved = matches!(
            &operation,
            Op::Analyze { save: true, .. }
                | Op::Annotate { .. }
                | Op::ClearAnnotation { .. }
                | Op::LinkGit { .. }
                | Op::LinkTestReport { .. }
                | Op::UnlinkEvidence { .. }
        );
        let history_read = matches!(&operation, Op::Summary {} | Op::List {});
        let mutates_history = refresh_saved || matches!(&operation, Op::Delete { .. });
        if mutates_history {
            self.invalidate_cards();
        }
        if history_read || mutates_history {
            self.clear_summary();
        }
        let (tx, rx) = async_channel::bounded(1);
        let store_dir = self.store_dir.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                service::execute(LocalInsightsRequest {
                    store_dir,
                    operation,
                })
            })
            .ok()
            .and_then(Result::ok);
            let _ = tx.send_blocking(result);
        });
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            let result = rx.recv().await.ok().flatten();
            let Some(view) = weak.upgrade() else {
                return;
            };
            if !view.flight.borrow_mut().finish() {
                if !view.flight.borrow().closed && view.root.is_mapped() {
                    view.controls.set_sensitive(true);
                    view.assessment.set_sensitive(true);
                    view.saved.set_sensitive(true);
                    view.summary_evidence.set_sensitive(true);
                    view.evidence_expander.set_sensitive(true);
                    view.episodes.set_sensitive(true);
                    view.episode_edit.set_sensitive(true);
                    if view.pending_refresh.get() {
                        view.refresh_history();
                    }
                }
                return;
            }
            view.controls.set_sensitive(true);
            view.assessment.set_sensitive(true);
            view.saved.set_sensitive(true);
            view.summary_evidence.set_sensitive(true);
            view.evidence_expander.set_sensitive(true);
            view.episodes.set_sensitive(true);
            view.episode_edit.set_sensitive(true);
            let mutation_notice = match &result {
                Some(Response::Analyze {
                    mutation_effects, ..
                })
                | Some(Response::Delete {
                    mutation_effects, ..
                }) => Some(render_mutation_notice(mutation_effects)),
                _ => None,
            };
            let invalidates_current_episode = match &result {
                Some(Response::Analyze {
                    mutation_effects, ..
                })
                | Some(Response::Delete {
                    mutation_effects, ..
                }) => view
                    .current_episode
                    .borrow()
                    .as_ref()
                    .is_some_and(|(id, _)| mutation_effects.invalidated_episode_ids.contains(id)),
                _ => false,
            };
            if invalidates_current_episode {
                view.invalidate_episode_draft();
            }
            match result {
                Some(Response::Analyze { insight, .. })
                | Some(Response::Explain { insight })
                | Some(Response::Annotate { insight })
                | Some(Response::ClearAnnotation { insight })
                | Some(Response::LinkGit { insight })
                | Some(Response::LinkTestReport { insight })
                | Some(Response::UnlinkEvidence { insight }) => {
                    view.detail.set_text(&render(&insight));
                    *view.current_id.borrow_mut() = if persisted {
                        Some(insight.id.clone())
                    } else {
                        None
                    };
                    view.assessment.set_visible(persisted);
                    view.render_evidence(&insight, persisted);
                    if evidence_mutation {
                        view.evidence_expander.set_expanded(true);
                    }
                    if let Some(a) = &insight.manual_annotation {
                        view.category.set_selected(category_index(a.category));
                        view.outcome.set_selected(outcome_index(a.outcome));
                    } else {
                        view.category.set_selected(0);
                        view.outcome.set_selected(0);
                    }
                    view.save.set_sensitive(view.selected.borrow().is_some());
                    view.status.set_text(if persisted {
                        copy("saved_status")
                    } else {
                        copy("unsaved")
                    });
                    if refresh_saved {
                        view.refresh_history();
                    } else {
                        view.refresh_episodes();
                    }
                }
                Some(Response::Summary { summary }) => {
                    view.render_summary(summary);
                    view.status.set_text(copy("refreshed"));
                    let selected = view.current_id.borrow().clone();
                    if let Some(id) = selected {
                        view.request(Op::Explain { id }, true);
                    } else {
                        view.refresh_episodes();
                    }
                }
                Some(Response::List { .. }) => view.refresh_history(),
                Some(Response::Delete { deleted, .. }) => {
                    view.clear_detail();
                    view.status.set_text(if deleted {
                        copy("deleted")
                    } else {
                        copy("already_absent")
                    });
                    view.refresh_history();
                }
                _ => {
                    let committed_notice = view.mutation_notice.text();
                    view.clear_summary();
                    view.summary.set_text(copy("summary_unavailable"));
                    while let Some(child) = view.saved.first_child() {
                        view.saved.remove(&child);
                    }
                    if view.current_id.borrow().is_some() {
                        view.clear_detail();
                    }
                    view.mutation_notice.set_text(if retains_committed_notice {
                        &committed_notice
                    } else {
                        ""
                    });
                    view.status.set_text(copy("error"));
                }
            }
            if let Some(notice) = mutation_notice {
                view.mutation_notice.set_text(&notice);
            }
            if view.pending_refresh.get() && !view.flight.borrow().busy && view.root.is_mapped() {
                view.refresh_history();
            }
        });
    }

    fn render_summary(self: &Rc<Self>, summary: Box<SavedInsightsSummary>) {
        self.summary.set_text(&render_summary_text(&summary));
        self.saved_heading.set_text(copy("saved"));
        let summary = Rc::new(*summary);
        let ids = summary
            .snapshots
            .iter()
            .map(|snapshot| snapshot.id.clone())
            .collect::<Vec<_>>();
        self.invalidate_cards();
        self.rebuild_episode_choices(&ids, &self.episode_choices_box);
        self.rebuild_card_snapshot_choices(&ids);
        self.render_saved(summary.snapshots.iter());
        for category in &summary.user_reported.categories {
            self.evidence_button(
                &format!(
                    "{} · {}",
                    copy("category"),
                    category_label(category.category)
                ),
                category.evidence_snapshot_ids.clone(),
                summary.clone(),
            );
        }
        for outcome in &summary.user_reported.outcomes {
            self.evidence_button(
                &format!("{} · {}", copy("outcome"), outcome_label(outcome.outcome)),
                outcome.evidence_snapshot_ids.clone(),
                summary.clone(),
            );
        }
        for metric in &summary.metrics {
            self.evidence_button(
                metric_label(metric.id),
                metric.evidence_snapshot_ids.clone(),
                summary.clone(),
            );
        }
        if self.summary_evidence.first_child().is_none() {
            self.summary_evidence
                .append(&label(copy("summary_no_evidence")));
        }
    }

    fn evidence_button(
        self: &Rc<Self>,
        title: &str,
        ids: Vec<String>,
        summary: Rc<SavedInsightsSummary>,
    ) {
        if ids.is_empty() {
            return;
        }
        let button = gtk::Button::with_label(&format!("{} · {}", title, copy("evidence")));
        let weak = Rc::downgrade(self);
        let title = title.to_owned();
        button.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.clear_detail();
                view.saved_heading
                    .set_text(&format!("{} · {}", copy("summary_evidence"), title));
                let ids: std::collections::BTreeSet<_> = ids.iter().collect();
                view.render_saved(
                    summary
                        .snapshots
                        .iter()
                        .filter(|snapshot| ids.contains(&snapshot.id)),
                );
                view.reveal_evidence();
            }
        });
        self.summary_evidence.append(&button);
    }

    fn render_saved<'a>(self: &Rc<Self>, insights: impl Iterator<Item = &'a SnapshotEvidence>) {
        let insights: Vec<_> = insights.collect();
        let current = self.current_id.borrow().clone();
        if let Some(id) = current {
            if !insights.iter().any(|insight| insight.id == id) {
                self.clear_detail();
            }
        }

        while let Some(child) = self.saved.first_child() {
            self.saved.remove(&child);
        }
        if insights.is_empty() {
            self.saved.append(&label(copy("empty")));
        }
        for insight in insights {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            row.append(&label(&format!(
                "{} · {}\n{}",
                source_label(insight.source_format),
                local_date(&insight.analyzed_at),
                insight.id
            )));
            let explain = gtk::Button::with_label(copy("explain"));
            let delete = gtk::Button::with_label(copy("delete"));
            row.append(&explain);
            row.append(&delete);
            self.saved.append(&row);
            let weak = Rc::downgrade(self);
            let id = insight.id.clone();
            explain.connect_clicked(move |_| {
                if let Some(v) = weak.upgrade() {
                    v.explain(id.clone());
                }
            });
            let weak = Rc::downgrade(self);
            let delete_id = insight.id.clone();
            delete.connect_clicked(move |_| {
                if let Some(v) = weak.upgrade() {
                    v.request(
                        Op::Delete {
                            id: delete_id.clone(),
                        },
                        false,
                    );
                }
            });
        }
    }
}
fn render_mutation_notice(
    effects: &trace_commons_contributor::insights::MutationEffects,
) -> String {
    if effects.invalidated_episode_ids.is_empty() {
        return String::new();
    }
    format!(
        "{}\n{}",
        copy("episode_invalidated_notice"),
        effects.invalidated_episode_ids.join("\n")
    )
}
fn episode_error_copy_key(error: &anyhow::Error) -> &'static str {
    if let Some(error) = error.downcast_ref::<EpisodeStoreError>() {
        return match error {
            EpisodeStoreError::NotFound => "episode_missing",
            EpisodeStoreError::MissingMembers => "episode_missing_members",
            EpisodeStoreError::Full => "episode_limit",
            EpisodeStoreError::RevisionConflict => "episode_revision_conflict",
            EpisodeStoreError::RevisionOverflow => "episode_invalid",
        };
    }
    if let Some(error) = error.downcast_ref::<EpisodeValidationError>() {
        return match error {
            EpisodeValidationError::MemberLimit => "episode_member_limit",
            EpisodeValidationError::Invalid | EpisodeValidationError::DuplicateMember => {
                "episode_invalid"
            }
        };
    }
    if error.downcast_ref::<service::ResponseTooLarge>().is_some() {
        return "episode_response_too_large";
    }
    "episode_detail_unavailable"
}
fn episode_failure_status(committed: Option<&str>, error: &str) -> String {
    committed
        .map(|notice| format!("{notice}\n{error}"))
        .unwrap_or_else(|| error.to_owned())
}
fn render_episode(episode: &LocalEpisode, overlap: &[EpisodeOverlap]) -> String {
    let mut lines = vec![
        format!("{}: {}", copy("episode_id"), episode.id),
        format!("{}: {}", copy("episode_revision"), episode.revision),
        format!(
            "{}: {}",
            copy("episode_membership_revision"),
            episode.membership_revision
        ),
        format!("{}: {}", copy("episode_created_at"), episode.created_at),
        format!("{}: {}", copy("episode_updated_at"), episode.updated_at),
        format!("{}:", copy("episode_members")),
    ];
    lines.extend(
        episode
            .members
            .iter()
            .map(|member| format!("  {}", member.snapshot_id)),
    );
    if let Some(assessment) = &episode.manual_assessment {
        lines.push(format!(
            "{}: {} / {}",
            copy("episode_assessment"),
            category_label(assessment.category),
            outcome_label(assessment.outcome)
        ));
    } else {
        lines.push(format!(
            "{}: {}",
            copy("episode_assessment"),
            copy("episode_unassessed")
        ));
    }
    if overlap.is_empty() {
        lines.push(copy("episode_no_overlap").to_owned());
    } else {
        lines.push(format!("{}:", copy("episode_overlaps")));
        lines.extend(
            overlap
                .iter()
                .map(|item| format!("  {}: {}", item.snapshot_id, item.episode_ids.join(", "))),
        );
    }
    lines.join("\n")
}
fn category_at(i: u32) -> TaskCategory {
    [
        TaskCategory::Unknown,
        TaskCategory::Refactor,
        TaskCategory::Tests,
        TaskCategory::Docs,
        TaskCategory::Debugging,
        TaskCategory::Other,
    ]
    .get(i as usize)
    .copied()
    .unwrap_or(TaskCategory::Unknown)
}
fn outcome_at(i: u32) -> TaskOutcome {
    [
        TaskOutcome::Unknown,
        TaskOutcome::Accepted,
        TaskOutcome::Partial,
        TaskOutcome::Rejected,
    ]
    .get(i as usize)
    .copied()
    .unwrap_or(TaskOutcome::Unknown)
}
fn category_index(value: TaskCategory) -> u32 {
    (0..6).find(|i| category_at(*i) == value).unwrap_or(0)
}
fn outcome_index(value: TaskOutcome) -> u32 {
    (0..4).find(|i| outcome_at(*i) == value).unwrap_or(0)
}
fn local_date(date: &chrono::DateTime<chrono::Utc>) -> String {
    gtk::glib::DateTime::from_unix_local(date.timestamp())
        .and_then(|date| date.format("%c %Z"))
        .map(|date| date.to_string())
        .unwrap_or_else(|_| date.to_rfc3339())
}

fn metric_label(id: trace_commons_protocol::insights::MetricId) -> &'static str {
    use trace_commons_protocol::insights::MetricId;
    copy(match id {
        MetricId::Sessions => "metric_sessions",
        MetricId::Events => "metric_events",
        MetricId::InputTokens => "metric_input_tokens",
        MetricId::OutputTokens => "metric_output_tokens",
        MetricId::ToolCalls => "metric_tool_calls",
        MetricId::ToolFailures => "metric_tool_failures",
        MetricId::KnownOutcomes => "metric_known_outcomes",
    })
}

fn render_summary_text(summary: &SavedInsightsSummary) -> String {
    let mut text = format!(
        "{}\n{}: {}\n{}: {}\n{}: {}\n{}: {} v{} · {}: {}\n",
        copy("summary_scope"),
        copy("summary_snapshots"),
        summary.saved_snapshots,
        copy("summary_assessed"),
        summary.user_reported.assessed_snapshots,
        copy("summary_unassessed"),
        summary.user_reported.unassessed_snapshots,
        copy("provider"),
        summary.provider.id,
        summary.provider.version,
        copy("rubric"),
        summary.provider.rubric_version
    );
    text.push_str(&format!(
        "{}: {}\n",
        copy("summary_analysis_range"),
        summary
            .snapshot_analysis_range
            .as_ref()
            .map(|range| format!(
                "{} – {}",
                local_date(&range.oldest),
                local_date(&range.newest)
            ))
            .unwrap_or_else(|| copy("unknown").to_owned())
    ));
    if summary.saved_snapshots == 0 {
        text.push_str(copy("summary_empty"));
        text.push('\n');
    }
    text.push_str(copy("summary_limitations"));
    text.push('\n');
    for limitation in &summary.limitations {
        text.push_str(copy(match limitation {
            SummaryLimitation::SelectedSavedSessionsAreNotVerifiedTasks => {
                "summary_limitation_selected_saved_sessions_are_not_verified_tasks"
            }
            SummaryLimitation::AssessmentsAreUserReported => {
                "summary_limitation_assessments_are_user_reported"
            }
            SummaryLimitation::ObservedSumsRequireBothCoverages => {
                "summary_limitation_observed_sums_require_both_coverages"
            }
            SummaryLimitation::AnalysisDatesAreNotActivityTime => {
                "summary_limitation_analysis_dates_are_not_activity_time"
            }
            SummaryLimitation::SourceFormatsAreNotModelIdentity => {
                "summary_limitation_source_formats_are_not_model_identity"
            }
            SummaryLimitation::NoModelRankingsTimeSavingsOrCost => {
                "summary_limitation_no_model_rankings_time_savings_or_cost"
            }
        }));
        text.push('\n');
    }
    text.push_str(copy("summary_categories"));
    text.push('\n');
    for category in &summary.user_reported.categories {
        text.push_str(&format!(
            "{}: {}\n",
            category_label(category.category),
            category.snapshots
        ));
    }
    text.push_str(copy("summary_outcomes"));
    text.push('\n');
    for outcome in &summary.user_reported.outcomes {
        text.push_str(&format!(
            "{}: {}\n",
            outcome_label(outcome.outcome),
            outcome.snapshots
        ));
    }
    text.push_str(copy("summary_metrics"));
    text.push('\n');
    for metric in &summary.metrics {
        text.push_str(&format!(
            "{} · {}: {}\n{}: {} · {}: {}\n{}: {} / {} {}\n",
            metric_label(metric.id),
            copy("summary_observed_sum"),
            metric
                .observed_value_sum
                .map(|n| n.to_string())
                .unwrap_or_else(|| copy("unknown").to_owned()),
            copy("summary_available"),
            metric.available_snapshots,
            copy("summary_missing"),
            metric.missing_snapshots,
            copy("summary_record_coverage"),
            metric.record_coverage.observed,
            metric.record_coverage.total,
            copy(match metric.coverage_unit {
                CoverageUnit::SessionSnapshots => "summary_unit_session_snapshots",
                CoverageUnit::NormalizedEvents => "summary_unit_normalized_events",
                CoverageUnit::ToolResults => "summary_unit_tool_results",
            })
        ));
    }
    text
}

fn render(insight: &LocalInsight) -> String {
    use trace_commons_protocol::insights::MetricId;
    let mut text = format!(
        "{}: {}\n{}\n{}: {}\n{}: {} v{} · {}: {}\n{}: {}\n{}\n{}\n",
        copy("source"),
        source_label(insight.source_format),
        copy("boundary_notice"),
        copy("analyzed_at"),
        local_date(&insight.analyzed_at),
        copy("provider"),
        insight.report.provider.id,
        insight.report.provider.version,
        copy("rubric"),
        insight.report.provider.rubric_version,
        copy("cost"),
        copy("unknown"),
        copy("unknown_notice"),
        copy("coverage_notice")
    );
    for metric in &insight.report.metrics {
        text.push_str(&format!(
            "{}: {} · {} {} / {}\n",
            metric_label(metric.id),
            metric
                .value
                .map(|n| n.to_string())
                .unwrap_or_else(|| copy("unknown").into()),
            copy("coverage"),
            metric.coverage.observed,
            metric.coverage.total
        ));
    }
    if insight
        .report
        .metrics
        .iter()
        .any(|m| m.id == MetricId::ToolCalls && m.coverage.observed < m.coverage.total)
    {
        text.push_str(copy("partial_notice"));
        text.push('\n');
    }
    text.push_str(copy("evidence_notice"));
    text.push('\n');
    for evidence in &insight.report.evidence {
        text.push_str(&format!(
            "{}: {}\n{}: {}\n",
            copy("evidence"),
            evidence.id,
            copy("source_digest"),
            evidence.source_digest
        ));
    }
    if let Some(a) = &insight.manual_annotation {
        text.push_str(&format!(
            "{}\n{}: {} · {}: {}\n{}: {}\n",
            copy("assessment_notice"),
            copy("category"),
            category_label(a.category),
            copy("outcome"),
            outcome_label(a.outcome),
            copy("recorded_at"),
            local_date(&a.recorded_at)
        ));
    }
    text
}
fn source_label(source: SourceFormat) -> &'static str {
    copy(match source {
        SourceFormat::Codex => "codex",
        SourceFormat::ClaudeCode => "claude_code",
        SourceFormat::Trajectory => "trajectory",
    })
}
fn category_label(category: TaskCategory) -> &'static str {
    copy(match category {
        TaskCategory::Unknown => "category_unknown",
        TaskCategory::Refactor => "category_refactor",
        TaskCategory::Tests => "category_tests",
        TaskCategory::Docs => "category_docs",
        TaskCategory::Debugging => "category_debugging",
        TaskCategory::Other => "category_other",
    })
}
fn outcome_label(outcome: TaskOutcome) -> &'static str {
    copy(match outcome {
        TaskOutcome::Unknown => "outcome_unknown",
        TaskOutcome::Accepted => "outcome_accepted",
        TaskOutcome::Partial => "outcome_partial",
        TaskOutcome::Rejected => "outcome_rejected",
    })
}

/// The default launch surface never constructs a Worker or probes sources.
pub fn present_local<F: Fn() + 'static>(application: &adw::Application, contribute: F) {
    super::style::install();
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Trace Commons · Insights")
        .default_width(840)
        .default_height(760)
        .build();
    let stack = local_first_stack(&window);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    let button = gtk::Button::with_label(copy("contributions"));
    header.pack_start(&button);
    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    header.set_title_widget(Some(&switcher));
    content.append(&header);
    content.append(&stack);
    stack.set_vexpand(true);
    window.set_content(Some(&content));
    button.connect_clicked(move |_| contribute());
    window.present();
}

pub(super) fn local_first_stack(window: &adw::ApplicationWindow) -> adw::ViewStack {
    let stack = adw::ViewStack::new();
    let insights = InsightsView::new(window);
    let missions = super::mission_drafts::MissionDraftsView::new(window);
    stack
        .add_titled(&insights.root, Some("insights"), copy("title"))
        .set_icon_name(Some("view-statistics-symbolic"));
    let mission_title = trace_commons_contributor::mission_draft_service::ui_copy()
        .remove("title")
        .expect("shared mission draft title");
    stack
        .add_titled(&missions.root, Some("mission-drafts"), &mission_title)
        .set_icon_name(Some("document-edit-symbolic"));
    stack.set_visible_child_name("insights");
    stack
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn local_first_window_exposes_insights_and_mission_drafts_without_a_worker() {
        assert_eq!(
            std::env::consts::OS,
            "linux",
            "requires a Linux GTK display"
        );
        let context = gtk::glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        let window = adw::ApplicationWindow::builder().build();
        let stack = local_first_stack(&window);
        assert!(stack.child_by_name("insights").is_some());
        assert!(stack.child_by_name("mission-drafts").is_some());
        stack.set_visible_child_name("mission-drafts");
        assert_eq!(
            stack.visible_child_name().as_deref(),
            Some("mission-drafts")
        );
        window.close();
    }
    #[test]
    fn episode_cleanup_notice_only_describes_reported_removed_groups() {
        use trace_commons_contributor::insights::MutationEffects;
        assert!(render_mutation_notice(&MutationEffects::default()).is_empty());
        let notice = render_mutation_notice(&MutationEffects {
            invalidated_episode_ids: vec!["group-a".into(), "group-b".into()],
            ..MutationEffects::default()
        });
        assert!(notice.starts_with(copy("episode_invalidated_notice")));
        assert!(notice.ends_with("group-a\ngroup-b"));
    }
    #[test]
    fn chooser_tickets_reject_switch_and_return_to_same_snapshot() {
        let ticket = ChooserTicket {
            generation: 7,
            snapshot: Some("original".into()),
        };
        assert!(ticket.matches(7, Some("original")));
        assert!(!ticket.matches(7, Some("different")));
        assert!(!ticket.matches(7, None));
        assert!(!ticket.matches(8, Some("original")));
        let preview = ChooserTicket {
            generation: 7,
            snapshot: None,
        };
        assert!(preview.matches(7, None));
        assert!(!preview.matches(8, None));
    }
    #[test]
    fn episode_drafts_bind_generation_id_and_revision() {
        let ticket = EpisodeDraftTicket {
            generation: 4,
            id: Some("episode-a".into()),
            revision: Some(7),
        };
        assert!(ticket.matches(4, Some(("episode-a", 7))));
        assert!(!ticket.matches(5, Some(("episode-a", 7))));
        assert!(!ticket.matches(4, Some(("episode-b", 7))));
        assert!(!ticket.matches(4, Some(("episode-a", 8))));
        assert!(!ticket.matches(4, None));
        let read = EpisodeReadTicket {
            generation: 9,
            requested_id: Some("episode-a".into()),
        };
        assert!(read.accepts(9, Some("episode-a")));
        assert!(!read.accepts(10, Some("episode-a")));
        assert!(!read.accepts(9, Some("episode-b")));
        assert_eq!(
            episode_failure_status(Some("Committed"), "Refresh failed"),
            "Committed\nRefresh failed"
        );
        assert_eq!(
            episode_failure_status(None, "Refresh failed"),
            "Refresh failed"
        );
    }

    #[test]
    fn card_completions_bind_generation_and_exact_evidence_selection() {
        let ticket = CardRequestTicket {
            generation: 7,
            snapshot_ids: vec!["snapshot-a".into()],
            episode_ids: vec!["episode-a".into()],
        };
        assert!(accept_card_completion(
            7,
            &["snapshot-a".into()],
            &["episode-a".into()],
            &ticket
        ));
        assert!(!accept_card_completion(
            8,
            &["snapshot-a".into()],
            &["episode-a".into()],
            &ticket
        ));
        assert!(!accept_card_completion(
            7,
            &["snapshot-b".into()],
            &["episode-a".into()],
            &ticket
        ));
        assert!(!accept_card_completion(
            7,
            &["snapshot-a".into()],
            &[],
            &ticket
        ));
    }
    #[test]
    fn cancelled_read_cannot_publish_and_next_request_waits_for_completion() {
        let mut f = Flight::default();
        assert!(f.begin());
        f.cancelled = true;
        assert!(!f.begin());
        assert!(!f.finish());
        assert!(f.begin());
        assert!(f.finish());
    }
    #[test]
    fn closed_window_rejects_pending_results_and_future_requests() {
        let mut f = Flight::default();
        assert!(f.begin());
        f.closed = true;
        assert!(!f.finish());
        assert!(!f.begin());
    }
    #[test]
    fn assessments_round_trip_and_unknown_selection_stays_unknown() {
        for i in 0..6 {
            assert_eq!(category_index(category_at(i)), i);
        }
        for i in 0..4 {
            assert_eq!(outcome_index(outcome_at(i)), i);
        }
        assert_eq!(category_at(u32::MAX), TaskCategory::Unknown);
    }
    #[test]
    fn partial_observations_keep_unknown_values_and_evidence_attribution() {
        use trace_commons_protocol::insights::{
            Coverage, EvidenceRef, InsightMetric, InsightReport, MetricId, ProviderManifest,
        };
        let insight = LocalInsight {
            task_attribution: None,
            id: "fixture".into(),
            source_format: SourceFormat::Codex,
            boundary: trace_commons_contributor::insights::EpisodeBoundary::SessionProxy,
            report: InsightReport {
                schema_version: 1,
                provider: ProviderManifest::first_party(),
                evidence: vec![EvidenceRef {
                    id: "source-1".into(),
                    source_digest: "a".repeat(64),
                }],
                metrics: vec![
                    InsightMetric {
                        id: MetricId::ToolCalls,
                        value: Some(1),
                        coverage: Coverage {
                            observed: 1,
                            total: 2,
                        },
                        evidence_ids: vec!["source-1".into()],
                    },
                    InsightMetric {
                        id: MetricId::InputTokens,
                        value: None,
                        coverage: Coverage {
                            observed: 0,
                            total: 2,
                        },
                        evidence_ids: vec!["source-1".into()],
                    },
                ],
            },
            estimated_cost_usd: None,
            cost_unavailable_reason: "unavailable".into(),
            task_category: None,
            manual_annotation: None,
            model_observations: None,
            outcome_links: Vec::new(),
            time_evidence: None,
            usage_evidence: None,
            analyzed_at: chrono::Utc::now(),
        };
        let text = render(&insight);
        assert!(text.contains(&format!(
            "{}: {}",
            copy("metric_input_tokens"),
            copy("unknown")
        )));
        assert!(text.contains("1 / 2"));
        assert!(text.contains(copy("partial_notice")));
        assert!(text.contains("trace-commons-local"));
        assert!(text.contains("descriptive-counts-v1"));
        assert!(text.contains(&"a".repeat(64)));
        assert!(text.contains(copy("boundary_notice")));
    }

    #[test]
    fn summary_renderer_preserves_empty_unknown_zero_coverage_units_and_limitations() {
        use trace_commons_protocol::insights::MetricId;
        let absent =
            std::env::temp_dir().join(format!("tc-summary-render-{}", uuid::Uuid::new_v4()));
        let mut summary =
            trace_commons_contributor::insights::summary::read_saved(Some(&absent)).unwrap();
        let empty = render_summary_text(&summary);
        assert!(empty.contains(copy("summary_empty")));
        assert!(empty.contains(&format!(
            "{}: {}",
            copy("summary_analysis_range"),
            copy("unknown")
        )));
        assert!(!absent.exists());
        summary.saved_snapshots = 2;
        summary.user_reported.assessed_snapshots = 1;
        summary.user_reported.unassessed_snapshots = 1;
        summary
            .user_reported
            .outcomes
            .iter_mut()
            .find(|o| o.outcome == TaskOutcome::Unknown)
            .unwrap()
            .snapshots = 1;
        let metric = summary
            .metrics
            .iter_mut()
            .find(|m| m.id == MetricId::ToolCalls)
            .unwrap();
        metric.observed_value_sum = Some(0);
        metric.available_snapshots = 1;
        metric.missing_snapshots = 1;
        metric.record_coverage.observed = 1;
        metric.record_coverage.total = 3;
        let text = render_summary_text(&summary);
        assert!(text.contains(&format!("{}: 1", copy("summary_unassessed"))));
        assert!(text.contains(&format!("{}: 1", copy("outcome_unknown"))));
        assert!(text.contains(&format!(
            "{} · {}: 0",
            metric_label(MetricId::ToolCalls),
            copy("summary_observed_sum")
        )));
        assert!(text.contains(&format!(
            "{} · {}: {}",
            metric_label(MetricId::InputTokens),
            copy("summary_observed_sum"),
            copy("unknown")
        )));
        assert!(text.contains(&format!(
            "{}: 1 / 3 {}",
            copy("summary_record_coverage"),
            copy("summary_unit_normalized_events")
        )));
        for key in [
            "summary_available",
            "summary_missing",
            "summary_unit_tool_results",
            "summary_unit_session_snapshots",
            "summary_limitation_selected_saved_sessions_are_not_verified_tasks",
            "summary_limitation_assessments_are_user_reported",
            "summary_limitation_observed_sums_require_both_coverages",
            "summary_limitation_analysis_dates_are_not_activity_time",
            "summary_limitation_source_formats_are_not_model_identity",
            "summary_limitation_no_model_rankings_time_savings_or_cost",
        ] {
            assert!(text.contains(copy(key)), "missing shared label {key}");
        }
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn account_free_view_analyzes_saves_explains_deletes_and_ignores_closed_results() {
        // Compile the whole scenario on every platform, but never initialize
        // macOS GTK on the Rust harness worker thread.
        assert_eq!(
            std::env::consts::OS,
            "linux",
            "requires a Linux GTK display"
        );
        let context = gtk::glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        adw::init().expect("GTK display unavailable");
        let temp = std::env::temp_dir().join(format!("tc-insights-ui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp).unwrap();
        let file = temp.join("selected.jsonl");
        std::fs::write(&file, concat!(
            "{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n",
            "{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"PRIVATE_BODY\"}\n"
        )).unwrap();
        let store = temp.join("insights");
        let window = adw::ApplicationWindow::builder().build();
        let view = InsightsView::with_store(&window, Some(store.clone()));
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
        assert!(
            !store.exists(),
            "initial summary must not initialize storage"
        );
        assert!(view.summary.text().contains(copy("summary_empty")));
        view.source.set_selected(1);
        *view.selected.borrow_mut() = Some(file.clone());
        view.analyze(false);
        settle();
        assert!(!store.exists());
        assert!(!view.detail.text().contains("PRIVATE_BODY"));
        assert!(view.detail.text().contains(copy("provider")));
        assert!(view.detail.text().contains(copy("unknown")));
        view.analyze(true);
        settle();
        let id = view.current_id.borrow().clone().expect("saved id");
        let first_choice = view
            .episode_choices
            .borrow()
            .first()
            .expect("saved snapshot choice")
            .1
            .clone();
        first_choice.set_active(true);
        view.create_episode();
        settle();
        let (episode_id, revision) = view
            .current_episode
            .borrow()
            .clone()
            .expect("created episode remains presented");
        assert_eq!(revision, 1);
        assert!(view.episode_detail.text().contains(&episode_id));
        assert!(
            view.episode_detail
                .text()
                .contains(copy("episode_no_overlap"))
        );
        view.card_snapshot_choices.borrow()[0].1.set_active(true);
        view.card_episode_choices.borrow()[0].1.set_active(true);
        view.request_cards();
        settle();
        assert_eq!(view.card_state.get(), CardPanelState::Presented);
        assert!(
            view.card_result
                .text()
                .contains(question_copy(InsightQuestionId::RecordedActivity).1)
        );
        assert!(view.card_evidence.first_child().is_some());
        let second_file = temp.join("second.jsonl");
        std::fs::write(
            &second_file,
            concat!(
                "{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n",
                "{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:01:00Z\",\"content\":\"SECOND_PRIVATE_BODY\"}\n"
            ),
        )
        .unwrap();
        let second = service::execute(LocalInsightsRequest {
            store_dir: Some(store.clone()),
            operation: Op::Analyze {
                source: SourceFormat::Trajectory,
                file: second_file,
                save: true,
            },
        })
        .unwrap();
        let Response::Analyze {
            insight: second, ..
        } = second
        else {
            panic!("second analyze response");
        };
        view.refresh_history();
        settle();
        assert_eq!(view.episode_choices.borrow().len(), 2);
        for (_, choice) in view.episode_choices.borrow().iter() {
            choice.set_active(true);
        }
        view.replace_episode_members();
        settle();
        assert_eq!(view.current_membership_revision.get(), Some(2));
        assert_eq!(view.current_episode.borrow().as_ref().unwrap().1, 2);
        assert!(view.status.text().contains(copy("episode_members_saved")));
        assert!(
            view.status
                .text()
                .contains(copy("episode_membership_changed"))
        );
        view.replace_episode_members();
        settle();
        assert_eq!(view.current_episode.borrow().as_ref().unwrap().1, 2);
        assert_eq!(view.status.text(), copy("episode_members_saved"));
        view.episode_category
            .set_selected(category_index(TaskCategory::Tests));
        view.episode_outcome
            .set_selected(outcome_index(TaskOutcome::Accepted));
        view.annotate_episode();
        settle();
        let revision = view.current_episode.borrow().as_ref().unwrap().1;
        assert_eq!(revision, 3);
        assert!(
            view.episode_detail
                .text()
                .contains(copy("outcome_accepted"))
        );
        service::open_store(Some(&store))
            .unwrap()
            .episode_annotate(
                &episode_id,
                revision,
                TaskCategory::Docs,
                TaskOutcome::Partial,
            )
            .unwrap();
        view.clear_episode_assessment_with(view.episode_ticket());
        settle();
        assert!(view.current_episode.borrow().is_none());
        assert_eq!(view.status.text(), copy("episode_revision_conflict"));
        view.open_episode(episode_id.clone());
        settle();
        assert_eq!(view.current_episode.borrow().as_ref().unwrap().1, 4);
        view.delete_episode_with(view.episode_ticket());
        settle();
        assert!(view.current_episode.borrow().is_none());
        assert!(
            service::open_store(Some(&store))
                .unwrap()
                .episode_list()
                .unwrap()
                .is_empty()
        );
        // Episode membership coverage needs two saved snapshots. Remove its
        // temporary second member before continuing the original single-
        // snapshot evidence and summary lifecycle assertions below.
        service::execute(LocalInsightsRequest {
            store_dir: Some(store.clone()),
            operation: Op::Delete { id: second.id },
        })
        .unwrap();
        view.refresh_history();
        settle();
        assert!(view.evidence_expander.is_visible());
        assert!(view.evidence_controls.is_visible());
        let report = temp.join("test-report.json");
        std::fs::write(&report, br#"{"schema_version":1,"runner":"synthetic-fixture","passed":0,"failed":1,"skipped":0,"observed_at":"2026-01-01T00:00:00Z","commit_id":null}"#).unwrap();
        let ticket = view.chooser_ticket();
        assert!(view.accepts_chooser(&ticket));
        view.link_chosen_evidence(&ticket, report.clone(), None);
        assert!(
            view.detail.text().is_empty(),
            "link clears previous successful detail while pending"
        );
        settle();
        let stored = service::open_store(Some(&store))
            .unwrap()
            .explain(&id)
            .unwrap();
        assert_eq!(stored.outcome_links.len(), 1);
        assert!(view.evidence_expander.is_expanded());
        assert!(
            !view.accepts_chooser(&ticket),
            "completed operation invalidates previous picker"
        );
        assert!(
            view.summary
                .text()
                .contains(&format!("{}: 1", copy("summary_snapshots")))
        );
        let unlink = view
            .evidence_body
            .last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        unlink.emit_clicked();
        settle();
        assert!(
            service::open_store(Some(&store))
                .unwrap()
                .explain(&id)
                .unwrap()
                .outcome_links
                .is_empty()
        );
        assert!(report.exists());
        // Exercise the directory/commit completion through the same shared
        // service path as the native picker, using an isolated local object.
        let repository = temp.join("repository.git");
        let init = std::process::Command::new("git")
            .args(["init", "--bare", "--template="])
            .arg(&repository)
            .output()
            .unwrap();
        assert!(init.status.success());
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(&repository)
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        let tree = git(&["hash-object", "-w", "-t", "tree", "--stdin"]);
        let commit = git(&["commit-tree", &tree]);
        view.link_chosen_evidence(
            &view.chooser_ticket(),
            repository.clone(),
            Some(commit.clone()),
        );
        settle();
        let stored = service::open_store(Some(&store))
            .unwrap()
            .explain(&id)
            .unwrap();
        assert_eq!(stored.outcome_links.len(), 1);
        let trace_commons_contributor::insights::outcomes::OutcomeEvidence::GitCommit(git) =
            &stored.outcome_links[0].evidence
        else {
            panic!("Git evidence");
        };
        assert_eq!(git.object_id, commit);
        view.evidence_body
            .last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        settle();
        assert!(
            service::open_store(Some(&store))
                .unwrap()
                .explain(&id)
                .unwrap()
                .outcome_links
                .is_empty()
        );
        assert!(repository.exists());
        let hidden_ticket = view.chooser_ticket();
        window.hide();
        window.present();
        settle();
        assert!(!view.accepts_chooser(&hidden_ticket));
        view.link_chosen_evidence(&hidden_ticket, report.clone(), None);
        assert!(
            !view.flight.borrow().busy,
            "late picker cannot mutate reopened view"
        );
        assert!(
            view.summary
                .text()
                .contains(&format!("{}: 1", copy("summary_snapshots")))
        );
        assert!(
            view.summary
                .text()
                .contains(&format!("{}: 1", copy("summary_unassessed")))
        );
        assert!(
            view.saved.first_child().is_some(),
            "save refreshes saved rows immediately"
        );
        let evidence = view
            .summary_evidence
            .first_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        evidence.emit_clicked();
        assert!(
            !view.summary_expander.is_expanded(),
            "filter reveals rows below the collapsed summary"
        );
        assert!(
            view.detail.text().is_empty(),
            "filter clears previous detail above its results"
        );
        assert!(view.current_id.borrow().is_none());
        let row = view.saved.first_child().unwrap();
        let open = row
            .first_child()
            .unwrap()
            .next_sibling()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        open.emit_clicked();
        settle();
        assert_eq!(
            view.current_id.borrow().as_deref(),
            Some(id.as_str()),
            "summary evidence opens its saved snapshot"
        );
        let prior_id = id.clone();
        let externally_replaced_ticket = view.chooser_ticket();
        std::fs::write(&file, concat!(
            "{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n",
            "{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"CHANGED_BODY\"}\n"
        )).unwrap();
        // Another shell replaces this path while the picker remains open. The
        // original ID must fail, never silently mutate its replacement.
        let replacement = service::execute(LocalInsightsRequest {
            store_dir: Some(store.clone()),
            operation: Op::Analyze {
                source: SourceFormat::Trajectory,
                file: file.clone(),
                save: true,
            },
        })
        .unwrap();
        let Response::Analyze {
            insight: replacement,
            ..
        } = replacement
        else {
            panic!("analyze response");
        };
        view.link_chosen_evidence(&externally_replaced_ticket, report.clone(), None);
        settle();
        assert_eq!(view.status.text(), copy("error"));
        assert!(view.detail.text().is_empty());
        assert!(!view.evidence_expander.is_visible());
        assert!(
            service::open_store(Some(&store))
                .unwrap()
                .explain(&replacement.id)
                .unwrap()
                .outcome_links
                .is_empty()
        );
        view.analyze(true);
        settle();
        let id = view.current_id.borrow().clone().expect("reimported id");
        assert_ne!(prior_id, id);
        let row = view.saved.first_child().expect("saved row");
        let title = row.first_child().unwrap().downcast::<gtk::Label>().unwrap();
        assert!(title.text().contains(&id));
        assert!(!title.text().contains(&prior_id));
        assert!(
            row.next_sibling().is_none(),
            "reimport removes obsolete digest row"
        );
        view.request(Op::Explain { id: id.clone() }, true);
        settle();
        assert!(view.assessment.is_visible());
        view.request(
            Op::Annotate {
                id: id.clone(),
                category: TaskCategory::Tests,
                outcome: TaskOutcome::Accepted,
            },
            true,
        );
        settle();
        assert!(view.detail.text().contains(copy("assessment_notice")));
        assert!(
            view.summary
                .text()
                .contains(&format!("{}: 1", copy("summary_assessed")))
        );
        view.request(Op::ClearAnnotation { id: id.clone() }, true);
        settle();
        assert!(
            view.summary
                .text()
                .contains(&format!("{}: 0", copy("summary_assessed")))
        );
        assert!(
            view.summary
                .text()
                .contains(&format!("{}: 1", copy("summary_unassessed")))
        );
        let group = service::open_store(Some(&store))
            .unwrap()
            .episode_create(std::slice::from_ref(&id))
            .unwrap();
        view.open_episode(group.id.clone());
        settle();
        assert_eq!(
            view.current_episode.borrow().as_ref().map(|(id, _)| id),
            Some(&group.id)
        );
        view.request(Op::Delete { id }, false);
        settle();
        assert!(
            view.current_episode.borrow().is_none(),
            "committed snapshot cleanup immediately invalidates the open episode"
        );
        assert!(
            view.mutation_notice.text().contains(&group.id),
            "cleanup notice survives automatic empty-history refresh"
        );
        let index_path = store.join("index.json");
        let healthy_index = std::fs::read(&index_path).unwrap();
        std::fs::write(&index_path, b"broken refresh fixture").unwrap();
        view.refresh_history();
        settle();
        assert_eq!(view.status.text(), copy("error"));
        assert!(
            view.mutation_notice.text().contains(&group.id),
            "confirmed removal survives a subsequent reconciliation failure"
        );
        std::fs::write(&index_path, healthy_index).unwrap();
        view.refresh_history();
        settle();
        assert!(file.exists());
        assert!(service::list_saved(Some(&store)).unwrap().is_empty());
        assert!(view.summary.text().contains(copy("summary_empty")));
        view.analyze(true);
        settle();
        assert!(
            view.mutation_notice.text().is_empty(),
            "next analysis clears prior cleanup notice"
        );
        let id = view.current_id.borrow().clone().unwrap();
        service::open_store(Some(&store))
            .unwrap()
            .delete(&id)
            .unwrap();
        view.refresh_history();
        settle();
        assert!(view.current_id.borrow().is_none());
        assert!(view.detail.text().is_empty());
        view.analyze(false);
        settle();
        let preview = view.detail.text();
        view.refresh_history();
        settle();
        assert_eq!(
            view.detail.text(),
            preview,
            "history refresh preserves unsaved preview"
        );
        assert!(!view.detail.text().is_empty());
        view.explain("deleted-snapshot".to_owned());
        assert!(
            view.detail.text().is_empty(),
            "explicit evidence selection clears previous unsaved preview immediately"
        );
        assert!(
            !view.summary_expander.is_expanded(),
            "explicit evidence is revealed above the collapsed summary"
        );
        settle();
        assert!(
            view.detail.text().is_empty(),
            "missing evidence cannot retain prior success"
        );
        assert!(view.current_id.borrow().is_none());
        assert!(!view.assessment.is_visible());
        view.analyze(false);
        settle();
        let index = store.join("index.json");
        let valid_index = std::fs::read(&index).unwrap();
        std::fs::write(&index, b"invalid saved store").unwrap();
        view.refresh_history();
        settle();
        assert_eq!(view.summary.text(), copy("summary_unavailable"));
        assert!(view.summary_evidence.first_child().is_none());
        assert!(view.saved.first_child().is_none());
        std::fs::write(&index, valid_index).unwrap();
        view.refresh_history();
        settle();
        assert!(view.summary.text().contains(copy("summary_empty")));
        let before = view.detail.text();
        view.request(
            Op::Analyze {
                source: SourceFormat::Trajectory,
                file,
                save: false,
            },
            false,
        );
        let declined_close = window.connect_close_request(|_| gtk::glib::Propagation::Stop);
        window.close();
        assert!(view.flight.borrow().cancelled);
        settle();
        assert!(
            view.controls.is_sensitive(),
            "declined close restores controls without show signal"
        );
        assert_eq!(view.detail.text(), before);
        window.disconnect(declined_close);
        view.refresh_history();
        window.hide();
        settle();
        assert_eq!(view.detail.text(), before);
        window.present();
        settle();
        assert!(view.controls.is_sensitive());
        view.refresh_history();
        window.close();
        assert!(
            view.flight.borrow().cancelled,
            "close synchronously invalidates queued presentation"
        );
        settle();
        assert!(view.flight.borrow().closed || view.flight.borrow().cancelled);
        assert_eq!(view.detail.text(), before);
        std::fs::remove_dir_all(temp).unwrap();
    }
}
