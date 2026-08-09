//! What has already gone, and what it earned.
//!
//! Three groups, never one column of mixed semantics, because "in the
//! commons", "being reviewed for privacy" and "waiting to be scored" mean
//! three different things and a contributor who reads quarantine as
//! rejection has been misled by the layout rather than by the words.
//!
//! Credit is a record, not a currency: no symbol, no estimate, no
//! projection, no date, and nothing resembling a streak or a level.

use std::rc::Rc;

use adw::prelude::*;

use super::App;
use crate::copy;
use crate::model::{HistoryRecord, HistoryRollup};

pub struct HistoryView {
    pub root: gtk::Box,
    content: gtk::Box,
}

impl Default for HistoryView {
    fn default() -> Self {
        Self::new()
    }
}

impl HistoryView {
    pub fn new() -> Self {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&scroller);
        Self { root, content }
    }
}

pub fn wire(_app: &Rc<App>) {}

pub fn refresh(app: &Rc<App>) {
    app.call("history_rollup", serde_json::json!({}), |app, result| {
        let rollup: HistoryRollup = result
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        app.call(
            "list_history",
            serde_json::json!({ "limit": 50 }),
            move |app, result| {
                let records: Vec<HistoryRecord> = result
                    .ok()
                    .and_then(|v| serde_json::from_value(v.get("history").cloned()?).ok())
                    .unwrap_or_default();
                render(app, &rollup, &records);
            },
        );
    });
}

fn render(app: &Rc<App>, rollup: &HistoryRollup, records: &[HistoryRecord]) {
    let content = &app.history.content;
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }

    // --- Credit ---------------------------------------------------------
    let credit = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let heading = gtk::Label::builder()
        .label(copy::CREDIT_HEADING)
        .xalign(0.0)
        .build();
    heading.add_css_class("title-4");
    credit.append(&heading);

    // `last_refreshed_at: null` renders as staleness, never as a confident
    // zero: a stale cache presented as current is a lie about a number
    // people will care about.
    let numbers = match rollup.last_refreshed_at {
        Some(_) => format!(
            "{:.1} credit points recorded  -  {:.1} still being scored",
            rollup.credit_final, rollup.credit_pending
        ),
        None => copy::NOT_SYNCED_YET.to_string(),
    };
    let numbers_label = gtk::Label::builder().label(numbers).xalign(0.0).build();
    numbers_label.add_css_class("title-2");
    credit.append(&numbers_label);

    let credit_body = gtk::Label::builder()
        .label(copy::CREDIT_BODY)
        .xalign(0.0)
        .wrap(true)
        .build();
    credit_body.add_css_class("dim-label");
    credit.append(&credit_body);
    content.append(&credit);

    // --- The three groups ------------------------------------------------
    let groups = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let in_commons = rollup.all_time.accepted;
    let waiting = rollup
        .all_time
        .submitted
        .saturating_sub(rollup.all_time.accepted + rollup.quarantined);
    for (mark, label, count) in [
        ("\u{2713}", copy::HISTORY_IN_THE_COMMONS, in_commons),
        ("\u{25f4}", copy::HISTORY_BEING_REVIEWED, rollup.quarantined),
        ("\u{00b7}", copy::HISTORY_WAITING_TO_BE_SCORED, waiting),
    ] {
        let row = gtk::Label::builder()
            .label(format!("{mark}  {label}    {count}"))
            .xalign(0.0)
            .build();
        groups.append(&row);
    }
    content.append(&groups);

    // --- Quarantine, expanded --------------------------------------------
    if rollup.quarantined > 0 {
        let expander = gtk::Expander::builder()
            .label(format!(
                "{} - {} traces",
                copy::QUARANTINE_HEADING,
                rollup.quarantined
            ))
            .expanded(true)
            .build();
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.set_margin_top(8);
        let body = gtk::Label::builder()
            .label(copy::QUARANTINE_BODY)
            .xalign(0.0)
            .wrap(true)
            .build();
        inner.append(&body);

        // Withdraw is first-class in the shared spec and has no method on
        // `trace_commons.daemon.v1_1`. Rather than draw a button that
        // cannot work, say plainly where the capability is -- and see the
        // report, which records this as a gap rather than a design choice.
        let withdraw_note = gtk::Label::builder()
            .label(
                "To pull these back, use `trace-commons-contributor` from a terminal. \
                 Withdrawing from this window isn't wired up yet.",
            )
            .xalign(0.0)
            .wrap(true)
            .build();
        withdraw_note.add_css_class("dim-label");
        inner.append(&withdraw_note);
        expander.set_child(Some(&inner));
        content.append(&expander);
    }

    // --- The records themselves ------------------------------------------
    for record in records {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.set_margin_top(8);
        let title = gtk::Label::builder()
            .label(&record.project_label)
            .xalign(0.0)
            .build();
        title.add_css_class("heading");
        card.append(&title);

        let status = gtk::Label::builder()
            .label(status_sentence(&record.status))
            .xalign(0.0)
            .wrap(true)
            .build();
        status.add_css_class("dim-label");
        card.append(&status);

        // Rendered verbatim. "Held because a passage looked like a personal
        // address" is enormously better than a status word, and it is the
        // server's sentence to write, not this window's to paraphrase.
        for explanation in &record.explanations {
            let line = gtk::Label::builder()
                .label(explanation)
                .xalign(0.0)
                .wrap(true)
                .build();
            card.append(&line);
        }
        content.append(&card);
    }
}

/// Status words, in sentences. Quarantine reads as held, never rejected,
/// and never carries a turnaround time.
fn status_sentence(status: &str) -> &'static str {
    match status {
        "accepted" => "In the commons.",
        "quarantined" => "Held for privacy review. Not rejected.",
        "submitted" | "pending" => "Sent. Waiting to be scored.",
        _ => "Sent.",
    }
}
