// Mounted consent tests for the Traces queue, nested in ui::queue. Linux
// only; run alone in the Weston session with --exact --ignored
// --test-threads=1.

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
    assert!(fixture.app.undo.borrow().is_some(), "cleared before the reply");

    fixture.answer("cancel", Err("not-cancelable".into()));
    assert!(fixture.app.undo.borrow().is_some(), "a refusal cleared the card");

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
    assert_eq!(fixture.calls_to("cancel")[0].1, json!({"entry_id": "entry-a"}));
    fixture.answer("cancel", Ok(json!(null)));
    assert!(fixture.app.undo.borrow().is_some());
    fixture.app.queue.undo_undo.emit_clicked();
    fixture.drive_until(|| fixture.calls_to("cancel").len() == 2);
    fixture.answer("cancel", Ok(json!({"ok": true})));
    assert!(fixture.app.undo.borrow().is_none());
}
