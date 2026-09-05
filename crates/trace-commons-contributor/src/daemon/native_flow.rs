//! One native wallet lifecycle, shared by socket and embedded clients. No logs
//! contain account inputs, ceremony IDs, browser URLs, or remote error bodies.
use super::{
    account_onboarding,
    ipc::{DaemonShared, Request, Response},
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
        }
    }
    fn busy(&self) -> bool {
        self.starting
            || matches!(
                self.state,
                WalletState::Checking | WalletState::WaitingForWallet
            )
            || self.attempt.is_some()
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
        self.attempt.take()
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
            self.refuse(witness_copy().wallet.failed);
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
        let attempt = {
            let mut map = flows().lock().expect("native flow lock");
            map.get_mut(shared.store.dir())
                .filter(|f| f.id == flow_id)
                .and_then(Flow::cancel)
        };
        if let Some(attempt) = attempt {
            cancel_attempt(shared, req.id, attempt);
        }
        return snapshot(shared, req.id, flow_id);
    }
    if action == "wait" {
        tokio::time::sleep(POLL).await;
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
    let value = result.result.unwrap_or_default();
    let cancel = {
        let mut map = flows().lock().expect("native flow lock");
        match map.get_mut(shared.store.dir()).filter(|f| f.id == flow_id) {
            Some(flow) if action == "start" => flow.finish_start(generation, &value),
            Some(flow) if flow.generation == generation && !flow.cancelled => {
                if value.get("ready").and_then(Value::as_bool) == Some(true) {
                    flow.state = WalletState::Ready;
                    flow.message = witness_copy().wallet.available;
                } else {
                    flow.refuse(witness_copy().wallet.unavailable);
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
    value["view"] = json!({"ready":ready,"state":if ready{"Ready"}else{"Refused"},"message":if ready{copy.ready}else{copy.failed},"tone":if ready{"neutral"}else{copy.refused_tone},"glyph":if ready{""}else{copy.refused_glyph}});
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
}
