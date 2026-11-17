//! The trace being contributed, kept or dismissed: a card at the foot of
//! the Traces map, above its legend and zoom, while a tab other than
//! Traces is shown (owner-approved for this shell, 2026-11-16).
//!
//! A port of `TraceTransferCard.swift`: the tool, what is happening to
//! the trace, its folder and time, and, behind Details, the note. Its X
//! closes it alone. Every word is the shared copy's.
//!
//! Deliberate platform differences:
//! - The memory line in Details says "Memory files stay on this computer."
//!   (`memory_stays_computer`) where the Mac's says "this Mac".
//! - The coin carries the tool's initials, not its logo (the look slice),
//!   and a batch from several tools carries none.
//! - The cards are a short column, newest nearest the legend, not the
//!   Mac's peeking stack; at most `Player::VISIBLE` are drawn.

use adw::prelude::*;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

use super::map_scene;
use super::transfers::{self, MapTransfer, Tone};

/// One card. `from_map` is the folder's name on the map, for a transfer
/// that carries none; `on_close` runs on its X.
pub fn card(
    transfer: &MapTransfer,
    from_map: Option<&str>,
    on_close: impl Fn() + 'static,
) -> gtk::Box {
    let screens = monitor_screens_copy();
    let words = map_scene::words();
    let tool = (!transfer.mixed_tools).then(|| transfers::tool_name(&transfer.tool));
    let (tag, tone, note) = transfers::card_state(transfer.action, tool.as_deref(), &words);
    let title = transfers::card_title(transfer, &words);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("tc-map-transfer-card");
    card.add_css_class("tc-glass-popover");
    card.set_accessible_role(gtk::AccessibleRole::Group);
    card.update_property(&[gtk::accessible::Property::Label(&title)]);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let mark = gtk::Label::new(Some(
        &tool.as_deref().map(map_scene::initials).unwrap_or_default(),
    ));
    mark.add_css_class("tc-map-transfer-mark");
    mark.set_valign(gtk::Align::Center);
    mark.set_accessible_role(gtk::AccessibleRole::Presentation);
    head.append(&mark);
    let name = gtk::Label::new(Some(&title));
    name.add_css_class("heading");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    head.append(&name);
    let badge = gtk::Label::new(Some(tag));
    badge.add_css_class("tc-map-transfer-tag");
    badge.add_css_class("tc-type-label");
    if tone == Tone::On {
        badge.add_css_class("tc-map-transfer-tag-on");
    }
    badge.set_valign(gtk::Align::Center);
    head.append(&badge);
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.add_css_class("flat");
    close.add_css_class("circular");
    close.set_valign(gtk::Align::Center);
    close.set_tooltip_text(Some(screens.close));
    close.update_property(&[gtk::accessible::Property::Label(screens.close)]);
    close.connect_clicked(move |_| on_close());
    head.append(&close);
    card.append(&head);

    // The heading over the trace, then its folder and time; the folder
    // gives way, never the time.
    let summary = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let eyebrow = gtk::Label::new(Some(
        &transfers::card_eyebrow(transfer, &words).to_uppercase(),
    ));
    eyebrow.add_css_class("caption");
    eyebrow.add_css_class("dim-label");
    summary.append(&eyebrow);
    if !transfer.mixed_folders {
        let folder = gtk::Label::new(Some(&transfers::card_folder(transfer, from_map)));
        folder.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        folder.set_xalign(0.0);
        folder.set_hexpand(true);
        summary.append(&folder);
    }
    if let Some(time) = transfers::card_time(transfer.at) {
        let when = gtk::Label::new(Some(&format!("\u{00B7} {time}")));
        when.add_css_class("dim-label");
        summary.append(&when);
    }
    card.append(&summary);

    if let Some(note) = note {
        let details = gtk::Expander::new(Some(screens.shell.notice_details));
        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        // The tool's memory files, and that they stay on this computer,
        // above the note, for one tool's trace (`TraceTransferCard`).
        if let Some((files, stays)) = transfers::card_memory(transfer, &words) {
            let memory = gtk::Box::new(gtk::Orientation::Vertical, 2);
            memory.add_css_class("tc-map-transfer-memory");
            if let Some(files) = files {
                let names = gtk::Label::new(Some(files));
                names.add_css_class("monospace");
                names.set_xalign(0.0);
                names.set_wrap(true);
                memory.append(&names);
            }
            let line = gtk::Label::new(Some(stays));
            line.add_css_class("caption");
            line.add_css_class("dim-label");
            line.set_xalign(0.0);
            line.set_wrap(true);
            memory.append(&line);
            body.append(&memory);
        }
        let text = gtk::Label::new(Some(&note));
        text.set_wrap(true);
        text.set_xalign(0.0);
        text.add_css_class("dim-label");
        body.append(&text);
        details.set_child(Some(&body));
        card.append(&details);
    }
    card
}
