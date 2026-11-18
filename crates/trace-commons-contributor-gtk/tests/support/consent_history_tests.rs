// Mounted consent test for History, nested in ui::history. Linux only; run
// alone in the Weston session with --exact --ignored --test-threads=1.

use adw::prelude::*;

use crate::ui::consent_fixture::Fixture;

fn buttons(root: &gtk::Widget) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            out.push(button.label().unwrap_or_default().to_string());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
    }
    out
}

/// A previous account's row offers no Withdraw, as on the Mac; the current
/// account's row in the same state does.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_previous_accounts_row_offers_no_withdraw() {
    let fixture = Fixture::new("HistoryPrevious", &[]);
    let record = |previous: bool| -> crate::model::HistoryRecord {
        serde_json::from_value(serde_json::json!({
            "submission_id": "sub-1", "status": "accepted",
            "project_label": "proj", "previous_account": previous,
        }))
        .unwrap()
    };
    let withdraw = crate::copy::WITHDRAW.to_string();
    let previous = super::record_row(&fixture.app, &record(true));
    assert!(!buttons(previous.upcast_ref()).contains(&withdraw));
    let current = super::record_row(&fixture.app, &record(false));
    assert!(buttons(current.upcast_ref()).contains(&withdraw));
}

fn texts(root: &gtk::Widget) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            out.push(label.text().to_string());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
    }
    out
}

fn dialogs() -> Vec<adw::MessageDialog> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .filter(|d| d.is_visible())
        .collect()
}

/// An opened record's contribution details are read from `history_detail`
/// and said in the core's words; a failed read says the core's line for
/// its label (Item 11).
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_an_opened_record_shows_its_contribution_details() {
    let fixture = Fixture::new("HistoryDetail", &["history_detail"]);
    let copy = trace_commons_contributor::public_run::public_run_copy();
    let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    crate::ui::history_detail::load(&fixture.app, &slot, "sub-1", "accepted", |_| true);
    fixture.drive_until(|| !fixture.calls_to("history_detail").is_empty());
    assert_eq!(
        fixture.calls_to("history_detail")[0].1,
        serde_json::json!({"submission_id": "sub-1"})
    );
    assert!(
        texts(slot.upcast_ref())
            .iter()
            .any(|t| t == copy.reading_record)
    );
    fixture.answer(
        "history_detail",
        Ok(serde_json::json!({
            "task": "fix-build", "evidence": [], "contribution_status": "accepted",
            "permitted_uses": [], "contributed_version": "v1",
            "consent_policy_version": "c1", "redaction_pipeline_version": "r1",
        })),
    );
    let shown = texts(slot.upcast_ref());
    assert!(shown.iter().any(|t| t == copy.contribution_details));
    assert!(shown.iter().any(|t| t == copy.no_permitted_uses));
    assert!(shown.iter().any(|t| t == "fix-build"));

    crate::ui::history_detail::load(&fixture.app, &slot, "sub-1", "accepted", |_| true);
    fixture.drive_until(|| fixture.calls_to("history_detail").len() == 2);
    fixture.answer("history_detail", Err("session-detail-not-found".into()));
    assert!(
        texts(slot.upcast_ref())
            .iter()
            .any(|t| t == copy.session_not_found)
    );
}

/// Withdraw asks first, and only its confirm sends `withdraw` for that
/// record.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_withdraw_asks_first_then_sends_the_record() {
    let fixture = Fixture::new("HistoryWithdraw", &["withdraw"]);
    let record: crate::model::HistoryRecord = serde_json::from_value(serde_json::json!({
        "submission_id": "sub-1", "status": "accepted", "project_label": "proj",
    }))
    .unwrap();
    let row = super::record_row(&fixture.app, &record);
    let mut pending = vec![row.clone().upcast::<gtk::Widget>()];
    let mut withdraw = None;
    while let Some(widget) = pending.pop() {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            if button.label().as_deref() == Some(crate::copy::WITHDRAW) {
                withdraw = Some(button.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
    }
    let withdraw = withdraw.expect("the row offers Withdraw");
    withdraw.emit_clicked();
    fixture.drive_until(|| !dialogs().is_empty());
    dialogs().remove(0).response("cancel");
    fixture.settle();
    assert!(fixture.calls_to("withdraw").is_empty());
    withdraw.emit_clicked();
    fixture.drive_until(|| !dialogs().is_empty());
    dialogs().remove(0).response("withdraw");
    fixture.drive_until(|| !fixture.calls_to("withdraw").is_empty());
    assert_eq!(
        fixture.calls_to("withdraw")[0].1,
        serde_json::json!({"submission_id": "sub-1"})
    );
}
