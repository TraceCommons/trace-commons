//! An opened History record's contribution details, ported from the
//! consent-relevant part of the Mac's `SessionDetailView`
//! (`SessionContributionOverview`): the task and its outcome, the
//! decisive correction, the supporting evidence, the processing status,
//! the permitted uses and the versions it was contributed under. Read
//! from `history_detail`; every word is the core's (`public_run_copy`).
//! Withdraw stays on the History row, as the Mac's Monitor pane keeps it
//! (`offersWithdrawal: false`). Public-run publishing is not ported.

use serde::Deserialize;
use serde_json::Value;

use std::rc::Rc;

use trace_commons_contributor::public_run::{PublicRunValueLabel, public_run_copy};

use adw::prelude::*;

use super::App;
use super::style::{self, Tone, space};

/// One evidence excerpt (`SessionEvidenceCandidate`).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Evidence {
    pub kind: String,
    pub excerpt: String,
}

/// `history_detail`'s answer, the fields the Mac's overview reads; the
/// ones the Mac requires are required here.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct SessionDetail {
    #[serde(default)]
    pub content_unavailable: Option<bool>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub task_success: Option<String>,
    #[serde(default)]
    pub user_feedback: Option<String>,
    #[serde(default)]
    pub human_correction: Option<String>,
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub contribution_status: Option<String>,
    #[serde(default)]
    pub permitted_uses: Option<Vec<String>>,
    pub contributed_version: String,
    pub consent_policy_version: String,
    pub redaction_pipeline_version: String,
}

pub(crate) fn session_detail(reply: &Result<Value, String>) -> Option<SessionDetail> {
    serde_json::from_value(reply.as_ref().ok()?.clone()).ok()
}

pub(crate) fn label_for(labels: &[PublicRunValueLabel], value: &str) -> Option<&'static str> {
    labels.iter().find(|l| l.value == value).map(|l| l.label)
}

/// The processing status: the detail's, else the record's
/// (`historyStatusLabel`), in the core's words.
pub(crate) fn status_line(detail: &SessionDetail, record_status: &str) -> &'static str {
    let copy = public_run_copy();
    let status = detail
        .contribution_status
        .as_deref()
        .unwrap_or(record_status);
    label_for(&copy.history_status_labels, status).unwrap_or(copy.contribution_status_unavailable)
}

pub(crate) fn use_line(value: &str) -> &'static str {
    let copy = public_run_copy();
    label_for(&copy.permitted_use_choices, value).unwrap_or(copy.unrecognized_value)
}

/// A missing value's word: "unavailable" when the server withheld the
/// content, the ordinary fallback otherwise.
pub(crate) fn missing_line(detail: &SessionDetail, ordinary: &'static str) -> &'static str {
    if detail.content_unavailable == Some(true) {
        public_run_copy().unavailable_value
    } else {
        ordinary
    }
}

pub(crate) fn failure_line(label: &str) -> &'static str {
    trace_commons_contributor::public_run::session_detail_error_line(label)
}

// --- Widgets ---------------------------------------------------------------

/// Read the opened record's detail into `slot`, saying it is reading
/// first. `still_open` decides, when the answer lands, whether the slot
/// still belongs to this record.
pub(crate) fn load(
    app: &Rc<App>,
    slot: &gtk::Box,
    submission_id: &str,
    record_status: &str,
    still_open: impl Fn(&Rc<App>) -> bool + 'static,
) {
    let copy = public_run_copy();
    clear(slot);
    style::append_caveat(slot, copy.reading_record);
    let slot = slot.clone();
    let status = record_status.to_string();
    app.call(
        "history_detail",
        serde_json::json!({ "submission_id": submission_id }),
        move |app, result| {
            if !still_open(app) {
                return;
            }
            clear(&slot);
            match session_detail(&result) {
                Some(detail) => slot.append(&panel(&detail, &status)),
                None => {
                    let label = result.err().unwrap_or_default();
                    let line = style::caveat(failure_line(&label));
                    line.add_css_class(Tone::Attention.css());
                    slot.append(&line);
                }
            }
        },
    );
}

fn clear(slot: &gtk::Box) {
    while let Some(child) = slot.first_child() {
        slot.remove(&child);
    }
}

fn eyebrow_card(title: &str) -> gtk::Box {
    let card = style::card(gtk::Orientation::Vertical, space::S);
    card.append(&style::eyebrow(title));
    card
}

fn value(container: &gtk::Box, text: Option<&str>, missing: &'static str) {
    match text {
        Some(text) => {
            let label = style::body(text);
            label.set_selectable(true);
            container.append(&label);
        }
        None => {
            let label = style::caveat(missing);
            label.add_css_class("tc-tertiary");
            container.append(&label);
        }
    }
}

/// The overview (`SessionContributionOverview`).
pub(crate) fn panel(detail: &SessionDetail, record_status: &str) -> gtk::Box {
    let copy = public_run_copy();
    let root = gtk::Box::new(gtk::Orientation::Vertical, space::S);
    if detail.content_unavailable == Some(true) {
        style::append_caveat(&root, copy.content_unavailable);
    }

    let task = eyebrow_card(copy.task);
    value(
        &task,
        detail.task.as_deref(),
        missing_line(detail, copy.no_task),
    );
    task.append(&style::eyebrow(copy.outcome));
    let outcome = detail
        .task_success
        .as_deref()
        .and_then(|v| label_for(&copy.task_outcome_choices, v));
    value(
        &task,
        outcome,
        missing_line(detail, copy.outcome_unavailable),
    );
    if let Some(feedback) = detail
        .user_feedback
        .as_deref()
        .and_then(|v| label_for(&copy.feedback_choices, v))
    {
        style::append_caveat(&task, feedback);
    }
    root.append(&task);

    let correction = eyebrow_card(copy.decisive_correction);
    value(
        &correction,
        detail.human_correction.as_deref(),
        missing_line(detail, copy.no_correction),
    );
    root.append(&correction);

    let evidence = eyebrow_card(copy.supporting_evidence);
    style::append_caveat(&evidence, copy.observed_in_version);
    if detail.evidence.is_empty() {
        value(&evidence, None, missing_line(detail, copy.no_evidence));
    }
    for item in &detail.evidence {
        evidence.append(&style::eyebrow(
            label_for(&copy.evidence_kind_choices, &item.kind).unwrap_or(copy.unrecognized_value),
        ));
        let excerpt = style::caveat(&item.excerpt);
        excerpt.set_selectable(true);
        evidence.append(&excerpt);
    }
    root.append(&evidence);

    let details = eyebrow_card(copy.contribution_details);
    let status = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    status.append(&style::caveat(copy.processing_status));
    status.append(&style::body(status_line(detail, record_status)));
    details.append(&status);
    style::append_caveat(&details, copy.permitted_uses);
    match detail.permitted_uses.as_deref() {
        Some(uses) if !uses.is_empty() => {
            for use_ in uses {
                details.append(&style::tag(use_line(use_), Tone::Clear));
            }
        }
        Some(_) => style::append_caveat(&details, copy.no_permitted_uses),
        None => style::append_caveat(&details, copy.permitted_uses_unavailable),
    }
    details.append(&style::eyebrow(copy.contributed_version));
    for (key, version) in [
        (copy.envelope_version, &detail.contributed_version),
        (copy.consent_policy_version, &detail.consent_policy_version),
        (copy.redaction_version, &detail.redaction_pipeline_version),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        row.append(&style::caveat(key));
        let value = style::caveat(version);
        value.add_css_class("tc-ledger");
        row.append(&value);
        details.append(&row);
    }
    root.append(&details);
    root
}

#[cfg(test)]
#[path = "history_detail_tests.rs"]
mod tests;
