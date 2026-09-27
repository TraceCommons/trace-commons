//! Client for the account's inference-connection routes.
//!
//! K12 of the connect-and-forget consent design: an invited contributor
//! receives a redaction witness by deliberately connecting inference, never
//! as a side effect of joining. The server half (#1019) mounts four routes,
//! documented in `docs/operator/inference-connection.md`:
//!
//! - `GET /v1/account/inference-connection/offers` -- identifiers and digests;
//! - `GET /v1/account/inference-connection` -- the account's current
//!   selection, with `reselection_required` when its revision was retired;
//! - `POST /v1/account/inference-connection` -- select one exact offer
//!   revision; the only response carrying witness installation material;
//! - `DELETE /v1/account/inference-connection/{connection_id}` -- disconnect.
//!
//! Every call presents the **account session** from `crate::account_auth`,
//! never the device key: the server refuses a device bearer for selection
//! and disconnect, and a stolen device key must not be worth choosing where
//! a contributor's transcripts go.
//!
//! Everything the server sends is validated with the protocol crate's
//! `validate()` and refused on any mismatch, so nothing malformed reaches a
//! shell or the contributor config. Errors are fixed labels: never a URL, a
//! token, or a response body.

use std::time::Duration;

use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use trace_commons_operator_client::{Client, Error as OcError};
use trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER;
use trace_commons_protocol::inference_connection::{
    DISCLOSURE_VERSION, InferenceConnectionOffer, SelectInferenceConnection,
    SelectedInferenceConnection, valid_digest, valid_identifier,
};

use crate::config::allowlist_for;

/// The `contract_version` both read routes answer with. A different value is
/// a contract this client does not know, and is refused rather than guessed.
pub const CONTRACT_VERSION: &str = "inference-connection-v1";
/// The server's catalog ceiling. A longer list is refused, not truncated.
pub const MAX_OFFERS: usize = 16;
/// Bounded: onboarding waits on these calls, and a hung commons must not hang
/// the shell with it.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

const OFFERS_PATH: &str = "/v1/account/inference-connection/offers";
const CURRENT_PATH: &str = "/v1/account/inference-connection";

/// The account's current selection as the server reports it. Carries no
/// witness material: the select response is the only one that does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionStatus {
    pub connection_id: Uuid,
    pub state_version: i64,
    pub offer_id: String,
    pub revision: String,
    pub config_digest: String,
    pub disclosure_version: String,
    /// The selected revision is no longer in the operator catalog. The
    /// contributor must choose again; the current catalog entry must never be
    /// substituted for what they chose.
    pub reselection_required: bool,
}

impl ConnectionStatus {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.connection_id.is_nil() {
            return Err("connection_id_invalid");
        }
        if self.state_version < 1 {
            return Err("state_version_invalid");
        }
        if !valid_identifier(&self.offer_id) {
            return Err("offer_id_invalid");
        }
        if !valid_digest(&self.revision) || !valid_digest(&self.config_digest) {
            return Err("digest_invalid");
        }
        // A retired selection may name a disclosure this build no longer
        // speaks; it is still reported so the contributor can reselect.
        if !self.reselection_required && self.disclosure_version != DISCLOSURE_VERSION {
            return Err("disclosure_version_invalid");
        }
        Ok(())
    }
}

/// What a disconnect reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disconnected {
    pub connection_id: Uuid,
    pub state_version: i64,
}

#[derive(Deserialize)]
struct OffersBody {
    contract_version: String,
    offers: Vec<InferenceConnectionOffer>,
}

#[derive(Deserialize)]
struct CurrentBody {
    contract_version: String,
    selection: Option<ConnectionStatus>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DisconnectBody {
    connection_id: Uuid,
    state_version: i64,
    status: String,
}

/// Fixed-label outcomes. Never carries a response body, a URL, or a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionError {
    /// The session was rejected: expired, revoked, or not an account session.
    SessionInvalid,
    /// `409 connection_reselection_required`: the selected revision was
    /// retired. Nothing may be installed from it.
    ReselectionRequired,
    /// `409 connection_version_conflict`: the account changed since the
    /// caller last read it.
    VersionConflict,
    /// `409 idempotency key reused` with a different request.
    IdempotencyConflict,
    /// `403 account is not eligible`.
    AccountIneligible,
    /// `400 invalid inference selection`: no such offer, or not the revision
    /// and digest the catalog now holds.
    InvalidSelection,
    /// `404 connection not found`.
    NotFound,
    /// A success response that failed validation or did not match the request.
    ResponseInvalid,
    /// Transport failure, refused host, or an unrecognized refusal.
    Unavailable,
}

impl ConnectionError {
    /// The IPC refusal label. `ReselectionRequired` keeps the server's own
    /// name, because the operator contract names it and a shell routes on it.
    pub fn label(self) -> &'static str {
        match self {
            Self::SessionInvalid => "account-session-required",
            Self::ReselectionRequired => "connection_reselection_required",
            Self::VersionConflict => "inference-connection-version-conflict",
            Self::IdempotencyConflict => "inference-connection-idempotency-conflict",
            Self::AccountIneligible => "inference-connection-account-ineligible",
            Self::InvalidSelection => "inference-connection-offer-invalid",
            Self::NotFound => "inference-connection-not-found",
            Self::ResponseInvalid => "inference-connection-response-invalid",
            Self::Unavailable => "inference-connection-unavailable",
        }
    }
}

/// One call's result, and the rotated account session the server handed back
/// with it. The server rotates on a refusal as well as a success, so the
/// caller must persist `rotated_token` whatever `result` is.
pub struct ConnectionCall<T> {
    pub result: Result<T, ConnectionError>,
    pub rotated_token: Option<String>,
}

/// Where a call goes and what it presents. Built from the same enrolled
/// `ingest_url` / `allowed_hosts` every other ingest call uses.
pub struct Endpoint<'a> {
    pub ingest_url: &'a str,
    pub allowed_hosts: Option<&'a str>,
    pub account_session_token: &'a str,
}

impl Endpoint<'_> {
    async fn call<Req, Resp>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Req>,
    ) -> ConnectionCall<Resp>
    where
        Req: Serialize + ?Sized,
        Resp: serde::de::DeserializeOwned,
    {
        let client = match Client::builder(
            self.ingest_url,
            "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
        )
        .bearer_token(self.account_session_token)
        .host_allowlist(allowlist_for(self.allowed_hosts))
        .timeout(REQUEST_TIMEOUT)
        .build()
        {
            Ok(client) => client,
            Err(_) => {
                return ConnectionCall {
                    result: Err(ConnectionError::Unavailable),
                    rotated_token: None,
                };
            }
        };
        let response = client
            .call_json_with_response_header::<Req, Resp>(
                method,
                path,
                &[],
                body,
                ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
            )
            .await;
        ConnectionCall {
            result: response.result.map_err(|error| classify(&error)),
            rotated_token: response.response_header,
        }
    }
}

fn then<T, U>(
    call: ConnectionCall<T>,
    check: impl FnOnce(T) -> Result<U, ConnectionError>,
) -> ConnectionCall<U> {
    ConnectionCall {
        result: call.result.and_then(check),
        rotated_token: call.rotated_token,
    }
}

/// The operator's published offers. Every offer is validated; one malformed
/// offer refuses the whole list rather than silently dropping it, so a shell
/// never shows a subset it believes is the catalog.
pub async fn list_offers(endpoint: &Endpoint<'_>) -> ConnectionCall<Vec<InferenceConnectionOffer>> {
    let call = endpoint
        .call::<(), OffersBody>(Method::GET, OFFERS_PATH, None)
        .await;
    then(call, |body| {
        if body.contract_version != CONTRACT_VERSION
            || body.offers.len() > MAX_OFFERS
            || body.offers.iter().any(|offer| offer.validate().is_err())
        {
            return Err(ConnectionError::ResponseInvalid);
        }
        let mut ids = std::collections::BTreeSet::new();
        if !body.offers.iter().all(|offer| ids.insert(&offer.offer_id)) {
            return Err(ConnectionError::ResponseInvalid);
        }
        Ok(body.offers)
    })
}

/// The account's current selection, or `None` when there is none.
pub async fn current_selection(
    endpoint: &Endpoint<'_>,
) -> ConnectionCall<Option<ConnectionStatus>> {
    let call = endpoint
        .call::<(), CurrentBody>(Method::GET, CURRENT_PATH, None)
        .await;
    then(call, |body| {
        if body.contract_version != CONTRACT_VERSION
            || body
                .selection
                .as_ref()
                .is_some_and(|selection| selection.validate().is_err())
        {
            return Err(ConnectionError::ResponseInvalid);
        }
        Ok(body.selection)
    })
}

/// Select exactly `request`: the offer revision, configuration digest and
/// disclosure version the contributor was shown.
///
/// A malformed request is refused before it leaves the machine. The response
/// must name the same revision, digest and disclosure, and pass validation, or
/// it is refused: a server answering a different configuration than the one
/// shown is exactly the silent replacement the operator contract forbids.
pub async fn select(
    endpoint: &Endpoint<'_>,
    request: &SelectInferenceConnection,
) -> ConnectionCall<SelectedInferenceConnection> {
    if request.validate().is_err() {
        return ConnectionCall {
            result: Err(ConnectionError::InvalidSelection),
            rotated_token: None,
        };
    }
    let call = endpoint
        .call::<SelectInferenceConnection, SelectedInferenceConnection>(
            Method::POST,
            CURRENT_PATH,
            Some(request),
        )
        .await;
    then(call, |selected| {
        if selected.validate().is_err()
            || selected.revision != request.revision
            || selected.config_digest != request.config_digest
            || selected.disclosure_version != request.disclosure_version
        {
            return Err(ConnectionError::ResponseInvalid);
        }
        Ok(selected)
    })
}

/// Disconnect `connection_id`. The server stops new use once a cooperating
/// client observes it; it recalls nothing already disclosed.
pub async fn disconnect(
    endpoint: &Endpoint<'_>,
    connection_id: Uuid,
) -> ConnectionCall<Disconnected> {
    let path = format!("{CURRENT_PATH}/{connection_id}");
    let call = endpoint
        .call::<(), DisconnectBody>(Method::DELETE, &path, None)
        .await;
    then(call, |body| {
        if body.status != "revoked" || body.connection_id != connection_id || body.state_version < 1
        {
            return Err(ConnectionError::ResponseInvalid);
        }
        Ok(Disconnected {
            connection_id: body.connection_id,
            state_version: body.state_version,
        })
    })
}

fn classify(error: &OcError) -> ConnectionError {
    let status = match error {
        OcError::ServerLabel { status, .. } | OcError::HttpFailure { status, .. } => *status,
        OcError::MalformedResponse { .. } => return ConnectionError::ResponseInvalid,
        _ => return ConnectionError::Unavailable,
    };
    match (status.as_u16(), error.server_label()) {
        (409, Some("connection_reselection_required")) => ConnectionError::ReselectionRequired,
        (409, Some("connection_version_conflict")) => ConnectionError::VersionConflict,
        (409, Some("idempotency key reused")) => ConnectionError::IdempotencyConflict,
        (403, Some("account is not eligible")) => ConnectionError::AccountIneligible,
        (400, Some("invalid inference selection")) => ConnectionError::InvalidSelection,
        (404, Some("connection not found")) => ConnectionError::NotFound,
        // 401 is an expired or revoked session; 403 otherwise is a device
        // bearer or a cross-origin refusal. Either way the contributor needs a
        // fresh account sign-in, not a retry.
        (401 | 403, _) => ConnectionError::SessionInvalid,
        _ => ConnectionError::Unavailable,
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A stub of the four #1019 routes on 127.0.0.1, recording what each call
    //! presented and sent. Shared with the daemon's IPC tests.

    use std::sync::{Arc, Mutex};

    use axum::{
        Json, Router,
        extract::Path,
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::{delete, get},
    };
    use trace_commons_protocol::inference_connection::{
        ConnectionWitnessConfig, InferenceConnectionOffer, SelectInferenceConnection,
        SelectedInferenceConnection,
    };
    use uuid::Uuid;

    pub fn revision() -> String {
        format!("sha256:{}", "a".repeat(64))
    }
    pub fn config_digest() -> String {
        format!("sha256:{}", "b".repeat(64))
    }

    pub fn offer() -> InferenceConnectionOffer {
        InferenceConnectionOffer {
            offer_id: "near-ai".into(),
            revision: revision(),
            provider_id: "near-ai".into(),
            disclosure_version: super::DISCLOSURE_VERSION.into(),
            config_digest: config_digest(),
        }
    }

    pub fn witness() -> ConnectionWitnessConfig {
        ConnectionWitnessConfig {
            url: "https://witness.example/v1".into(),
            signing_address: format!("0x{}", "ab".repeat(20)),
            expected_measurements: vec![format!("mrtd={}", "ab".repeat(48))],
        }
    }

    /// What the stub answers. Tests mutate it between calls.
    pub struct Stub {
        pub connection_id: Uuid,
        pub state_version: i64,
        pub witness: ConnectionWitnessConfig,
        pub receipt: Option<String>,
        /// `Some(status, label)` refuses the select route.
        pub select_refusal: Option<(StatusCode, &'static str)>,
        /// Echo a different config digest than the one requested.
        pub select_wrong_digest: bool,
        /// The current route's selection: `None` is "no selection".
        pub current: Option<super::ConnectionStatus>,
        pub disconnect_refusal: Option<(StatusCode, &'static str)>,
        pub offers: Vec<InferenceConnectionOffer>,
    }

    #[derive(Default)]
    pub struct Seen {
        pub auth: Vec<String>,
        pub selects: Vec<serde_json::Value>,
        pub disconnects: Vec<String>,
    }

    pub struct Server {
        pub base: String,
        pub stub: Arc<Mutex<Stub>>,
        pub seen: Arc<Mutex<Seen>>,
    }

    impl Server {
        /// The selection the current route should report once a select
        /// succeeded, matching the stub's own state.
        pub fn status(&self, reselection_required: bool) -> super::ConnectionStatus {
            let stub = self.stub.lock().unwrap();
            super::ConnectionStatus {
                connection_id: stub.connection_id,
                state_version: stub.state_version,
                offer_id: "near-ai".into(),
                revision: revision(),
                config_digest: config_digest(),
                disclosure_version: super::DISCLOSURE_VERSION.into(),
                reselection_required,
            }
        }
    }

    fn record(seen: &Mutex<Seen>, headers: &HeaderMap) {
        seen.lock().unwrap().auth.push(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_string(),
        );
    }

    fn refusal(status: StatusCode, label: &str) -> Response {
        (status, Json(serde_json::json!({ "error": label }))).into_response()
    }

    pub async fn spawn() -> Server {
        let stub = Arc::new(Mutex::new(Stub {
            connection_id: Uuid::new_v4(),
            state_version: 1,
            witness: witness(),
            receipt: None,
            select_refusal: None,
            select_wrong_digest: false,
            current: None,
            disconnect_refusal: None,
            offers: vec![offer()],
        }));
        let seen = Arc::new(Mutex::new(Seen::default()));
        let offers = {
            let (stub, seen) = (stub.clone(), seen.clone());
            move |headers: HeaderMap| {
                let (stub, seen) = (stub.clone(), seen.clone());
                async move {
                    record(&seen, &headers);
                    Json(serde_json::json!({
                        "contract_version": "inference-connection-v1",
                        "inference_connection_selection_required": true,
                        "description": "server copy",
                        "offers": stub.lock().unwrap().offers,
                    }))
                }
            }
        };
        let current = {
            let (stub, seen) = (stub.clone(), seen.clone());
            move |headers: HeaderMap| {
                let (stub, seen) = (stub.clone(), seen.clone());
                async move {
                    record(&seen, &headers);
                    Json(serde_json::json!({
                        "contract_version": "inference-connection-v1",
                        "selection": stub.lock().unwrap().current,
                        "install_on_this_device": false,
                    }))
                }
            }
        };
        let select = {
            let (stub, seen) = (stub.clone(), seen.clone());
            move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                let (stub, seen) = (stub.clone(), seen.clone());
                async move {
                    record(&seen, &headers);
                    seen.lock().unwrap().selects.push(body.clone());
                    let Ok(request) = serde_json::from_value::<SelectInferenceConnection>(body)
                    else {
                        return refusal(StatusCode::BAD_REQUEST, "invalid inference selection");
                    };
                    let stub = stub.lock().unwrap();
                    if let Some((status, label)) = stub.select_refusal {
                        return refusal(status, label);
                    }
                    let selected = SelectedInferenceConnection {
                        connection_id: stub.connection_id,
                        state_version: stub.state_version,
                        revision: request.revision,
                        config_digest: if stub.select_wrong_digest {
                            format!("sha256:{}", "c".repeat(64))
                        } else {
                            request.config_digest
                        },
                        disclosure_version: request.disclosure_version,
                        witness: stub.witness.clone(),
                        inference_receipt_endpoint: stub.receipt.clone(),
                    };
                    Json(selected).into_response()
                }
            }
        };
        let disconnect = {
            let (stub, seen) = (stub.clone(), seen.clone());
            move |headers: HeaderMap, Path(id): Path<String>| {
                let (stub, seen) = (stub.clone(), seen.clone());
                async move {
                    record(&seen, &headers);
                    seen.lock().unwrap().disconnects.push(id.clone());
                    let stub = stub.lock().unwrap();
                    if let Some((status, label)) = stub.disconnect_refusal {
                        return refusal(status, label);
                    }
                    Json(serde_json::json!({
                        "connection_id": id,
                        "state_version": stub.state_version + 1,
                        "status": "revoked",
                    }))
                    .into_response()
                }
            }
        };
        let router = Router::new()
            .route("/v1/account/inference-connection/offers", get(offers))
            .route(
                "/v1/account/inference-connection",
                get(current).post(select),
            )
            .route(
                "/v1/account/inference-connection/{connection_id}",
                delete(disconnect),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Server {
            base: format!("http://{addr}"),
            stub,
            seen,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use axum::http::StatusCode;

    fn endpoint(base: &str) -> Endpoint<'_> {
        Endpoint {
            ingest_url: base,
            allowed_hosts: None,
            account_session_token: "tcn1_account.session",
        }
    }

    fn request() -> SelectInferenceConnection {
        SelectInferenceConnection {
            offer_id: "near-ai".into(),
            revision: revision(),
            config_digest: config_digest(),
            disclosure_version: DISCLOSURE_VERSION.into(),
            idempotency_key: Uuid::new_v4(),
            expected_current_version: None,
        }
    }

    #[tokio::test]
    async fn every_route_presents_the_account_session() {
        let server = spawn().await;
        let ep = endpoint(&server.base);
        assert_eq!(list_offers(&ep).await.result.unwrap(), vec![offer()]);
        assert!(current_selection(&ep).await.result.unwrap().is_none());
        let id = server.stub.lock().unwrap().connection_id;
        select(&ep, &request()).await.result.unwrap();
        disconnect(&ep, id).await.result.unwrap();
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.auth.len(), 4);
        assert!(
            seen.auth
                .iter()
                .all(|auth| auth == "Bearer tcn1_account.session")
        );
    }

    #[tokio::test]
    async fn select_sends_exactly_the_shown_revision_digest_and_disclosure() {
        let server = spawn().await;
        let sent = request();
        let selected = select(&endpoint(&server.base), &sent).await.result.unwrap();
        assert_eq!(selected.witness, witness());
        let seen = server.seen.lock().unwrap();
        assert_eq!(
            seen.selects,
            vec![serde_json::to_value(&sent).unwrap()],
            "the wire request must be the shown values and nothing else"
        );
    }

    #[tokio::test]
    async fn a_response_for_a_different_configuration_is_refused() {
        let server = spawn().await;
        server.stub.lock().unwrap().select_wrong_digest = true;
        let result = select(&endpoint(&server.base), &request()).await.result;
        assert_eq!(result.unwrap_err(), ConnectionError::ResponseInvalid);
    }

    #[tokio::test]
    async fn a_malformed_witness_in_the_response_is_refused() {
        let server = spawn().await;
        server.stub.lock().unwrap().witness.url = "http://witness.example".into();
        let result = select(&endpoint(&server.base), &request()).await.result;
        assert_eq!(result.unwrap_err(), ConnectionError::ResponseInvalid);
    }

    #[tokio::test]
    async fn a_malformed_request_never_leaves_the_machine() {
        let server = spawn().await;
        let mut bad = request();
        bad.disclosure_version = "other".into();
        let result = select(&endpoint(&server.base), &bad).await.result;
        assert_eq!(result.unwrap_err(), ConnectionError::InvalidSelection);
        assert!(server.seen.lock().unwrap().selects.is_empty());
    }

    #[tokio::test]
    async fn reselection_required_is_its_own_outcome() {
        let server = spawn().await;
        server.stub.lock().unwrap().select_refusal =
            Some((StatusCode::CONFLICT, "connection_reselection_required"));
        let result = select(&endpoint(&server.base), &request()).await.result;
        assert_eq!(result.unwrap_err(), ConnectionError::ReselectionRequired);
        assert_eq!(
            ConnectionError::ReselectionRequired.label(),
            "connection_reselection_required"
        );
    }

    #[tokio::test]
    async fn a_device_bearer_refusal_asks_for_an_account_session() {
        let server = spawn().await;
        server.stub.lock().unwrap().select_refusal =
            Some((StatusCode::FORBIDDEN, "account session required"));
        let result = select(&endpoint(&server.base), &request()).await.result;
        assert_eq!(result.unwrap_err(), ConnectionError::SessionInvalid);
        assert_eq!(
            ConnectionError::SessionInvalid.label(),
            "account-session-required"
        );
    }

    #[tokio::test]
    async fn one_malformed_offer_refuses_the_list() {
        let server = spawn().await;
        let mut bad = offer();
        bad.offer_id = "second".into();
        bad.config_digest = "sha256:short".into();
        server.stub.lock().unwrap().offers.push(bad);
        let result = list_offers(&endpoint(&server.base)).await.result;
        assert_eq!(result.unwrap_err(), ConnectionError::ResponseInvalid);
    }

    #[tokio::test]
    async fn disconnect_names_the_connection_and_reports_the_revocation() {
        let server = spawn().await;
        let id = Uuid::new_v4();
        let outcome = disconnect(&endpoint(&server.base), id)
            .await
            .result
            .unwrap();
        assert_eq!(outcome.connection_id, id);
        assert_eq!(
            server.seen.lock().unwrap().disconnects,
            vec![id.to_string()]
        );
        server.stub.lock().unwrap().disconnect_refusal =
            Some((StatusCode::NOT_FOUND, "connection not found"));
        let result = disconnect(&endpoint(&server.base), id).await.result;
        assert_eq!(result.unwrap_err(), ConnectionError::NotFound);
    }
}
