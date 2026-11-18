use super::*;
use serde_json::json;

fn plan_json(outcome: &str) -> Value {
    json!({
        "outcome": outcome, "project_ids": ["a", "b"],
        "turns_off_automatic": 1, "pending": 3,
        "confirmation": {"title": "T", "body": "B", "button": "Go", "keep": "Keep",
                         "help": null, "destructive": true},
        "bulk_modes": ["notify_only", "ignore"],
    })
}

/// The plan is decoded as the Mac decodes `ProjectsBulkPlan`: every field
/// it requires, or nothing.
#[test]
fn a_plan_is_read_as_the_mac_reads_it() {
    let plan = bulk_plan(&Ok(plan_json("confirm"))).expect("decodes");
    assert_eq!(plan.outcome, Outcome::Confirm);
    assert_eq!(plan.project_ids, ["a", "b"]);
    assert_eq!(plan.bulk_modes, ["notify_only", "ignore"]);
    assert!(plan.confirmation.as_ref().unwrap().destructive);
    let mut no_modes = plan_json("apply");
    no_modes.as_object_mut().unwrap().remove("bulk_modes");
    assert!(bulk_plan(&Ok(no_modes)).is_none());
    assert!(bulk_plan(&Ok(json!({"outcome": "maybe"}))).is_none());
    assert!(bulk_plan(&Err("x".into())).is_none());
}

/// The menu offers the daemon's modes and never Automatic, even if a
/// reply named it.
#[test]
fn the_menu_offers_only_ask_and_never() {
    let modes = vec![
        "notify_only".to_string(),
        "auto_upload".into(),
        "ignore".into(),
    ];
    assert_eq!(offered_modes(&modes), ["notify_only", "ignore"]);
}

/// `set_project_modes` carries the agreed counts only after a
/// confirmation (`DaemonClient.setProjectModes`).
#[test]
fn the_change_sends_what_was_agreed() {
    let plan = bulk_plan(&Ok(plan_json("confirm"))).unwrap();
    let ids = vec!["a".to_string(), "b".to_string()];
    assert_eq!(
        set_params(&ids, "ignore", Some(&plan)),
        json!({"project_ids": ["a", "b"], "mode": "ignore",
               "agreed": {"turns_off_automatic": 1, "pending": 3}})
    );
    assert_eq!(
        set_params(&ids, "notify_only", None),
        json!({"project_ids": ["a", "b"], "mode": "notify_only"})
    );
}

/// The reply as `AppModel.setProjectModes` reads it: ran (with how many
/// changed of how many), stale (plan and ask again), or failed.
#[test]
fn a_change_reply_is_read_as_the_mac_reads_it() {
    assert_eq!(
        change_outcome(&Ok(json!({"changed": ["a"], "total": 2, "purged": 0}))),
        Change::Ran {
            changed: vec!["a".into()],
            total: 2
        }
    );
    assert_eq!(
        change_outcome(&Err("bulk-confirmation-stale".into())),
        Change::Stale
    );
    assert_eq!(change_outcome(&Err("refused".into())), Change::Failed);
    assert_eq!(change_outcome(&Ok(json!({"changed": "a"}))), Change::Failed);
}

/// Only the projects that changed leave the selection; a change stopped
/// part way says how many of how many, in the core's words.
#[test]
fn only_changed_projects_leave_the_selection() {
    let selection: BTreeSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let after = after_change(&selection, &["a".to_string()]);
    assert_eq!(after.into_iter().collect::<Vec<_>>(), ["b", "c"]);
    assert_eq!(changed_line(2, 2), None);
    let words = trace_commons_contributor::shell_words_copy::projects_manager_words();
    assert_eq!(
        changed_line(1, 2),
        Some(
            words
                .changed_some
                .replace("{count}", "1")
                .replace("{total}", "2")
        )
    );
}
