//! The queue: what is waiting for a decision.
//!
//! The row carries what identifies a session to its author -- the project
//! label, the agent, when, how long, and the redacted opening prompt -- plus
//! what would be sent and what scrubbing found. What it deliberately does
//! **not** carry is an approve button. Approving from the row is approving
//! without looking, which is the misclick the preview-then-approve rule
//! exists to prevent; `Contribute` lives in the preview sheet and nowhere
//! else.
//!
//! No filesystem path is ever rendered here. `project_label` is what a
//! contributor sees and `project_id` is what goes back to the daemon; the
//! path does not cross the socket at all, so there is nothing to leak.

use std::rc::Rc;

use adw::prelude::*;

use super::App;
use super::style::{self, Tone, space};
use crate::copy;
use crate::model::{QueueEntry, human_bytes, human_when};

pub struct QueueView {
    pub root: gtk::Box,
    list: gtk::Box,
    empty: adw::StatusPage,
    scroller: gtk::ScrolledWindow,
    heading: gtk::Label,
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
            .spacing(space::M)
            .build();

        // A reading column. Trust copy is read, not skimmed, and a sentence
        // that runs the full width of a maximised window is a sentence
        // nobody finishes. It also keeps a card's two actions within one
        // eye movement of each other.
        let heading = gtk::Label::builder().xalign(0.0).wrap(true).build();
        heading.add_css_class("tc-screen-title");

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(space::M)
            .margin_top(space::XL)
            .margin_bottom(space::XL)
            .margin_start(space::L)
            .margin_end(space::L)
            .build();
        column.append(&heading);
        column.append(&list);

        let clamp = adw::Clamp::builder()
            .maximum_size(840)
            .tightening_threshold(680)
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

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("tc-root");
        root.append(&empty);
        root.append(&scroller);

        Self {
            root,
            list,
            empty,
            scroller,
            heading,
        }
    }
}

pub fn wire(_app: &Rc<App>) {}

pub fn render(app: &Rc<App>) {
    let view = &app.queue;
    while let Some(child) = view.list.first_child() {
        view.list.remove(&child);
    }

    let entries = app.entries.borrow();
    let pending: Vec<&QueueEntry> = entries.iter().filter(|e| e.state == "pending").collect();

    view.empty.set_visible(pending.is_empty());
    view.scroller.set_visible(!pending.is_empty());
    view.heading.set_text(&match pending.len() {
        1 => "1 session is waiting for your decision".to_string(),
        n => format!("{n} sessions are waiting for your decision"),
    });

    for (index, entry) in pending.iter().enumerate() {
        view.list.append(&row(app, entry, index));
    }

    // Sessions that were queued and then resolved without being sent.
    // Surfacing them is what keeps "not sent" distinguishable from "sent",
    // and it is why the queue can always explain itself.
    let resolved: Vec<&QueueEntry> = entries
        .iter()
        .filter(|e| matches!(e.state.as_str(), "refused" | "expired" | "superseded"))
        .collect();
    if !resolved.is_empty() {
        let expander = gtk::Expander::builder()
            .label(format!("Not sent ({})", resolved.len()))
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
        view.list.append(&expander);
    }
}

/// One session, as a declaration form.
///
/// Every card is built the same way and in the same order -- who and when,
/// the opening prompt, the manifest strip, the caveat, the two actions --
/// so a person reading a column of them can stop reading and start
/// scanning. See `style::manifest`.
fn row(app: &Rc<App>, entry: &QueueEntry, index: usize) -> gtk::Widget {
    let card = style::card(gtk::Orientation::Vertical, space::S);

    let preview = app.previews.borrow().get(&entry.entry_id).cloned();
    let redactions: Option<u32> = preview.as_ref().map(|p| p.redactions.values().sum());

    let top = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    let title = gtk::Label::builder()
        .label(&entry.project_label)
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .build();
    title.add_css_class("tc-card-title");
    top.append(&title);
    // The one card-level state worth putting a badge on: scrubbing that
    // matched nothing at all. Everything else is left to the strip, because
    // a badge on every card is a badge on none.
    if redactions == Some(0) {
        card.add_css_class("tc-flagged");
        top.append(&style::tag("Nothing matched", Tone::Attention));
    }
    card.append(&top);

    let facts = gtk::Label::builder()
        .label(format!(
            "{} - {}",
            entry.agent_label(),
            human_when(entry.discovered_at)
        ))
        .xalign(0.0)
        .wrap(true)
        .build();
    facts.add_css_class("tc-meta");
    card.append(&facts);

    // The redacted opening prompt: what actually identifies a session to the
    // person who ran it. A timestamp does not.
    let prompt = gtk::Label::builder()
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
    prompt.add_css_class("tc-body");
    card.append(&prompt);

    card.append(&manifest_for(preview.as_ref()));

    // Never hidden behind a disclosure: conceding that scrubbing is
    // imperfect is what makes the rest credible. What changes here is that
    // the sentence describes this session rather than repeating a constant
    // -- see `copy::residual_risk_line`.
    let risk = gtk::Label::builder()
        .label(match redactions {
            Some(total) => copy::residual_risk_line(total),
            None => copy::CHECKING.to_string(),
        })
        .xalign(0.0)
        .wrap(true)
        .build();
    risk.add_css_class("tc-caveat");
    if redactions == Some(0) {
        risk.add_css_class("tc-attention");
    }
    card.append(&risk);

    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(space::S)
        .margin_top(space::XXS)
        .build();
    let look = gtk::Button::with_label(copy::LOOK_INSIDE);
    look.add_css_class("suggested-action");
    look.add_css_class("tc-primary");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let skip = gtk::Button::with_label(copy::NOT_THIS_ONE);
    skip.add_css_class("tc-quiet");
    skip.set_tooltip_text(Some(copy::NOT_THIS_ONE_TOOLTIP));
    buttons.append(&look);
    buttons.append(&spacer);
    buttons.append(&skip);
    card.append(&buttons);

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

    card.upcast()
}

/// The manifest strip: the same four fields, in the same order, wherever a
/// session is shown. The preview sheet builds the identical strip from the
/// identical function, so a person who scanned a row and then opened it is
/// looking at the same four numbers in the same four places.
///
/// A row whose preview has not arrived yet shows the fields with em dashes
/// rather than collapsing the strip. The rhythm is the whole point, and a
/// strip that appears and disappears as previews land destroys it.
pub fn manifest_for(preview: Option<&crate::model::PreviewSummary>) -> gtk::Box {
    let Some(p) = preview else {
        return style::manifest(&[
            ("Turns", "-".into(), Tone::Neutral),
            ("Would send", "-".into(), Tone::Neutral),
            ("Scrubbed", "checking".into(), Tone::Neutral),
            ("Personal info", "-".into(), Tone::Neutral),
        ]);
    };
    let total: u32 = p.redactions.values().sum();
    style::manifest(&[
        ("Turns", format!("{}", p.event_count), Tone::Neutral),
        ("Would send", human_bytes(p.would_send_bytes), Tone::Neutral),
        (
            "Scrubbed",
            // The strip carries figures, so the receipt's prose form
            // ("scrubbed: 12 secrets, 4 tokens") belongs in the sheet and
            // the count belongs here. A zero is stated, never hidden.
            match total {
                0 => "nothing".to_string(),
                n => format!("{n} removed"),
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
