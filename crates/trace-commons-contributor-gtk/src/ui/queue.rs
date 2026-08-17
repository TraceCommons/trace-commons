//! The queue: what is waiting for a decision.
//!
//! The row carries what identifies a session to its author -- the project
//! label, the agent, when, and the redacted opening prompt -- plus what would
//! be sent and what scrubbing found. What it deliberately does **not** carry
//! is an approve button. Approving from the row is approving without looking,
//! which is the misclick the preview-then-approve rule exists to prevent;
//! `Contribute` lives in the preview sheet and nowhere else.
//!
//! Recovery lives here too, on the undo bar, rather than behind the sheet.
//! A toast takes the only path back with it when it fades; a bar on the
//! screen a contributor is already looking at does not.
//!
//! No filesystem path is ever rendered here. `project_label` is what a
//! contributor sees and `project_id` is what goes back to the daemon; the
//! path does not cross the socket at all, so there is nothing to leak.

use std::rc::Rc;

use adw::prelude::*;

use super::App;
use super::style::{self, Tone, space};
use crate::copy;
use crate::model::{PreviewSummary, QueueEntry, human_bytes, human_when};

/// §4.1's `space.5.5`, the bottom of the Linux content stack, and `space.6`,
/// the gap between the two manifest pairs. The shared scale in
/// `style::space` runs 16 / 20 / 28 and has neither, so they are named here
/// rather than written as bare numbers at the call site.
// TODO(tokens): promote to `style::space` when style.rs is next opened.
const CONTENT_BOTTOM: i32 = 22;
const METRIC_GAP: i32 = 24;

/// The content stack's own gap, §4.1's `space.3.5`. Also absent from the
/// shared scale.
// TODO(tokens): promote to `style::space` when style.rs is next opened.
const CONTENT_GAP: i32 = 14;

pub struct QueueView {
    pub root: gtk::Box,
    /// Persistent rather than rebuilt each render: it is the only widget in
    /// this window that updates once a second, and rebuilding it under the
    /// pointer would move `Undo` out from under a contributor reaching for
    /// it.
    undo_bar: gtk::Box,
    undo_headline: gtk::Label,
    undo_held: gtk::Label,
    undo_undo: gtk::Button,
    undo_let_it_send: gtk::Button,
    heading: gtk::Label,
    list: gtk::Box,
    disclosure: gtk::Box,
    week: gtk::Box,
    empty: adw::StatusPage,
    scroller: gtk::ScrolledWindow,
}

impl Default for QueueView {
    fn default() -> Self {
        Self::new()
    }
}

impl QueueView {
    pub fn new() -> Self {
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(CONTENT_GAP)
            .build();

        // A reading column. Trust copy is read, not skimmed, and a sentence
        // that runs the full width of a maximised window is a sentence
        // nobody finishes. It also keeps a card's two actions within one
        // eye movement of each other.
        let heading = gtk::Label::builder().xalign(0.0).wrap(true).build();
        heading.add_css_class("tc-screen-title");

        let undo_headline = gtk::Label::builder().xalign(0.0).wrap(true).build();
        undo_headline.add_css_class("tc-card-title");
        let undo_held = gtk::Label::new(None);
        undo_held.add_css_class("tc-ledger");
        let undo_undo = gtk::Button::with_label(copy::UNDO);
        undo_undo.add_css_class("suggested-action");
        undo_undo.add_css_class("tc-primary");
        let undo_let_it_send = gtk::Button::with_label(copy::LET_IT_SEND);
        undo_let_it_send.add_css_class("tc-quiet");
        let undo_bar = build_undo_bar(&undo_headline, &undo_held, &undo_undo, &undo_let_it_send);

        let disclaimer = gtk::Label::builder()
            .label(copy::STANDING_DISCLAIMER)
            .xalign(0.0)
            .wrap(true)
            .build();
        disclaimer.add_css_class("tc-caveat");

        let disclosure = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let week = gtk::Box::new(gtk::Orientation::Vertical, space::S);

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(CONTENT_GAP)
            .margin_top(space::L)
            .margin_bottom(CONTENT_BOTTOM)
            .margin_start(space::XL)
            .margin_end(space::XL)
            .build();
        column.append(&heading);
        column.append(&list);
        column.append(&disclaimer);
        column.append(&disclosure);
        column.append(&week);

        let clamp = adw::Clamp::builder()
            .maximum_size(super::COLUMN_MAX)
            .tightening_threshold(super::COLUMN_TIGHTEN)
            .child(&column)
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("emblem-ok-symbolic")
            .title(copy::QUEUE_EMPTY_TITLE)
            .description(copy::QUEUE_EMPTY_BODY)
            .vexpand(true)
            .build();
        empty.add_css_class("tc-empty");

        // The undo bar sits in the page root, ABOVE the empty state and the
        // scroller, rather than at the top of the scrolling column.
        //
        // It has to outlive the thing it is undoing. Approving the last
        // waiting session empties the queue, and `render` hides the whole
        // scroller when there is nothing pending -- so a bar parented inside
        // it would be hidden by an ancestor at exactly the moment a person
        // most wants it, with `set_visible(true)` on the bar itself having no
        // effect. That is the common case, not an edge one: the sheet
        // approves and advances, and the last advance empties the queue.
        //
        // Its own clamp keeps it on the same measure as the column below.
        let undo_clamp = adw::Clamp::builder()
            .maximum_size(super::COLUMN_MAX)
            .tightening_threshold(super::COLUMN_TIGHTEN)
            .margin_top(space::L)
            .margin_start(space::XL)
            .margin_end(space::XL)
            .child(&undo_bar)
            .build();

        // The wrapper follows the bar rather than being toggled separately,
        // so `render_undo` still has exactly one thing to set. Without this
        // the clamp keeps its margins while the bar is hidden and leaves a
        // band of dead space above the queue.
        undo_bar
            .bind_property("visible", &undo_clamp, "visible")
            .sync_create()
            .build();

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("tc-root");
        root.append(&undo_clamp);
        root.append(&empty);
        root.append(&scroller);

        Self {
            root,
            undo_bar,
            undo_headline,
            undo_held,
            undo_undo,
            undo_let_it_send,
            heading,
            list,
            disclosure,
            week,
            empty,
            scroller,
        }
    }
}

/// The undo bar's frame. Built once; only its two labels ever change.
fn build_undo_bar(
    headline: &gtk::Label,
    held: &gtk::Label,
    undo: &gtk::Button,
    let_it_send: &gtk::Button,
) -> gtk::Box {
    let bar = style::card(gtk::Orientation::Vertical, space::S);
    bar.set_visible(false);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    // Blue, and a glyph, and words: held is a state, not a failure, and it
    // has to read as one without colour.
    let clock = gtk::Label::new(Some(Tone::Held.glyph()));
    clock.add_css_class("tc-icon-held");
    top.append(&clock);
    headline.set_hexpand(true);
    top.append(headline);
    top.append(held);
    bar.append(&top);

    let body = gtk::Label::builder()
        .label(copy::UNDO_BODY)
        .xalign(0.0)
        .wrap(true)
        .build();
    body.add_css_class("tc-caveat");
    bar.append(&body);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    actions.append(undo);
    actions.append(let_it_send);
    bar.append(&actions);

    bar
}

pub fn wire(app: &Rc<App>) {
    let view = &app.queue;

    let app_for_undo = Rc::clone(app);
    view.undo_undo.connect_clicked(move |_| {
        // Read the id out and let the borrow end before anything else
        // touches the cell: `dismiss_undo` takes it mutably.
        let entry_id = {
            let held = app_for_undo.undo.borrow();
            match held.as_ref() {
                Some(undo) => undo.entry_id.clone(),
                None => return,
            }
        };
        app_for_undo.dismiss_undo();
        app_for_undo.call(
            "cancel",
            serde_json::json!({ "entry_id": entry_id }),
            |app, result| {
                match result {
                    Ok(_) => app.toast("Not sent. It's back in the queue."),
                    // `cancel` is guaranteed to succeed for the whole hold,
                    // so this is the rare late press -- and it says so
                    // rather than pretending the press worked.
                    Err(_) => app.toast("Too late to take that one back -- it has already gone."),
                }
                app.refresh();
            },
        );
    });

    let app_for_send = Rc::clone(app);
    view.undo_let_it_send.connect_clicked(move |_| {
        // Nothing is told to the daemon: the hold expires on its own, and
        // this only says the contributor has stopped considering taking it
        // back.
        app_for_send.dismiss_undo();
    });
}

pub fn render(app: &Rc<App>) {
    let view = &app.queue;
    clear(&view.list);
    clear(&view.disclosure);
    clear(&view.week);

    let entries = app.entries.borrow();
    let pending: Vec<&QueueEntry> = entries.iter().filter(|e| e.state == "pending").collect();

    app.set_queue_count(pending.len());
    view.empty.set_visible(pending.is_empty());
    view.scroller.set_visible(!pending.is_empty());
    view.heading.set_text(&copy::waiting_heading(pending.len()));

    for (index, entry) in pending.iter().enumerate() {
        view.list.append(&row(app, entry, index));
    }

    // Sessions that were queued and then resolved without being sent.
    // Surfacing them is what keeps "not sent" distinguishable from "sent",
    // and it is why the queue can always explain itself. Collapsed, because
    // they are a record rather than a decision owed.
    let resolved: Vec<&QueueEntry> = entries
        .iter()
        .filter(|e| matches!(e.state.as_str(), "refused" | "expired" | "superseded"))
        .collect();
    if !resolved.is_empty() {
        let expander = gtk::Expander::builder()
            .label(copy::no_longer_waiting(resolved.len()))
            .build();
        let inner = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        inner.set_margin_top(space::S);
        for entry in resolved {
            let reason = entry
                .reason_label
                .as_deref()
                .map(copy::reason_sentence)
                .unwrap_or("Nothing was sent.");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
            line.append(&style::tag("Not sent", Tone::Refused));
            let text = gtk::Label::builder()
                .label(format!("{} - {}", entry.project_label, reason))
                .xalign(0.0)
                .wrap(true)
                .build();
            text.add_css_class("tc-meta");
            line.append(&text);
            inner.append(&line);
        }
        expander.set_child(Some(&inner));
        view.disclosure.append(&expander);
    }

    render_week(app);
    render_undo(app);
}

/// The week band: three figures, and the same three tones they carry
/// everywhere else in this window.
fn render_week(app: &Rc<App>) {
    let view = &app.queue;
    let rollup = app.rollup.borrow();

    view.week.append(&style::section(copy::THIS_WEEK));
    let cards = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(space::M)
        .homogeneous(true)
        .build();
    cards.append(&stat_card(
        Tone::Clear,
        copy::CONTRIBUTED,
        rollup.week.submitted,
    ));
    cards.append(&stat_card(
        Tone::Held,
        copy::QUARANTINE_HEADING,
        rollup.week.quarantined,
    ));
    // All time, not this week: "in the commons" is a standing total, and a
    // weekly slice of it would read as the commons shrinking every Monday.
    cards.append(&stat_card(
        Tone::Neutral,
        copy::HISTORY_IN_THE_COMMONS,
        rollup.all_time.accepted,
    ));
    view.week.append(&cards);
}

fn stat_card(tone: Tone, label: &str, value: u32) -> gtk::Box {
    let card = style::card(gtk::Orientation::Vertical, space::XS);
    card.set_hexpand(true);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, space::XXS);
    let glyph = gtk::Label::new(Some(tone.glyph()));
    glyph.add_css_class(match tone {
        // The held clock takes the mark's blue rather than the text blue:
        // it is a glyph, not a sentence.
        Tone::Held => "tc-icon-held",
        other => other.css(),
    });
    head.append(&glyph);
    head.append(&style::eyebrow(label));
    card.append(&head);

    let figure = gtk::Label::builder()
        .label(value.to_string())
        .xalign(0.0)
        .build();
    figure.add_css_class("tc-figure");
    card.append(&figure);
    card
}

/// Show or hide the undo bar, and move its elapsed figure.
///
/// Called once a second while an approval is held, so it must not rebuild
/// anything -- see `QueueView::undo_bar`.
pub fn render_undo(app: &Rc<App>) {
    let view = &app.queue;
    let pending = app.undo.borrow();
    let Some(undo) = pending.as_ref() else {
        view.undo_bar.set_visible(false);
        return;
    };
    view.undo_headline
        .set_text(&copy::undo_headline(&undo.project_label));
    // Elapsed, not remaining. The daemon's sweep is what actually sends, and
    // this window cannot see it, so a countdown would be a promise about
    // somebody else's clock.
    let seconds = (chrono::Utc::now() - undo.approved_at).num_seconds().max(0);
    view.undo_held.set_text(&format!("held {seconds}s"));
    view.undo_bar.set_visible(true);
}

/// One session, as a declaration form.
///
/// Every card is built the same way and in the same order -- who and when,
/// the opening prompt, the inset manifest block with the two actions in it,
/// and nothing else -- so a person reading a column of them can stop reading
/// and start scanning.
fn row(app: &Rc<App>, entry: &QueueEntry, index: usize) -> gtk::Widget {
    let card = style::card(gtk::Orientation::Vertical, space::S);

    let preview = app.previews.borrow().get(&entry.entry_id).cloned();
    let redactions: Option<u32> = preview.as_ref().map(|p| p.redactions.values().sum());

    // The one card-level state worth taking the gold rule: scrubbing that
    // matched nothing at all. Everything else is left to the block below,
    // because a flagged card on every row is a flag on none.
    if redactions == Some(0) {
        card.add_css_class("tc-flagged");
    }

    // Who, what ran it, and when -- on one line, with the time hung on the
    // right where a column of them can be read down.
    let head = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    let title = gtk::Label::builder()
        .label(&entry.project_label)
        .xalign(0.0)
        .wrap(true)
        .build();
    title.add_css_class("tc-card-title");
    head.append(&title);
    let agent = gtk::Label::new(Some(entry.agent_label()));
    agent.add_css_class("tc-meta");
    agent.set_hexpand(true);
    agent.set_xalign(0.0);
    head.append(&agent);
    let when = gtk::Label::new(Some(&human_when(entry.discovered_at)));
    when.add_css_class("tc-meta");
    when.add_css_class("tc-tertiary");
    head.append(&when);
    card.append(&head);

    // The redacted opening prompt: what actually identifies a session to the
    // person who ran it. A timestamp does not.
    let summary = gtk::Label::builder()
        .label(
            preview
                .as_ref()
                .map(|p| first_line(&p.opening_prompt))
                .unwrap_or_else(|| copy::CHECKING.to_string()),
        )
        .xalign(0.0)
        .wrap(true)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    summary.add_css_class("tc-body");
    card.append(&summary);

    card.append(&manifest_block(app, entry, index, preview.as_ref()));

    card.upcast()
}

/// The inset manifest block: what would leave, what pattern scrubbing took
/// out of it, the concession, and the only two things a row can do.
///
/// The actions live inside the block rather than under it because the block
/// is the sentence they answer. "Look inside" next to "would send 148 KB" is
/// a reply; the same button under a paragraph is a button.
fn manifest_block(
    app: &Rc<App>,
    entry: &QueueEntry,
    index: usize,
    preview: Option<&PreviewSummary>,
) -> gtk::Box {
    let block = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(space::M)
        // Flex-start: the attention variant's right-hand column is a chip
        // rather than a figure, and centring makes the pair look misaligned.
        .valign(gtk::Align::Start)
        .build();
    block.add_css_class("tc-manifest");

    let facts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(space::XS)
        .hexpand(true)
        .build();

    let redactions: Option<u32> = preview.map(|p| p.redactions.values().sum());
    let pairs = gtk::Box::new(gtk::Orientation::Horizontal, METRIC_GAP);
    pairs.set_valign(gtk::Align::Start);
    pairs.append(&style::manifest_field(
        copy::WOULD_SEND,
        // An em dash rather than a collapsed field: the rhythm is the whole
        // point, and a strip that appears as previews land destroys it.
        &preview
            .map(|p| human_bytes(p.would_send_bytes))
            .unwrap_or_else(|| "-".to_string()),
        Tone::Neutral,
    ));
    match (preview, redactions) {
        // Scrubbing found nothing. That is the case worth weighing, so it
        // gets the chip rather than a figure reading "nothing" -- which is
        // the same word a reassuring answer would use.
        (Some(_), Some(0)) => {
            let chip = style::tag(copy::NOTHING_MATCHED, Tone::Attention);
            chip.set_valign(gtk::Align::Start);
            pairs.append(&chip);
        }
        (Some(p), _) => pairs.append(&style::manifest_field(
            copy::REMOVED_BY_PATTERN,
            &removed_by_pattern(p),
            Tone::Neutral,
        )),
        (None, _) => pairs.append(&style::manifest_field(
            copy::REMOVED_BY_PATTERN,
            "checking",
            Tone::Neutral,
        )),
    }
    facts.append(&pairs);

    // Never hidden behind a disclosure: conceding that scrubbing is
    // imperfect is what makes the rest credible. What changes here is that
    // the sentence describes this session rather than repeating a constant
    // -- see `copy::residual_risk_line`.
    let caption = gtk::Label::builder()
        .label(match redactions {
            Some(total) => copy::residual_risk_line(total),
            None => copy::CHECKING.to_string(),
        })
        .xalign(0.0)
        .wrap(true)
        .build();
    caption.add_css_class("tc-caveat");
    if redactions == Some(0) {
        caption.add_css_class("tc-attention");
    }
    facts.append(&caption);
    block.append(&facts);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    actions.set_valign(gtk::Align::Start);
    let skip = gtk::Button::with_label(copy::NOT_THIS_ONE);
    skip.add_css_class("tc-quiet");
    skip.set_tooltip_text(Some(copy::NOT_THIS_ONE_TOOLTIP));
    let look = gtk::Button::with_label(copy::LOOK_INSIDE);
    look.add_css_class("suggested-action");
    look.add_css_class("tc-primary");
    actions.append(&skip);
    actions.append(&look);
    block.append(&actions);

    let app_for_look = Rc::clone(app);
    look.connect_clicked(move |_| super::preview::open(&app_for_look, index));

    let app_for_skip = Rc::clone(app);
    let entry_id = entry.entry_id.clone();
    skip.connect_clicked(move |_| {
        app_for_skip.call(
            "dismiss",
            serde_json::json!({ "entry_id": entry_id }),
            |app, result| {
                if result.is_ok() {
                    app.refresh();
                }
            },
        );
    });

    block
}

/// The "Removed by pattern" figure: the receipt's own categories, in the
/// receipt's own order, separated as the design writes them.
///
/// Built from `PreviewSummary::scrubbed_line` rather than from the raw map
/// so the queue and the preview sheet cannot disagree about how a category
/// is worded, pluralised, or ranked.
fn removed_by_pattern(preview: &PreviewSummary) -> String {
    preview
        .scrubbed_line()
        .strip_prefix("scrubbed: ")
        .unwrap_or("nothing")
        .replace(", ", " \u{00b7} ")
}

/// The manifest strip as the preview sheet draws it: four fields rather than
/// the row's two, because the sheet is where a person reads rather than
/// scans and has room for the whole declaration.
///
/// A sheet whose preview has not arrived yet shows the fields with dashes
/// rather than collapsing the strip.
pub fn manifest_for(preview: Option<&PreviewSummary>) -> gtk::Box {
    let Some(p) = preview else {
        return style::manifest(&[
            ("Turns", "-".into(), Tone::Neutral),
            (copy::WOULD_SEND, "-".into(), Tone::Neutral),
            (copy::REMOVED_BY_PATTERN, "checking".into(), Tone::Neutral),
            ("Personal info", "-".into(), Tone::Neutral),
        ]);
    };
    let total: u32 = p.redactions.values().sum();
    style::manifest(&[
        ("Turns", format!("{}", p.event_count), Tone::Neutral),
        (
            copy::WOULD_SEND,
            human_bytes(p.would_send_bytes),
            Tone::Neutral,
        ),
        (
            copy::REMOVED_BY_PATTERN,
            // The strip carries figures, so the receipt's prose form
            // ("scrubbed: 12 secrets, 4 tokens") belongs in the sheet's body
            // and the categories belong here. A zero is stated, never
            // hidden.
            match total {
                0 => copy::NOTHING_MATCHED.to_string(),
                _ => removed_by_pattern(p),
            },
            if total == 0 {
                Tone::Attention
            } else {
                Tone::Neutral
            },
        ),
        (
            "Personal info",
            if p.pii_labels_present.is_empty() {
                "none found".to_string()
            } else {
                p.pii_labels_present.join(", ")
            },
            if p.pii_labels_present.is_empty() {
                Tone::Neutral
            } else {
                Tone::Attention
            },
        ),
    ])
}

/// The opening prompt, trimmed to something a row can hold. This is
/// redacted trace content under the preview exemption -- it may be
/// displayed, and it must never be copied into a log line, a notification,
/// or a receipt.
fn first_line(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return "(no opening prompt)".to_string();
    }
    trimmed.lines().next().unwrap_or(trimmed).to_string()
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
