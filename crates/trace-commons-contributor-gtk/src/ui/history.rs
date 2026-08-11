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
use super::style::{self, Tone, space};
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
            .spacing(space::L)
            .margin_top(space::XL)
            .margin_bottom(space::XL)
            .margin_start(space::L)
            .margin_end(space::L)
            .build();
        let clamp = adw::Clamp::builder()
            .maximum_size(840)
            .tightening_threshold(680)
            .child(&content)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("tc-root");
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
    // Credit is a record, not a currency, so it is set as a ledger figure:
    // monospaced, unadorned, no symbol and nothing that could read as a
    // score. The prose beside it is what stops the number being mistaken
    // for one.
    content.append(&style::section(copy::CREDIT_HEADING));
    let credit = style::card(gtk::Orientation::Vertical, space::M);

    // `last_refreshed_at: null` renders as staleness, never as a confident
    // zero: a stale cache presented as current is a lie about a number
    // people will care about.
    match rollup.last_refreshed_at {
        Some(_) => {
            let figures = gtk::Box::new(gtk::Orientation::Horizontal, space::XXL);
            for (label, value, tone) in [
                (
                    "Recorded",
                    format!("{:.1}", rollup.credit_final),
                    Tone::Clear,
                ),
                (
                    "Still being scored",
                    format!("{:.1}", rollup.credit_pending),
                    Tone::Held,
                ),
            ] {
                let column = gtk::Box::new(gtk::Orientation::Vertical, space::XXS);
                column.append(&style::eyebrow(label));
                let figure = gtk::Label::builder().label(value).xalign(0.0).build();
                figure.add_css_class("tc-figure");
                figure.add_css_class(tone.css());
                column.append(&figure);
                figures.append(&column);
            }
            credit.append(&figures);
        }
        None => {
            let stale = gtk::Label::builder()
                .label(copy::NOT_SYNCED_YET)
                .xalign(0.0)
                .build();
            // Deliberately not `tc-figure`: the ledger face is for
            // figures, and setting a sentence in it made "Not synced yet"
            // read as though it were the number.
            stale.add_css_class("tc-card-title");
            stale.add_css_class("tc-neutral");
            credit.append(&stale);
        }
    }

    let credit_body = gtk::Label::builder()
        .label(copy::CREDIT_BODY)
        .xalign(0.0)
        .wrap(true)
        .build();
    credit_body.add_css_class("tc-caveat");
    credit.append(&credit_body);
    content.append(&credit);

    // --- The three groups ------------------------------------------------
    // Three tones for three different meanings, never one column of mixed
    // semantics. Each row carries a glyph and words as well as a colour, so
    // "held" cannot be read as "rejected" by anyone, in any palette.
    content.append(&style::section("What you have sent"));
    let groups = style::card(gtk::Orientation::Vertical, space::S);
    let in_commons = rollup.all_time.accepted;
    let waiting = rollup
        .all_time
        .submitted
        .saturating_sub(rollup.all_time.accepted + rollup.quarantined);
    for (tone, label, count) in [
        (Tone::Clear, copy::HISTORY_IN_THE_COMMONS, in_commons),
        (Tone::Held, copy::HISTORY_BEING_REVIEWED, rollup.quarantined),
        (Tone::Neutral, copy::HISTORY_WAITING_TO_BE_SCORED, waiting),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
        row.append(&style::tag(label, tone));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        let figure = gtk::Label::builder()
            .label(format!("{count}"))
            .xalign(1.0)
            .build();
        figure.add_css_class("tc-ledger");
        row.append(&figure);
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
        let inner = style::card(gtk::Orientation::Vertical, space::S);
        inner.set_margin_top(space::S);
        let body = gtk::Label::builder()
            .label(copy::QUARANTINE_BODY)
            .xalign(0.0)
            .wrap(true)
            .build();
        body.add_css_class("tc-body");
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
        withdraw_note.add_css_class("tc-caveat");
        inner.append(&withdraw_note);
        expander.set_child(Some(&inner));
        content.append(&expander);
    }

    // --- The records themselves ------------------------------------------
    if !records.is_empty() {
        content.append(&style::section("Sent"));
    }
    for record in records {
        let card = style::card(gtk::Orientation::Vertical, space::S);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let title = gtk::Label::builder()
            .label(&record.project_label)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .build();
        title.add_css_class("tc-card-title");
        top.append(&title);
        top.append(&style::tag(
            status_word(&record.status),
            status_tone(&record.status),
        ));
        card.append(&top);

        let status = gtk::Label::builder()
            .label(status_sentence(&record.status))
            .xalign(0.0)
            .wrap(true)
            .build();
        status.add_css_class("tc-meta");
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
            line.add_css_class("tc-body");
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

/// The same three meanings as the groups above, so a badge on a record and
/// a row in the summary cannot say different things about one state.
fn status_word(status: &str) -> &'static str {
    match status {
        "accepted" => copy::HISTORY_IN_THE_COMMONS,
        "quarantined" => copy::HISTORY_BEING_REVIEWED,
        _ => copy::HISTORY_WAITING_TO_BE_SCORED,
    }
}

fn status_tone(status: &str) -> Tone {
    match status {
        "accepted" => Tone::Clear,
        "quarantined" => Tone::Held,
        _ => Tone::Neutral,
    }
}
