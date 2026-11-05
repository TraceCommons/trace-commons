//! Notification messages selected from facts at the canonical renderer, not
//! reverse-engineered from completed English. IDs, subjects and tool names
//! remain raw data. Existing English fields remain available to older shells.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use serde_json::{Value, json};
use trace_commons_protocol::settlement_estimate::SettlementTotal;

use super::{english, plural, text};
use crate::{nudge_copy as copy, nudge_render as render};
use render::{Batch, Verdicts};

macro_rules! line {
    ($name:ident, $parameters:expr) => {
        text(
            concat!("notification.", stringify!($name)),
            copy::$name,
            $parameters,
        )
    };
    ($name:ident) => {
        line!($name, json!({}))
    };
}

macro_rules! counted {
    ($many:ident, $one:ident, $count:expr, $parameter:literal, $values:expr) => {
        plural(
            concat!("notification.", stringify!($many)),
            &copy::$one.replace(concat!("{", $parameter, "}"), "{count}"),
            &copy::$many.replace(concat!("{", $parameter, "}"), "{count}"),
            $count,
            $values,
        )
    };
}

/// Joining is an explicit message too: translators can reorder clauses,
/// while inserted subjects can never become templates. No English matching.
pub fn join(first: Value, second: Value, newline: bool) -> Value {
    if first
        .as_str()
        .map_or_else(|| english(&first).is_empty(), str::is_empty)
    {
        return second;
    }
    if second
        .as_str()
        .map_or_else(|| english(&second).is_empty(), str::is_empty)
    {
        return first;
    }
    text(
        if newline {
            "notification.join.lines"
        } else {
            "notification.join.sentences"
        },
        if newline {
            "{first}\n{second}"
        } else {
            "{first} {second}"
        },
        json!({"first": first, "second": second}),
    )
}

fn empty() -> Value {
    json!("")
}

fn decimal(value: String) -> Value {
    let digits = value.split_once('.').map_or(0, |(_, tail)| tail.len());
    json!({"decimal": value, "english": value,
        "minimum_fraction_digits": digits, "maximum_fraction_digits": digits})
}

fn usd(micros: u64) -> Value {
    // Match the canonical integer-only truncation, including whole dollars.
    let original = render::usd_text(micros);
    let digits = if original.contains('.') { 2 } else { 0 };
    json!({"currency": original.trim_start_matches('$'), "code": "USD", "english": original,
        "minimum_fraction_digits": digits, "maximum_fraction_digits": digits})
}

fn days(days: u32) -> Value {
    // Canonical copy::days_phrase owns these words, including zero.
    plural(
        "notification.days",
        &copy::days_phrase(1),
        &copy::days_phrase(2).replace('2', "{count}"),
        u64::from(days),
        json!({}),
    )
}

fn tools(tools: &[String]) -> Value {
    let cap = |s: &str| {
        s.chars()
            .take(render::TOOL_NAME_MAX_CHARS)
            .collect::<String>()
    };
    match tools {
        [] => empty(),
        [one] => json!(cap(one)),
        [a, b] => line!(TOOL_LIST_TWO, json!({"a": cap(a), "b": cap(b)})),
        many => {
            // This branch only receives three or more tools. English has
            // one form here, but another locale still needs its count rule.
            let template = copy::TOOL_LIST_MANY.replace("{k}", "{count}");
            plural(
                "notification.TOOL_LIST_MANY",
                &template,
                &template,
                many.len() as u64,
                json!({}),
            )
        }
    }
}

fn batch_values(batch: &Batch) -> Value {
    json!({"days": days(batch.idle_days), "tool": tools(&batch.tools)})
}

fn batch_clauses(mut first: Value, batch: &Batch, estimate: bool) -> Value {
    if let Some(count) = batch.mission_fit.filter(|m| *m > 0) {
        first = join(
            first,
            counted!(
                NUDGE_MISSION_FIT_CLAUSE,
                NUDGE_MISSION_FIT_CLAUSE_ONE,
                count,
                "m",
                json!({})
            ),
            false,
        );
    }
    if let Some(e) = batch
        .estimate
        .filter(|e| estimate && e.drawn && e.known > 0)
    {
        let values = json!({"low": decimal(render::band_text(e.low)),
            "high": decimal(render::band_text(e.high)), "known": e.known});
        let clause = if e.known >= batch.count {
            line!(NUDGE_ESTIMATE_CLAUSE, values)
        } else {
            line!(NUDGE_ESTIMATE_CLAUSE_PARTIAL, values)
        };
        first = join(first, clause, false);
    }
    first
}

fn action(id: &str, label: Value) -> Value {
    json!({"id": id, "label": label})
}

pub fn idle_card(batch: &Batch) -> Value {
    let values = batch_values(batch);
    let title = if batch.tools.is_empty() {
        counted!(
            NUDGE_IDLE_TITLE_NO_TOOL,
            NUDGE_IDLE_TITLE_NO_TOOL_ONE,
            batch.count,
            "n",
            values.clone()
        )
    } else {
        counted!(
            NUDGE_IDLE_TITLE,
            NUDGE_IDLE_TITLE_ONE,
            batch.count,
            "n",
            values.clone()
        )
    };
    json!({"title": title,
        "body": batch_clauses(counted!(NUDGE_IDLE_BODY, NUDGE_IDLE_BODY_ONE, batch.count, "n", json!({})), batch, true),
        "panel_row": counted!(NUDGE_PANEL_IDLE, NUDGE_PANEL_IDLE_ONE, batch.count, "n", values),
        "actions": [action(render::ACTION_REVIEW, line!(NUDGE_IDLE_REVIEW)),
            action(render::ACTION_NOT_NOW, line!(NUDGE_NOT_NOW))]})
}

pub fn backlog_card(batch: &Batch) -> Value {
    json!({"title": counted!(NUDGE_BACKLOG_TITLE, NUDGE_BACKLOG_TITLE_ONE, batch.count, "n", json!({})),
        "body": batch_clauses(counted!(NUDGE_BACKLOG_BODY, NUDGE_BACKLOG_BODY_ONE, batch.count, "n", json!({})), batch, true),
        "panel_row": counted!(NUDGE_PANEL_BACKLOG, NUDGE_PANEL_BACKLOG_ONE, batch.count, "n", json!({})),
        "actions": [action(render::ACTION_REVIEW, line!(NUDGE_BACKLOG_REVIEW)),
            action(render::ACTION_NOT_NOW, line!(NUDGE_NOT_NOW))]})
}

fn verdict_values(v: &Verdicts) -> Value {
    json!({"a": v.accepted, "h": v.held,
        "date": {"date": v.since.to_string(), "english": render::date_text(v.since)},
        "x": v.credit_final_tenths.map(|t| decimal(render::credit_text(t))).unwrap_or_else(empty)})
}

fn final_clause(v: &Verdicts) -> Value {
    if v.credit_final_tenths.is_some() {
        line!(NUDGE_VERDICTS_FINAL_CLAUSE, verdict_values(v))
    } else {
        empty()
    }
}

pub fn verdicts_card(v: &Verdicts) -> Value {
    let values = verdict_values(v);
    let (title, body, panel) = if v.accepted == 0 && v.held == 0 {
        (
            line!(NUDGE_VERDICTS_TITLE_FINAL_ONLY, values.clone()),
            empty(),
            line!(NUDGE_PANEL_VERDICTS_FINAL_ONLY, values),
        )
    } else {
        // The canonical title omits zero clauses. These are variants of
        // that same template, not new notices or inferred English matches.
        let (key, title) = if v.accepted == 0 {
            (
                "notification.NUDGE_VERDICTS_TITLE.held",
                copy::NUDGE_VERDICTS_TITLE.replace("{a} accepted, ", ""),
            )
        } else if v.held == 0 {
            (
                "notification.NUDGE_VERDICTS_TITLE.accepted",
                copy::NUDGE_VERDICTS_TITLE.replace(", {h} held for privacy review", ""),
            )
        } else {
            (
                "notification.NUDGE_VERDICTS_TITLE.both",
                copy::NUDGE_VERDICTS_TITLE.to_owned(),
            )
        };
        let panel = if v.held == 0 {
            line!(NUDGE_PANEL_VERDICTS_ACCEPTED_ONLY, values.clone())
        } else if v.accepted == 0 {
            line!(NUDGE_PANEL_VERDICTS_HELD_ONLY, values.clone())
        } else {
            line!(NUDGE_PANEL_VERDICTS, values.clone())
        };
        (text(key, &title, values), final_clause(v), panel)
    };
    json!({"title": title, "body": body, "panel_row": panel,
        "actions": [action(render::ACTION_SEE_HISTORY, line!(NUDGE_VERDICTS_SEE))]})
}

pub fn verdict_sentence(v: &Verdicts) -> Value {
    if v.accepted == 0 && v.held == 0 {
        return final_clause(v);
    }
    let values = verdict_values(v);
    let sentence = if v.held == 0 {
        counted!(
            NOTIFY_VERDICTS_BODY_ACCEPTED_ONLY,
            NOTIFY_VERDICTS_BODY_ACCEPTED_ONLY_ONE,
            u64::from(v.accepted),
            "a",
            values
        )
    } else if v.accepted == 0 {
        counted!(
            NOTIFY_VERDICTS_BODY_HELD_ONLY,
            NOTIFY_VERDICTS_BODY_HELD_ONLY_ONE,
            u64::from(v.held),
            "h",
            values
        )
    } else {
        counted!(
            NOTIFY_VERDICTS_BODY,
            NOTIFY_VERDICTS_BODY_ONE,
            u64::from(v.accepted),
            "a",
            values
        )
    };
    join(sentence, final_clause(v), false)
}

pub fn idle_sentence(batch: &Batch, fold: bool) -> Value {
    let values = batch_values(batch);
    let sentence = match (fold, batch.tools.is_empty()) {
        (true, true) => counted!(
            DIGEST_IDLE_SENTENCE_NO_TOOL,
            DIGEST_IDLE_SENTENCE_NO_TOOL_ONE,
            batch.count,
            "n",
            values
        ),
        (true, false) => counted!(
            DIGEST_IDLE_SENTENCE,
            DIGEST_IDLE_SENTENCE_ONE,
            batch.count,
            "n",
            values
        ),
        (false, true) => counted!(
            NOTIFY_IDLE_BODY_NO_TOOL,
            NOTIFY_IDLE_BODY_NO_TOOL_ONE,
            batch.count,
            "n",
            values
        ),
        (false, false) => counted!(
            NOTIFY_IDLE_BODY,
            NOTIFY_IDLE_BODY_ONE,
            batch.count,
            "n",
            values
        ),
    };
    if fold {
        batch_clauses(sentence, batch, false)
    } else {
        sentence
    }
}

pub fn idle_notification(batch: &Batch) -> Value {
    json!({"title": line!(NOTIFY_TITLE), "body": idle_sentence(batch, false),
        "actions": [action(render::ACTION_REVIEW, line!(NOTIFY_ACTION_REVIEW_IDLE)),
            action(render::ACTION_NOT_NOW, line!(NOTIFY_ACTION_NOT_NOW))]})
}

pub fn verdicts_notification(v: &Verdicts) -> Value {
    json!({"title": line!(NOTIFY_TITLE), "body": verdict_sentence(v),
        "actions": [action(render::ACTION_SEE_HISTORY, line!(NOTIFY_ACTION_SEE_HISTORY))]})
}

pub fn settlement_notification(total: &SettlementTotal, pricing_changed: bool) -> Value {
    let values = json!({"low": usd(total.low_usd_micros), "high": usd(total.high_usd_micros)});
    let body = match (pricing_changed, total.low_usd_micros == 0) {
        (true, true) => line!(NOTIFY_SETTLEMENT_BODY_PRICING_UP_TO, values),
        (true, false) => line!(NOTIFY_SETTLEMENT_BODY_PRICING, values),
        (false, true) => line!(NOTIFY_SETTLEMENT_BODY_UP_TO, values),
        (false, false) => line!(NOTIFY_SETTLEMENT_BODY, values),
    };
    json!({"title": line!(NOTIFY_SETTLEMENT_TITLE), "body": body,
        "actions": [action(render::ACTION_REVIEW, line!(NOTIFY_SETTLEMENT_ACTION_REVIEW)),
            action(render::ACTION_NOT_NOW, line!(NOTIFY_SETTLEMENT_ACTION_NOT_NOW))]})
}

pub fn near_ai_credit_notification(nanos: i64) -> Value {
    let amount = crate::private_inference_copy::amount_message(nanos, 9, false);
    json!({"title": line!(NOTIFY_NEAR_AI_CREDIT_TITLE),
        "body": line!(NOTIFY_NEAR_AI_CREDIT_BODY, json!({"amount": amount})),
        "actions": [action(render::ACTION_REVIEW, line!(NOTIFY_NEAR_AI_CREDIT_ACTION_VIEW)),
            action(render::ACTION_NOT_NOW, line!(NOTIFY_NEAR_AI_CREDIT_ACTION_NOT_NOW))]})
}

pub fn mark_news(v: &Verdicts) -> Value {
    let values = verdict_values(v);
    let accessibility = if v.accepted == 0 && v.held == 0 {
        line!(MARK_A11Y_VERDICTS_FINAL_ONLY, values)
    } else if v.held == 0 {
        line!(MARK_A11Y_VERDICTS_ACCEPTED_ONLY, values)
    } else if v.accepted == 0 {
        line!(MARK_A11Y_VERDICTS_HELD_ONLY, values)
    } else {
        line!(MARK_A11Y_VERDICTS, values)
    };
    json!({"accessibility": accessibility, "tooltip": line!(MARK_TOOLTIP_VERDICTS)})
}

pub fn mark_ready(batch: &Batch) -> Value {
    let values = json!({"n": batch.count, "days": days(batch.idle_days)});
    json!({"accessibility": counted!(MARK_A11Y_IDLE, MARK_A11Y_IDLE_ONE, batch.count, "n", values.clone()),
        "tooltip": line!(MARK_TOOLTIP_IDLE, values)})
}

pub fn mark_settlement() -> Value {
    json!({"accessibility": line!(MARK_A11Y_SETTLEMENT), "tooltip": line!(MARK_TOOLTIP_SETTLEMENT)})
}

pub fn digest_help(evening: bool, interval: Option<u64>) -> Option<Value> {
    if evening {
        return Some(line!(SETTING_DIGEST_HELP_EVENING));
    }
    let secs = interval?;
    if secs < 3600 || secs % 3600 != 0 {
        return None;
    }
    Some(counted!(
        SETTING_DIGEST_HELP_INTERVAL,
        SETTING_DIGEST_HELP_INTERVAL_ONE,
        secs / 3600,
        "hours",
        json!({})
    ))
}

/// The original pending digest names up to three sorted nonempty labels.
pub fn pending(count: u64, projects: &BTreeSet<String>) -> Value {
    let projects: Vec<_> = projects.iter().filter(|p| !p.is_empty()).collect();
    let mut list = empty();
    for name in projects.iter().take(3) {
        list = if list.as_str() == Some("") {
            json!(name)
        } else {
            text(
                "notification.join.projects",
                "{first}, {second}",
                json!({"first": list, "second": name}),
            )
        };
    }
    let more = projects.len().saturating_sub(3);
    let (key, one, other) = if projects.is_empty() {
        (
            "notification.digest.pending",
            "{count} trace ready to contribute",
            "{count} traces ready to contribute",
        )
    } else if more > 0 {
        (
            "notification.digest.pending_more",
            "{count} trace ready to contribute from {projects} and {more} more",
            "{count} traces ready to contribute from {projects} and {more} more",
        )
    } else {
        (
            "notification.digest.pending_projects",
            "{count} trace ready to contribute from {projects}",
            "{count} traces ready to contribute from {projects}",
        )
    };
    plural(
        key,
        one,
        other,
        count,
        json!({"projects": list, "more": more}),
    )
}

pub fn contribution(
    count: u64,
    projects: &BTreeSet<String>,
    credit_final: f64,
    credit_pending: f64,
) -> Value {
    let mut named = projects.iter().filter(|p| !p.trim().is_empty());
    let mut message = if let (Some(only), None) = (named.next(), named.next()) {
        plural(
            "notification.digest.contributed_project",
            "{count} trace contributed from {project}.",
            "{count} traces contributed from {project}.",
            count,
            json!({"project": only}),
        )
    } else {
        plural(
            "notification.digest.contributed",
            "{count} trace contributed.",
            "{count} traces contributed.",
            count,
            json!({}),
        )
    };
    for (figure, key, template) in [
        (
            credit_final,
            "notification.digest.credit_earned",
            crate::daemon::notify::DIGEST_CREDIT_EARNED,
        ),
        (
            credit_pending,
            "notification.digest.credit_pending",
            crate::daemon::notify::DIGEST_CREDIT_PENDING,
        ),
    ] {
        if let Some(tenths) = crate::daemon::nudge::tenths_of(figure) {
            message = join(
                message,
                text(
                    key,
                    template,
                    json!({"x": decimal(render::credit_text(tenths))}),
                ),
                false,
            );
        }
    }
    message
}

/// Explicit ABI facts, shared with daemon-side constructors. Unknown
/// operations fail closed; no supplied key/template is trusted as copy.
pub fn message(operation: &str, input: &Value) -> Option<Value> {
    let batch = || -> Option<Batch> {
        Some(Batch {
            count: input["count"].as_u64()?,
            tools: input["tools"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            idle_days: u32::try_from(input["idle_days"].as_u64().unwrap_or(1)).ok()?,
            mission_fit: input["mission_fit"].as_u64(),
            estimate: input
                .get("estimate")
                .filter(|e| !e.is_null())
                .map(|e| render::EstimateSum {
                    low: e["low"].as_f64().unwrap_or(0.0),
                    high: e["high"].as_f64().unwrap_or(0.0),
                    known: e["known"].as_u64().unwrap_or(0),
                    drawn: e["drawn"].as_bool().unwrap_or(false),
                }),
        })
    };
    let verdicts = || -> Option<Verdicts> {
        Some(Verdicts {
            accepted: u32::try_from(input["accepted"].as_u64()?).ok()?,
            held: u32::try_from(input["held"].as_u64()?).ok()?,
            credit_final_tenths: input["credit_final_tenths"].as_u64(),
            since: input["since"].as_str()?.parse().ok()?,
        })
    };
    let projects = || {
        input["projects"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(match operation {
        "notification_idle" => idle_notification(&batch()?),
        "notification_verdicts" => verdicts_notification(&verdicts()?),
        "notification_near_ai_credit" => {
            near_ai_credit_notification(input["received_nanos"].as_i64()?)
        }
        "notification_settlement" => settlement_notification(
            &SettlementTotal {
                low_usd_micros: input["low_usd_micros"].as_u64()?,
                high_usd_micros: input["high_usd_micros"].as_u64()?,
                known: 0,
                unknown: 0,
            },
            input["pricing_changed"].as_bool().unwrap_or(false),
        ),
        "nudge_idle" => idle_card(&batch()?),
        "nudge_backlog" => backlog_card(&batch()?),
        "nudge_verdicts" => verdicts_card(&verdicts()?),
        "nudge_mark_ready" => mark_ready(&batch()?),
        "nudge_mark_news" => mark_news(&verdicts()?),
        "nudge_mark_settlement" => mark_settlement(),
        "digest_idle" => idle_sentence(&batch()?, true),
        "digest_verdicts" => verdict_sentence(&verdicts()?),
        "digest_pending" => pending(input["count"].as_u64()?, &projects()),
        "digest_contribution" => contribution(
            input["count"].as_u64()?,
            &projects(),
            input["credit_final"].as_f64().unwrap_or(0.0),
            input["credit_pending"].as_f64().unwrap_or(0.0),
        ),
        "digest_help" => digest_help(
            input["evening"].as_bool().unwrap_or(false),
            input["interval_secs"].as_u64(),
        )?,
        _ => return None,
    })
}

/// Enumerate every structural selection, recursively collecting message
/// leaves. Only constants and bounded sample facts are used, never English
/// lookup to determine identity. Parameters are not catalog source text.
pub fn templates() -> Vec<Value> {
    fn collect(value: &Value, out: &mut BTreeMap<String, Value>) {
        if let Some(key) = value["key"].as_str() {
            out.insert(key.to_owned(), value.clone());
        }
        match value {
            Value::Object(fields) => {
                for child in fields.values() {
                    collect(child, out);
                }
            }
            Value::Array(values) => {
                for child in values {
                    collect(child, out);
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    for tool_count in 0..=3 {
        for count in [1, 3] {
            for known in [0, 1, 3] {
                let batch = Batch {
                    count,
                    tools: (0..tool_count).map(|n| format!("tool{n}")).collect(),
                    idle_days: 2,
                    mission_fit: Some(1),
                    estimate: Some(render::EstimateSum {
                        low: 1.0,
                        high: 2.5,
                        known,
                        drawn: true,
                    }),
                };
                for value in [
                    idle_card(&batch),
                    backlog_card(&batch),
                    idle_notification(&batch),
                    idle_sentence(&batch, true),
                    mark_ready(&batch),
                ] {
                    collect(&value, &mut out);
                }
            }
        }
    }
    for accepted in [0, 1, 2] {
        for held in [0, 1, 2] {
            let v = Verdicts {
                accepted,
                held,
                credit_final_tenths: Some(10),
                since: NaiveDate::from_ymd_opt(2026, 10, 8).expect("valid sample date"),
            };
            for value in [verdicts_card(&v), verdicts_notification(&v), mark_news(&v)] {
                collect(&value, &mut out);
            }
        }
    }
    for low in [0, 500_000] {
        for pricing in [false, true] {
            collect(
                &settlement_notification(
                    &SettlementTotal {
                        low_usd_micros: low,
                        high_usd_micros: 1_000_000,
                        known: 1,
                        unknown: 0,
                    },
                    pricing,
                ),
                &mut out,
            );
        }
    }
    for names in [vec![], vec!["a"], vec!["a", "b", "c", "d"]] {
        let projects = names.into_iter().map(str::to_owned).collect();
        collect(&pending(2, &projects), &mut out);
        collect(&contribution(2, &projects, 1.0, 2.0), &mut out);
    }
    collect(&near_ai_credit_notification(1_000_000_000), &mut out);
    collect(&mark_settlement(), &mut out);
    collect(
        &digest_help(false, Some(7200)).expect("whole hours"),
        &mut out,
    );
    collect(&digest_help(true, None).expect("evening"), &mut out);
    // At least two sentences and lines appear in live digest composition.
    collect(
        &join(line!(NOTIFY_TITLE), line!(NOTIFY_TITLE), true),
        &mut out,
    );
    out.into_values().collect()
}
