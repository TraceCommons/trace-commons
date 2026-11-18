// Mounted consent tests for Settings, nested in ui::settings. Linux only;
// run alone in the Weston session with --exact --ignored --test-threads=1.

use adw::prelude::*;

use crate::ui::consent_fixture::Fixture;

pub(super) fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
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

/// The Mac offers no control over the approval hold; neither does this
/// window. Only the quiet-time and interruption knobs remain.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_settings_have_no_undo_window_control() {
    let fixture = Fixture::new("NoHoldKnob", &[]);
    let view = &fixture.app.settings;
    let known: [&gtk::Widget; 3] = [
        view.quiescence_minutes.upcast_ref(),
        view.digest_hours.upcast_ref(),
        view.routing_port.upcast_ref(),
    ];
    for spin in descendants(view.root.upcast_ref())
        .into_iter()
        .filter(|w| w.is::<gtk::SpinButton>())
    {
        assert!(
            known.contains(&&spin),
            "a dial other than quiet time, interruptions or the routing port"
        );
    }
}

fn grant_answered(fixture: &Fixture, grant: serde_json::Value) {
    crate::ui::consent_settings::refresh(&fixture.app);
    fixture.drive_until(|| !fixture.calls_to("automatic_grant").is_empty());
    fixture.answer("automatic_grant", Ok(grant));
}

fn dialogs() -> Vec<adw::MessageDialog> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .filter(|d| d.is_visible())
        .collect()
}

/// Ask each time, after setup: the Mac's withdrawal, with folders it armed
/// put back to Ask, and the shared line when it is refused.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_sharing_ask_each_time_withdraws_the_grant() {
    let fixture = Fixture::new(
        "SharingAsk",
        &["automatic_grant", "withdraw_automatic_grant"],
    );
    let card = &fixture.app.settings.sharing;
    assert!(!card.ask.is_sensitive(), "pressable before the daemon said");
    grant_answered(&fixture, serde_json::json!({"granted": true, "include_existing": true}));
    assert!(card.automatic.is_active());
    card.ask.set_active(true);
    fixture.drive_until(|| !fixture.calls_to("withdraw_automatic_grant").is_empty());
    assert_eq!(
        fixture.calls_to("withdraw_automatic_grant")[0].1,
        serde_json::json!({"return_folders_to_ask": true})
    );
    assert!(card.automatic.is_active(), "shown changed before the daemon confirmed");
    fixture.answer("withdraw_automatic_grant", Err("unavailable".into()));
    assert!(card.refusal.is_visible());
    assert_eq!(
        card.refusal.text(),
        trace_commons_contributor::first_run_copy::first_run_copy()
            .uses
            .settings_sharing_save_failed
    );
}

/// Automatic asks first about folders set to Ask, in the core's words, and
/// Cancel grants nothing.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_sharing_automatic_asks_about_ask_folders_first() {
    let fixture = Fixture::new("SharingKeepAsks", &["automatic_grant"]);
    grant_answered(&fixture, serde_json::json!({"granted": false}));
    let project: crate::model::Project = serde_json::from_value(serde_json::json!({
        "project_id": "p", "mode": "notify_only", "configured": true,
    }))
    .unwrap();
    crate::ui::consent_settings::note_projects(&fixture.app, &[project.clone(), project]);
    let card = &fixture.app.settings.sharing;
    assert!(card.ask.is_active());
    card.automatic.set_active(true);
    fixture.drive_until(|| !dialogs().is_empty());
    let uses = trace_commons_contributor::first_run_copy::first_run_copy().uses;
    let dialog = dialogs().remove(0);
    assert_eq!(dialog.heading().as_deref(), Some(uses.keep_asks_title));
    assert_eq!(
        dialog.body().as_str(),
        uses.keep_asks_body.replace("{count}", "2")
    );
    dialog.response("cancel");
    fixture.settle();
    assert!(fixture.calls_to("grant_automatic_including_existing").is_empty());
    assert!(card.ask.is_active());
}

/// An unreadable configuration discloses nothing and grants nothing.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_sharing_automatic_refuses_on_an_unreadable_configuration() {
    let fixture = Fixture::new("SharingNoConfig", &["automatic_grant"]);
    std::fs::write(fixture.app.worker.dir.join("contributor.json"), "not json").unwrap();
    grant_answered(&fixture, serde_json::json!({"granted": false}));
    let card = &fixture.app.settings.sharing;
    card.automatic.set_active(true);
    fixture.settle();
    assert!(dialogs().is_empty(), "a disclosure drawn on a guess");
    assert!(card.refusal.is_visible());
    assert!(fixture.calls_to("grant_automatic_including_existing").is_empty());
}

/// Uses after setup: the write starts from the daemon's own list, and a
/// refusal says the shared line.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_uses_sends_the_daemons_list_with_one_scope_changed() {
    let fixture = Fixture::new("UsesToggle", &["consent_options", "set_consent_scopes"]);
    *fixture.app.settings.uses.reported.borrow_mut() = Some((true, vec!["eval".into()]));
    crate::ui::consent_settings::refresh(&fixture.app);
    fixture.drive_until(|| !fixture.calls_to("consent_options").is_empty());
    fixture.answer(
        "consent_options",
        Ok(serde_json::json!({"scopes": [
            {"name": "contribution", "title": "Contribution", "always_on": true},
            {"name": "training", "title": "Training", "grants_data_use": true},
            {"name": "eval", "title": "Evaluation", "grants_data_use": true},
        ]})),
    );
    let checks: Vec<gtk::CheckButton> = descendants(fixture.app.settings.uses.root.upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .collect();
    let training = checks
        .iter()
        .find(|c| c.label().as_deref() == Some("Training"))
        .expect("the optional scope is a row");
    assert!(!training.is_active());
    assert!(
        checks
            .iter()
            .all(|c| c.label().as_deref() != Some("Contribution")),
        "an always-on scope is not a toggle"
    );
    training.set_active(true);
    fixture.drive_until(|| !fixture.calls_to("set_consent_scopes").is_empty());
    assert_eq!(
        fixture.calls_to("set_consent_scopes")[0].1,
        serde_json::json!({"scopes": ["contribution", "eval", "training"]})
    );
    fixture.answer("set_consent_scopes", Err("refused".into()));
    let refusal = &fixture.app.settings.uses.refusal;
    assert!(refusal.is_visible());
    assert_eq!(
        refusal.text(),
        crate::ui::consent_settings::consent_refusal_line()
    );
}
