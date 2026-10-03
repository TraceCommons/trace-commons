//! Connecting inference (K12): the optional onboarding step through which a
//! contributor chooses an operator-published inference connection, the
//! route to a witness for an invited contributor.
//!
//! Every command is a thin pass to the daemon's `inference_connection_*`
//! methods (`docs/contributor-daemon-ipc-v1_1.md`, "Connecting inference").
//! The words come from `consent_copy`, and so does the choice of words for an
//! offer: an offer whose disclosure version this build cannot describe is
//! marked so here and refused at select, before the daemon is asked.

use serde_json::{Value, json};
use tauri::State;
use trace_commons_contributor::consent_copy;

use crate::{
    ipc::{call_daemon, shared_state},
    state::AppState,
};

/// The step's sentences, from the contributor core.
#[tauri::command]
pub(crate) fn inference_connection_copy() -> Value {
    json!(consent_copy::inference_connection_copy())
}

/// Each offer as the daemon validated it, plus `disclosure`: the core's
/// words for its `disclosure_version`, or `null` when this build has none,
/// in which case the shell does not offer it. A response that is not a list
/// of objects is passed on unchanged for the frontend parser to refuse.
fn with_disclosures(mut response: Value) -> Value {
    if let Some(offers) = response.get_mut("offers").and_then(Value::as_array_mut) {
        for offer in offers {
            let disclosure = offer
                .get("disclosure_version")
                .and_then(Value::as_str)
                .and_then(consent_copy::inference_connection_disclosure);
            if let Some(object) = offer.as_object_mut() {
                object.insert("disclosure".to_owned(), json!(disclosure));
            }
        }
    }
    response
}

#[tauri::command]
pub(crate) async fn inference_connection_offers(
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let response = call_daemon(
        shared_state(&state)?,
        "inference_connection_offers",
        json!({}),
    )
    .await?;
    Ok(with_disclosures(response))
}

#[tauri::command]
pub(crate) async fn inference_connection_current(
    state: State<'_, AppState>,
) -> Result<Value, String> {
    call_daemon(
        shared_state(&state)?,
        "inference_connection_current",
        json!({}),
    )
    .await
}

/// The offer exactly as the daemon listed it. The daemon requires every
/// field and sends them unchanged; nothing here fills one in.
#[derive(serde::Deserialize)]
pub(crate) struct ChosenOffer {
    offer_id: String,
    provider_id: String,
    revision: String,
    config_digest: String,
    disclosure_version: String,
}

/// The select's parameters, or the label it is refused with before the
/// daemon is asked: without the contributor's confirmation, or for an offer
/// whose disclosure this build cannot show. The labels carry no content.
///
/// `confirmed` now also rides on the call to the daemon: `handle_select`
/// requires it too (previously this file's own refusal, above, was the only
/// place that was ever checked), so this call is always already-true by the
/// time it reaches `params` -- this file's own behaviour does not change --
/// but the gate itself no longer lives only here.
fn select_params(
    confirmed: bool,
    offer: &ChosenOffer,
    expected_current_version: Option<i64>,
) -> Result<Value, &'static str> {
    if !confirmed {
        return Err("inference-connection-confirmation-required");
    }
    if consent_copy::inference_connection_disclosure(&offer.disclosure_version).is_none() {
        return Err("inference-connection-disclosure-unknown");
    }
    let mut params = json!({
        "offer_id": offer.offer_id,
        "provider_id": offer.provider_id,
        "revision": offer.revision,
        "config_digest": offer.config_digest,
        "disclosure_version": offer.disclosure_version,
        "confirmed": confirmed,
    });
    if let Some(version) = expected_current_version {
        params["expected_current_version"] = json!(version);
    }
    Ok(params)
}

/// Record the contributor's choice on their account. Installs nothing.
#[tauri::command]
pub(crate) async fn inference_connection_select(
    state: State<'_, AppState>,
    confirmed: bool,
    offer: ChosenOffer,
    expected_current_version: Option<i64>,
) -> Result<Value, String> {
    let params =
        select_params(confirmed, &offer, expected_current_version).map_err(str::to_owned)?;
    call_daemon(shared_state(&state)?, "inference_connection_select", params).await
}

/// The separate, confirmed step that writes the selected witness on this
/// device. Installing changes the witness, so it voids armed grants; the
/// step says so before the contributor confirms.
#[tauri::command]
pub(crate) async fn inference_connection_install(
    state: State<'_, AppState>,
    confirmed: bool,
    connection_id: String,
    config_digest: String,
) -> Result<Value, String> {
    if !confirmed {
        return Err("inference-connection-confirmation-required".to_owned());
    }
    call_daemon(
        shared_state(&state)?,
        "inference_connection_install",
        json!({
            "connection_id": connection_id,
            "config_digest": config_digest,
            "confirmed": confirmed,
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn inference_connection_disconnect(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Value, String> {
    call_daemon(
        shared_state(&state)?,
        "inference_connection_disconnect",
        json!({ "connection_id": connection_id }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{ChosenOffer, inference_connection_copy, select_params, with_disclosures};
    use serde_json::json;
    use trace_commons_contributor::consent_copy;

    // The protocol crate's `DISCLOSURE_VERSION`; this crate does not depend on
    // it, and the core test `a_connection_is_described_only_in_a_known_disclosure_version`
    // holds the two together.
    const DISCLOSURE_VERSION: &str = "inference-connection-disclosure-v1";

    fn offer(disclosure_version: &str) -> ChosenOffer {
        serde_json::from_value(json!({
            "offer_id": "near-ai",
            "provider_id": "near-ai",
            "revision": "a".repeat(64),
            "config_digest": "b".repeat(64),
            "disclosure_version": disclosure_version,
        }))
        .expect("an offer")
    }

    #[test]
    fn an_offer_carries_the_cores_words_or_none() {
        let response = with_disclosures(json!({ "offers": [
            { "offer_id": "a", "disclosure_version": DISCLOSURE_VERSION },
            { "offer_id": "b", "disclosure_version": "inference-connection-disclosure-v9" },
        ]}));
        assert_eq!(
            response["offers"][0]["disclosure"],
            consent_copy::INFERENCE_DISCLOSURE_V1
        );
        assert!(response["offers"][1]["disclosure"].is_null());
        // Not a list: passed on for the frontend to refuse, never invented.
        assert_eq!(
            with_disclosures(json!({ "offers": 3 })),
            json!({ "offers": 3 })
        );
    }

    #[test]
    fn select_needs_confirmation_and_a_known_disclosure() {
        assert_eq!(
            select_params(false, &offer(DISCLOSURE_VERSION), None),
            Err("inference-connection-confirmation-required")
        );
        assert_eq!(
            select_params(true, &offer("inference-connection-disclosure-v9"), None),
            Err("inference-connection-disclosure-unknown")
        );
    }

    /// The daemon sends exactly what the contributor was shown, so the
    /// shell passes every field through unchanged and adds only the
    /// version it read, plus the `confirmed` the daemon's own
    /// `inference_connection_select` now also requires (it used to be
    /// checked only by this file's own refusal above).
    #[test]
    fn select_passes_the_shown_offer_through_unchanged() {
        let params = select_params(true, &offer(DISCLOSURE_VERSION), None).unwrap();
        assert_eq!(
            params,
            json!({
                "offer_id": "near-ai",
                "provider_id": "near-ai",
                "revision": "a".repeat(64),
                "config_digest": "b".repeat(64),
                "disclosure_version": DISCLOSURE_VERSION,
                "confirmed": true,
            })
        );
        let replacing = select_params(true, &offer(DISCLOSURE_VERSION), Some(3)).unwrap();
        assert_eq!(replacing["expected_current_version"], 3);
    }

    #[test]
    fn the_step_copy_is_the_cores() {
        assert_eq!(
            inference_connection_copy(),
            json!(consent_copy::inference_connection_copy())
        );
    }
}
