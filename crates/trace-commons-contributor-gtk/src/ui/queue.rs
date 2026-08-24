//! The queue: what is waiting for a decision.
//!
//! The row carries what identifies a session to its author -- the project
//! label, the agent, when, and the redacted opening prompt -- plus what would
//! be sent and what scrubbing found, and now `Submit`: one click that builds,
//! pins, approves, and raises the toast `crate::toast` renders from the
//! response. `Contribute` in the preview sheet is still the only way to
//! *look* first -- `Submit` is the way to say yes without opening it, which
//! `docs/superpowers/specs/2026-08-20-one-click-submit-design.md` adds
//! deliberately: the hold window and `Undo` are what make it safe rather
//! than a confirmation dialog.
//!
//! Every project with more than one session waiting gets the same gesture at
//! its header -- `Submit all` calls `approve` with `project_id`, never by
//! enumerating entry ids client-side, so "everything in this project" means
//! exactly what the daemon selects for it.
//!
//! Recovery lives here too, on the undo bar, rather than behind the sheet.
//! A toast takes the only path back with it when it fades; a bar on the
//! screen a contributor is already looking at does not.
//!
//! No filesystem path is ever rendered here. `project_label` is what a
//! contributor sees and `project_id` is what goes back to the daemon; the
//! path does not cross the socket at all, so there is nothing to leak.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::App;
use super::style::{self, Tone, space};
use crate::copy;
use crate::model::{ApproveResult, PreviewSummary, QueueEntry, human_bytes, human_when};

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
    /// Exposed so `App` can debounce a scroll settle against this same
    /// viewport -- see `App::schedule_visible_preview_update`.
    pub scroller: gtk::ScrolledWindow,
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
        // Read the ids out and let the borrow end before anything else
        // touches the cell: `dismiss_undo` takes it mutably.
        let entry_ids = {
            let held = app_for_undo.undo.borrow();
            match held.as_ref() {
                Some(undo) => undo.entry_ids.clone(),
                None => return,
            }
        };
        app_for_undo.dismiss_undo();
        if entry_ids.is_empty() {
            return;
        }
        // A row's Undo cancels one entry; a project group's Undo cancels
        // every entry that call approved. `cancel` takes one id at a time --
        // there is no bulk form on this socket -- so a group's Undo fires
        // one call per id and waits for all of them before it says anything,
        // rather than reporting the first reply as though it spoke for the
        // rest.
        let total = entry_ids.len();
        let outcome = Rc::new(RefCell::new((0usize, 0usize)));
        for entry_id in entry_ids {
            let outcome = Rc::clone(&outcome);
            app_for_undo.call(
                "cancel",
                serde_json::json!({ "entry_id": entry_id }),
                move |app, result| {
                    let (completed, failed) = {
                        let mut o = outcome.borrow_mut();
                        o.0 += 1;
                        if result.is_err() {
                            o.1 += 1;
                        }
                        *o
                    };
                    if completed != total {
                        return;
                    }
                    app.toast(match failed {
                        0 if total == 1 => "Not sent. It's back in the queue.",
                        0 => "Not sent. They're back in the queue.",
                        // `cancel` is guaranteed to succeed for the whole
                        // hold, so this is the rare late press -- and it
                        // says so rather than pretending the press worked.
                        f if f == total && total == 1 => {
                            "Too late to take that one back -- it has already gone."
                        }
                        f if f == total => "Too late to take those back -- they have already gone.",
                        _ => {
                            "Some of those were already too late to take back; \
                              the rest are in the queue again."
                        }
                    });
                    app.refresh();
                },
            );
        }
    });

    let app_for_send = Rc::clone(app);
    view.undo_let_it_send.connect_clicked(move |_| {
        // Nothing is told to the daemon: the hold expires on its own, and
        // this only says the contributor has stopped considering taking it
        // back.
        app_for_send.dismiss_undo();
    });

    // Debounced rather than sent per pixel: a drag through 500 cards would
    // otherwise fire `preview_visible` continuously. Visibility only ever
    // reorders scheduled work -- see item 3 of "What each shell must do" in
    // the preview-scheduler design -- so a settle a quarter-second after the
    // last movement is early enough to matter and late enough to be one
    // call, not hundreds.
    let app_for_scroll = Rc::clone(app);
    view.scroller
        .vadjustment()
        .connect_value_changed(move |_| app_for_scroll.schedule_visible_preview_update());
}

pub fn render(app: &Rc<App>) {
    let view = &app.queue;
    clear(&view.list);
    clear(&view.disclosure);
    clear(&view.week);
    // Rebuilt below alongside `view.list`: every row is redrawn from
    // scratch each render, so the old widgets in here are already gone.
    app.card_widgets.borrow_mut().clear();

    let entries = app.entries.borrow();
    let pending: Vec<&QueueEntry> = entries.iter().filter(|e| e.state == "pending").collect();

    app.set_queue_count(pending.len());
    view.empty.set_visible(pending.is_empty());
    view.scroller.set_visible(!pending.is_empty());
    view.heading.set_text(&copy::waiting_heading(pending.len()));

    // Grouped by project, in the order each project's first entry appears,
    // so a project's header always sits above its own cards. Each entry
    // keeps the index it had in the flat `pending` list -- `Look inside`
    // opens the preview sheet by that index, and the sheet re-derives its
    // own copy of `pending` with the identical filter, so the position this
    // loop hands to `row` must stay the one that list agrees on, whatever
    // order the cards are drawn in.
    let mut groups: Vec<(&str, &str, Vec<(usize, &QueueEntry)>)> = Vec::new();
    for (index, entry) in pending.iter().enumerate() {
        match groups.iter_mut().find(|(id, _, _)| *id == entry.project_id) {
            Some((_, _, members)) => members.push((index, entry)),
            None => groups.push((
                &entry.project_id,
                &entry.project_label,
                vec![(index, entry)],
            )),
        }
    }

    for (project_id, project_label, members) in &groups {
        // `Submit all` only earns its place when it says something a row's
        // own `Submit` does not -- see `project_header` -- but the header
        // itself, and its `Ignore project`, are drawn at every group size:
        // ignoring a project with one waiting session is exactly as
        // meaningful as ignoring one with ten.
        view.list.append(&project_header(
            app,
            project_id,
            project_label,
            members.len(),
        ));
        for (index, entry) in members {
            let widget = row(app, entry, *index);
            // Kept so a scroll settle can ask each widget its own bounds
            // against the scroller -- see
            // `App::schedule_visible_preview_update`.
            app.card_widgets
                .borrow_mut()
                .insert(entry.entry_id.clone(), widget.clone());
            view.list.append(&widget);
        }
    }

    // Sessions that were queued and then resolved without being sent.
    // Surfacing them is what keeps "not sent" distinguishable from "sent",
    // and it is why the queue can always explain itself. Collapsed, because
    // they are a record rather than a decision owed.
    //
    // Drawn from `queue_outcome_counts`, not from `entries`. This used to
    // filter `entries` for the resolved states and list them one per line,
    // which looked right and could never show anything: `list_pending`
    // answers with `queue.pending()`, which is entries in the pending state
    // and no others, so the filter had nothing to find. The daemon rolls
    // these up by `reason_label` instead, and that roll-up is the only way
    // to reach them over this socket -- at the cost of the per-session
    // project label, which the count carries no room for.
    let counts = app.outcome_counts.borrow();
    let total: u64 = counts.values().sum();
    if total > 0 {
        let expander = gtk::Expander::builder()
            .label(copy::no_longer_waiting(total))
            .build();
        let inner = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        inner.set_margin_top(space::S);
        for (label, count) in counts.iter() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
            line.append(&style::tag("Not sent", Tone::Refused));
            let text = gtk::Label::builder()
                .label(format!("{count} - {}", copy::reason_sentence(label)))
                .xalign(0.0)
                .wrap(true)
                .build();
            text.add_css_class("tc-meta");
            line.append(&text);
            inner.append(&line);
        }
        // What this list does not cover, said rather than left to be
        // assumed. See `copy::NOT_OFFERED_BOUND`.
        let bound = gtk::Label::builder()
            .label(copy::NOT_OFFERED_BOUND)
            .xalign(0.0)
            .wrap(true)
            .build();
        bound.add_css_class("tc-caveat");
        inner.append(&bound);
        expander.set_child(Some(&inner));
        view.disclosure.append(&expander);
    }

    render_week(app);
    render_undo(app);

    // The set of cards just changed -- new rows, or none at all yet
    // measured -- so re-derive what is actually on screen once GTK has had
    // a chance to allocate them, rather than trusting whatever was visible
    // before this render.
    app.schedule_visible_preview_update();
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

/// What this shell currently knows about one card's scheduled preview --
/// built from `App::previews` and `App::previews_too_large`, which
/// `App::handle_preview_request_result` fills in as `preview_request` and
/// `preview_ready` resolve each entry. The two maps are mutually exclusive
/// by construction (an entry is inserted into exactly one, once, and never
/// moves), so this is a rendering convenience over them rather than a third
/// source of truth.
enum CardPreview<'a> {
    /// Requested but not yet answered, or not requested at all -- the same
    /// "checking" state the fan-out this replaces used while a preview
    /// pipeline pass was in flight.
    Checking,
    Ready(&'a PreviewSummary),
    /// Refused by the daemon's admission control before anything was
    /// parsed. Carries only `raw_session_bytes`, a `stat` -- never a
    /// would-send estimate, and never `limit_bytes` either: the design
    /// calls for showing the raw size alone, not a comparison to a cap the
    /// contributor has no reason to know about. See
    /// `copy::TOO_LARGE_TO_PREVIEW`.
    TooLarge {
        raw_session_bytes: u64,
    },
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
    let too_large = app
        .previews_too_large
        .borrow()
        .get(&entry.entry_id)
        .copied();
    let state = match (&preview, too_large) {
        (Some(p), _) => CardPreview::Ready(p),
        (None, Some((raw_session_bytes, _limit_bytes))) => {
            CardPreview::TooLarge { raw_session_bytes }
        }
        (None, None) => CardPreview::Checking,
    };
    let redactions: Option<u32> = match &state {
        CardPreview::Ready(p) => Some(p.redactions.values().sum()),
        CardPreview::TooLarge { .. } | CardPreview::Checking => None,
    };

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
        .label(match &state {
            CardPreview::Ready(p) => first_line(&p.opening_prompt),
            CardPreview::TooLarge { .. } => copy::TOO_LARGE_OPENING_LINE.to_string(),
            CardPreview::Checking => copy::CHECKING.to_string(),
        })
        .xalign(0.0)
        .wrap(true)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    summary.add_css_class("tc-body");
    card.append(&summary);

    card.append(&manifest_block(app, entry, index, state));

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
    state: CardPreview<'_>,
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

    let redactions: Option<u32> = match &state {
        CardPreview::Ready(p) => Some(p.redactions.values().sum()),
        CardPreview::TooLarge { .. } | CardPreview::Checking => None,
    };
    let pairs = gtk::Box::new(gtk::Orientation::Horizontal, METRIC_GAP);
    pairs.set_valign(gtk::Align::Start);
    pairs.append(&style::manifest_field(
        copy::WOULD_SEND,
        // An em dash rather than a collapsed field: the rhythm is the whole
        // point, and a strip that appears as previews land destroys it. The
        // too-large case gets its own words rather than a number, because
        // there never was a would-send figure to show -- see
        // `copy::TOO_LARGE_TO_PREVIEW`.
        &match &state {
            CardPreview::Ready(p) => human_bytes(p.would_send_bytes),
            CardPreview::TooLarge { .. } => copy::TOO_LARGE_TO_PREVIEW.to_string(),
            CardPreview::Checking => "-".to_string(),
        },
        Tone::Neutral,
    ));
    match (&state, redactions) {
        // Scrubbing found nothing. That is the case worth weighing, so it
        // gets the chip rather than a figure reading "nothing" -- which is
        // the same word a reassuring answer would use.
        (CardPreview::Ready(_), Some(0)) => {
            let chip = style::tag(copy::NOTHING_MATCHED, Tone::Attention);
            chip.set_valign(gtk::Align::Start);
            pairs.append(&chip);
        }
        (CardPreview::Ready(p), _) => pairs.append(&style::manifest_field(
            copy::REMOVED_BY_PATTERN,
            &removed_by_pattern(p),
            Tone::Neutral,
        )),
        (CardPreview::TooLarge { .. }, _) => pairs.append(&style::manifest_field(
            copy::REMOVED_BY_PATTERN,
            copy::NOT_PREVIEWED,
            Tone::Neutral,
        )),
        (CardPreview::Checking, _) => pairs.append(&style::manifest_field(
            copy::REMOVED_BY_PATTERN,
            "checking",
            Tone::Neutral,
        )),
    }
    facts.append(&pairs);

    // Never hidden behind a disclosure: conceding that scrubbing is
    // imperfect is what makes the rest credible. What changes here is that
    // the sentence describes this session rather than repeating a constant
    // -- see `copy::residual_risk_line`. The too-large caption states the
    // one real number involved, `raw_session_bytes`, and never a would-send
    // estimate -- see `copy::too_large_caption`.
    let caption = gtk::Label::builder()
        .label(match &state {
            CardPreview::Ready(_) => copy::residual_risk_line(redactions.unwrap_or(0)),
            CardPreview::TooLarge {
                raw_session_bytes, ..
            } => copy::too_large_caption(*raw_session_bytes),
            CardPreview::Checking => copy::CHECKING.to_string(),
        })
        .xalign(0.0)
        .wrap(true)
        .build();
    caption.add_css_class("tc-caveat");
    if redactions == Some(0) {
        caption.add_css_class("tc-attention");
    }
    facts.append(&caption);

    // What this one card actually covers, and -- the half the contract makes
    // mandatory -- whether any of it was left out to fit. Absent entirely
    // when there is nothing to report, so a conversation that delegated
    // nothing carries no line about subagents at all. Independent of the
    // preview: both counts are load-time facts on the entry itself, so this
    // is as true while the card still reads "checking" as it is after.
    if let Some(text) = copy::subagent_line(entry.subagent_count, entry.subagents_dropped) {
        let extent = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .wrap(true)
            .build();
        extent.add_css_class("tc-caveat");
        if entry.subagents_dropped > 0 {
            extent.add_css_class("tc-attention");
        }
        facts.append(&extent);
    }
    block.append(&facts);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    actions.set_valign(gtk::Align::Start);
    let skip = gtk::Button::with_label(copy::NOT_THIS_ONE);
    skip.add_css_class("tc-quiet");
    skip.set_tooltip_text(Some(copy::NOT_THIS_ONE_TOOLTIP));
    let look = gtk::Button::with_label(copy::LOOK_INSIDE);
    look.add_css_class("suggested-action");
    look.add_css_class("tc-primary");
    // `Submit` is the one-click path this row gained alongside `Look
    // inside`: it never shows a redacted transcript, so it earns its
    // prominence from the toast and the hold window that follow it, not
    // from crowding `Look inside` out of the accent colour.
    let submit = gtk::Button::with_label(copy::SUBMIT);
    submit.add_css_class("suggested-action");
    submit.add_css_class("tc-primary");
    submit.set_tooltip_text(Some(copy::SUBMIT_TOOLTIP));
    actions.append(&skip);
    actions.append(&look);
    actions.append(&submit);
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

    let app_for_submit = Rc::clone(app);
    let entry_id = entry.entry_id.clone();
    let project_label = entry.project_label.clone();
    submit.connect_clicked(move |_| {
        submit_and_toast(
            &app_for_submit,
            serde_json::json!({ "entry_id": entry_id }),
            project_label.clone(),
            vec![entry_id.clone()],
        );
    });

    block
}

/// A project's header. `Submit all` is built only when there is more than
/// one session waiting -- one waiting session from a project is already
/// covered by that row's own `Submit`, so the button would say nothing new
/// -- but `Ignore project` is built regardless of `waiting`: ignoring a
/// project with one card pending is exactly as meaningful as ignoring one
/// with ten. `Submit all` calls `approve` with `project_id` rather than
/// enumerating this project's entry ids itself: the daemon is what decides
/// which entries that selects, exactly once, and this shell does not keep
/// its own copy of that rule. `Ignore project` calls `set_project_mode` the
/// same way -- see `set_mode` in `ui/settings.rs`.
fn project_header(
    app: &Rc<App>,
    project_id: &str,
    project_label: &str,
    waiting: usize,
) -> gtk::Widget {
    let bar = style::card(gtk::Orientation::Horizontal, space::M);
    bar.set_valign(gtk::Align::Center);

    let heading = gtk::Label::builder()
        .label(copy::project_group_heading(project_label, waiting))
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .build();
    heading.add_css_class("tc-card-title");
    bar.append(&heading);

    if waiting > 1 {
        let submit_all = gtk::Button::with_label(copy::SUBMIT_ALL);
        submit_all.add_css_class("suggested-action");
        submit_all.add_css_class("tc-primary");
        submit_all.set_tooltip_text(Some(copy::SUBMIT_ALL_TOOLTIP));
        bar.append(&submit_all);

        let app_for_submit = Rc::clone(app);
        let project_id_for_submit = project_id.to_string();
        let project_label_for_submit = project_label.to_string();
        submit_all.connect_clicked(move |_| {
            // Read fresh at click time rather than off what `render` captured
            // when the header was drawn: the queue can change between a
            // render and a click, and this is only ever the CANDIDATE set
            // for the undo bar -- see `submit_and_toast` -- never what tells
            // the daemon what to approve. `project_id` alone does that.
            let candidates: Vec<String> = app_for_submit
                .entries
                .borrow()
                .iter()
                .filter(|e| e.state == "pending" && e.project_id == project_id_for_submit)
                .map(|e| e.entry_id.clone())
                .collect();
            submit_and_toast(
                &app_for_submit,
                serde_json::json!({ "project_id": project_id_for_submit }),
                project_label_for_submit.clone(),
                candidates,
            );
        });
    }

    let ignore = gtk::Button::with_label(copy::IGNORE_PROJECT);
    ignore.add_css_class("tc-chip");
    ignore.set_tooltip_text(Some(copy::IGNORE_PROJECT_TOOLTIP));
    bar.append(&ignore);

    let app_for_ignore = Rc::clone(app);
    let project_id_for_ignore = project_id.to_string();
    let project_label_for_ignore = project_label.to_string();
    let pending_count = waiting;
    ignore.connect_clicked(move |_| {
        let dialog = adw::MessageDialog::new(
            Some(&app_for_ignore.window),
            Some(&copy::ignore_project_title(&project_label_for_ignore)),
            Some(&copy::ignore_project_body(pending_count)),
        );
        dialog.add_responses(&[("cancel", "Cancel"), ("ignore", copy::IGNORE_PROJECT)]);
        dialog.set_close_response("cancel");
        // It sits beside a control that uploads these same traces. The
        // destructive appearance is what stops the two reading alike.
        dialog.set_response_appearance("ignore", adw::ResponseAppearance::Destructive);

        let app = Rc::clone(&app_for_ignore);
        let project_id = project_id_for_ignore.clone();
        let project_label_for_response = project_label_for_ignore.clone();
        dialog.connect_response(None, move |dialog, response| {
            dialog.close();
            if response != "ignore" {
                return;
            }
            // Mirrors `set_mode` in `ui/settings.rs`.
            //
            // `purged` is read back rather than assumed: the dialog had to
            // name a count before the call, so it named the one this shell
            // could see, and the queue can move between the render and the
            // click. The daemon's number is the authority, and when the two
            // disagree the contributor is told -- see
            // `copy::ignore_project_reconciled`.
            let label = project_label_for_response.clone();
            app.call(
                "set_project_mode",
                serde_json::json!({ "project_id": project_id, "mode": "ignore" }),
                move |app, result| {
                    match result {
                        Err(_) => app.toast(
                            "That couldn't be changed just now. Nothing else changed either.",
                        ),
                        Ok(value) => {
                            let purged = value.get("purged").and_then(|v| v.as_u64());
                            // An older daemon does not send the field at all.
                            // Silence is not a disagreement.
                            if let Some(purged) = purged {
                                if let Some(line) =
                                    copy::ignore_project_reconciled(&label, pending_count, purged)
                                {
                                    app.toast(&line);
                                }
                            }
                        }
                    }
                    app.refresh();
                },
            );
        });
        dialog.present();
    });

    bar.upcast()
}

/// One call to `approve`, rendered through `crate::toast` and, when it
/// earns one, an undo bar -- the shared path for the row's `Submit` and the
/// project group's `Submit all`.
///
/// `candidate_entry_ids` is every entry this call was asked to cover,
/// known on the client before the response arrives. `approve` never
/// returns the ids it approved -- only a count -- so the undo bar's set is
/// derived here: `candidate_entry_ids` minus whatever the response's
/// `skipped` list names by id. For a row that is exactly one id or none;
/// for a project group it is best-effort against a queue that can move
/// between the click and the reply, which is the same race `approve`
/// itself resolves server-side by re-checking under its own lock.
///
/// A refusal (an unrecognised `entry_id` or `project_id`, or any other
/// transport failure) is reported plainly rather than folded into the
/// toast's skip clause: nothing about the request was honoured, so none of
/// the four toast clauses describe it.
fn submit_and_toast(
    app: &Rc<App>,
    params: serde_json::Value,
    project_label: String,
    candidate_entry_ids: Vec<String>,
) {
    app.call("approve", params, move |app, result| {
        match result {
            Ok(value) => match serde_json::from_value::<ApproveResult>(value) {
                Ok(approve) => {
                    let skipped_ids: std::collections::HashSet<&str> = approve
                        .skipped
                        .iter()
                        .map(|s| s.entry_id.as_str())
                        .collect();
                    let entry_ids: Vec<String> = candidate_entry_ids
                        .into_iter()
                        .filter(|id| !skipped_ids.contains(id.as_str()))
                        .collect();
                    app.render_submit_response(&approve, entry_ids, &project_label);
                }
                Err(_) => app.toast(copy::SUBMIT_FAILED),
            },
            Err(_) => app.toast(copy::SUBMIT_FAILED),
        }
        app.refresh();
    });
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
