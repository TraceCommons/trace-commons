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
    assert!(!shown.contains(&format!("button:{}", crate::copy::SUBMIT)));
    assert!(shown.contains(&format!("button:{}", crate::copy::LOOK_INSIDE)));
}
