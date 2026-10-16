//! "Open sessions in": the terminal app managed sessions open in, in
//! Settings beside the Private AI card, as the macOS app has it (#1406).
//!
//! The label is the core's (`terminal_choice` in
//! `managed::copy`); the options are the app names the daemon sends in
//! the managed snapshot's `capabilities.terminal_choices`, which are the
//! terminals it found on this computer. Nothing here is typed in this
//! shell. The card is hidden when the daemon offers none, which is always
//! the case under Flatpak.
use super::{App, style};
use adw::prelude::*;
use serde_json::{Value, json};
use std::rc::Rc;

/// The picker's model from a managed snapshot: each offered app's id and
/// name, and the position of the current choice among them. `None` when
/// nothing is offered. A current choice that is no longer offered (or
/// that the daemon does not know) selects nothing rather than the first.
pub fn model(snapshot: &Value) -> Option<(Vec<(String, String)>, Option<u32>)> {
    let capabilities = &snapshot["capabilities"];
    let choices: Vec<(String, String)> = capabilities["terminal_choices"]
        .as_array()?
        .iter()
        .filter_map(|choice| {
            Some((
                choice["id"].as_str()?.to_owned(),
                choice["name"].as_str()?.to_owned(),
            ))
        })
        .collect();
    if choices.is_empty() {
        return None;
    }
    let current = capabilities["terminal_current"].as_str();
    let selected = current
        .and_then(|id| choices.iter().position(|(choice, _)| choice == id))
        .map(|position| position as u32);
    Some((choices, selected))
}

fn text(key: &str) -> String {
    trace_commons_contributor::managed::copy::copy()[key]
        .as_str()
        .unwrap_or_default()
        .into()
}

fn clear(root: &gtk::Box) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
}

/// Ask the daemon for the snapshot and draw the card.
pub fn refresh(app: &Rc<App>) {
    app.call("managed_snapshot", json!({}), |app, result| {
        render(app, result.ok().as_ref());
    });
}

fn render(app: &Rc<App>, snapshot: Option<&Value>) {
    let root = &app.settings.managed_terminal;
    clear(root);
    let Some((choices, selected)) = snapshot.and_then(model) else {
        root.set_visible(false);
        return;
    };
    let label_text = text("terminal_choice");
    let card = style::card(gtk::Orientation::Horizontal, style::space::M);
    let label = gtk::Label::builder()
        .label(&label_text)
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .build();
    label.add_css_class("tc-body");
    card.append(&label);
    let names: Vec<&str> = choices.iter().map(|(_, name)| name.as_str()).collect();
    let dropdown = gtk::DropDown::from_strings(&names);
    dropdown.update_property(&[gtk::accessible::Property::Label(&label_text)]);
    dropdown.set_selected(selected.unwrap_or(gtk::INVALID_LIST_POSITION));
    let ids: Vec<String> = choices.into_iter().map(|(id, _)| id).collect();
    let a = Rc::clone(app);
    dropdown.connect_selected_notify(move |dropdown| {
        let position = dropdown.selected();
        if Some(position) == selected {
            return;
        }
        let Some(wanted) = ids.get(position as usize) else {
            return;
        };
        dropdown.set_sensitive(false);
        a.call(
            "set_managed_terminal",
            json!({ "terminal": wanted }),
            |app, result| {
                if result.is_err() {
                    app.toast(&text("action_failed"));
                }
                // Moved only by a write the daemon accepted: re-read.
                refresh(app);
            },
        );
    });
    card.append(&dropdown);
    root.append(&card);
    root.set_visible(true);
}

#[cfg(test)]
mod tests {
    use super::model;
    use serde_json::json;

    #[test]
    fn the_picker_offers_the_daemons_apps_and_selects_the_current_one() {
        let snapshot = json!({"capabilities":{
            "terminal_current":"konsole",
            "terminal_choices":[
                {"id":"gnome_terminal","name":"GNOME Terminal"},
                {"id":"konsole","name":"Konsole"}
            ]
        }});
        let (choices, selected) = model(&snapshot).unwrap();
        assert_eq!(
            choices,
            [
                ("gnome_terminal".to_owned(), "GNOME Terminal".to_owned()),
                ("konsole".to_owned(), "Konsole".to_owned())
            ]
        );
        assert_eq!(selected, Some(1));
    }

    #[test]
    fn a_choice_no_longer_found_selects_nothing() {
        for current in [json!("ghostty"), json!(null)] {
            let snapshot = json!({"capabilities":{
                "terminal_current": current,
                "terminal_choices":[{"id":"konsole","name":"Konsole"}]
            }});
            assert_eq!(model(&snapshot).unwrap().1, None);
        }
    }

    #[test]
    fn nothing_offered_draws_nothing() {
        assert_eq!(
            model(&json!({"capabilities":{"terminal_choices":[]}})),
            None
        );
        // An older daemon sends no choices at all.
        assert_eq!(model(&json!({"capabilities":{}})), None);
        assert_eq!(model(&json!({})), None);
    }

    /// The label is the core's approved words.
    #[test]
    fn the_label_is_the_cores() {
        assert_eq!(super::text("terminal_choice"), "Open sessions in");
    }
}
