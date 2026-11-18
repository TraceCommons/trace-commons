//! Train coding models for traces already sent, ported from the Mac's
//! `SentTracesTrainingModel`: after a confirmed Uses write turns the scope
//! on, read how many sent traces lack it and, while there are any, offer
//! the use on all of them once. Nothing is preselected and nothing is sent
//! until the person chooses Allow; Not now sends nothing. Every sentence is
//! the core's (`consent_copy::sent_traces_training_copy`).

use serde_json::Value;
use trace_commons_contributor::consent_copy::SentTracesTrainingCopy;

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;

use super::App;

/// The consent scope this offer is about.
pub(crate) const SCOPE: &str = "model_training";

/// Whether this write turned Train coding models on: absent from what the
/// daemon last reported, present in what it confirmed.
pub(crate) fn turns_on(before: &[String], after: &[String]) -> bool {
    !before.iter().any(|s| s == SCOPE) && after.iter().any(|s| s == SCOPE)
}

/// `model_training_consent_status`' eligible count, when there is one to
/// offer (`ModelTrainingConsentStatus`); any refusal offers nothing.
pub(crate) fn eligible(reply: &Result<Value, String>) -> Option<u64> {
    reply
        .as_ref()
        .ok()?
        .get("eligible_count")?
        .as_u64()
        .filter(|count| *count > 0)
}

/// How a grant ended (`SentTracesTrainingRefusal`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Grant {
    Answered,
    /// The offer hides: the ceiling does not include training, the server
    /// cannot record it, or nobody is enrolled.
    Hidden,
    Failed,
}

pub(crate) fn grant_outcome(reply: &Result<Value, String>) -> Grant {
    match reply {
        Ok(value) => {
            let results = value.get("results").and_then(Value::as_array);
            let well_formed = results.is_some_and(|results| {
                results.iter().all(|r| {
                    r.get("submission_id").is_some_and(Value::is_string)
                        && r.get("outcome").is_some_and(Value::is_string)
                        && r.get("award").is_some_and(Value::is_string)
                })
            });
            if well_formed && value.get("remaining_eligible").is_some_and(Value::is_u64) {
                Grant::Answered
            } else {
                Grant::Failed
            }
        }
        Err(label) => match label.as_str() {
            "model-training-not-permitted"
            | "model-training-consent-unavailable"
            | "not-logged-in" => Grant::Hidden,
            _ => Grant::Failed,
        },
    }
}

/// The core's table, whole or not at all.
pub(crate) fn words() -> Option<SentTracesTrainingCopy> {
    let copy = trace_commons_contributor::consent_copy::sent_traces_training_copy();
    complete(&copy).then_some(copy)
}

pub(crate) fn complete(copy: &SentTracesTrainingCopy) -> bool {
    [
        copy.title,
        copy.body_one,
        copy.body_other,
        copy.scope,
        copy.allow_button,
        copy.allow_accessibility_name,
        copy.not_now_button,
        copy.not_now_accessibility_name,
        copy.failed,
    ]
    .iter()
    .all(|s| !s.trim().is_empty())
}

/// The sheet's body: the one-trace sentence for 1, the counted one
/// otherwise.
pub(crate) fn body(copy: &SentTracesTrainingCopy, count: u64) -> String {
    if count == 1 {
        copy.body_one.to_string()
    } else {
        trace_commons_contributor::consent_copy::render_count(copy.body_other, count)
    }
}

// --- The sheet ---------------------------------------------------------------

thread_local! {
    /// A sheet is up or its grant is in flight: no second one is raised.
    static OPEN: Cell<bool> = const { Cell::new(false) };
}

/// Train coding models was just turned on: read the eligible traces and,
/// while there are any, raise the sheet once.
pub(crate) fn offer_after_turn_on(app: &Rc<App>) {
    let Some(copy) = words() else { return };
    if OPEN.with(Cell::get) {
        return;
    }
    app.call(
        "model_training_consent_status",
        serde_json::json!({}),
        move |app, result| {
            if let Some(count) = eligible(&result) {
                raise(app, copy, count, false);
            }
        },
    );
}

/// The sheet. Its two buttons carry equal weight: neither is the default
/// and neither is taken by Return; closing it is Not now, which sends
/// nothing. A failed grant raises it again saying so.
fn raise(app: &Rc<App>, copy: SentTracesTrainingCopy, count: u64, failed: bool) {
    if OPEN.with(|open| open.replace(true)) && !failed {
        return;
    }
    let mut text = format!("{}\n\n{}", body(&copy, count), copy.scope);
    if failed {
        text.push_str("\n\n");
        text.push_str(copy.failed);
    }
    let dialog = adw::MessageDialog::new(Some(&app.window), Some(copy.title), Some(&text));
    dialog.add_responses(&[
        ("not_now", copy.not_now_button),
        ("allow", copy.allow_button),
    ]);
    dialog.set_close_response("not_now");
    dialog.set_default_response(None);
    let app = Rc::clone(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        if response != "allow" {
            OPEN.with(|open| open.set(false));
            return;
        }
        app.call(
            "model_training_consent_grant",
            serde_json::json!({ "all_eligible": true }),
            move |app, result| match grant_outcome(&result) {
                Grant::Failed => raise(app, copy, count, true),
                Grant::Answered | Grant::Hidden => {
                    OPEN.with(|open| open.set(false));
                    app.refresh();
                }
            },
        );
    });
    dialog.present();
}

#[cfg(test)]
#[path = "sent_training_tests.rs"]
mod tests;
