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
    // Eyebrows are uppercased by `style::eyebrow`.
    assert!(
        shown
            .iter()
            .any(|t| *t == copy.contribution_details.to_uppercase())
    );
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

fn describe(widget: &gtk::Widget) -> String {
    let name = widget
        .downcast_ref::<gtk::Button>()
        .and_then(|b| b.label().or_else(|| b.tooltip_text()))
        .map(|l| l.to_string())
        .unwrap_or_default();
    format!("{}({name})", widget.type_().name())
}

/// Run the loop for `span`, as the frames between two key presses do: Tab
/// orders a container's children by where they were laid out.
fn frames(fixture: &Fixture, span: std::time::Duration) {
    let until = std::time::Instant::now() + span;
    while std::time::Instant::now() < until {
        fixture.context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Tab forward from the window `presses` times, as GtkWindow's Tab binding
/// does, and say where focus landed each time.
fn tab_walk(fixture: &Fixture, presses: usize) -> Vec<gtk::Widget> {
    let window: &gtk::Window = fixture.app.window.upcast_ref();
    let mut landed = Vec::new();
    for _ in 0..presses {
        window.child_focus(gtk::DirectionType::TabForward);
        frames(fixture, std::time::Duration::from_millis(15));
        if let Some(focus) = gtk::prelude::GtkWindowExt::focus(window) {
            landed.push(focus);
        }
    }
    landed
}

/// Keyboard reach in Traces > History > a folder (KDE box, 2026-11-18):
/// Tab went from the update notice's Close, or with the notice closed from
/// the "Traces" breadcrumb, and never reached the records. Every record's
/// Open is on the Tab chain, with the notices open or closed.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_tab_reaches_a_history_folders_records() {
    let fixture = Fixture::new("HistoryTab", &["history_rollup", "list_history"]);
    let app = &fixture.app;
    app.window.present();
    *app.history_location.borrow_mut() = crate::queue_folders::Location::Project("p1".into());
    assert!(app.shell.open(app, "history"));
    super::refresh(app);
    fixture.drive_until(|| !fixture.calls_to("history_rollup").is_empty());
    fixture.answer("history_rollup", Ok(serde_json::json!({})));
    fixture.drive_until(|| !fixture.calls_to("list_history").is_empty());
    let record = |id: &str| {
        serde_json::json!({
            "submission_id": id, "status": "accepted",
            "project_label": "proj", "project_id": "p1",
        })
    };
    fixture.answer(
        "list_history",
        Ok(serde_json::json!({"history": [record("sub-1"), record("sub-2")]})),
    );
    let open_word = trace_commons_contributor::preview_copy::monitor_screens_copy()
        .shell
        .open;
    let opens = || -> Vec<gtk::Widget> {
        let mut out = Vec::new();
        let mut pending = vec![app.history.root.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = pending.pop() {
            if widget
                .downcast_ref::<gtk::Button>()
                .is_some_and(|b| b.label().as_deref() == Some(open_word))
            {
                out.push(widget.clone());
            }
            let mut child = widget.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                pending.push(current);
            }
        }
        out
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while opens().len() < 2 || !opens().iter().all(|w| w.is_drawable()) {
        assert!(std::time::Instant::now() < deadline, "the records drew");
        fixture.context.iteration(false);
    }
    frames(&fixture, std::time::Duration::from_millis(300));
    let check = |when: &str| {
        let landed = tab_walk(&fixture, 80);
        let trail: Vec<String> = landed.iter().map(describe).collect();
        for open in opens() {
            assert!(
                landed.contains(&open),
                "{when}: Tab never reached a record's {open_word}; it went {trail:?}"
            );
        }
    };
    check("with the notices as they open");
    // Close every notice the window shows, then walk again.
    for button in descendants_of(app.window.upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .filter(|b| {
            b.is_drawable()
                && b.icon_name().as_deref() == Some("window-close-symbolic")
                && b.ancestor(gtk::Overlay::static_type()).is_some()
        })
    {
        button.emit_clicked();
        fixture.settle();
    }
    frames(&fixture, std::time::Duration::from_millis(300));
    check("with the notices closed");
}

fn descendants_of(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
        out.push(widget);
    }
    out
}
