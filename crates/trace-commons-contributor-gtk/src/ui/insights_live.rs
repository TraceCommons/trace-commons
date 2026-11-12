//! Read-only presentation of the daemon's live weekly work figures.
//! Unknown or malformed figures are omitted, never replaced with zero.
use super::super::style::brand_tokens::insights as metrics;
use adw::prelude::*;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};
use trace_commons_contributor::insights::{service, week_glance::WeekOverview, week_rollup::Feed};

fn text(key: &str) -> String {
    static COPY: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    COPY.get_or_init(service::ui_copy)
        .get(key)
        .cloned()
        .unwrap_or_default()
}

/// Match the native live-feed gate before decoding the shared overview shape.
pub(crate) fn live_overview(value: &Value) -> Option<WeekOverview> {
    if value["enabled"] != true
        || value["readable"] != true
        || value["feed"] != "counter_pass"
        || !value["rollup"].is_object()
    {
        return None;
    }
    let overview: WeekOverview = serde_json::from_value(value.get("overview")?.clone()).ok()?;
    (overview.feed == Feed::CounterPass).then_some(overview)
}

#[derive(Clone, Deserialize)]
struct Totals {
    priced_sessions: u64,
    #[serde(default)]
    near_ai_sessions: Option<u64>,
    usd_micros_known: u64,
    usd_partial: bool,
}
impl Totals {
    fn known(&self) -> bool {
        self.priced_sessions > 0
            || self.near_ai_sessions.unwrap_or(0) > 0
            || self.usd_micros_known > 0
    }
}
#[derive(Deserialize)]
struct Model {
    key: String,
    tool: String,
    usd_micros_known: u64,
    usd_partial: bool,
}
#[derive(Deserialize)]
struct Tool {
    key: Option<String>,
    sessions: u64,
}
#[derive(Deserialize)]
struct Work {
    basis: String,
    week: Totals,
    last_week: Option<Totals>,
    by_model: Vec<Model>,
    #[serde(default)]
    by_tool: Vec<Tool>,
}

/// Match native precision without floating-point money arithmetic. Values under
/// one cent retain up to four decimals; under 100 microdollars keep one
/// significant digit. Currency is USD, as the daemon's pricing basis specifies.
fn dollars(micros: u64) -> String {
    if micros >= 10_000 {
        let cents = (u128::from(micros) + 5_000) / 10_000;
        return format!("${}.{:02}", cents / 100, cents % 100);
    }
    if micros > 0 && micros < 100 {
        let rounded = if micros < 10 {
            micros
        } else {
            ((micros + 5) / 10) * 10
        };
        let fraction = format!("{rounded:06}");
        return format!("$0.{}", fraction.trim_end_matches('0'));
    }
    let units = (micros + 50) / 100;
    let fraction = format!("{units:04}");
    let fraction = fraction.trim_end_matches('0');
    format!("$0.{:0<2}", fraction)
}

fn spend_change(work: &Work) -> Option<String> {
    let last = work.last_week.as_ref()?;
    if work.week.usd_partial
        || last.usd_partial
        || work.week.priced_sessions == 0
        || last.usd_micros_known == 0
        || !work.week.known()
        || !last.known()
    {
        return None;
    }
    let now = work.week.usd_micros_known;
    let before = last.usd_micros_known;
    let percent =
        (u128::from(now.abs_diff(before)) * 100 + u128::from(before) / 2) / u128::from(before);
    if percent == 0 {
        return None;
    }
    Some(
        text(if now > before {
            "analytics_change_up"
        } else {
            "analytics_change_down"
        })
        .replace("{p}", &percent.to_string()),
    )
}

fn tool_name(tool: &str) -> String {
    let name = text(&format!("analytics_credit_tool_{tool}"));
    if name.is_empty() {
        text("analytics_unknown_label")
    } else {
        name
    }
}

fn api_prices(work: &Work) -> String {
    let mut tools: Vec<_> = work.by_model.iter().map(|row| row.tool.as_str()).collect();
    tools.extend(
        work.by_tool
            .iter()
            .filter(|tool| tool.sessions > 0)
            .filter_map(|tool| tool.key.as_deref()),
    );
    tools.sort_unstable();
    tools.dedup();
    text(match tools.as_slice() {
        ["claude_code"] => "analytics_tile_api_prices_claude",
        ["codex"] => "analytics_tile_api_prices_codex",
        _ => "analytics_tile_api_prices",
    })
}

fn label(value: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class(class);
    label
}
fn card(title: &str) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, metrics::HEADING_GAP);
    card.add_css_class("tc-insights-card");
    card.append(&label(title, "heading"));
    card
}

/// The only inputs are the daemon's returned totals; entering this view never
/// refreshes accounts, starts providers, or calculates prices from tokens.
pub(super) fn append_work_cards(body: &gtk::Box, response: &Value) {
    let Some(work) = response
        .get("work")
        .and_then(|v| serde_json::from_value::<Work>(v.clone()).ok())
    else {
        return;
    };
    if work.basis != "api_list" {
        return;
    }
    let prices = api_prices(&work);
    if work.week.known() {
        let spend = card(&text("analytics_tile_spend"));
        spend.append(&label(&dollars(work.week.usd_micros_known), "title-3"));
        spend.append(&label(&prices, "caption"));
        if work.week.usd_partial {
            spend.append(&label(&text("analytics_state_partial"), "caption"));
        }
        body.append(&spend);
    }
    let mut models: Vec<_> = work
        .by_model
        .iter()
        .filter(|m| m.usd_micros_known > 0)
        .collect();
    models.sort_by(|a, b| {
        b.usd_micros_known
            .cmp(&a.usd_micros_known)
            .then_with(|| a.key.cmp(&b.key))
    });
    let total: u128 = models.iter().map(|m| u128::from(m.usd_micros_known)).sum();
    if let Some(top) = models.first() {
        let percent = (u128::from(top.usd_micros_known) * 100 + total / 2) / total;
        let top_card = card(&text("analytics_tile_top_model"));
        top_card.append(&label(&top.key, "title-3"));
        let detail = text("analytics_tile_share_of_spend").replace("{p}", &percent.to_string());
        top_card.append(&label(&detail, "caption"));
        top_card.append(&label(&tool_name(&top.tool), "caption"));
        body.append(&top_card);
    }
    if !models.is_empty() {
        let rows = card(&text("analytics_by_model_title"));
        rows.append(&label(&prices, "caption"));
        for model in models {
            let row = gtk::Box::new(gtk::Orientation::Vertical, metrics::LINE_GAP);
            row.append(&label(&model.key, "heading"));
            row.append(&label(&tool_name(&model.tool), "caption"));
            row.append(&label(&dollars(model.usd_micros_known), "body"));
            if model.usd_partial {
                row.append(&label(&text("analytics_state_partial"), "caption"));
            }
            let bar = gtk::LevelBar::for_interval(0.0, 1.0);
            for offset in ["low", "high", "full"] {
                bar.remove_offset_value(Some(offset));
            }
            bar.set_value(model.usd_micros_known as f64 / total as f64);
            row.append(&bar);
            rows.append(&row);
        }
        body.append(&rows);
    }
    if let Some(change) = spend_change(&work) {
        let trends = card(&text("analytics_trends_title"));
        trends.append(&label(&text("analytics_tile_spend"), "heading"));
        trends.append(&label(&change, "body"));
        body.append(&trends);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn totals(micros: u64) -> Totals {
        Totals {
            priced_sessions: 1,
            near_ai_sessions: None,
            usd_micros_known: micros,
            usd_partial: false,
        }
    }
    fn work(now: u64, last: u64) -> Work {
        Work {
            basis: "api_list".into(),
            week: totals(now),
            last_week: Some(totals(last)),
            by_model: vec![],
            by_tool: vec![],
        }
    }
    #[test]
    fn unknown_is_not_measured_zero() {
        let mut value = totals(0);
        assert!(value.known());
        value.priced_sessions = 0;
        assert!(!value.known());
        value.usd_micros_known = 1;
        assert!(value.known());
    }
    #[test]
    fn money_keeps_small_nonzero_values() {
        for (micros, expected) in [
            (0, "$0.00"),
            (1, "$0.000001"),
            (14, "$0.00001"),
            (99, "$0.0001"),
            (100, "$0.0001"),
            (1234, "$0.0012"),
            (9999, "$0.01"),
            (10_000, "$0.01"),
            (1_234_567, "$1.23"),
        ] {
            assert_eq!(dollars(micros), expected);
        }
    }
    #[test]
    fn partial_and_missing_baselines_never_produce_trends() {
        let mut value = work(2_000_000, 1_000_000);
        assert_eq!(
            spend_change(&value),
            Some(text("analytics_change_up").replace("{p}", "100"))
        );
        value.week.usd_partial = true;
        assert!(spend_change(&value).is_none());
        value.week.usd_partial = false;
        value.last_week.as_mut().unwrap().usd_partial = true;
        assert!(spend_change(&value).is_none());
        value.last_week = None;
        assert!(spend_change(&value).is_none());
        assert!(spend_change(&work(1, 0)).is_none());
        assert!(spend_change(&work(1, 1)).is_none());
    }
    #[test]
    fn live_gate_requires_readable_counter_pass_and_shared_overview() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/insights-analytics/week_overview.json"
        ))
        .unwrap();
        let mut overview = fixture["overview"].clone();
        overview["feed"] = serde_json::json!("counter_pass");
        let accepted = serde_json::json!({"enabled":true,"readable":true,"feed":"counter_pass","rollup":{},"overview":overview});
        assert!(live_overview(&accepted).is_some());
        for (key, value) in [
            ("enabled", serde_json::json!(false)),
            ("readable", serde_json::json!(false)),
            ("feed", serde_json::json!("saved")),
            ("rollup", Value::Null),
        ] {
            let mut rejected = accepted.clone();
            rejected[key] = value;
            assert!(live_overview(&rejected).is_none(), "{key}");
        }
        let mut mismatch = accepted;
        mismatch["overview"]["feed"] = serde_json::json!("saved");
        assert!(live_overview(&mismatch).is_none());
    }

    #[test]
    fn malformed_or_disabled_live_data_is_not_accepted() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"enabled":true,"readable":true,"feed":"counter_pass","rollup":{},"overview":{}}),
        ] {
            assert!(live_overview(&value).is_none());
        }
        assert!(
            serde_json::from_value::<Totals>(
                serde_json::json!({"priced_sessions":1,"usd_micros_known":-1,"usd_partial":false})
            )
            .is_err()
        );
    }
}
