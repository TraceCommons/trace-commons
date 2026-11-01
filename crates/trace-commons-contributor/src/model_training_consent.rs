//! Adding the `model_training` use to traces already sent (owner decisions
//! 2026-10-28; wire contract in
//! `trace_commons_protocol::model_training_consent`).
//!
//! A trace sent without "Train coding models" had its scoring award
//! withheld. These calls read how many of the contributor's traces could
//! take the use, and send the grant. Both are authenticated like
//! [`crate::submit::status`]: a device-signed status claim, which the issuer
//! fills with the device's full grant ceiling, so the server can refuse a
//! grant the ceiling does not allow.
//!
//! The grant is add-only and dated by the server when it accepts it. It
//! never removes a use: that is withdrawal.

use anyhow::Result;
use chrono::Utc;
use reqwest::Method;
use trace_commons_operator_client::{Client, Error as OcError};
use trace_commons_protocol::model_training_consent::{
    MODEL_TRAINING_CONSENT_BAD_SELECTION_LABEL, MODEL_TRAINING_CONSENT_PATH,
    MODEL_TRAINING_CONSENT_UNAVAILABLE_LABEL, MODEL_TRAINING_NOT_PERMITTED_LABEL,
    ModelTrainingConsentEligibility, ModelTrainingConsentGrantRequest,
    ModelTrainingConsentGrantResponse,
};

use crate::config::{ConfigStore, ContributorConfig, config_allowlist};
use crate::identity::DeviceIdentity;
use crate::issuer_client::IssuerClient;

/// Why a call did not answer. Each maps to one fixed IPC label; a remote
/// body never reaches a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentCallError {
    /// The device's grant ceiling, or the deployment's policy, does not
    /// include model training. A shell does not offer the grant.
    NotPermitted,
    /// The deployment cannot record a grant (no database), or does not have
    /// the route yet (an older server).
    Unavailable,
    /// The request named no ids, or too many.
    BadSelection,
    /// Anything else: no identity, a mint failure, transport, a 5xx.
    Failed,
}

impl ConsentCallError {
    /// The IPC error message for this refusal.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NotPermitted => "model-training-not-permitted",
            Self::Unavailable => "model-training-consent-unavailable",
            Self::BadSelection => "model-training-consent-bad-selection",
            Self::Failed => "model-training-consent-failed",
        }
    }
}

fn classify(error: &OcError) -> ConsentCallError {
    if let Some(label) = error.server_label() {
        return match label {
            MODEL_TRAINING_NOT_PERMITTED_LABEL => ConsentCallError::NotPermitted,
            MODEL_TRAINING_CONSENT_UNAVAILABLE_LABEL => ConsentCallError::Unavailable,
            MODEL_TRAINING_CONSENT_BAD_SELECTION_LABEL => ConsentCallError::BadSelection,
            _ => ConsentCallError::Failed,
        };
    }
    match error {
        // A server from before the route answers 404 or 405 with no label.
        OcError::HttpFailure { status, .. } if matches!(status.as_u16(), 404 | 405) => {
            ConsentCallError::Unavailable
        }
        _ => ConsentCallError::Failed,
    }
}

async fn ingest_client(
    store: &ConfigStore,
    cfg: &ContributorConfig,
) -> Result<Client, ConsentCallError> {
    let device = DeviceIdentity::load_or_generate_async(store)
        .await
        .map_err(|_| ConsentCallError::Failed)?;
    let issuer = IssuerClient::new(config_allowlist(cfg)).map_err(|_| ConsentCallError::Failed)?;
    let token = crate::submit::mint_status_claim(&issuer, cfg, &device, Utc::now())
        .await
        .map_err(|_| ConsentCallError::Failed)?;
    crate::submit::build_ingest_client(cfg, &token).map_err(|_| ConsentCallError::Failed)
}

/// How many of the contributor's traces could take the use, and which.
pub async fn fetch_eligibility(
    store: &ConfigStore,
    cfg: &ContributorConfig,
) -> Result<ModelTrainingConsentEligibility, ConsentCallError> {
    let client = ingest_client(store, cfg).await?;
    eligibility_with(&client).await
}

/// Send the grant.
pub async fn send_grant(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    request: &ModelTrainingConsentGrantRequest,
) -> Result<ModelTrainingConsentGrantResponse, ConsentCallError> {
    request
        .selected_ids()
        .map_err(|_| ConsentCallError::BadSelection)?;
    let client = ingest_client(store, cfg).await?;
    grant_with(&client, request).await
}

pub(crate) async fn eligibility_with(
    client: &Client,
) -> Result<ModelTrainingConsentEligibility, ConsentCallError> {
    client
        .call_json::<(), _>(Method::GET, MODEL_TRAINING_CONSENT_PATH, &[], None)
        .await
        .map_err(|error| classify(&error))
}

pub(crate) async fn grant_with(
    client: &Client,
    request: &ModelTrainingConsentGrantRequest,
) -> Result<ModelTrainingConsentGrantResponse, ConsentCallError> {
    client
        .call_json(
            Method::POST,
            MODEL_TRAINING_CONSENT_PATH,
            &[],
            Some(request),
        )
        .await
        .map_err(|error| classify(&error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode},
        routing::get,
    };
    use std::sync::{Arc, Mutex};
    use trace_commons_protocol::model_training_consent::{
        ModelTrainingAwardState, ModelTrainingConsentGrantItem, ModelTrainingConsentOutcome,
    };
    use uuid::Uuid;

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn client(base: &str) -> Client {
        Client::builder(base, "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV")
            .bearer_token("claim-token")
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn the_grant_is_sent_with_the_claim_and_answered() {
        let id = Uuid::new_v4();
        let seen: Arc<Mutex<Vec<(String, serde_json::Value)>>> = Arc::default();
        let recorded = seen.clone();
        let router = Router::new().route(
            MODEL_TRAINING_CONSENT_PATH,
            get(|| async { Json(serde_json::json!({"eligible_count": 22, "submission_ids": []})) })
                .post(
                    move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                        let recorded = recorded.clone();
                        async move {
                            let bearer = headers
                                .get("authorization")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or_default()
                                .to_string();
                            recorded.lock().unwrap().push((bearer, body));
                            Json(ModelTrainingConsentGrantResponse {
                                results: vec![ModelTrainingConsentGrantItem {
                                    submission_id: id,
                                    outcome: ModelTrainingConsentOutcome::Granted,
                                    granted_at: Some(Utc::now()),
                                    award: ModelTrainingAwardState::Credited,
                                    award_withheld_reason: None,
                                }],
                                remaining_eligible: 21,
                            })
                        }
                    },
                ),
        );
        let base = spawn(router).await;
        let client = client(&base);
        let eligible = eligibility_with(&client).await.unwrap();
        assert_eq!(eligible.eligible_count, 22);
        let answer = grant_with(&client, &ModelTrainingConsentGrantRequest::AllEligible)
            .await
            .unwrap();
        assert_eq!(answer.remaining_eligible, 21);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "Bearer claim-token");
        assert_eq!(seen[0].1, serde_json::json!({"selection": "all_eligible"}));
    }

    #[tokio::test]
    async fn refusals_map_to_fixed_labels() {
        let router = Router::new().route(
            MODEL_TRAINING_CONSENT_PATH,
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": MODEL_TRAINING_NOT_PERMITTED_LABEL})),
                )
            })
            .post(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({"error": MODEL_TRAINING_CONSENT_UNAVAILABLE_LABEL})),
                )
            }),
        );
        let base = spawn(router).await;
        let client = client(&base);
        assert_eq!(
            eligibility_with(&client).await.unwrap_err(),
            ConsentCallError::NotPermitted
        );
        assert_eq!(
            grant_with(&client, &ModelTrainingConsentGrantRequest::AllEligible)
                .await
                .unwrap_err(),
            ConsentCallError::Unavailable
        );
        // A server from before the route.
        let old = spawn(Router::new()).await;
        assert_eq!(
            eligibility_with(&super::tests::client(&old))
                .await
                .unwrap_err(),
            ConsentCallError::Unavailable
        );
    }

    #[test]
    fn labels_are_fixed() {
        for (error, label) in [
            (
                ConsentCallError::NotPermitted,
                "model-training-not-permitted",
            ),
            (
                ConsentCallError::Unavailable,
                "model-training-consent-unavailable",
            ),
            (
                ConsentCallError::BadSelection,
                "model-training-consent-bad-selection",
            ),
            (ConsentCallError::Failed, "model-training-consent-failed"),
        ] {
            assert_eq!(error.label(), label);
        }
    }
}
