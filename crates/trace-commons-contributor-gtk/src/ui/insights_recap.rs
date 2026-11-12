//! The live feed's core-owned closed-week recap. No notification or network IO.
use super::*;
use chrono::NaiveDate;
use serde_json::Value;
use trace_commons_contributor::insights::{
    goals::WeekFigures,
    recap::{Recap, RecapItem, WeekComparisons, counter_weeks_valid},
};

fn request(value: &Value) -> Option<Op> {
    let overview = super::super::live::live_overview(value)?;
    let weeks: Vec<WeekFigures> = serde_json::from_value(value.get("history")?.clone()).ok()?;
    if !counter_weeks_valid(&weeks) {
        return None;
    }
    Some(Op::Comparisons {
        counter_weeks: Some(weeks),
        week_start: Some(overview.week_start),
        tz: local_offset(),
        recap_card_enabled: value["recap_card_enabled"] == true,
    })
}
fn accepted(comparisons: WeekComparisons, week: NaiveDate) -> Option<Recap> {
    (comparisons.feed == Feed::CounterPass && comparisons.week_start == week)
        .then_some(comparisons.recap)
        .flatten()
}
fn percent(value: u64) -> String {
    ((u128::from(value) + 5) / 10).to_string()
}
fn lines(recap: &Recap) -> Vec<String> {
    let mut lines = Vec::new();
    for source in &recap.sources {
        lines.push(figure(Some(source.tokens)));
        lines.push(source_line(source.source));
        if let Some(change) = source.change_permille {
            lines.push(fill(
                &text(if change < 0 {
                    "analytics_recap_fewer"
                } else {
                    "analytics_recap_more"
                }),
                &[
                    ("n", &recap.sessions.to_string()),
                    ("p", &percent(change.unsigned_abs())),
                ],
            ));
        }
    }
    for item in &recap.items {
        // Native no longer presents pattern observations in the recap.
        if let RecapItem::BestCacheWeek {
            permille,
            previous_best_permille,
            ..
        } = item
        {
            lines.push(fill(
                &text("analytics_recap_best_cache"),
                &[
                    ("p", &percent(*permille)),
                    ("q", &percent(*previous_best_permille)),
                ],
            ));
        }
    }
    if let Some(past) = recap.past_threshold {
        lines.push(fill(
            &text("analytics_recap_threshold"),
            &[
                ("n", &past.sessions.to_string()),
                ("threshold", &figure(Some(past.threshold))),
            ],
        ));
    }
    lines
}

fn execute(store_dir: Option<PathBuf>, operation: Op) -> async_channel::Receiver<Option<Response>> {
    let (tx, rx) = async_channel::bounded(1);
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
    rx
}

impl AnalyticsTabs {
    pub(super) fn load_recap(self: &Rc<Self>, value: &Value, generation: u64) {
        let Some(operation) = request(value) else {
            return;
        };
        let Op::Comparisons {
            week_start: Some(week),
            ..
        } = operation
        else {
            return;
        };
        let slot = gtk::Box::new(gtk::Orientation::Vertical, metrics::HEADING_GAP);
        slot.set_visible(false);
        self.overview_body.append(&slot);
        let body = slot.downgrade();
        let rx = execute(self.store_dir.clone(), operation);
        let weak = Rc::downgrade(self);
        gtk::glib::spawn_future_local(async move {
            let result = rx.recv().await.ok().flatten();
            let (Some(tabs), Some(body)) = (weak.upgrade(), body.upgrade()) else {
                return;
            };
            if !tabs.recap_current(generation) {
                return;
            }
            let Some(Response::Comparisons { comparisons }) = result else {
                return;
            };
            if let Some(recap) = accepted(*comparisons, week) {
                tabs.present_recap(&body, &recap, generation);
                body.set_visible(true);
            }
        });
    }
    fn recap_current(&self, generation: u64) -> bool {
        accept_live_result(
            self.active.get(),
            self.shown.get(),
            self.generation(Slot::Overview).get(),
            generation,
        )
    }
    fn present_recap(self: &Rc<Self>, body: &gtk::Box, recap: &Recap, generation: u64) {
        let card = card_box();
        card.append(&styled(
            &fill(
                &text("analytics_your_week"),
                &[("range", &week_range(recap.week_start))],
            ),
            "title-2",
        ));
        for line in lines(recap) {
            card.append(&label(&line));
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, metrics::HEADING_GAP);
        let open = gtk::Button::with_label(&text("analytics_recap_open"));
        open.add_css_class("suggested-action");
        let hide = gtk::Button::with_label(&text("analytics_recap_turn_off"));
        hide.add_css_class("flat");
        actions.append(&open);
        actions.append(&hide);
        card.append(&actions);
        body.append(&card);
        let busy = Rc::new(Cell::new(false));
        let week = recap.week_start;
        let weak = Rc::downgrade(self);
        let controls = actions.downgrade();
        let flight = busy.clone();
        open.connect_clicked(move |_| {
            let Some(tabs) = weak.upgrade() else { return };
            if !tabs.recap_current(generation) || flight.replace(true) { return; }
            if let Some(controls) = controls.upgrade() { controls.set_sensitive(false); }
            let rx = execute(tabs.store_dir.clone(), Op::RecapOpened { week_start: week });
            let weak = weak.clone();
            let controls = controls.clone();
            let flight = flight.clone();
            gtk::glib::spawn_future_local(async move {
                let result = rx.recv().await.ok().flatten();
                flight.set(false);
                let Some(tabs) = weak.upgrade() else { return };
                if !tabs.recap_current(generation) { return; }
                if let Some(controls) = controls.upgrade() { controls.set_sensitive(true); }
                if matches!(result, Some(Response::RecapOpened { state }) if state.recap_opened_week == Some(week)) {
                    tabs.overview_week.set(Some(week));
                    tabs.load(Tab::Overview);
                }
            });
        });
        let weak = Rc::downgrade(self);
        let controls = actions.downgrade();
        hide.connect_clicked(move |_| {
            let Some(tabs) = weak.upgrade() else { return };
            if !tabs.recap_current(generation) || busy.get() {
                return;
            }
            let Some(app) = tabs.app.borrow().upgrade() else {
                return;
            };
            busy.set(true);
            if let Some(controls) = controls.upgrade() {
                controls.set_sensitive(false);
            }
            let weak = weak.clone();
            let controls = controls.clone();
            let busy = busy.clone();
            app.call(
                "set_settings",
                serde_json::json!({"insights_recap_card_enabled":false}),
                move |app, result| {
                    busy.set(false);
                    let Some(tabs) = weak.upgrade() else { return };
                    if !tabs.recap_current(generation) {
                        return;
                    }
                    if let Some(controls) = controls.upgrade() {
                        controls.set_sensitive(true);
                    }
                    if result.is_ok() {
                        tabs.load(Tab::Overview);
                    } else {
                        app.toast(crate::copy::KNOB_NOT_CHANGED);
                    }
                },
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_contributor::insights::{
        goal_store::GoalState,
        recap::{ComparisonsRequest, week_comparisons},
    };
    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()
    }
    fn comparisons(weeks: Option<&[WeekFigures]>) -> WeekComparisons {
        week_comparisons(
            weeks,
            ComparisonsRequest {
                week_start: monday(),
                current_week: monday(),
                recap_card_enabled: true,
            },
            &GoalState::default(),
        )
    }
    #[test]
    fn missing_partial_and_current_week_only_have_no_recap() {
        assert!(comparisons(None).recap.is_none());
        let mut week = WeekFigures {
            week_start: monday(),
            comparable: true,
            ..WeekFigures::default()
        };
        assert!(comparisons(Some(&[week.clone()])).recap.is_none());
        week.week_start -= chrono::Duration::weeks(1);
        week.comparable = false;
        assert!(comparisons(Some(&[week])).recap.is_none());
    }
    #[test]
    fn response_requires_same_live_week_and_navigation_rejects_stale_work() {
        let week = WeekFigures {
            week_start: monday() - chrono::Duration::weeks(1),
            comparable: true,
            ..WeekFigures::default()
        };
        let result = comparisons(Some(&[week]));
        assert!(accepted(result.clone(), monday()).is_some());
        assert!(accepted(result.clone(), monday() - chrono::Duration::weeks(1)).is_none());
        let mut saved = result;
        saved.feed = Feed::Saved;
        assert!(accepted(saved, monday()).is_none());
        assert!(!accept_live_result(false, Tab::Overview, 3, 3));
        assert!(!accept_live_result(true, Tab::Patterns, 3, 3));
        assert!(!accept_live_result(true, Tab::Overview, 4, 3));
    }
    #[test]
    fn malformed_or_saved_response_never_starts_comparison_read() {
        assert!(request(&serde_json::json!({})).is_none());
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/insights-analytics/week_overview.json"
        ))
        .unwrap();
        let mut overview = fixture["overview"].clone();
        overview["feed"] = serde_json::json!("counter_pass");
        let mut value = serde_json::json!({"enabled":true,"readable":true,"feed":"counter_pass","rollup":{},"overview":overview,"history":[],"recap_card_enabled":true});
        assert!(request(&value).is_some());
        value["history"] = Value::Null;
        assert!(request(&value).is_none());
        value["history"] = serde_json::json!([]);
        value["feed"] = serde_json::json!("saved");
        assert!(request(&value).is_none());
    }
    #[test]
    fn unknown_comparison_has_no_change_line_and_harnesses_are_not_summed() {
        let recap = Recap {
            week_start: monday(),
            week_end: monday() + chrono::Duration::days(6),
            sessions: 2,
            sources: vec![trace_commons_contributor::insights::recap::RecapSource {
                source: trace_commons_contributor::insights::week_rollup::AnalyticsSource::Codex,
                tokens: 0,
                change_permille: None,
            }],
            items: vec![],
            past_threshold: None,
        };
        assert_eq!(lines(&recap).len(), 2);
        assert_eq!(lines(&recap)[0], "0");
    }
}
