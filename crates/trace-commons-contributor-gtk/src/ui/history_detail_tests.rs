use super::*;
use serde_json::json;

fn detail_json() -> Value {
    json!({
        "content_unavailable": false,
        "task": "fix-build", "task_success": "succeeded", "user_feedback": null,
        "human_correction": null,
        "evidence": [{"event_id": "e1", "kind": "test_result", "excerpt": "ok"}],
        "contribution_status": "accepted",
        "permitted_uses": ["evaluation"],
        "contributed_version": "v1", "consent_policy_version": "c1",
        "redaction_pipeline_version": "r1",
        "publication": null, "publication_version": 0,
    })
}

/// Decoded as the Mac decodes `SessionDetail`: the three versions and the
/// evidence list are required; the rest may be absent.
#[test]
fn a_detail_is_read_as_the_mac_reads_it() {
    let detail = session_detail(&Ok(detail_json())).expect("decodes");
    assert_eq!(detail.task.as_deref(), Some("fix-build"));
    assert_eq!(detail.evidence.len(), 1);
    assert_eq!(
        detail.permitted_uses.as_deref(),
        Some(&["evaluation".to_string()][..])
    );
    let mut missing = detail_json();
    missing
        .as_object_mut()
        .unwrap()
        .remove("consent_policy_version");
    assert!(session_detail(&Ok(missing)).is_none());
    assert!(session_detail(&Err("x".into())).is_none());
}

/// The lines it draws are the core's: the status by the record's status
/// when the detail has none, an unknown use as the unrecognized word, and
/// "unavailable" for missing content when the server withheld it.
#[test]
fn the_details_say_the_cores_words() {
    let copy = trace_commons_contributor::public_run::public_run_copy();
    let mut detail = session_detail(&Ok(detail_json())).unwrap();
    assert_eq!(
        status_line(&detail, "received"),
        label_for(&copy.history_status_labels, "accepted")
            .unwrap_or(copy.contribution_status_unavailable)
    );
    detail.contribution_status = None;
    assert_eq!(
        status_line(&detail, "nonsense"),
        copy.contribution_status_unavailable
    );
    assert_eq!(use_line("not-a-use"), copy.unrecognized_value);
    assert_eq!(missing_line(&detail, copy.no_task), copy.no_task);
    detail.content_unavailable = Some(true);
    assert_eq!(missing_line(&detail, copy.no_task), copy.unavailable_value);
    // A failed read says the core's line for its label.
    assert_eq!(
        failure_line("session-detail-not-found"),
        copy.session_not_found
    );
}
