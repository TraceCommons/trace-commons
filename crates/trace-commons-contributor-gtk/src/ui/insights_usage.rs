//! Passive cached subscription and NEAR AI usage presentation.

use super::live::{card, dollars, label, text};
use adw::prelude::*;
use serde_json::Value;

#[derive(Debug)]
struct UsageCard {
    title: String,
    rows: Vec<(String, String)>,
    notes: Vec<String>,
    days: Vec<(String, Option<f64>, String)>,
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn unknown() -> String {
    text("analytics_unavailable")
}
fn number(value: Option<u64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_else(unknown)
}
fn money(value: Option<u64>) -> String {
    value.map(dollars).unwrap_or_else(unknown)
}
fn day_money(value: &Value) -> Option<u64> {
    value["usd_micros"].as_u64().or_else(|| {
        (value["usd_partial"] == true)
            .then(|| value["usd_micros_known"].as_u64())
            .flatten()
    })
}
fn reset(window: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(window["resets_at"].as_str()?)
        .ok()
        .map(|t| t.timestamp())
}
fn live_window(window: &Value, now: i64) -> bool {
    window["used_permille"].as_u64().is_some_and(|v| v <= 1000)
        && (window["resets_at"].is_null() || reset(window).is_some_and(|t| t > now))
}
fn weekly(window: &Value) -> bool {
    window["lane"] != "model"
        && (window["duration_mins"] == 10080
            || (window["duration_mins"].is_null() && window["lane"] == "weekly"))
}
fn session(window: &Value) -> bool {
    window["lane"] != "model"
        && !weekly(window)
        && (window["lane"] == "session"
            || window["duration_mins"].as_u64().is_some_and(|n| n < 10080))
}
fn remaining(window: &Value) -> String {
    window["used_permille"]
        .as_u64()
        .filter(|v| *v <= 1000)
        .map(|used| format!("{}%", (1000 - used + 5) / 10))
        .unwrap_or_else(unknown)
}
fn duration(seconds: i64) -> String {
    let minutes = seconds.max(60) / 60;
    if minutes >= 1440 {
        format!("{}d {}h", minutes / 1440, minutes % 1440 / 60)
    } else if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}
fn pace(window: &Value, now: i64) -> Option<String> {
    let used = window["used_permille"].as_u64()? as f64 / 1000.;
    let until = (reset(window)? - now) as f64;
    let length = window["duration_mins"].as_u64()? as f64 * 60.;
    let elapsed = length - until;
    if length <= 0. || until <= 0. || elapsed < length * 0.05 {
        return None;
    }
    let runout = if used == 0. {
        f64::INFINITY
    } else {
        (1. - used) * elapsed / used
    };
    Some(if runout < until {
        text("analytics_limits_headline_runs_out")
            .replace(
                "{window}",
                &text(if weekly(window) {
                    "analytics_subscriptions_weekly"
                } else {
                    "analytics_subscriptions_session"
                }),
            )
            .replace("{time}", &duration(runout as i64))
    } else {
        text("analytics_limits_headline_on_pace")
    })
}
fn cards(week: &Value, subscriptions: &Value, now: i64) -> Vec<UsageCard> {
    let mut cards = Vec::new();
    for provider in array(&subscriptions["providers"]) {
        let Some(name @ ("codex" | "claude")) = provider["provider"].as_str() else {
            continue;
        };
        if provider["enabled"] != true {
            continue;
        }
        let logins = array(&provider["logins"]);
        for login in logins {
            let title = text(&format!("analytics_subscriptions_provider_{name}"));
            let title = if logins.len() > 1 {
                format!(
                    "{title} · {}",
                    if login["default_login"] == true {
                        text("analytics_subscriptions_default_login")
                    } else {
                        login["label"].as_str().unwrap_or("").to_owned()
                    }
                )
            } else {
                title
            };
            let windows = array(&login["windows"]);
            let measured = login["state"] == "observed" && !windows.is_empty();
            let mut card = UsageCard {
                title,
                rows: vec![],
                notes: vec![],
                days: vec![],
            };
            let mut pace_window = None;
            for (predicate, left_key, reset_key) in [
                (
                    session as fn(&Value) -> bool,
                    "analytics_limits_session_left",
                    "analytics_usage_session_resets",
                ),
                (
                    weekly as fn(&Value) -> bool,
                    "analytics_limits_weekly_left",
                    "analytics_usage_weekly_resets",
                ),
            ] {
                if measured && !windows.iter().any(predicate) {
                    continue;
                }
                let live = windows
                    .iter()
                    .find(|w| measured && predicate(w) && live_window(w, now));
                card.rows
                    .push((text(left_key), live.map(remaining).unwrap_or_else(unknown)));
                if measured {
                    card.rows.push((
                        text(reset_key),
                        live.and_then(reset)
                            .map(|t| duration(t - now))
                            .unwrap_or_else(unknown),
                    ));
                }
                if let Some(window) = live {
                    pace_window = Some(window);
                }
            }
            if let Some(headline) = pace_window.and_then(|window| pace(window, now)) {
                card.notes.push(headline);
            } else {
                card.rows.push((text("analytics_limits_pace"), unknown()));
            }
            if let Some(banked) = login["banked_resets"].as_u64() {
                card.rows
                    .push((text("analytics_subscriptions_banked"), banked.to_string()));
            }
            if !measured {
                card.notes.push(text("analytics_subscriptions_not_checked"));
            }
            if login["stale"] == true && login["reason"] != "captured_on_refresh" {
                card.notes.push(text(if login["reason"].is_null() {
                    "analytics_subscriptions_stale"
                } else {
                    "analytics_subscriptions_cached"
                }));
            }
            for day in array(&week["usage_by_day"]) {
                let amount = day[if name == "claude" {
                    "claude_code"
                } else {
                    "codex"
                }]
                .as_u64();
                card.days.push((
                    day["date"].as_str().unwrap_or("").into(),
                    amount.map(|v| v as f64),
                    number(amount),
                ));
            }
            cards.push(card);
        }
    }
    let credit = &week["near_ai_credit"];
    if credit["last_31_days"]["calls"]
        .as_u64()
        .is_some_and(|calls| calls > 0)
    {
        let mut card = UsageCard {
            title: text("analytics_usage_near_ai"),
            rows: vec![],
            notes: vec![text("analytics_credit_scope_near_ai")],
            days: vec![],
        };
        for (field, key) in [
            ("week", "analytics_this_week"),
            ("today", "analytics_today"),
            ("last_31_days", "analytics_usage_last_31_days"),
        ] {
            let value = &credit[field];
            let spend = money(
                value["usd_micros"]
                    .as_u64()
                    .or(value["usd_micros_known"].as_u64()),
            );
            let tokens = value["tokens"].as_u64().or(value["tokens_known"].as_u64());
            let tokens = tokens
                .map(|v| text("analytics_usage_tokens").replace("{t}", &v.to_string()))
                .unwrap_or_else(unknown);
            card.rows.push((text(key), format!("{spend} · {tokens}")));
        }
        for day in array(&credit["by_day"]) {
            let amount = day_money(day);
            card.days.push((
                day["date"].as_str().unwrap_or("").into(),
                amount.map(|v| v as f64 / 1_000_000.),
                money(
                    day["usd_micros"]
                        .as_u64()
                        .or(day["usd_micros_known"].as_u64()),
                ),
            ));
        }
        cards.push(card);
    }
    cards
}

pub(super) fn append(body: &gtk::Box, week: &Value, subscriptions: &Value) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 12);
    render(&section, week, subscriptions);
    body.append(&section);
    let weak = section.downgrade();
    let week = week.clone();
    let subscriptions = subscriptions.clone();
    // Re-evaluate reset windows while held on screen without provider IO.
    gtk::glib::timeout_add_local(std::time::Duration::from_secs(60), move || {
        let Some(section) = weak.upgrade() else {
            return gtk::glib::ControlFlow::Break;
        };
        while let Some(child) = section.first_child() {
            section.remove(&child);
        }
        render(&section, &week, &subscriptions);
        gtk::glib::ControlFlow::Continue
    });
}

fn render(body: &gtk::Box, week: &Value, subscriptions: &Value) {
    let cards = cards(week, subscriptions, chrono::Utc::now().timestamp());
    if cards.is_empty() {
        return;
    }
    body.append(&label(&text("analytics_usage_title"), "heading"));
    for item in cards {
        let view = card(&item.title);
        for note in item.notes {
            view.append(&label(&note, "caption"));
        }
        for (name, value) in item.rows {
            view.append(&label(&format!("{name}  {value}"), "body"));
        }
        if !item.days.is_empty() {
            view.append(&label(&text("analytics_usage_last_7_days"), "caption"));
            let maximum = item
                .days
                .iter()
                .filter_map(|d| d.1)
                .fold(0_f64, f64::max)
                .max(1.);
            for (date, amount, words) in item.days {
                view.append(&label(&format!("{date}  {words}"), "caption"));
                if let Some(amount) = amount {
                    let bar = gtk::LevelBar::for_interval(0., maximum);
                    for offset in ["low", "high", "full"] {
                        bar.remove_offset_value(Some(offset));
                    }
                    bar.set_value(amount);
                    bar.update_property(&[gtk::accessible::Property::Label(&format!(
                        "{date}: {words}"
                    ))]);
                    view.append(&bar);
                }
            }
        }
        body.append(&view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn disabled_providers_and_unknown_daily_tokens_remain_distinct() {
        let mut usage = json!({"providers":[{"provider":"codex","enabled":false,"logins":[{"state":"unknown","windows":[]}]}]});
        assert!(cards(&json!({}), &usage, 0).is_empty());
        usage["providers"][0]["enabled"] = json!(true);
        let cards = cards(
            &json!({"usage_by_day":[{"date":"2026-11-10","codex":null},{"date":"2026-11-11","codex":0}]}),
            &usage,
            0,
        );
        assert_eq!(cards[0].days[0].1, None);
        assert_eq!(cards[0].days[1].1, Some(0.));
    }
    #[test]
    fn pace_needs_enough_elapsed_time_and_measured_session_omits_weekly() {
        let window = json!({"lane":"session","duration_mins":60,"used_permille":900,"resets_at":"1970-01-01T01:00:00Z"});
        assert!(pace(&window, 10).is_none());
        assert!(pace(&window, 1800).unwrap().contains("before"));
        let usage = json!({"providers":[{"provider":"codex","enabled":true,"logins":[{"state":"observed","windows":[window]}]}]});
        let cards = cards(&json!({}), &usage, 1800);
        assert!(
            !cards[0]
                .rows
                .iter()
                .any(|r| r.0 == text("analytics_limits_weekly_left"))
        );
        assert_eq!(cards[0].rows[0].1, "10%");
    }
    #[test]
    fn weekly_window_with_unknown_pace_never_borrows_session_forecast() {
        let usage = json!({"providers":[{"provider":"codex","enabled":true,"logins":[{"state":"observed","windows":[
            {"lane":"session","duration_mins":60,"used_permille":900,"resets_at":"1970-01-01T01:00:00Z"},
            {"lane":"weekly","used_permille":100,"resets_at":"1970-01-02T00:00:00Z"}
        ]}]}]});
        let cards = cards(&json!({}), &usage, 1800);
        assert!(cards[0].notes.is_empty());
        assert!(
            cards[0]
                .rows
                .contains(&(text("analytics_limits_pace"), unknown()))
        );
    }
    #[test]
    fn unknown_money_is_not_measured_zero_and_partial_days_use_known_part() {
        assert_eq!(day_money(&json!({})), None);
        assert_eq!(day_money(&json!({"usd_micros":0})), Some(0));
        assert_eq!(day_money(&json!({"usd_micros_known":42})), None);
        assert_eq!(
            day_money(&json!({"usd_micros_known":42,"usd_partial":true})),
            Some(42)
        );
        assert_eq!(day_money(&json!({"usd_micros":-1})), None);
    }
    #[test]
    fn inactive_credit_and_unknown_providers_are_hidden() {
        assert!(
            cards(
                &json!({"near_ai_credit":{"last_31_days":{"calls":0}}}),
                &json!({"providers":[{"provider":"other","logins":[{}]}]}),
                0
            )
            .is_empty()
        );
        let cards = cards(
            &json!({"near_ai_credit":{"last_31_days":{"calls":1}}}),
            &json!({}),
            0,
        );
        assert_eq!(cards.len(), 1);
        assert!(cards[0].days.is_empty());
    }
    #[test]
    fn unmeasured_login_keeps_unknown_rows_and_expired_windows_are_unknown() {
        let usage = json!({"providers":[{"provider":"codex","enabled":true,"logins":[{"state":"not_observed","windows":[]}]}]});
        let rows = cards(&json!({}), &usage, 100);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rows[0].1, text("analytics_unavailable"));
        let window =
            json!({"lane":"weekly","used_permille":250,"resets_at":"1970-01-01T00:00:01Z"});
        assert!(!live_window(&window, 100));
        assert!(live_window(&window, 0));
        assert_eq!(remaining(&window), "75%");
    }
}
