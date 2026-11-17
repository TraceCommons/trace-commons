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
            // The Mac's lock sits beside the files and the line.
            let memory = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            memory.add_css_class("tc-map-transfer-memory");
            let lock = super::map::lock_icon();
            lock.set_valign(gtk::Align::Start);
            lock.set_margin_top(2);
            memory.append(&lock);
            let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
            lines.set_hexpand(true);
            if let Some(files) = files {
                let names = gtk::Label::new(Some(files));
                names.add_css_class("monospace");
                names.set_xalign(0.0);
                names.set_wrap(true);
                lines.append(&names);
            }
            let line = gtk::Label::new(Some(stays));
            line.add_css_class("caption");
            line.add_css_class("dim-label");
            line.set_xalign(0.0);
            line.set_wrap(true);
            lines.append(&line);
            memory.append(&lines);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::monitor::transfers::Action;

    fn descendants(root: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        let mut child = root.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            out.push(widget.clone());
            descendants(&widget, out);
        }
    }

    /// The memory well in Details carries the Mac's lock beside the files.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_the_memory_well_has_the_lock() {
        gtk::init().expect("GTK display");
        let transfer = MapTransfer::new("t1", "folder", "claude-code", Action::Contribute);
        let card = card(&transfer, None, || {});
        // A collapsed GtkExpander keeps its child out of the widget tree,
        // so open Details first, as the viewer does.
        let mut all = Vec::new();
        descendants(card.upcast_ref(), &mut all);
        let details = all
            .iter()
            .find_map(|w| w.downcast_ref::<gtk::Expander>().cloned())
            .expect("the Details expander");
        details.set_expanded(true);
        let mut all = Vec::new();
        descendants(card.upcast_ref(), &mut all);
        let well = all
            .iter()
            .find(|w| w.has_css_class("tc-map-transfer-memory"))
            .expect("the memory well");
        let first = well.first_child().expect("the well's first child");
        assert!(
            first.is::<gtk::DrawingArea>(),
            "the well opens with {}",
            first.type_().name()
        );
    }
}
