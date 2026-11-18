use super::*;
use serde_json::json;

/// Offered only when this write turned the scope on: absent before,
/// confirmed after (`SentTracesTrainingState.turnsOn`).
#[test]
fn the_offer_follows_a_confirmed_turn_on_only() {
    let on = vec!["model_training".to_string()];
    assert!(turns_on(&[], &on));
    assert!(!turns_on(&on, &on));
    assert!(!turns_on(&[], &[]));
}

/// The status read as the Mac decodes it: a count is required, the ids
/// default to none; nothing eligible offers nothing.
#[test]
fn the_eligible_count_is_read_as_the_mac_reads_it() {
    assert_eq!(
        eligible(&Ok(json!({"eligible_count": 3, "submission_ids": ["a"]}))),
        Some(3)
    );
    assert_eq!(eligible(&Ok(json!({"eligible_count": 3}))), Some(3));
    assert_eq!(eligible(&Ok(json!({"eligible_count": 0}))), None);
    assert_eq!(eligible(&Ok(json!({"eligible_count": "3"}))), None);
    assert_eq!(eligible(&Err("not-logged-in".into())), None);
}

/// A grant answer is the Mac's `ModelTrainingGrantAnswer`; a refusal hides
/// the offer for its three labels and fails it otherwise.
#[test]
fn a_grant_reply_is_read_as_the_mac_reads_it() {
    let answer = json!({"results": [{"submission_id": "a", "outcome": "granted", "award": "pending"}],
                        "remaining_eligible": 0});
    assert_eq!(grant_outcome(&Ok(answer)), Grant::Answered);
    assert_eq!(grant_outcome(&Ok(json!({"results": []}))), Grant::Failed);
    for hidden in [
        "model-training-not-permitted",
        "model-training-consent-unavailable",
        "not-logged-in",
    ] {
        assert_eq!(
            grant_outcome(&Err(hidden.into())),
            Grant::Hidden,
            "{hidden}"
        );
    }
    assert_eq!(grant_outcome(&Err("unavailable".into())), Grant::Failed);
}

/// The core's words, whole or not at all, and the body chosen by count.
#[test]
fn the_sheet_says_the_cores_words() {
    let copy = words().expect("complete");
    assert_eq!(body(&copy, 1), copy.body_one);
    assert_eq!(body(&copy, 4), copy.body_other.replace("{count}", "4"));
    assert!(complete(&copy));
    let mut partial = copy;
    partial.scope = " ";
    assert!(!complete(&partial));
}
