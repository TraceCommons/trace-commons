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
use crate::copy;
use crate::model::{QueueEntry, human_bytes, human_when};

pub struct QueueView {
    pub root: gtk::Box,
    list: gtk::Box,
    empty: adw::StatusPage,
    scroller: gtk::ScrolledWindow,
}

impl QueueView {
    pub fn new() -> Self {
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("emblem-ok-symbolic")
            .title(copy::QUEUE_EMPTY_TITLE)
            .description(copy::QUEUE_EMPTY_BODY)
            .vexpand(true)
            .build();

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&empty);
        root.append(&scroller);

        Self {
            root,
            list,
            empty,
            scroller,
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
            .label(&format!("Not sent ({}) ", resolved.len()))
            .build();
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
        inner.set_margin_top(6);
        for entry in resolved {
            let reason = entry
                .reason_label
                .as_deref()
                .map(copy::reason_sentence)
                .unwrap_or("Nothing was sent.");
            let line = gtk::Label::builder()
                .label(format!("{} - {}", entry.project_label, reason))
                .xalign(0.0)
                .wrap(true)
                .build();
            line.add_css_class("dim-label");
            inner.append(&line);
        }
        expander.set_child(Some(&inner));
        view.list.append(&expander);
    }
}

fn row(app: &Rc<App>, entry: &QueueEntry, index: usize) -> gtk::Widget {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&card));

    let title = gtk::Label::builder()
        .label(&entry.project_label)
        .xalign(0.0)
        .wrap(true)
        .build();
    title.add_css_class("title-4");
    card.append(&title);

    let preview = app.previews.borrow().get(&entry.entry_id).cloned();

    // Turn count comes from the preview's event count; the queue entry
    // itself does not carry one.
    let turns = preview
        .as_ref()
        .map(|p| format!(" - {} turns", p.event_count))
        .unwrap_or_default();
    let facts = gtk::Label::builder()
        .label(format!(
            "{} - {}{}",
            entry.agent_label(),
            human_when(entry.discovered_at),
            turns
        ))
        .xalign(0.0)
        .wrap(true)
        .build();
    facts.add_css_class("dim-label");
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
    card.append(&prompt);

    let receipt = gtk::Label::builder()
        .label(match preview.as_ref() {
            Some(p) => format!(
                "Would send {}  -  {}",
                human_bytes(p.would_send_bytes),
                p.scrubbed_line()
            ),
            None => copy::CHECKING.to_string(),
        })
        .xalign(0.0)
        .wrap(true)
        .build();
    card.append(&receipt);

    // Always shown, never hidden behind a disclosure: conceding that
    // scrubbing is imperfect is what makes the rest credible.
    let risk = gtk::Label::builder()
        .label(copy::RESIDUAL_RISK)
        .xalign(0.0)
        .wrap(true)
        .build();
    risk.add_css_class("dim-label");
    card.append(&risk);

    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_top(6)
        .build();
    let look = gtk::Button::with_label(copy::LOOK_INSIDE);
    look.add_css_class("suggested-action");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let skip = gtk::Button::with_label(copy::NOT_THIS_ONE);
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

    frame.upcast()
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
