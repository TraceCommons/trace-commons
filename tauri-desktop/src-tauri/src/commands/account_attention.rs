use tauri::State;

use crate::{
    ipc::{call_daemon, shared_state},
    state::AppState,
};

/// What the account section shows while `status.account_state` is
/// `needs_attention`: the title, the reason's sentence, the Private AI line,
/// the button and its confirmation, all from the contributor core. `null`
/// when there is nothing to show.
#[tauri::command]
pub(crate) fn account_attention_copy(status: serde_json::Value) -> serde_json::Value {
    trace_commons_contributor::consent_copy::account_attention_for_wire(&status)
        .and_then(|copy| serde_json::to_value(copy).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// The sentence for what an `unenroll` answer says was left behind, or
/// `None` when the NEAR AI sign-in went with the enrollment (or the daemon
/// predates the fields). Not cleared outranks not revoked: nothing was
/// attempted.
pub(crate) fn near_ai_left_behind(answer: &serde_json::Value) -> Option<&'static str> {
    use trace_commons_contributor::private_inference_copy as pi;
    if answer.get("near_ai_cleared") == Some(&serde_json::Value::Bool(false)) {
        return Some(pi::NEAR_AI_STILL_SIGNED_IN);
    }
    if answer.get("near_ai_revoked") == Some(&serde_json::Value::Bool(false)) {
        return Some(pi::CREDENTIAL_FORGET_NOT_REVOKED);
    }
    None
}

/// Unenroll, which also signs out of NEAR AI. Answers the core's sentence
/// for anything the sign-out left behind, or `null`. Only the confirmation's
/// own button calls this.
#[tauri::command]
pub(crate) async fn set_up_again(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let answer = call_daemon(shared_state(&state)?, "unenroll", serde_json::json!({})).await?;
    Ok(serde_json::json!({ "left_behind": near_ai_left_behind(&answer) }))
}

#[cfg(test)]
mod tests {
    use super::{account_attention_copy, near_ai_left_behind};
    use serde_json::json;
    use trace_commons_contributor::{consent_copy as copy, private_inference_copy as pi};

    #[test]
    fn the_copy_is_the_contributor_cores() {
        let status = json!({
            "account_state": "needs_attention",
            "account_state_reason": "account_record_unreadable",
            "near_ai_login_unconfirmed": true,
        });
        assert_eq!(
            account_attention_copy(status.clone()),
            serde_json::to_value(copy::account_attention_for_wire(&status).unwrap()).unwrap()
        );
        let shown = account_attention_copy(status);
        assert_eq!(shown["title"], "Needs attention");
        assert_eq!(shown["action"], "Set up again");
        assert_eq!(shown["failed"], "Nothing changed.");
    }

    #[test]
    fn a_healthy_old_or_malformed_status_shows_nothing() {
        assert!(account_attention_copy(json!({"account_state": "ok"})).is_null());
        assert!(account_attention_copy(json!({"logged_in": true})).is_null());
        assert!(account_attention_copy(json!("needs_attention")).is_null());
    }

    #[test]
    fn what_unenroll_left_behind_is_said_in_the_cores_words() {
        assert_eq!(
            near_ai_left_behind(&json!({"near_ai_cleared": false, "near_ai_revoked": false})),
            Some(pi::NEAR_AI_STILL_SIGNED_IN)
        );
        assert_eq!(
            near_ai_left_behind(&json!({"near_ai_cleared": true, "near_ai_revoked": false})),
            Some(pi::CREDENTIAL_FORGET_NOT_REVOKED)
        );
        assert_eq!(
            near_ai_left_behind(&json!({"near_ai_cleared": true, "near_ai_revoked": true})),
            None
        );
        assert_eq!(
            near_ai_left_behind(&json!({"near_ai_cleared": true, "near_ai_revoked": null})),
            None
        );
        // A daemon that predates the fields says nothing.
        assert_eq!(near_ai_left_behind(&json!({"unenrolled": true})), None);
    }
}
