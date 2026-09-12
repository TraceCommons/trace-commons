//! Account-free selected-file Insights. No worker, configuration or discovery.
//! A single bounded IO request runs on an OS thread. Cancellation suppresses
//! presentation; already-started writes finish. Weak callbacks never own a view.
use adw::prelude::*;
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use trace_commons_contributor::insights::service::{
    self, LocalInsightsOperation as Op, LocalInsightsRequest, LocalInsightsResponse as Response,
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
    saved: gtk::Box,
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
        content.append(&label(copy("saved")));
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
            saved,
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
                v.request(Op::List {}, false);
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

    fn request(self: &Rc<Self>, operation: Op, persisted: bool) {
        if !self.flight.borrow_mut().begin() {
            return;
        }
        self.controls.set_sensitive(false);
        self.assessment.set_sensitive(false);
        self.saved.set_sensitive(false);
        self.status.set_text(copy("working"));
        let refresh_saved = matches!(
            &operation,
            Op::Analyze { save: true, .. } | Op::Annotate { .. } | Op::ClearAnnotation { .. }
        );
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
                }
                return;
            }
            view.controls.set_sensitive(true);
            view.assessment.set_sensitive(true);
            view.saved.set_sensitive(true);
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
                        view.request(Op::List {}, false);
                    }
                }
                Some(Response::List { insights }) => {
                    view.render_saved(insights);
                    view.status.set_text(copy("refreshed"));
                }
                Some(Response::Delete { deleted }) => {
                    view.detail.set_text("");
                    view.assessment.set_visible(false);
                    *view.current_id.borrow_mut() = None;
                    view.status.set_text(if deleted {
                        copy("deleted")
                    } else {
                        copy("already_absent")
                    });
                    view.request(Op::List {}, false);
                }
                _ => view.status.set_text(copy("error")),
            }
        });
    }

    fn render_saved(self: &Rc<Self>, insights: Vec<LocalInsight>) {
        let current = self.current_id.borrow().clone();
        if let Some(id) = current {
            if let Some(insight) = insights.iter().find(|insight| insight.id == id) {
                self.detail.set_text(&render(insight));
                self.category.set_selected(
                    insight
                        .manual_annotation
                        .as_ref()
                        .map(|a| category_index(a.category))
                        .unwrap_or(0),
                );
                self.outcome.set_selected(
                    insight
                        .manual_annotation
                        .as_ref()
                        .map(|a| outcome_index(a.outcome))
                        .unwrap_or(0),
                );
            } else {
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
                    v.request(Op::Explain { id: id.clone() }, true);
                }
            });
            let weak = Rc::downgrade(self);
            delete.connect_clicked(move |_| {
                if let Some(v) = weak.upgrade() {
                    v.request(
                        Op::Delete {
                            id: insight.id.clone(),
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
        let key = match metric.id {
            MetricId::Sessions => "metric_sessions",
            MetricId::Events => "metric_events",
            MetricId::InputTokens => "metric_input_tokens",
            MetricId::OutputTokens => "metric_output_tokens",
            MetricId::ToolCalls => "metric_tool_calls",
            MetricId::ToolFailures => "metric_tool_failures",
            MetricId::KnownOutcomes => "metric_known_outcomes",
        };
        text.push_str(&format!(
            "{}: {} · {} {} / {}\n",
            copy(key),
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
            view.saved.first_child().is_some(),
            "save refreshes saved rows immediately"
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
        view.request(Op::Delete { id }, false);
        settle();
        assert!(file.exists());
        assert!(service::list_saved(Some(&store)).unwrap().is_empty());
        view.analyze(true);
        settle();
        let id = view.current_id.borrow().clone().unwrap();
        service::open_store(Some(&store))
            .unwrap()
            .delete(&id)
            .unwrap();
        view.request(Op::List {}, false);
        settle();
        assert!(view.current_id.borrow().is_none());
        assert!(view.detail.text().is_empty());
        view.analyze(false);
        settle();
        let preview = view.detail.text();
        view.request(Op::List {}, false);
        settle();
        assert_eq!(
            view.detail.text(),
            preview,
            "history refresh preserves unsaved preview"
        );
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
        view.request(Op::List {}, false);
        window.hide();
        settle();
        assert_eq!(view.detail.text(), before);
        window.present();
        assert!(view.controls.is_sensitive());
        view.request(Op::List {}, false);
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
