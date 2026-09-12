//! Account-free selected-file Insights. No worker, configuration or discovery.
//! A single bounded IO request runs on an OS thread. Cancellation suppresses
//! presentation; already-started writes finish. Weak callbacks never own a view.
use adw::prelude::*;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
use trace_commons_contributor::insights::service::{
    self, LocalInsightsOperation as Op, LocalInsightsRequest, LocalInsightsResponse as Response,
};
use trace_commons_contributor::insights::summary::{
    CoverageUnit, SavedInsightsSummary, SnapshotEvidence, SummaryLimitation,
};
use trace_commons_contributor::insights::{LocalInsight, SourceFormat, TaskCategory, TaskOutcome};

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

pub struct InsightsView {
    pub root: gtk::Box,
    controls: gtk::Box,
    source: gtk::DropDown,
    selected: RefCell<Option<PathBuf>>,
    selected_label: gtk::Label,
    status: gtk::Label,
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
    category: gtk::DropDown,
    outcome: gtk::DropDown,
    current_id: RefCell<Option<String>>,
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
        let source = gtk::DropDown::from_strings(&[copy("codex"), copy("trajectory")]);
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
        let detail = label("");
        content.append(&detail);
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
            category,
            outcome,
            current_id: RefCell::new(None),
            flight: RefCell::new(Flight::default()),
            store_dir,
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
            let weak = Rc::downgrade(&view);
            chooser.connect_response(move |dialog, response| {
                if response == gtk::ResponseType::Accept {
                    if let Some(view) = weak.upgrade() {
                        if view.flight.borrow().closed {
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
        let weak = Rc::downgrade(&view);
        save.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.analyze(true);
            }
        });
        let weak = Rc::downgrade(&view);
        refresh.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.summary_expander.set_expanded(true);
                v.refresh_history();
            }
        });
        let weak = Rc::downgrade(&view);
        cancel.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.flight.borrow_mut().cancelled = true;
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
            }
            gtk::glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&view);
        window.connect_hide(move |_| {
            if let Some(view) = weak.upgrade() {
                view.flight.borrow_mut().cancelled = true;
            }
        });
        let weak = Rc::downgrade(&view);
        window.connect_show(move |_| {
            if let Some(view) = weak.upgrade() {
                if !view.flight.borrow().busy && !view.flight.borrow().closed {
                    view.controls.set_sensitive(true);
                    view.assessment.set_sensitive(true);
                    view.saved.set_sensitive(true);
                }
            }
        });
        let weak = Rc::downgrade(&view);
        view.root.connect_map(move |_| {
            if let Some(view) = weak.upgrade() {
                if !view.flight.borrow().busy && !view.flight.borrow().closed {
                    view.controls.set_sensitive(true);
                    view.assessment.set_sensitive(true);
                    view.saved.set_sensitive(true);
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
        let file = self.selected.borrow().clone();
        if let Some(file) = file {
            if self.flight.borrow().busy || self.flight.borrow().closed {
                return;
            }
            self.save.set_sensitive(false);
            self.detail.set_text("");
            self.assessment.set_visible(false);
            *self.current_id.borrow_mut() = None;
            self.request(
                Op::Analyze {
                    source: if self.source.selected() == 0 {
                        SourceFormat::Codex
                    } else {
                        SourceFormat::Trajectory
                    },
                    file,
                    save,
                },
                save,
            );
        }
    }

    fn clear_detail(&self) {
        self.detail.set_text("");
        self.assessment.set_visible(false);
        *self.current_id.borrow_mut() = None;
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
        self.controls.set_sensitive(false);
        self.assessment.set_sensitive(false);
        self.saved.set_sensitive(false);
        self.summary_evidence.set_sensitive(false);
        self.status.set_text(copy("working"));
        let refresh_saved = matches!(
            &operation,
            Op::Analyze { save: true, .. } | Op::Annotate { .. } | Op::ClearAnnotation { .. }
        );
        let history_read = matches!(&operation, Op::Summary {} | Op::List {});
        let mutates_history = refresh_saved || matches!(&operation, Op::Delete { .. });
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
            match result {
                Some(Response::Analyze { insight })
                | Some(Response::Explain { insight })
                | Some(Response::Annotate { insight })
                | Some(Response::ClearAnnotation { insight }) => {
                    view.detail.set_text(&render(&insight));
                    *view.current_id.borrow_mut() = if persisted {
                        Some(insight.id.clone())
                    } else {
                        None
                    };
                    view.assessment.set_visible(persisted);
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
                    }
                }
                Some(Response::Summary { summary }) => {
                    view.render_summary(summary);
                    view.status.set_text(copy("refreshed"));
                    let selected = view.current_id.borrow().clone();
                    if let Some(id) = selected {
                        view.request(Op::Explain { id }, true);
                    }
                }
                Some(Response::List { .. }) => view.refresh_history(),
                Some(Response::Delete { deleted }) => {
                    view.detail.set_text("");
                    view.assessment.set_visible(false);
                    *view.current_id.borrow_mut() = None;
                    view.status.set_text(if deleted {
                        copy("deleted")
                    } else {
                        copy("already_absent")
                    });
                    view.refresh_history();
                }
                _ => {
                    view.clear_summary();
                    view.summary.set_text(copy("summary_unavailable"));
                    while let Some(child) = view.saved.first_child() {
                        view.saved.remove(&child);
                    }
                    if view.current_id.borrow().is_some() {
                        view.detail.set_text("");
                        view.assessment.set_visible(false);
                        *view.current_id.borrow_mut() = None;
                    }
                    view.status.set_text(copy("error"));
                }
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
                view.saved_heading
                    .set_text(&format!("{} · {}", copy("summary_evidence"), title));
                let ids: std::collections::BTreeSet<_> = ids.iter().collect();
                view.render_saved(
                    summary
                        .snapshots
                        .iter()
                        .filter(|snapshot| ids.contains(&snapshot.id)),
                );
            }
        });
        self.summary_evidence.append(&button);
    }

    fn render_saved<'a>(self: &Rc<Self>, insights: impl Iterator<Item = &'a SnapshotEvidence>) {
        let insights: Vec<_> = insights.collect();
        let current = self.current_id.borrow().clone();
        if let Some(id) = current {
            if !insights.iter().any(|insight| insight.id == id) {
                self.detail.set_text("");
                self.assessment.set_visible(false);
                *self.current_id.borrow_mut() = None;
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
    let page = InsightsView::new(&window);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    let button = gtk::Button::with_label(copy("contributions"));
    header.pack_start(&button);
    content.append(&header);
    content.append(&page.root);
    window.set_content(Some(&content));
    button.connect_clicked(move |_| contribute());
    window.present();
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn account_free_view_analyzes_saves_explains_deletes_and_ignores_closed_results() {
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
        std::fs::write(&file, concat!(
            "{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n",
            "{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"CHANGED_BODY\"}\n"
        )).unwrap();
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
        view.request(Op::Delete { id }, false);
        settle();
        assert!(file.exists());
        assert!(service::list_saved(Some(&store)).unwrap().is_empty());
        assert!(view.summary.text().contains(copy("summary_empty")));
        view.analyze(true);
        settle();
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
