// Mounted consent tests for the Traces queue, nested in ui::queue. Linux
// only; run alone in the Weston session with --exact --ignored
// --test-threads=1.

use adw::prelude::*;
use serde_json::json;

use crate::ui::PendingUndo;
use crate::ui::consent_fixture::Fixture;

fn offer(fixture: &Fixture, folder: Option<&str>) {
    let now = chrono::Utc::now();
    *fixture.app.undo.borrow_mut() = Some(PendingUndo {
        entry_ids: vec!["entry-a".into()],
        project_label: "proj".into(),
        folder: folder.map(str::to_string),
        approved_at: now,
        hold_until: now + chrono::Duration::seconds(30),
    });
    super::render_undo(&fixture.app);
}

/// A folder's Undo sends the Mac's one `cancel{project_id}`, keeps the card
/// through a refusal, and clears it only once the daemon reports `canceled`.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_folder_undo_cancels_by_project_and_waits_for_the_daemon() {
    let fixture = Fixture::new("FolderUndo", &["cancel"]);
    offer(&fixture, Some("proj-1"));
    fixture.app.queue.undo_undo.emit_clicked();
    fixture.drive_until(|| !fixture.calls_to("cancel").is_empty());
    let calls = fixture.calls_to("cancel");
    assert_eq!(calls.len(), 1, "one cancel for the whole folder");
    assert_eq!(calls[0].1, json!({"project_id": "proj-1"}));
    assert!(
        fixture.app.undo.borrow().is_some(),
        "cleared before the reply"
    );

    fixture.answer("cancel", Err("not-cancelable".into()));
    assert!(
        fixture.app.undo.borrow().is_some(),
        "a refusal cleared the card"
    );

    fixture.app.queue.undo_undo.emit_clicked();
    fixture.drive_until(|| fixture.calls_to("cancel").len() == 2);
    fixture.answer("cancel", Ok(json!({"canceled": 3})));
    assert!(fixture.app.undo.borrow().is_none());
}

/// One trace's Undo is `cancel{entry_id}`, and a reply that is not an
/// object leaves the card up.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_trace_undo_cancels_by_entry_id() {
    let fixture = Fixture::new("TraceUndo", &["cancel"]);
    offer(&fixture, None);
    fixture.app.queue.undo_undo.emit_clicked();
    fixture.drive_until(|| !fixture.calls_to("cancel").is_empty());
    assert_eq!(
        fixture.calls_to("cancel")[0].1,
        json!({"entry_id": "entry-a"})
    );
    fixture.answer("cancel", Ok(json!(null)));
    assert!(fixture.app.undo.borrow().is_some());
    fixture.app.queue.undo_undo.emit_clicked();
    fixture.drive_until(|| fixture.calls_to("cancel").len() == 2);
    fixture.answer("cancel", Ok(json!({"ok": true})));
    assert!(fixture.app.undo.borrow().is_none());
}

fn texts(root: &gtk::Widget) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            out.push(label.text().to_string());
        }
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            out.push(format!("button:{}", button.label().unwrap_or_default()));
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
    }
    out
}

/// A trace held for a second look says so in the core's words, says why,
/// and offers no Submit on its row, as on the Mac; Look inside stays.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_held_trace_shows_its_reasons_and_no_row_submit() {
    let fixture = Fixture::new("HeldRow", &[]);
    let held: crate::model::QueueEntry = serde_json::from_value(json!({
        "entry_id": "e1", "session_hash": "h", "source": "claude-code",
        "project_label": "proj", "state": "pending",
        "reason_label": "second-look-review-required",
        "second_look": ["looks-unsure"],
        "second_look_lines": ["The scrubber was unsure about some of it."],
        "unsure_spans": 2,
    }))
    .unwrap();
    let shown = texts(&super::row(&fixture.app, &held, 0));
    let traces = trace_commons_contributor::preview_copy::monitor_traces_copy();
    assert!(shown.iter().any(|t| t == traces.held));
    assert!(
        shown
            .iter()
            .any(|t| t == "The scrubber was unsure about some of it.")
    );
    assert!(shown.iter().any(|t| t == &format!("{}: 2", traces.unsure)));
    assert!(!shown.contains(&format!("button:{}", crate::copy::submit())));
    assert!(shown.contains(&format!("button:{}", crate::copy::look_inside())));
}

fn dialogs() -> Vec<adw::MessageDialog> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .filter(|d| d.is_visible())
        .collect()
}

fn button_labelled(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            if button.label().as_deref() == Some(label) {
                return Some(button.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
    }
    None
}

fn held_entry() -> crate::model::QueueEntry {
    serde_json::from_value(json!({
        "entry_id": "e1", "session_hash": "h", "source": "claude-code",
        "project_id": "p", "project_label": "proj", "state": "pending",
        "reason_label": "second-look-review-required",
        "second_look": ["looks-unsure"],
        "second_look_lines": ["The scrubber was unsure about some of it."],
        "unsure_spans": 2, "size_bytes": 2048,
    }))
    .unwrap()
}

/// The Undo reply settles the undo that was pressed: a second press while
/// the first cancel is out sends nothing, and a later Submit's card that
/// replaced the folder's survives the folder's confirmation (F1).
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_an_undo_reply_clears_only_the_undo_that_was_pressed() {
    let fixture = Fixture::new("UndoRace", &["cancel"]);
    offer(&fixture, Some("proj-1"));
    fixture.app.queue.undo_undo.emit_clicked();
    fixture.drive_until(|| !fixture.calls_to("cancel").is_empty());
    fixture.app.queue.undo_undo.emit_clicked();
    fixture.settle();
    assert_eq!(
        fixture.calls_to("cancel").len(),
        1,
        "a second press went out"
    );
    fixture.app.offer_undo(
        vec!["entry-b".into()],
        "proj",
        None,
        chrono::Utc::now() + chrono::Duration::seconds(30),
    );
    fixture.answer("cancel", Ok(json!({"canceled": 1})));
    let held = fixture
        .app
        .undo
        .borrow()
        .as_ref()
        .map(|undo| undo.entry_ids.clone());
    assert_eq!(held, Some(vec!["entry-b".to_string()]));
}

/// Not this one asks first in the core's words, as the Mac's
/// `dismissModal`: Keep it sends nothing, and only Dismiss sends
/// `dismiss`.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_not_this_one_asks_first() {
    let fixture = Fixture::new("DismissAsk", &["dismiss"]);
    let row = super::row(&fixture.app, &held_entry(), 0);
    let skip = button_labelled(row.upcast_ref(), crate::copy::not_this_one()).expect("the dismiss");
    let tree = trace_commons_contributor::preview_copy::monitor_traces_copy().tree;
    skip.emit_clicked();
    fixture.drive_until(|| !dialogs().is_empty());
    let dialog = dialogs().remove(0);
    assert_eq!(
        dialog.heading().as_deref(),
        Some(tree.dismiss_session_title)
    );
    dialog.response("keep");
    fixture.settle();
    assert!(fixture.calls_to("dismiss").is_empty());
    skip.emit_clicked();
    fixture.drive_until(|| !dialogs().is_empty());
    dialogs().remove(0).response("dismiss");
    fixture.drive_until(|| !fixture.calls_to("dismiss").is_empty());
    assert_eq!(fixture.calls_to("dismiss")[0].1, json!({"entry_id": "e1"}));
}

/// The preview sheet says why a trace is held and how unsure, as the
/// row does; with no preview to pin, Contribute stays off and pressing it
/// sends nothing (F4).
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_preview_sheet_says_why_held_and_sends_nothing() {
    let fixture = Fixture::new("PreviewHeld", &[]);
    *fixture.app.entries.borrow_mut() = vec![held_entry()];
    crate::ui::preview::open(&fixture.app, 0);
    fixture.settle();
    let traces = trace_commons_contributor::preview_copy::monitor_traces_copy();
    let sheet = gtk::Window::list_toplevels()
        .into_iter()
        .filter(|w| w.is_visible())
        .find(|w| {
            texts(w)
                .iter()
                .any(|t| t == "The scrubber was unsure about some of it.")
                && w != fixture.app.window.upcast_ref::<gtk::Widget>()
        })
        .expect("the sheet says the held reason");
    let shown = texts(&sheet);
    assert!(shown.iter().any(|t| t == traces.held));
    assert!(shown.iter().any(|t| t == &format!("{}: 2", traces.unsure)));
    let contribute = button_labelled(&sheet, crate::copy::CONTRIBUTE).expect("Contribute");
    assert!(
        !contribute.is_sensitive(),
        "sendable with no pinned preview"
    );
    contribute.emit_clicked();
    fixture.settle();
    assert!(fixture.calls_to("approve").is_empty());
}

/// The sheet's own buttons end inside it when it opens at its minimum
/// height, here pushed past its default by three held reasons. On the
/// GNOME box (libadwaita 1.5, 2026-11-18) the content was laid out as tall
/// as it is at its narrowest, 24 px taller than the sheet, and Dismiss
/// trace, Close and Contribute were cut by the sheet's bottom edge.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_the_sheet_buttons_end_inside_the_sheet() {
    let fixture = Fixture::new("PreviewSheetFit", &[]);
    let lines = [
        "No personal details matched, only file paths or nothing at all; that is why this one waits.",
        "Something here looks like an email, phone number or key that was not matched; that is why this one waits.",
        "Trimmed to fit the upload limit, so part of this trace is not in what would be sent. That is why this one waits.",
    ];
    let mut entry = held_entry();
    entry.second_look = Some(vec![
        "nothing-matched".into(),
        "looks-unsure".into(),
        "trimmed-to-fit".into(),
    ]);
    entry.second_look_lines = Some(lines.iter().map(|line| line.to_string()).collect());
    *fixture.app.entries.borrow_mut() = vec![entry];
    crate::ui::preview::open(&fixture.app, 0);
    fixture.settle();
    let sheet = gtk::Window::list_toplevels()
        .into_iter()
        .filter(|w| w.is_visible())
        .find(|w| {
            texts(w).iter().any(|t| t == lines[0])
                && w != fixture.app.window.upcast_ref::<gtk::Widget>()
        })
        .expect("the sheet says the held reasons");
    let content = sheet
        .downcast_ref::<adw::Window>()
        .and_then(|window| window.content())
        .expect("the sheet's content");
    fixture.drive_until(|| content.height() > 0);
    // Let the compositor answer any resize the first layout asked for.
    let quiet = std::time::Instant::now() + std::time::Duration::from_millis(800);
    fixture.drive_until(|| std::time::Instant::now() > quiet);
    for label in [
        crate::copy::CONTRIBUTE,
        crate::copy::CLOSE,
        crate::copy::not_this_one(),
    ] {
        let button = button_labelled(&sheet, label).expect("the sheet's button");
        let bounds = button
            .compute_bounds(&sheet)
            .expect("the button is laid out in the sheet");
        assert!(
            bounds.y() + bounds.height() <= sheet.height() as f32,
            "{label} ends at {} in a sheet {} tall",
            bounds.y() + bounds.height(),
            sheet.height()
        );
    }
}
