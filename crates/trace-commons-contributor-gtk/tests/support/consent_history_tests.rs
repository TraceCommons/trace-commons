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
