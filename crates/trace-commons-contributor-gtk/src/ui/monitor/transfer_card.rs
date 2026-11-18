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
//! - A batch from several tools shows the stock stacked-pages icon where
//!   the Mac draws its `square.stack` symbol (owner, 2026-11-18: stock
//!   symbolic icons for the Mac's SF Symbols).
//! - The cards are a short column, newest nearest the legend, not the
//!   Mac's peeking stack; at most `Player::VISIBLE` are drawn.

use adw::prelude::*;
use trace_commons_contributor::preview_copy::monitor_screens_copy;

use super::map::{self, MapColours, MarkFont};
use super::map_scene::{self, Point};
use super::tool_art;
use super::traces_tree::SourceKind;
use super::transfers::{self, MapTransfer, Tone};

/// The coin's side (`GlassNoticeMark.side`).
pub const COIN_SIDE: i32 = 30;

/// What the card's coin carries (`TraceTransferCard.toolMark`).
#[derive(Clone, Debug, PartialEq)]
pub enum CardMark {
    /// A tool this shell knows: its logo or initials, in its tint.
    Tool(SourceKind),
    /// Another tool: two letters of its name, in the primary ink.
    Other(String),
    /// A batch from several tools: no tool's mark.
    Mixed,
}

pub fn card_mark(transfer: &MapTransfer) -> CardMark {
    if transfer.mixed_tools {
        return CardMark::Mixed;
    }
    match SourceKind::from_wire(&transfer.tool) {
        Some(kind) => CardMark::Tool(kind),
        None => CardMark::Other(map_scene::initials(&transfer.tool)),
    }
}

/// The coin as `TraceTransferCard.coin` draws it, `COIN_SIDE` across at
/// the context's origin: an opaque face, a 2-point rim in the tool's tint
/// inside its edge, and the logo 14 across or the initials, monospaced 10
/// bold, in the tint. A mixed batch's coin is drawn as a stock icon
/// instead, so here only its face.
pub fn draw_coin(cr: &gtk::cairo::Context, mark: &CardMark, colours: &MapColours) {
    let side = f64::from(COIN_SIDE);
    let centre = Point::new(side / 2.0, side / 2.0);
    map::circle(cr, centre, side / 2.0);
    map::source(cr, &colours.pane_opaque, 1.0);
    let _ = cr.fill();
    let (tint, logo, initials) = match mark {
        CardMark::Tool(kind) => {
            let art = tool_art::tool_art(*kind);
            (colours.tint(art.tint), art.logo, art.initials.to_string())
        }
        CardMark::Other(letters) => (colours.text_primary, None, letters.clone()),
        CardMark::Mixed => return,
    };
    map::circle(cr, centre, side / 2.0 - 1.0);
    cr.set_line_width(2.0);
    map::source(cr, &tint, 1.0);
    let _ = cr.stroke();
    map::source(cr, &tint, 1.0);
    match logo {
        Some(logo) => logo.fill(cr, (centre.x - 7.0, centre.y - 7.0), 14.0),
        None => {
            let font = MarkFont::coin(10.0 / 12.0);
            map::centred_text(cr, &initials, centre, font.size, font.family);
        }
    }
}

/// The card's coin, drawn in the current scheme.
fn coin(mark: CardMark) -> gtk::Widget {
    if mark == CardMark::Mixed {
        let icon = gtk::Image::from_icon_name("edit-copy-symbolic");
        icon.set_pixel_size(16);
        icon.set_size_request(COIN_SIDE, COIN_SIDE);
        icon.add_css_class("dim-label");
        return icon.upcast();
    }
    let area = gtk::DrawingArea::new();
    area.set_content_width(COIN_SIDE);
    area.set_content_height(COIN_SIDE);
    area.set_draw_func(move |_, cr, _, _| {
        let dark = adw::StyleManager::default().is_dark();
        draw_coin(cr, &mark, &MapColours::scheme(dark));
    });
    area.upcast()
}

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
    let mark = coin(card_mark(transfer));
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
    use crate::ui::monitor::map::MapColours;
    use crate::ui::monitor::tool_art::Tint;
    use crate::ui::monitor::transfers::Action;

    fn descendants(root: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        let mut child = root.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            out.push(widget.clone());
            descendants(&widget, out);
        }
    }

    /// The card's coin carries what `TraceTransferCard.toolMark` gives it:
    /// a known tool's own mark (Codex its logo, Gemini CLI "Gm", Cline
    /// "Cl"), another tool two letters of its name, and a batch from
    /// several tools no tool's mark.
    #[test]
    fn the_coin_carries_the_tools_own_mark() {
        let mark = |tool: &str, mixed: bool| {
            let mut transfer = MapTransfer::new("t1", "folder", tool, Action::Contribute);
            transfer.mixed_tools = mixed;
            card_mark(&transfer)
        };
        assert_eq!(mark("codex", false), CardMark::Tool(SourceKind::Codex));
        assert_eq!(
            mark("gemini-cli", false),
            CardMark::Tool(SourceKind::GeminiCli)
        );
        assert_eq!(mark("aider", false), CardMark::Other("Ai".into()));
        assert_eq!(mark("codex", true), CardMark::Mixed);
    }

    /// The pixels of a coin drawn 30 across (`GlassNoticeMark.side`) in
    /// the light scheme, unpremultiplied, with any alpha.
    fn coin_pixels(mark: &CardMark) -> Vec<[u8; 4]> {
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 30, 30).unwrap();
        {
            let cr = gtk::cairo::Context::new(&surface).unwrap();
            draw_coin(&cr, mark, &MapColours::scheme(false));
        }
        surface.flush();
        let data = surface.data().unwrap();
        data.chunks(4)
            .filter(|px| px[3] > 200)
            .map(|px| {
                let a = f64::from(px[3]) / 255.0;
                let c = |v: u8| (f64::from(v) / a).round().min(255.0) as u8;
                [c(px[2]), c(px[1]), c(px[0]), px[3]]
            })
            .collect()
    }

    fn near(px: [u8; 4], colour: &gtk::gdk::RGBA) -> bool {
        let target = [colour.red(), colour.green(), colour.blue()];
        (0..3).all(|k| (f32::from(px[k]) - target[k] * 255.0).abs() <= 14.0)
    }

    /// The coin is the Mac's: an opaque face, a 2-point rim in the tool's
    /// tint, and the tool's logo (14 across) or initials in that tint. A
    /// tool with a logo paints more of its tint inside the rim than the rim
    /// alone; Gemini CLI's initials are in the Antigravity tint.
    #[test]
    fn the_coin_is_drawn_in_the_tools_tint() {
        let colours = MapColours::scheme(false);
        let codex = coin_pixels(&CardMark::Tool(SourceKind::Codex));
        let tint = colours.tint(Tint::Codex);
        let face = codex
            .iter()
            .filter(|px| near(**px, &colours.pane_opaque))
            .count();
        assert!(face > 150, "the face drew {face} pixels");
        let inside = |pixels: &[[u8; 4]], colour: &gtk::gdk::RGBA| {
            pixels.iter().filter(|px| near(**px, colour)).count()
        };
        let rim_only = inside(&coin_pixels(&CardMark::Mixed), &tint);
        assert!(
            inside(&codex, &tint) > 150,
            "Codex's rim and logo drew {} pixels in its tint",
            inside(&codex, &tint)
        );
        assert_eq!(rim_only, 0, "a mixed batch's coin carries no tool's tint");
        let gemini = coin_pixels(&CardMark::Tool(SourceKind::GeminiCli));
        assert!(inside(&gemini, &colours.tint(Tint::Antigravity)) > 80);
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
