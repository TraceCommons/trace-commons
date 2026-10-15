//! One native wallet lifecycle, shared by socket and embedded clients. No logs
//! contain account inputs, ceremony IDs, browser URLs, or remote error bodies.
use super::{
    account_onboarding,
    ipc::{DaemonShared, Request, Response},
    nearai_credential::{
        ceremony,
        near_wallet::{NearWalletChallenge, NearWalletError},
    },
};
use crate::witness_copy::witness_copy;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// A challenge with less than this left is replaced, not handed to the signer:
/// the passkey and consent ceremony takes longer than a few seconds.
const NATIVE_EXPIRY_MARGIN_MS: u64 = 60_000;
const POLL: Duration = Duration::from_secs(2);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum WalletState {
    Unsupported,
    Idle,
    Checking,
    Ready,
    WaitingForWallet,
    Refused,
    Complete,
}
#[derive(Clone, Serialize)]
pub struct WalletView {
    pub flow_id: String,
    pub state: WalletState,
    pub busy: bool,
    pub can_check: bool,
    pub can_start: bool,
    pub can_edit: bool,
    pub can_cancel: bool,
    pub wait: bool,
    pub message: &'static str,
    pub tone: &'static str,
    pub glyph: &'static str,
    pub browser_url: Option<String>,
}
struct Flow {
    id: String,
    state: WalletState,
    origin: Option<String>,
    attempt: Option<String>,
    browser: Option<String>,
    generation: u64,
    starting: bool,
    cancelled: bool,
    message: &'static str,
    /// The live NEP-413 challenge handed to the native signer, if one was
    /// asked for. Never serialized, logged or re-derived from the reply.
    challenge: Option<NearWalletChallenge>,
    /// The NEAR AI credential ceremony a native-signer proof was handed to.
    credential: Option<String>,
}
impl Flow {
    fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            state: WalletState::Idle,
            origin: None,
            attempt: None,
            browser: None,
            generation: 0,
            starting: false,
            cancelled: false,
            message: "",
            challenge: None,
            credential: None,
        }
    }
    fn busy(&self) -> bool {
        self.starting
            || matches!(
                self.state,
                WalletState::Checking | WalletState::WaitingForWallet
            )
            || self.attempt.is_some()
            || self.credential.is_some()
    }
    fn view(&self) -> WalletView {
        let refused = self.state == WalletState::Refused;
        WalletView {
            flow_id: self.id.clone(),
            state: self.state,
            busy: self.busy(),
            can_check: !self.busy(),
            can_start: self.state == WalletState::Ready,
            can_edit: !self.busy(),
            can_cancel: self.busy(),
            wait: matches!(
                self.state,
                WalletState::Checking | WalletState::WaitingForWallet
            ),
            message: self.message,
            tone: if refused { "refused" } else { "neutral" },
            glyph: if refused {
                witness_copy().wallet.refused_glyph
            } else {
                ""
            },
            browser_url: self.browser.clone(),
        }
    }
    fn refuse(&mut self, message: &'static str) {
        self.state = WalletState::Refused;
        self.message = message;
        self.browser = None;
    }
    fn cancel(&mut self) -> Option<String> {
        self.generation += 1;
        self.cancelled = true;
        self.state = WalletState::Idle;
        self.message = witness_copy().wallet.cancelled;
        self.browser = None;
        self.starting = false;
        self.challenge = None;
        self.attempt.take()
    }
    /// Progress of a native-signer proof inside the credential ceremony.
    fn credential_progress(&mut self, status: Option<&str>) {
        match status {
            Some(ceremony::STATUS_COMPLETE) => {
                self.credential = None;
                self.state = WalletState::Complete;
                self.message = "";
            }
            Some("starting" | "waiting_for_browser") => {}
            _ => {
                self.credential = None;
                self.refuse(witness_copy().wallet.failed);
            }
        }
    }
    fn finish_start(&mut self, generation: u64, value: &Value) -> Option<String> {
        let attempt = value
            .get("attempt_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if self.generation != generation || self.cancelled {
            return attempt;
        }
        self.starting = false;
        let browser = value.get("browser_url").and_then(Value::as_str);
        if value.get("status").and_then(Value::as_str) != Some("waiting_for_wallet")
            || attempt.is_none()
            || !browser
                .is_some_and(|browser| same_origin(self.origin.as_deref().unwrap_or(""), browser))
        {
            self.refuse(crate::witness_copy::wallet_start_refusal_line(
                value.get("reason").and_then(Value::as_str),
            ));
            return attempt;
        }
        self.attempt = attempt;
        self.browser = browser.map(str::to_owned);
        self.state = WalletState::WaitingForWallet;
        self.message = witness_copy().wallet.waiting;
        None
    }
    fn progress(&mut self, value: &Value) {
        match value.get("status").and_then(Value::as_str) {
            Some("complete") => {
                self.attempt = None;
                self.browser = None;
                self.state = WalletState::Complete;
                self.message = "";
            }
            Some("starting" | "waiting_for_wallet") => {}
            Some("failed" | "cancelled" | "expired") => {
                self.attempt = None;
                self.refuse(witness_copy().wallet.failed);
            }
            _ => self.refuse(witness_copy().wallet.failed),
        }
    }
}
fn flows() -> &'static Mutex<HashMap<PathBuf, Flow>> {
    static FLOWS: OnceLock<Mutex<HashMap<PathBuf, Flow>>> = OnceLock::new();
    FLOWS.get_or_init(Default::default)
}
fn same_origin(origin: &str, browser: &str) -> bool {
    let (Ok(a), Ok(b)) = (reqwest::Url::parse(origin), reqwest::Url::parse(browser)) else {
        return false;
    };
    a.scheme() == "https"
        && b.scheme() == "https"
        && a.host_str().is_some()
        && a.origin() == b.origin()
        && a.username().is_empty()
        && b.username().is_empty()
        && a.password().is_none()
        && b.password().is_none()
}
fn request(id: u64, method: &str, params: Value) -> Request {
    Request {
        id,
        method: method.into(),
        params,
    }
}
fn snapshot(shared: &DaemonShared, id: u64, flow_id: &str) -> Response {
    let map = flows().lock().expect("native flow lock");
    match map
        .get(shared.store.dir())
        .filter(|flow| flow.id == flow_id)
    {
        Some(flow) => Response::ok(id, serde_json::to_value(flow.view()).unwrap_or_default()),
        None => Response::ok(
            id,
            json!({"state":"Unsupported","busy":false,"can_check":false,"can_start":false,"can_edit":false,"can_cancel":false,"wait":false,"flow_id":"","message":"","tone":"neutral","glyph":""}),
        ),
    }
}
fn cancel_attempt(shared: &DaemonShared, id: u64, attempt: String) {
    let _ = account_onboarding::handle_cancel(
        shared,
        &request(id, "near_account_cancel", json!({"attempt_id":attempt})),
    );
}

/// UI command boundary. Only check/start perform remote actions; wait owns the
/// cadence and observes the underlying ceremony. A close invalidates pending
/// replies before cancelling a known attempt, including late start replies.
pub async fn handle_wallet(shared: &DaemonShared, req: &Request) -> Response {
    let action = req
        .params
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("");
    let flow_id = req
        .params
        .get("flow_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if action == "open" {
        let mut map = flows().lock().expect("native flow lock");
        if map.get(shared.store.dir()).is_some_and(Flow::busy) {
            return Response::err(req.id, super::ipc::ERR_UNAVAILABLE, "native_wallet_busy");
        }
        if map.len() >= 32 && !map.contains_key(shared.store.dir()) {
            return Response::err(
                req.id,
                super::ipc::ERR_UNAVAILABLE,
                "native_wallet_capacity",
            );
        }
        let flow = Flow::new();
        let result = serde_json::to_value(flow.view()).unwrap_or_default();
        map.insert(shared.store.dir().to_path_buf(), flow);
        return Response::ok(req.id, result);
    }
    if action == "cancel" {
        let (attempt, credential) = {
            let mut map = flows().lock().expect("native flow lock");
            map.get_mut(shared.store.dir())
                .filter(|f| f.id == flow_id)
                .map(|f| {
                    let credential = f.credential.take();
                    (f.cancel(), credential)
                })
                .unwrap_or_default()
        };
        if let Some(attempt) = attempt {
            cancel_attempt(shared, req.id, attempt);
        }
        if let Some(credential) = credential {
            ceremony::cancel(shared.store.dir(), Some(&credential));
        }
        return snapshot(shared, req.id, flow_id);
    }
    if action == "native_payload" {
        return native_payload(shared, req.id, flow_id);
    }
    if action == "native_complete" {
        return native_complete(shared, req, flow_id).await;
    }
    if action == "wait" {
        tokio::time::sleep(POLL).await;
        let credential = {
            let map = flows().lock().expect("native flow lock");
            map.get(shared.store.dir())
                .filter(|f| f.id == flow_id && !f.cancelled)
                .and_then(|f| f.credential.clone().map(|c| (c, f.generation)))
        };
        if let Some((credential, generation)) = credential {
            let result = super::nearai_credential::handle_status_async(
                shared,
                &request(
                    req.id,
                    "near_ai_credential_status",
                    json!({"attempt_id":credential}),
                ),
            )
            .await
            .result
            .unwrap_or_default();
            let mut map = flows().lock().expect("native flow lock");
            if let Some(flow) = map.get_mut(shared.store.dir()).filter(|f| {
                f.id == flow_id
                    && f.generation == generation
                    && !f.cancelled
                    && f.credential.as_deref() == Some(credential.as_str())
            }) {
                flow.credential_progress(result.get("attempt_status").and_then(Value::as_str));
            }
            return snapshot(shared, req.id, flow_id);
        }
        let pair = {
            let map = flows().lock().expect("native flow lock");
            map.get(shared.store.dir())
                .filter(|f| f.id == flow_id && !f.cancelled)
                .and_then(|f| f.attempt.clone().map(|a| (a, f.generation)))
        };
        if let Some((attempt, generation)) = pair {
            let result = account_onboarding::handle_status(
                shared,
                &request(req.id, "near_account_status", json!({"attempt_id":attempt})),
            );
            let mut map = flows().lock().expect("native flow lock");
            if let Some(flow) = map
                .get_mut(shared.store.dir())
                .filter(|f| f.id == flow_id && f.generation == generation && !f.cancelled)
            {
                flow.progress(&result.result.unwrap_or_default());
            }
        }
        return snapshot(shared, req.id, flow_id);
    }
    let origin = req
        .params
        .get("ingest_url")
        .and_then(Value::as_str)
        .unwrap_or("");
    let account = req
        .params
        .get("account_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let generation = {
        let mut map = flows().lock().expect("native flow lock");
        let Some(flow) = map
            .get_mut(shared.store.dir())
            .filter(|f| f.id == flow_id && !f.busy())
        else {
            return Response::err(req.id, super::ipc::ERR_BAD_PARAMS, "native_wallet_state");
        };
        flow.cancelled = false;
        match action {
            "check" => {
                flow.origin = Some(origin.to_owned());
                flow.state = WalletState::Checking;
                flow.message = "";
            }
            "start"
                if flow.state == WalletState::Ready
                    && flow.origin.as_deref() == Some(origin)
                    && !account.is_empty() =>
            {
                flow.starting = true;
                flow.state = WalletState::Checking;
                flow.message = witness_copy().wallet.opening;
            }
            _ => {
                flow.refuse(witness_copy().wallet.failed);
                return Response::ok(
                    req.id,
                    serde_json::to_value(flow.view()).unwrap_or_default(),
                );
            }
        }
        flow.generation += 1;
        flow.generation
    };
    let result = if action == "check" {
        account_onboarding::handle_capabilities(
            shared,
            &request(
                req.id,
                "near_account_capabilities",
                json!({"ingest_url":origin}),
            ),
        )
        .await
    } else {
        account_onboarding::handle_start(
            shared,
            &request(
                req.id,
                "near_account_start",
                json!({"ingest_url":origin,"account_id":account}),
            ),
        )
        .await
    };
    let value = match result.error {
        Some(error) => json!({"reason": error.message}),
        None => result.result.unwrap_or_default(),
    };
    let cancel = {
        let mut map = flows().lock().expect("native flow lock");
        match map.get_mut(shared.store.dir()).filter(|f| f.id == flow_id) {
            Some(flow) if action == "start" => flow.finish_start(generation, &value),
            Some(flow) if flow.generation == generation && !flow.cancelled => {
                if value.get("ready").and_then(Value::as_bool) == Some(true) {
                    flow.state = WalletState::Ready;
                    flow.message = witness_copy().wallet.available;
                } else {
                    // The daemon says which class of refusal this was; the
                    // shared mapper turns that into the one sentence every
                    // shell shows. This surface used to say "unavailable"
                    // for all of them, including for the refusal that made
                    // signup impossible in every shipped application.
                    flow.refuse(crate::witness_copy::wallet_refusal_line(
                        value.get("reason").and_then(Value::as_str),
                    ));
                }
                None
            }
            _ => value
                .get("attempt_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    };
    if let Some(attempt) = cancel {
        cancel_attempt(shared, req.id, attempt);
    }
    snapshot(shared, req.id, flow_id)
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// The NEP-413 payload of this flow's live challenge, minting one when the
/// flow has none or the last one expired. The browser route's constants.
fn native_payload(shared: &DaemonShared, id: u64, flow_id: &str) -> Response {
    let mut map = flows().lock().expect("native flow lock");
    let Some(flow) = map
        .get_mut(shared.store.dir())
        .filter(|f| f.id == flow_id && !f.busy())
    else {
        return Response::err(id, super::ipc::ERR_BAD_PARAMS, "native_wallet_state");
    };
    let now = unix_millis();
    if flow.challenge.as_ref().is_none_or(|challenge| {
        now.saturating_add(NATIVE_EXPIRY_MARGIN_MS) >= challenge.expires_at_ms()
    }) {
        match NearWalletChallenge::native(now) {
            Ok(challenge) => flow.challenge = Some(challenge),
            Err(_) => {
                flow.challenge = None;
                return Response::err(id, super::ipc::ERR_UNAVAILABLE, "native_wallet_state");
            }
        }
    }
    match flow.challenge.as_ref() {
        Some(challenge) => Response::ok(id, challenge.native_payload()),
        None => Response::err(id, super::ipc::ERR_BAD_PARAMS, "native_wallet_state"),
    }
}

/// Verify the native signer's proof against this flow's own challenge, then
/// hand it to the NEAR AI credential ceremony, which continues exactly as for
/// a verified browser callback. No input reaches a reply or a log.
async fn native_complete(shared: &DaemonShared, req: &Request, flow_id: &str) -> Response {
    let field = |name: &str| req.params.get(name).and_then(Value::as_str).unwrap_or("");
    let (proof, generation) = {
        let mut map = flows().lock().expect("native flow lock");
        let Some(flow) = map
            .get_mut(shared.store.dir())
            .filter(|f| f.id == flow_id && !f.busy())
        else {
            return Response::err(req.id, super::ipc::ERR_BAD_PARAMS, "native_wallet_state");
        };
        let Some(challenge) = flow.challenge.as_mut() else {
            return Response::err(req.id, super::ipc::ERR_BAD_PARAMS, "native_wallet_state");
        };
        match challenge.verify_signed(
            field("account_id"),
            field("public_key"),
            field("signature"),
            unix_millis(),
        ) {
            Ok(proof) => {
                flow.challenge = None;
                flow.cancelled = false;
                flow.starting = true;
                flow.state = WalletState::Checking;
                flow.message = "";
                flow.generation += 1;
                (proof, flow.generation)
            }
            Err(NearWalletError::Invalid) => {
                return Response::err(
                    req.id,
                    super::ipc::ERR_BAD_PARAMS,
                    "native_wallet_signature_invalid",
                );
            }
            Err(_) => {
                flow.challenge = None;
                return Response::err(req.id, super::ipc::ERR_BAD_PARAMS, "native_wallet_state");
            }
        }
    };
    let started = ceremony::begin_verified(&shared.store, proof).await;
    let attempt = started.as_ref().ok().and_then(|value| {
        value
            .get("attempt_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    let late = {
        let mut map = flows().lock().expect("native flow lock");
        match map.get_mut(shared.store.dir()).filter(|f| f.id == flow_id) {
            Some(flow) if flow.generation == generation && !flow.cancelled => {
                flow.starting = false;
                match attempt {
                    Some(attempt) => {
                        flow.credential = Some(attempt);
                        flow.state = WalletState::WaitingForWallet;
                    }
                    None => flow.refuse(witness_copy().wallet.failed),
                }
                None
            }
            _ => attempt,
        }
    };
    if let Some(attempt) = late {
        ceremony::cancel(shared.store.dir(), Some(&attempt));
    }
    snapshot(shared, req.id, flow_id)
}

/// One expiry decision, attached to the original daemon response. Existing raw
/// status/expiry fields remain intact for older clients; native adapters render view.
pub fn admission_response(mut response: Response, now: i64) -> Response {
    let value = response.result.get_or_insert_with(|| json!({}));
    let ready = response.error.is_none()
        && value.get("status").and_then(Value::as_str) == Some("ready_for_next_inference")
        && value
            .get("expires_at")
            .and_then(Value::as_i64)
            .is_some_and(|expiry| expiry > now);
    let copy = witness_copy().admission;
    let refused = crate::witness_copy::admission_refusal_line(
        response.error.as_ref().map(|error| error.message.as_str()),
    );
    value["view"] = json!({"ready":ready,"state":if ready{"Ready"}else{"Refused"},"message":if ready{copy.ready}else{refused},"tone":if ready{"neutral"}else{copy.refused_tone},"glyph":if ready{""}else{copy.refused_glyph}});
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    fn starting() -> Flow {
        let mut f = Flow::new();
        f.origin = Some("https://commons.example".into());
        f.starting = true;
        f.state = WalletState::Checking;
        f
    }
    #[test]
    fn ceremony_mismatch_reaches_native_views_without_a_browser_or_retry_loop() {
        let mut flow = Flow::new();
        flow.starting = true;
        let response = json!({"reason": account_onboarding::CEREMONY_MISMATCH});
        assert!(flow.finish_start(0, &response).is_none());
        let view = flow.view();
        assert_eq!(view.state, WalletState::Refused);
        assert!(view.message.contains("disagree"));
        assert!(!view.message.contains(account_onboarding::CEREMONY_MISMATCH));
        assert!(!view.busy);
        assert!(!view.wait);
        assert!(view.browser_url.is_none());
        flow.cancel();
        flow.finish_start(0, &response);
        assert_eq!(flow.view().state, WalletState::Idle);
    }

    #[test]
    fn wallet_origin_requires_exact_https_and_no_credentials() {
        for invalid in [
            "http://commons.example/x",
            "https://elsewhere.example/x",
            "https://commons.example:444/x",
            "https://user@commons.example/x",
        ] {
            assert!(!same_origin("https://commons.example", invalid));
        }
        assert!(same_origin(
            "https://commons.example",
            "https://commons.example:443/x"
        ));
    }
    #[test]
    fn closing_pending_start_cancels_late_attempt_without_browser() {
        let mut f = starting();
        f.cancel();
        let generation = f.generation - 1;
        assert_eq!(f.finish_start(generation,&json!({"status":"waiting_for_wallet","attempt_id":"fixture","browser_url":"https://commons.example/x"})).as_deref(),Some("fixture"));
        assert!(f.view().browser_url.is_none());
        assert!(!f.view().busy);
    }
    #[test]
    fn wrong_origin_start_refuses_and_cancels_attempt() {
        let mut f = starting();
        assert_eq!(f.finish_start(0,&json!({"status":"waiting_for_wallet","attempt_id":"fixture","browser_url":"https://wrong.example"})).as_deref(),Some("fixture"));
        assert_eq!(f.state, WalletState::Refused);
        assert_eq!(f.view().glyph, "⊘");
    }
    #[test]
    fn lifecycle_unknown_status_is_cancellable_and_never_complete() {
        let mut f = starting();
        assert!(f.finish_start(0,&json!({"status":"waiting_for_wallet","attempt_id":"fixture","browser_url":"https://commons.example/x"})).is_none());
        assert!(f.view().wait);
        f.progress(&json!({"status":"unknown"}));
        assert_eq!(f.state, WalletState::Refused);
        assert!(f.view().can_cancel);
        f.progress(&json!({"status":"complete"}));
        assert_eq!(f.state, WalletState::Complete);
        assert!(!f.view().busy);
    }
    /// The check path must carry the daemon's refusal class all the way to
    /// the sentence the shells print. This drives `handle_wallet` itself
    /// rather than the mapper, because the seam that used to be wrong is the
    /// one line between them: every refusal became "unavailable" there.
    #[tokio::test]
    async fn a_refused_address_and_an_unanswered_one_reach_the_shell_differently() {
        use crate::config::tests_support::temp_store;
        use crate::witness_copy::witness_copy;

        async fn check(origin: &str) -> Value {
            let (dir, store) = temp_store();
            std::mem::forget(dir);
            let shared = DaemonShared::load(store).unwrap();
            let open = handle_wallet(
                &shared,
                &request(1, "native_wallet_flow", json!({"action":"open"})),
            )
            .await;
            let flow_id = open.result.unwrap()["flow_id"]
                .as_str()
                .unwrap()
                .to_string();
            let checked = handle_wallet(
                &shared,
                &request(
                    2,
                    "native_wallet_flow",
                    json!({"action":"check","flow_id":flow_id,"ingest_url":origin}),
                ),
            )
            .await;
            checked.result.unwrap()
        }

        // Nothing is listening here, so the daemon dials and gets nothing.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let refused = check("http://commons.example").await;
        let unanswered = check(&format!("https://127.0.0.1:{port}")).await;

        let copy = witness_copy().wallet;
        assert_eq!(refused["message"], copy.address_refused);
        assert_eq!(unanswered["message"], copy.unreachable);
        assert_ne!(refused["message"], unanswered["message"]);
        // Both are refusals, and neither is the sentence for a commons that
        // answered and declined.
        assert_eq!(refused["state"], "Refused");
        assert_eq!(unanswered["state"], "Refused");
        assert_ne!(unanswered["message"], copy.unavailable);
    }

    async fn opened() -> (DaemonShared, String) {
        let (dir, store) = crate::config::tests_support::temp_store();
        std::mem::forget(dir);
        let shared = DaemonShared::load(store).unwrap();
        let open = handle_wallet(
            &shared,
            &request(1, "native_wallet_flow", json!({"action":"open"})),
        )
        .await;
        let flow_id = open.result.unwrap()["flow_id"]
            .as_str()
            .unwrap()
            .to_string();
        (shared, flow_id)
    }

    #[tokio::test]
    async fn native_complete_without_open_flow_is_refused() {
        let (dir, store) = crate::config::tests_support::temp_store();
        std::mem::forget(dir);
        let shared = DaemonShared::load(store).unwrap();
        let params = json!({
            "action":"native_complete","flow_id":"not-a-flow",
            "account_id":"alice.near","public_key":"ed25519:x","signature":"x"
        });
        let refused = handle_wallet(&shared, &request(1, "native_wallet_flow", params)).await;
        let error = refused.error.expect("refused");
        assert_eq!(error.message, "native_wallet_state");
        // An open flow that never asked for a payload has nothing to complete.
        let (shared, flow_id) = opened().await;
        let params = json!({
            "action":"native_complete","flow_id":flow_id,
            "account_id":"alice.near","public_key":"ed25519:x","signature":"x"
        });
        let refused = handle_wallet(&shared, &request(2, "native_wallet_flow", params)).await;
        assert_eq!(
            refused.error.expect("refused").message,
            "native_wallet_state"
        );
        let payload = handle_wallet(
            &shared,
            &request(
                3,
                "native_wallet_flow",
                json!({"action":"native_payload","flow_id":"other"}),
            ),
        )
        .await;
        assert_eq!(
            payload.error.expect("refused").message,
            "native_wallet_state"
        );
    }

    #[tokio::test]
    async fn native_complete_with_a_bad_signature_is_labelled_and_retryable() {
        let (shared, flow_id) = opened().await;
        let payload = handle_wallet(
            &shared,
            &request(
                1,
                "native_wallet_flow",
                json!({"action":"native_payload","flow_id":flow_id}),
            ),
        )
        .await
        .result
        .expect("payload");
        let params = json!({
            "action":"native_complete","flow_id":flow_id,
            "account_id":"alice.near","public_key":"ed25519:x","signature":"x"
        });
        let refused = handle_wallet(&shared, &request(2, "native_wallet_flow", params)).await;
        let error = refused.error.expect("refused");
        assert_eq!(error.message, "native_wallet_signature_invalid");
        assert!(!error.message.contains("alice"));
        let again = handle_wallet(
            &shared,
            &request(
                3,
                "native_wallet_flow",
                json!({"action":"native_payload","flow_id":flow_id}),
            ),
        )
        .await
        .result
        .expect("payload");
        assert_eq!(
            payload, again,
            "a refused signature keeps the live challenge"
        );
    }

    #[tokio::test]
    async fn native_payload_matches_browser_configuration() {
        use base64::Engine;
        let (shared, flow_id) = opened().await;
        let payload = handle_wallet(
            &shared,
            &request(
                1,
                "native_wallet_flow",
                json!({"action":"native_payload","flow_id":flow_id}),
            ),
        )
        .await
        .result
        .expect("payload");
        let browser = {
            let map = flows().lock().unwrap();
            map.get(shared.store.dir())
                .and_then(|flow| flow.challenge.as_ref())
                .expect("live challenge")
                .browser_configuration()
        };
        assert_eq!(payload["message"], browser["message"]);
        assert_eq!(payload["recipient"], browser["recipient"]);
        let nonce = base64::engine::general_purpose::STANDARD
            .decode(payload["nonce"].as_str().unwrap())
            .unwrap();
        assert_eq!(json!(nonce), browser["nonce"]);
    }

    fn base58(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let mut input = bytes.to_vec();
        let zeros = input.iter().take_while(|byte| **byte == 0).count();
        let mut out = Vec::new();
        let mut start = zeros;
        while start < input.len() {
            let mut rem = 0u16;
            for byte in &mut input[start..] {
                let value = rem * 256 + u16::from(*byte);
                *byte = (value / 58) as u8;
                rem = value % 58;
            }
            out.push(ALPHABET[rem as usize]);
            while start < input.len() && input[start] == 0 {
                start += 1;
            }
        }
        out.extend(std::iter::repeat_n(b'1', zeros));
        out.reverse();
        String::from_utf8(out).unwrap()
    }

    #[tokio::test]
    async fn native_complete_with_a_good_signature_consumes_the_challenge_and_hands_off() {
        use crate::daemon::credential_store::{
            CredentialError, CredentialReference, SecretBackend,
        };
        use base64::Engine;
        use ring::signature::{Ed25519KeyPair, KeyPair};
        use sha2::{Digest, Sha256};
        use std::sync::Arc;

        struct Unentitled;
        impl SecretBackend for Unentitled {
            fn read(&self, _: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
                Err(CredentialError::Unentitled)
            }
            fn write(&self, _: &CredentialReference, _: &[u8]) -> Result<(), CredentialError> {
                Err(CredentialError::Unavailable)
            }
            fn delete(&self, _: &CredentialReference) -> Result<(), CredentialError> {
                Err(CredentialError::Unavailable)
            }
        }

        let (shared, flow_id) = opened().await;
        // The hand-off stops at the storage probe, before any network.
        crate::daemon::cloud_credential_test_support::install_backend(
            &shared.store,
            Arc::new(Unentitled),
        );
        let payload = handle_wallet(
            &shared,
            &request(
                1,
                "native_wallet_flow",
                json!({"action":"native_payload","flow_id":flow_id}),
            ),
        )
        .await
        .result
        .expect("payload");
        let nonce: [u8; 32] = base64::engine::general_purpose::STANDARD
            .decode(payload["nonce"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        // NEP-413: SHA256(u32-LE tag || Borsh(message, nonce, recipient, none)).
        let mut encoded = (2_147_484_061u32).to_le_bytes().to_vec();
        let message = payload["message"].as_str().unwrap();
        encoded.extend_from_slice(&(message.len() as u32).to_le_bytes());
        encoded.extend_from_slice(message.as_bytes());
        encoded.extend_from_slice(&nonce);
        let recipient = payload["recipient"].as_str().unwrap();
        encoded.extend_from_slice(&(recipient.len() as u32).to_le_bytes());
        encoded.extend_from_slice(recipient.as_bytes());
        encoded.push(0);
        let digest = Sha256::digest(&encoded);
        let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let params = json!({
            "action":"native_complete","flow_id":flow_id,
            "account_id":"alice.near",
            "public_key": format!("ed25519:{}", base58(key.public_key().as_ref())),
            "signature": base64::engine::general_purpose::STANDARD.encode(key.sign(&digest).as_ref()),
        });
        let done = handle_wallet(&shared, &request(2, "native_wallet_flow", params.clone())).await;
        let view = done
            .result
            .expect("a verified proof gets a view, not an error");
        assert_eq!(
            view["state"], "Refused",
            "the storage probe refused the hand-off"
        );
        assert_eq!(view["message"], witness_copy().wallet.failed);
        assert!(view["browser_url"].is_null());
        // The challenge is single use: the same proof cannot be replayed.
        let replay = handle_wallet(&shared, &request(3, "native_wallet_flow", params)).await;
        assert_eq!(
            replay.error.expect("refused").message,
            "native_wallet_state"
        );
        // Cancelling afterwards is safe and leaves the flow idle.
        let cancelled = handle_wallet(
            &shared,
            &request(
                4,
                "native_wallet_flow",
                json!({"action":"cancel","flow_id":flow_id}),
            ),
        )
        .await;
        assert_eq!(cancelled.result.expect("view")["state"], "Idle");
    }

    #[tokio::test]
    async fn a_challenge_close_to_expiry_is_replaced_not_handed_out() {
        let (shared, flow_id) = opened().await;
        let request_payload = |id| {
            request(
                id,
                "native_wallet_flow",
                json!({"action":"native_payload","flow_id":flow_id}),
            )
        };
        handle_wallet(&shared, &request_payload(1))
            .await
            .result
            .expect("payload");
        // Leave the live challenge 30 s of life.
        let near_expiry = {
            let mut map = flows().lock().unwrap();
            let flow = map.get_mut(shared.store.dir()).unwrap();
            let aged =
                unix_millis() - crate::daemon::nearai_credential::near_wallet::MAX_AGE_MS + 30_000;
            let challenge = NearWalletChallenge::native(aged).unwrap();
            let payload = challenge.native_payload();
            flow.challenge = Some(challenge);
            payload
        };
        let second = handle_wallet(&shared, &request_payload(2))
            .await
            .result
            .expect("payload");
        let third = handle_wallet(&shared, &request_payload(3))
            .await
            .result
            .expect("payload");
        assert_ne!(near_expiry["nonce"], second["nonce"]);
        assert_eq!(second, third, "a fresh challenge is kept, not re-minted");
    }

    #[test]
    fn admission_requires_fresh_integer_expiry_and_success() {
        for value in [
            json!({}),
            json!({"status":"ready_for_next_inference","expires_at":5}),
            json!({"status":"ready_for_next_inference","expires_at":"later"}),
        ] {
            assert_eq!(
                admission_response(Response::ok(1, value), 10)
                    .result
                    .unwrap()["view"]["ready"],
                false
            );
        }
        assert_eq!(
            admission_response(
                Response::ok(
                    1,
                    json!({"status":"ready_for_next_inference","expires_at":11})
                ),
                10
            )
            .result
            .unwrap()["view"]["ready"],
            true
        );
    }

    fn admission_message(label: &str) -> String {
        admission_response(Response::err(1, "unavailable", label), 10)
            .result
            .expect("view")["view"]["message"]
            .as_str()
            .expect("message")
            .to_string()
    }

    /// `admission_receipt_endpoint_required` is the one admission failure a
    /// contributor can do nothing about by retrying, and the only one whose
    /// cause is configuration rather than a session, a proxy or a permission.
    /// Collapsed into the generic sentence it reads as "try again", which is
    /// advice that can never work.
    #[test]
    fn a_missing_receipt_endpoint_does_not_borrow_the_generic_failure_sentence() {
        // `admission_setup_unavailable` is the one code that stays generic --
        // it covers transport and filesystem failures this build cannot name.
        // Every other code now classifies, so picking a named one here would
        // compare two distinct sentences and pass for the wrong reason.
        let generic = admission_message("admission_setup_unavailable");
        assert_eq!(
            generic,
            witness_copy().admission.failed,
            "an unclassified failure still says the generic sentence"
        );
        assert_ne!(
            admission_message("admission_receipt_endpoint_required"),
            generic,
            "a contributor whose commons published no receipt endpoint is told to retry \
             a thing that cannot succeed until the endpoint arrives"
        );
        assert_eq!(
            admission_message("admission_receipt_endpoint_required"),
            witness_copy().admission.failed_receipt_endpoint,
            "the sentence comes from the shared copy, so the three shells say the same \
             thing this one does"
        );
    }

    /// Every cause a person must act on differently gets its own sentence.
    ///
    /// One label having escaped the generic sentence is not the fix: a person
    /// who has not granted the inference-body permission, one whose IronWire
    /// is not running, and one whose session came from an agent this build
    /// cannot read are three different problems with three different actions,
    /// and all three are told to check their settings and try again.
    #[test]
    fn each_distinct_admission_cause_gets_its_own_sentence() {
        let generic = admission_message("admission_setup_unavailable");
        assert_eq!(
            generic,
            witness_copy().admission.failed,
            "an unclassified failure still says the generic sentence"
        );
        let distinct = [
            "admission_setup_consent_required",
            "admission_setup_unenrolled",
            "admission_setup_proxy_missing",
            "admission_setup_source_unsupported",
            "admission_setup_endpoint_untrusted",
            "admission_receipt_endpoint_required",
        ];
        for label in distinct {
            assert_ne!(
                admission_message(label),
                generic,
                "{label} still borrows the sentence that tells a person to retry"
            );
        }
        let mut seen: Vec<String> = distinct.iter().map(|l| admission_message(l)).collect();
        seen.sort();
        let before = seen.len();
        seen.dedup();
        assert_eq!(
            seen.len(),
            before,
            "two causes needing different actions were given the same words"
        );
    }

    /// Its own sentence, not its own outcome: this is still a refusal, and a
    /// distinct message must not leak into the `ready` flag, the state or the
    /// tone.
    #[test]
    fn a_missing_receipt_endpoint_is_still_refused() {
        let view = admission_response(
            Response::err(1, "unavailable", "admission_receipt_endpoint_required"),
            10,
        )
        .result
        .expect("view")["view"]
            .clone();
        assert_eq!(view["ready"], false);
        assert_eq!(view["state"], "Refused");
        assert_eq!(view["tone"], witness_copy().admission.refused_tone);
        assert_eq!(view["glyph"], witness_copy().admission.refused_glyph);
    }
}
