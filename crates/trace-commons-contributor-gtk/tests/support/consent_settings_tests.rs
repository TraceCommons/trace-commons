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

/// The cards wait for first run (R-43); these tests are about the state
/// after it, which a status read would show.
fn after_first_run(fixture: &Fixture) {
    fixture.app.settings.sharing.root.set_visible(true);
    fixture.app.settings.uses.root.set_visible(true);
}

fn grant_answered(fixture: &Fixture, grant: serde_json::Value) {
    after_first_run(fixture);
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
    grant_answered(
        &fixture,
        serde_json::json!({"granted": true, "include_existing": true}),
    );
    assert!(card.automatic.is_active());
    // A status named the account this is pressed on (`GrantOwner`).
    card.owner.borrow_mut().observe(Some("acct-a"));
    card.ask.set_active(true);
    fixture.drive_until(|| !fixture.calls_to("withdraw_automatic_grant").is_empty());
    assert_eq!(
        fixture.calls_to("withdraw_automatic_grant")[0].1,
        serde_json::json!({"return_folders_to_ask": true})
    );
    assert!(
        card.automatic.is_active(),
        "shown changed before the daemon confirmed"
    );
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
    crate::ui::consent_settings::note_projects(&fixture.app, Some(&[project.clone(), project]));
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
    assert!(
        fixture
            .calls_to("grant_automatic_including_existing")
            .is_empty()
    );
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
    assert!(
        fixture
            .calls_to("grant_automatic_including_existing")
            .is_empty()
    );
}

/// Uses after setup: the write starts from the daemon's own list, and a
/// refusal says the shared line.
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_uses_sends_the_daemons_list_with_one_scope_changed() {
    let fixture = Fixture::new("UsesToggle", &["consent_options", "set_consent_scopes"]);
    after_first_run(&fixture);
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

fn enrolled_status(override_mode: Option<&str>) -> crate::model::Status {
    serde_json::from_value(serde_json::json!({
        "logged_in": true, "tenant_id": "t", "setup_complete": true,
        "account_scope": "acct-a",
        "contribution_override": override_mode.map(|mode| serde_json::json!({
            "mode": mode, "since": "2026-11-01T00:00:00Z",
        })),
    }))
    .unwrap()
}

/// An override in force: the radios show nothing picked and cannot be
/// pressed (never "Ask each time" while every folder sends), the card says
/// what is in force, offers no clear (the Mac has none), and drops the line
/// that folders set to Ask or Never keep their own setting, which is untrue
/// while the override decides (F-1).
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_an_override_shows_on_the_sharing_card_with_no_clear() {
    let fixture = Fixture::new("Override", &["automatic_grant"]);
    grant_answered(&fixture, serde_json::json!({"granted": false}));
    let card = &fixture.app.settings.sharing;
    let note = card.sharing_note.as_ref().expect("the core has the note");
    let modes = trace_commons_contributor::project_copy::contribution_mode_copy();
    let shown_texts = |root: &gtk::Widget| -> Vec<String> {
        descendants(root)
            .into_iter()
            .filter(|w| {
                // Shown as far as the card goes: the Settings page itself
                // need not be on screen.
                let mut at = Some(w.clone());
                while let Some(widget) = at {
                    if !widget.is_visible() {
                        return false;
                    }
                    if &widget == root {
                        return true;
                    }
                    at = widget.parent();
                }
                false
            })
            .filter_map(|w| w.downcast::<gtk::Label>().ok())
            .map(|l| l.text().to_string())
            .collect()
    };
    let buttons = |root: &gtk::Widget| -> Vec<String> {
        descendants(root)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .filter_map(|b| b.label().map(|l| l.to_string()))
            .collect()
    };
    assert!(card.ask.is_active(), "no override: Ask each time");
    assert!(note.is_visible(), "no override: the note is said");

    crate::ui::consent_settings::note_status(&fixture.app, &enrolled_status(Some("auto_upload")));
    fixture.settle();
    assert!(!card.ask.is_active() && !card.automatic.is_active());
    assert!(!card.ask.is_sensitive() && !card.automatic.is_sensitive());
    assert!(
        !note.is_visible(),
        "an Automatic override sends Ask folders"
    );
    let shown = shown_texts(card.root.upcast_ref());
    assert!(shown.iter().any(|t| t == modes.title));
    assert!(shown.iter().any(|t| t == modes.override_active));
    assert!(
        !buttons(card.root.upcast_ref())
            .iter()
            .any(|b| b == modes.clear),
        "the Mac offers no clear"
    );

    crate::ui::consent_settings::note_status(&fixture.app, &enrolled_status(Some("ignore")));
    assert!(!note.is_visible());
    crate::ui::consent_settings::note_status(&fixture.app, &enrolled_status(None));
    assert!(note.is_visible(), "the override gone, the note is back");
    assert!(card.ask.is_sensitive());
}

/// The bulk change asks the daemon first, asks the person in the daemon's
/// words, sends the counts they agreed to, and plans again when the
/// daemon says those counts went stale (Item 8).
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_bulk_change_plans_asks_and_sends_what_was_agreed() {
    let fixture = Fixture::new("Bulk", &["plan_project_modes", "set_project_modes"]);
    let named = || {
        fixture
            .calls_to("plan_project_modes")
            .into_iter()
            .filter(|(_, p)| p["project_ids"] == serde_json::json!(["a", "b"]))
            .count()
    };
    crate::ui::projects_bulk::run(&fixture.app, vec!["a".into(), "b".into()], "ignore".into());
    fixture.drive_until(|| named() == 1);
    fixture.answer(
        "plan_project_modes",
        Ok(serde_json::json!({
            "outcome": "confirm", "project_ids": ["a", "b"],
            "turns_off_automatic": 1, "pending": 3,
            "confirmation": {"title": "Heading", "body": "Body", "button": "Go",
                             "keep": "Keep", "destructive": true},
            "bulk_modes": ["notify_only", "ignore"],
        })),
    );
    fixture.drive_until(|| !dialogs().is_empty());
    let dialog = dialogs().remove(0);
    assert_eq!(dialog.heading().as_deref(), Some("Heading"));
    dialog.response("confirm");
    fixture.drive_until(|| !fixture.calls_to("set_project_modes").is_empty());
    assert_eq!(
        fixture.calls_to("set_project_modes")[0].1,
        serde_json::json!({"project_ids": ["a", "b"], "mode": "ignore",
                           "agreed": {"turns_off_automatic": 1, "pending": 3}})
    );
    fixture.answer("set_project_modes", Err("bulk-confirmation-stale".into()));
    fixture.drive_until(|| named() == 2);
}

/// Turning Train coding models on in Uses, once the daemon confirms it,
/// offers the use on the traces already sent; Allow sends the grant for
/// every eligible trace (Item 6).
#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn mounted_turning_on_training_offers_it_on_sent_traces() {
    let fixture = Fixture::new(
        "TrainingOffer",
        &[
            "consent_options",
            "set_consent_scopes",
            "model_training_consent_status",
            "model_training_consent_grant",
        ],
    );
    *fixture.app.settings.uses.reported.borrow_mut() = Some((true, Vec::new()));
    crate::ui::consent_settings::refresh(&fixture.app);
    fixture.drive_until(|| !fixture.calls_to("consent_options").is_empty());
    fixture.answer(
        "consent_options",
        Ok(serde_json::json!({"scopes": [
            {"name": "contribution", "title": "Contribution", "always_on": true},
            {"name": "model_training", "title": "Training", "grants_data_use": true},
        ]})),
    );
    let training = descendants(fixture.app.settings.uses.root.upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .find(|c| c.label().as_deref() == Some("Training"))
        .expect("the optional scope is a row");
    training.set_active(true);
    fixture.drive_until(|| !fixture.calls_to("set_consent_scopes").is_empty());
    fixture.answer(
        "set_consent_scopes",
        Ok(serde_json::json!({"consent_scopes": ["contribution", "model_training"]})),
    );
    fixture.drive_until(|| !fixture.calls_to("model_training_consent_status").is_empty());
    fixture.answer(
        "model_training_consent_status",
        Ok(serde_json::json!({"eligible_count": 2, "submission_ids": ["s1", "s2"]})),
    );
    fixture.drive_until(|| !dialogs().is_empty());
    let words = trace_commons_contributor::consent_copy::sent_traces_training_copy();
    let dialog = dialogs().remove(0);
    assert_eq!(dialog.heading().as_deref(), Some(words.title));
    assert!(
        fixture.calls_to("model_training_consent_grant").is_empty(),
        "nothing sent before Allow"
    );
    dialog.response("allow");
    fixture.drive_until(|| !fixture.calls_to("model_training_consent_grant").is_empty());
    assert_eq!(
        fixture.calls_to("model_training_consent_grant")[0].1,
        serde_json::json!({"all_eligible": true})
    );
}
