//! What has already gone, and what it earned.
//!
//! Three groups, never one column of mixed semantics, because "in the
//! commons", "held for privacy review" and "waiting to be scored" mean
//! three different things and a contributor who reads quarantine as
//! rejection has been misled by the layout rather than by the words.
//!
//! Credit is a record, not a currency: no symbol, no estimate, no
//! projection, no date, and nothing resembling a streak or a level.
//!
//! ## The shape, from the design spec
//!
//! `design-import/DESIGN-SPEC.md` §5.3, Linux column, top to bottom: three
//! stat cards, the held-for-review disclosure, the Credit section, then
//! "Everything you've contributed" over the record rows. The Linux frame
//! carries no in-content title -- the header bar's view switcher already
//! says History -- so this column starts at the stat cards.
//!
//! ## The Community section is drawn in another language on purpose
//!
//! §5.5 and §7.3: when a contributor is on the public roster, History
//! gains a Community panel rendered in the *site's* brand rather than this
//! window's -- 2px black frames, Helvetica, uppercase display type, mint,
//! and no rounded corners anywhere inside it. The seam is the point. The
//! black frame is the exact boundary of what becomes public, and smoothing
//! it into GNOME conventions would erase the one visual cue that says so.
//! Those colours are therefore NOT `tc_*` tokens -- the token set is the
//! native palette, and the community brand is a separate, light-only
//! palette (§2.2). They live in [`community_brand`], the one stylesheet
//! Settings' public surfaces share, scoped to `tc-brand-*` classes that
//! nothing outside those two sections uses.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::App;
use super::community_brand;
use super::mark::Scheme;
use super::style::{self, Tone, space};
use crate::copy;
use crate::model::{HistoryRecord, HistoryRollup, human_when};

/// What the public roster says about this contributor, as §5.5's snapshot
/// payload words.
///
/// **Nothing on `trace_commons.daemon.v1_1` carries this today.** There is
/// no roster method in `docs/contributor-daemon-ipc-v1_1.md`, so this is
/// parsed leniently out of the `history_rollup` answer that History
/// already asks for -- the contract is explicitly additive, and a field
/// that is not there yet simply leaves the section unrendered, which is
/// exactly §5.5's non-roster state ("the section simply does not render").
/// No new daemon call is made for it and nothing is invented on screen:
/// until the daemon publishes a roster snapshot, no contributor sees this
/// panel.
// TODO(contract): give the daemon a roster snapshot to answer with, and
// pin these names against it.
#[derive(Debug, Clone, serde::Deserialize)]
struct RosterStanding {
    /// Position on the public roster. `None` renders as a dash rather than
    /// as a confident `#0`.
    #[serde(default)]
    rank: Option<u32>,
    #[serde(default)]
    novelty_credit: f64,
    /// Accepted inside the published window, whose length is
    /// `window_label`.
    #[serde(default)]
    accepted_in_window: u32,
    /// A fraction in `0..=1`, rendered as a percentage.
    #[serde(default)]
    accept_rate: Option<f64>,
    /// The roster's own window, as the server labels it ("7d").
    #[serde(default)]
    window_label: String,
    #[serde(default)]
    public_since: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    snapshot_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The public profile page. Absent means no link is drawn at all --
    /// better than a link that goes nowhere.
    #[serde(default)]
    profile_url: Option<String>,
    /// §7.3: analytics that are withheld are stated in words, never as an
    /// empty chart. Defaults to withheld, because that is the server's
    /// position until an approved noise mechanism exists and the safe
    /// reading of a missing field is the more conservative one.
    #[serde(default = "withheld")]
    analytics_withheld: bool,
}

fn withheld() -> bool {
    true
}

impl RosterStanding {
    /// Read the standing out of a `history_rollup` answer, if it carries
    /// one. Anything malformed is treated as absent: a public surface is
    /// the last place to render a half-parsed number.
    fn from_rollup(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.get("community")?.clone()).ok()
    }
}

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
        community_brand::install();

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(space::L)
            .margin_top(space::L)
            .margin_bottom(space::XL)
            .margin_start(space::XL)
            .margin_end(space::XL)
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
        let value = result.unwrap_or(serde_json::Value::Null);
        let rollup: HistoryRollup = serde_json::from_value(value.clone()).unwrap_or_default();
        // Same call, one more optional field. See `RosterStanding`.
        let standing = RosterStanding::from_rollup(&value);
        app.call(
            "list_history",
            serde_json::json!({ "limit": 50 }),
            move |app, result| {
                let records: Vec<HistoryRecord> = result
                    .ok()
                    .and_then(|v| serde_json::from_value(v.get("history").cloned()?).ok())
                    .unwrap_or_default();
                render(app, &rollup, standing.as_ref(), &records);
            },
        );
    });
}

fn render(
    app: &Rc<App>,
    rollup: &HistoryRollup,
    standing: Option<&RosterStanding>,
    records: &[HistoryRecord],
) {
    let content = &app.history.content;
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }

    // --- The three states, as stat cards ---------------------------------
    // §5.3: three cards, never one column of mixed semantics. Each carries
    // its own glyph as well as its own colour, so the three states survive
    // greyscale.
    let waiting = rollup
        .all_time
        .submitted
        .saturating_sub(rollup.all_time.accepted + rollup.quarantined);
    let stats = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
    stats.set_homogeneous(true);
    for (glyph, label, count) in [
        (
            Glyph::Accepted,
            copy::HISTORY_IN_THE_COMMONS,
            rollup.all_time.accepted,
        ),
        (Glyph::Held, copy::QUARANTINE_HEADING, rollup.quarantined),
        (Glyph::Waiting, copy::HISTORY_WAITING_TO_BE_SCORED, waiting),
    ] {
        stats.append(&stat_card(glyph, label, count));
    }
    content.append(&stats);

    // --- Held for privacy review, as a disclosure ------------------------
    // Held reads as held. The group exists so a contributor can see what is
    // sitting still without it being mixed into the accepted column, and it
    // never states a turnaround time.
    if rollup.quarantined > 0 {
        content.append(&held_group(rollup.quarantined));
    }

    // --- Credit ----------------------------------------------------------
    // Credit is a record, not a currency, so it is set as a ledger figure:
    // monospaced, unadorned, no symbol and nothing that could read as a
    // score. The prose beside it is what stops the number being mistaken
    // for one.
    content.append(&style::section(copy::CREDIT_SECTION));
    let credit = style::card(gtk::Orientation::Vertical, space::S);

    // `last_refreshed_at: null` renders as staleness, never as a confident
    // zero: a stale cache presented as current is a lie about a number
    // people will care about.
    match rollup.last_refreshed_at {
        Some(refreshed_at) => {
            // §5.3 sets the figure pair at a 32px gap; `space::XXL` is the
            // scale's nearest step. See the report for the token gap.
            let figures = gtk::Box::new(gtk::Orientation::Horizontal, space::XXL);
            // §5.3 sets the recorded figure in plain ink and the one that
            // is still moving in the second rank -- the settled number is
            // the one being reported, and colouring it would make the pair
            // read as good news and bad news rather than as one ledger.
            for (label, value, muted) in [
                ("Recorded", format!("{:.1}", rollup.credit_final), false),
                (
                    "Still being scored",
                    format!("{:.1}", rollup.credit_pending),
                    true,
                ),
            ] {
                let column = gtk::Box::new(gtk::Orientation::Vertical, space::XXS);
                column.append(&style::eyebrow(label));
                let figure = gtk::Label::builder().label(value).xalign(0.0).build();
                figure.add_css_class("tc-figure");
                if muted {
                    figure.add_css_class(Tone::Neutral.css());
                }
                column.append(&figure);
                figures.append(&column);
            }
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            figures.append(&spacer);

            // When the figures were last known to be true. Bottom-aligned
            // with them, in the third rank of ink -- it is a caveat on the
            // number, not a number of its own.
            let refreshed = gtk::Label::builder()
                .label(format!("Refreshed {}", human_when(Some(refreshed_at))))
                .xalign(1.0)
                .valign(gtk::Align::End)
                .build();
            refreshed.add_css_class("tc-meta");
            refreshed.add_css_class("tc-tertiary");
            figures.append(&refreshed);
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

    // --- Community, in the public surface's own language ------------------
    // §5.5 places it directly below the Credit card, and only for a roster
    // member. Everyone else sees nothing here at all -- there is no empty
    // state for a section that is a consequence of a setting.
    if let Some(standing) = standing {
        content.append(&community_panel(standing));
    }

    // --- The records themselves ------------------------------------------
    // The count is what the rollup says was ever submitted, not how many
    // rows this list happens to be holding: the section header is a fact
    // about the account, and the rows below it are a page of that account.
    if !records.is_empty() {
        let header = style::section(copy::EVERYTHING_CONTRIBUTED);
        let count = gtk::Label::builder()
            .label(format!("{}", rollup.all_time.submitted))
            .xalign(1.0)
            .build();
        count.add_css_class("tc-ledger");
        count.add_css_class("tc-tertiary");
        header.append(&count);
        content.append(&header);
    }
    for record in records {
        content.append(&record_row(record));
    }
}

/// One of §5.3's three stat cards: glyph, eyebrow, figure.
fn stat_card(glyph: Glyph, label: &str, count: u32) -> gtk::Box {
    let card = style::card(gtk::Orientation::Vertical, space::XS);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    head.append(&icon(glyph, 11));
    let caption = style::eyebrow(label);
    // §6.5 puts a stat card's eyebrow in the third rank of ink rather than
    // the second. `tc-tertiary` is declared after `tc-eyebrow` in
    // `style.css`, so at equal specificity it is the colour that lands.
    caption.add_css_class("tc-tertiary");
    caption.set_wrap(true);
    head.append(&caption);
    card.append(&head);
    let figure = gtk::Label::builder()
        .label(format!("{count}"))
        .xalign(0.0)
        .build();
    figure.add_css_class("tc-figure");
    card.append(&figure);
    card
}

/// §5.3's group disclosure row, expanded: chevron, clock, and the count in
/// words. The body inside says what "held" means; it never estimates when
/// the wait ends, because nobody can.
fn held_group(quarantined: u32) -> gtk::Expander {
    let label = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    label.append(&icon(Glyph::Held, 12));
    let heading = gtk::Label::new(Some(&format!(
        "{} \u{2014} {} {}",
        copy::QUARANTINE_HEADING,
        quarantined,
        if quarantined == 1 { "trace" } else { "traces" }
    )));
    heading.add_css_class("tc-card-title");
    label.append(&heading);

    let expander = gtk::Expander::builder().expanded(true).build();
    expander.set_label_widget(Some(&label));

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
    // `trace_commons.daemon.v1_1` this shell can reach -- see the report,
    // which records this as a gap rather than a design choice. Rather than
    // draw a button that cannot work, say plainly where the capability is.
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
    expander
}

/// One record: name and when on the first line, the state as a chip on the
/// right, then whatever that state owes an explanation for.
fn record_row(record: &HistoryRecord) -> gtk::Box {
    let card = style::card(gtk::Orientation::Vertical, space::S);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    let title = gtk::Label::builder()
        .label(&record.project_label)
        .xalign(0.0)
        .wrap(true)
        .build();
    title.add_css_class("tc-card-title");
    top.append(&title);
    let when = gtk::Label::builder()
        .label(human_when(record.submitted_at))
        .xalign(0.0)
        .hexpand(true)
        .build();
    when.add_css_class("tc-meta");
    when.add_css_class("tc-tertiary");
    when.set_halign(gtk::Align::Start);
    top.append(&when);
    top.append(&style::tag(
        status_word(&record.status),
        status_tone(&record.status),
    ));
    card.append(&top);

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
    // Only a held record gets a sentence it did not earn from the server,
    // and only when the server sent none: it is the one state a person can
    // misread as a refusal. The other three are said by the chip and
    // repeating them under it is noise.
    if record.status == "quarantined" && record.explanations.is_empty() {
        let body = gtk::Label::builder()
            .label(copy::HELD_ROW_BODY)
            .xalign(0.0)
            .wrap(true)
            .build();
        body.add_css_class("tc-body");
        card.append(&body);
    }

    // The record's own figures, in the ledger face. Recorded credit is
    // final; anything still being scored is stated as such rather than
    // being added to it, and neither carries a symbol.
    if let Some(figures) = credit_line(record) {
        let line = gtk::Label::builder().label(figures).xalign(0.0).build();
        line.add_css_class("tc-ledger");
        line.add_css_class("tc-neutral");
        card.append(&line);
    }
    card
}

/// The mono figure line under a record, or nothing when there is no figure
/// to state. A withdrawn record keeps whatever it recorded: withdrawal does
/// not reverse settled credit, and implying it did would be a lie about
/// what the action achieved.
fn credit_line(record: &HistoryRecord) -> Option<String> {
    match (record.credit_points_final, record.credit_points_pending) {
        (Some(final_points), _) => Some(format!("credit {final_points:.1}")),
        (None, pending) if pending > 0.0 => {
            Some(format!("credit {pending:.1}, still being scored"))
        }
        _ => None,
    }
}

/// The four states a record can be in, in the same words the stat cards and
/// the chips use, so a badge on a record and a card at the top of the
/// screen cannot say different things about one state.
fn status_word(status: &str) -> &'static str {
    match status {
        "accepted" => copy::HISTORY_IN_THE_COMMONS,
        "quarantined" => copy::QUARANTINE_HEADING,
        "withdrawn" => copy::WITHDRAWN_BY_YOU,
        _ => copy::HISTORY_WAITING_TO_BE_SCORED,
    }
}

fn status_tone(status: &str) -> Tone {
    match status {
        "accepted" => Tone::Clear,
        "quarantined" => Tone::Held,
        "withdrawn" => Tone::Refused,
        _ => Tone::Neutral,
    }
}

// --- Glyphs ---------------------------------------------------------------

/// §5.3's four glyphs, transcribed from the mockup's own `viewBox="0 0 16
/// 16"` paths so a state is legible without its colour.
#[derive(Clone, Copy)]
enum Glyph {
    /// Check in a circle. `m5.3 8.3 1.9 1.9 3.5-4.2`.
    Accepted,
    /// Clock. `M8 4.8V8l2.3 1.4`.
    Held,
    /// The same circle, dashed: nothing has happened to this yet.
    /// `stroke-dasharray="1.8 2.6"`.
    Waiting,
    /// The undo arrow, kept for the surfaces that draw a withdrawn record
    /// with an icon rather than a chip.
    #[allow(dead_code)]
    Withdrawn,
}

impl Glyph {
    /// The ink the glyph is stroked in: the role colour for its state, as
    /// a literal for each scheme.
    ///
    /// A cairo path needs floating-point components and GTK offers no
    /// supported way to read a `@define-color` back out of a provider, so
    /// these are repeated here exactly as `mark.rs` repeats the mark's
    /// inks -- and with the same rule: if one ever drifts from `style.rs`,
    /// `style.rs` is right. They are the text-safe twins (`tc_green_text`,
    /// `tc_blue_icon`, `tc_muted`, `tc_coral_text`), which is what the
    /// mockup strokes an 11px glyph in.
    fn ink(self, scheme: Scheme) -> &'static str {
        match (self, scheme) {
            (Glyph::Accepted, Scheme::Light) => "#0F7256",
            (Glyph::Accepted, Scheme::Dark) => "#5CD3AF",
            (Glyph::Held, Scheme::Light) => "#315FBA",
            (Glyph::Held, Scheme::Dark) => "#9DB6F1",
            (Glyph::Waiting, Scheme::Light) => "#5C635B",
            (Glyph::Waiting, Scheme::Dark) => "#A6AC9F",
            (Glyph::Withdrawn, Scheme::Light) => "#B8483B",
            (Glyph::Withdrawn, Scheme::Dark) => "#F79C8F",
        }
    }
}

/// A status glyph at `size` logical pixels, drawn rather than shipped, on
/// the mockup's 16-unit coordinate space.
fn icon(glyph: Glyph, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(size)
        .content_height(size)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    // Decoration: the words beside it are what a screen reader reads, and
    // announcing "circle" before each of them is noise.
    area.set_can_focus(false);
    area.set_draw_func(move |_, cr, width, height| {
        draw_glyph(cr, glyph, Scheme::current(), width as f64, height as f64);
    });
    follow_scheme(&area);
    area
}

/// Redraw when the desktop flips between light and dark.
///
/// The same shape as `mark::follow_scheme`, and for the same reason: the
/// style manager is a process-wide singleton, so a widget that simply
/// connected to it would be kept alive by that connection for the life of
/// the application. The handler holds a weak reference and is disconnected
/// when the widget goes away.
fn follow_scheme(area: &gtk::DrawingArea) {
    let weak = area.downgrade();
    let handler = RefCell::new(Some(adw::StyleManager::default().connect_dark_notify(
        move |_| {
            if let Some(area) = weak.upgrade() {
                area.queue_draw();
            }
        },
    )));
    area.connect_destroy(move |_| {
        if let Some(id) = handler.borrow_mut().take() {
            adw::StyleManager::default().disconnect(id);
        }
    });
}

/// The mockup's 16-unit space, scaled to whatever the widget was given.
const GLYPH_VIEW: f64 = 16.0;

fn draw_glyph(cr: &gtk::cairo::Context, glyph: Glyph, scheme: Scheme, width: f64, height: f64) {
    let unit = width.min(height) / GLYPH_VIEW;
    if unit <= 0.0 {
        return;
    }
    let _ = cr.save();
    cr.scale(unit, unit);
    set_source(cr, glyph.ink(scheme));
    cr.set_line_width(1.5);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_join(gtk::cairo::LineJoin::Round);

    match glyph {
        Glyph::Accepted => {
            circle(cr);
            let _ = cr.stroke();
            cr.move_to(5.3, 8.3);
            cr.line_to(7.2, 10.2);
            cr.line_to(10.7, 6.0);
            let _ = cr.stroke();
        }
        Glyph::Held => {
            circle(cr);
            let _ = cr.stroke();
            cr.move_to(8.0, 4.8);
            cr.line_to(8.0, 8.0);
            cr.line_to(10.3, 9.4);
            let _ = cr.stroke();
        }
        Glyph::Waiting => {
            cr.set_dash(&[1.8, 2.6], 0.0);
            circle(cr);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
        Glyph::Withdrawn => {
            // `M12 12.5V7.8a3 3 0 0 0-3-3H4.5` -- up the right side, a
            // quarter turn back over the top, then left.
            cr.move_to(12.0, 12.5);
            cr.line_to(12.0, 7.8);
            cr.arc_negative(9.0, 7.8, 3.0, 0.0, -std::f64::consts::FRAC_PI_2);
            cr.line_to(4.5, 4.8);
            let _ = cr.stroke();
            // `M6.8 2.5 4.5 4.8l2.3 2.3` -- the arrowhead on its end.
            cr.move_to(6.8, 2.5);
            cr.line_to(4.5, 4.8);
            cr.line_to(6.8, 7.1);
            let _ = cr.stroke();
        }
    }
    let _ = cr.restore();
}

/// `<circle cx="8" cy="8" r="5.7"/>`, which three of the four glyphs sit
/// inside.
fn circle(cr: &gtk::cairo::Context) {
    cr.new_sub_path();
    cr.arc(8.0, 8.0, 5.7, 0.0, std::f64::consts::TAU);
}

/// Set a `#rrggbb` literal as the source colour. The inks are compile-time
/// constants in this module, so a malformed one is a typo rather than a
/// runtime condition; it leaves the source alone rather than panicking a
/// draw handler.
fn set_source(cr: &gtk::cairo::Context, hex: &str) {
    let Some(hex) = hex.strip_prefix('#') else {
        return;
    };
    if hex.len() != 6 {
        return;
    }
    let channel = |i: usize| {
        u8::from_str_radix(&hex[i..i + 2], 16)
            .ok()
            .map(|v| f64::from(v) / 255.0)
    };
    if let (Some(r), Some(g), Some(b)) = (channel(0), channel(2), channel(4)) {
        cr.set_source_rgb(r, g, b);
    }
}

// --- Community -------------------------------------------------------------

/// §5.5's brand panel: heading and link, a four-cell metric strip, the meta
/// row, and the withheld-analytics notice. Everything inside the 2px black
/// frame is public; the frame is where this window stops.
fn community_panel(standing: &RosterStanding) -> gtk::Box {
    // The panel and its footnote, stacked. The footnote is native type on
    // the window's own ground -- it is this window talking about the panel,
    // not part of it.
    let column = gtk::Box::new(gtk::Orientation::Vertical, space::M);

    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .build();
    panel.add_css_class("tc-brand-panel");

    // 1. Heading, and the way out to the public page.
    let head = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
    let heading = gtk::Label::builder()
        .label(copy::COMMUNITY_HEADING.to_uppercase())
        .xalign(0.0)
        .hexpand(true)
        .build();
    heading.add_css_class("tc-brand-display");
    head.append(&heading);
    // Only drawn when there is somewhere for it to go.
    if let Some(url) = standing.profile_url.as_deref() {
        let link = gtk::LinkButton::builder()
            .uri(url)
            .label(copy::VIEW_PUBLIC_PROFILE.to_uppercase())
            .valign(gtk::Align::End)
            .build();
        link.add_css_class("tc-brand-link");
        head.append(&link);
    }
    panel.append(&head);

    // 2. The metric strip: one framed box divided into four equal cells by
    // 1px rules, which is the site's table, not a row of cards.
    let strip = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .homogeneous(true)
        .build();
    strip.add_css_class("tc-brand-box");
    let cells = [
        (
            "Rank".to_string(),
            standing.rank.map(|r| format!("#{r}")).unwrap_or_else(dash),
        ),
        (
            "Novelty credit".to_string(),
            grouped(standing.novelty_credit),
        ),
        (
            format!("Accepted \u{00b7} {}", window_label(standing)),
            format!("{}", standing.accepted_in_window),
        ),
        (
            "Accept rate".to_string(),
            standing
                .accept_rate
                .map(|r| format!("{:.0}%", r * 100.0))
                .unwrap_or_else(dash),
        ),
    ];
    let last = cells.len() - 1;
    for (index, (label, value)) in cells.iter().enumerate() {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 6);
        cell.add_css_class("tc-brand-cell");
        // The 1px rule belongs to the cell that has a neighbour to its
        // right, so the last cell simply never asks for one.
        if index != last {
            cell.add_css_class("tc-brand-divided");
        }
        let caption = gtk::Label::builder()
            .label(label.to_uppercase())
            .xalign(0.0)
            .build();
        caption.add_css_class("tc-brand-label");
        cell.append(&caption);
        let figure = gtk::Label::builder().label(value).xalign(0.0).build();
        figure.add_css_class("tc-brand-figure");
        cell.append(&figure);
        strip.append(&cell);
    }
    panel.append(&strip);

    // 3. What the figures are figures of. Stated, never assumed.
    let meta = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    let mut facts = vec![format!("Window {}", window_label(standing))];
    if let Some(since) = standing.public_since {
        facts.push(format!("Public since {}", since.format("%B %-d, %Y")));
    }
    if let Some(snapshot) = standing.snapshot_at {
        facts.push(format!("Snapshot {}", human_when(Some(snapshot))));
    }
    for fact in facts {
        let label = gtk::Label::builder().label(fact.to_uppercase()).build();
        label.add_css_class("tc-brand-label");
        meta.append(&label);
    }
    panel.append(&meta);

    // 4. §7.3: analytics that are withheld are stated in words, never as an
    // empty chart. There is deliberately nothing chart-shaped in here.
    if standing.analytics_withheld {
        let notice = gtk::Box::new(gtk::Orientation::Vertical, 0);
        notice.add_css_class("tc-brand-notice");
        let body = gtk::Label::builder()
            .label(copy::COMMUNITY_ANALYTICS_WITHHELD)
            .xalign(0.0)
            .wrap(true)
            .build();
        body.add_css_class("tc-brand-body");
        notice.append(&body);
        panel.append(&notice);
    }

    column.append(&panel);

    // 5. The footnote, outside the frame, because it is about a switch in
    // this window rather than about the public page.
    let footnote = gtk::Label::builder()
        .label(copy::COMMUNITY_FOOTNOTE)
        .xalign(0.0)
        .wrap(true)
        .build();
    footnote.add_css_class("tc-meta");
    footnote.add_css_class("tc-tertiary");
    column.append(&footnote);
    column
}

/// The roster's window as the server labels it, falling back to nothing
/// rather than to a length this shell guessed.
fn window_label(standing: &RosterStanding) -> &str {
    if standing.window_label.is_empty() {
        "\u{2014}"
    } else {
        &standing.window_label
    }
}

fn dash() -> String {
    "\u{2014}".to_string()
}

/// A brand figure, grouped in threes. The strip is set in tabular figures
/// and the site writes "1,240", so the separator is part of the type rather
/// than a formatting flourish.
fn grouped(value: f64) -> String {
    let rounded = value.round().abs() as u64;
    let digits = rounded.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if value < 0.0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn withdrawn_records_read_as_withdrawn() {
        // The rule this exists to hold: a withdrawn trace stays on the
        // list and says so. It must never fall through to "waiting to be
        // scored", which would read as though it were still in flight.
        assert_eq!(status_word("withdrawn"), copy::WITHDRAWN_BY_YOU);
        assert!(matches!(status_tone("withdrawn"), Tone::Refused));
    }

    #[test]
    fn quarantine_never_reads_as_a_refusal() {
        assert_eq!(status_word("quarantined"), copy::QUARANTINE_HEADING);
        assert!(matches!(status_tone("quarantined"), Tone::Held));
        let words = status_word("quarantined").to_lowercase();
        assert!(!words.contains("reject"));
        assert!(!words.contains("fail"));
        // And the row-level sentence carries the same denial the section
        // body does, without ever estimating a wait.
        let body = copy::HELD_ROW_BODY.to_lowercase();
        assert!(body.contains("has not been rejected"));
        for forbidden in ["hours", "business days", "within a week", "usually takes"] {
            assert!(
                !body.contains(forbidden),
                "no turnaround time may be stated"
            );
        }
    }

    #[test]
    fn an_unknown_status_is_never_claimed_to_be_in_the_commons() {
        for status in ["submitted", "pending", "something-new"] {
            assert_eq!(status_word(status), copy::HISTORY_WAITING_TO_BE_SCORED);
        }
    }

    #[test]
    fn no_credit_figure_carries_a_symbol_or_a_projection() {
        let record = HistoryRecord {
            submitted_at: None,
            project_label: String::new(),
            status: "accepted".to_string(),
            credit_points_pending: 0.0,
            credit_points_final: Some(12.5),
            explanations: vec![],
        };
        let line = credit_line(&record).expect("a final figure is a figure");
        assert_eq!(line, "credit 12.5");
        for forbidden in ['$', '€', '£'] {
            assert!(!line.contains(forbidden));
        }
    }

    #[test]
    fn a_record_with_nothing_scored_yet_states_that_rather_than_a_zero() {
        let mut record = HistoryRecord {
            submitted_at: None,
            project_label: String::new(),
            status: "submitted".to_string(),
            credit_points_pending: 3.0,
            credit_points_final: None,
            explanations: vec![],
        };
        assert_eq!(
            credit_line(&record).as_deref(),
            Some("credit 3.0, still being scored")
        );
        record.credit_points_pending = 0.0;
        assert_eq!(credit_line(&record), None);
    }

    #[test]
    fn brand_figures_are_grouped_in_threes() {
        assert_eq!(grouped(1240.0), "1,240");
        assert_eq!(grouped(9.0), "9");
        assert_eq!(grouped(1_000_000.4), "1,000,000");
    }

    #[test]
    fn the_community_section_stays_dark_until_the_daemon_publishes_one() {
        // No roster payload, no panel: §5.5's non-roster state is that the
        // section simply does not render, and nothing here invents a rank.
        let empty = serde_json::json!({ "credit_final": 12.0 });
        assert!(RosterStanding::from_rollup(&empty).is_none());

        let present = serde_json::json!({
            "community": { "rank": 14, "novelty_credit": 1240.0, "window_label": "7d" }
        });
        let standing = RosterStanding::from_rollup(&present).expect("a roster payload parses");
        assert_eq!(standing.rank, Some(14));
        // Withheld is the default, because it is the conservative reading
        // of a field the server has not sent.
        assert!(standing.analytics_withheld);
        assert_eq!(standing.accept_rate, None);
    }
}
