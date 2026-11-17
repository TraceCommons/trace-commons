//! Scale conformance without provider credentials or network traffic.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use futures_util::{StreamExt, stream};
use ironwire_core::capability::Capabilities;
use ironwire_core::config::Config;
use ironwire_core::protocol::{BackendId, BackendKind, ClientIdentity, ModelTier};
use ironwire_core::quota::QuotaSnapshot;
use ironwire_creds::{ConsentLedger, SubscriptionLogin, SubscriptionProduct};
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BACKEND_CHECK_CONCURRENCY, BackendRegistry};
use ironwire_upstream::anthropic::{anthropic_capabilities, anthropic_models};
use ironwire_upstream::backend::{
    Backend, BackendStatus, UpstreamError, UpstreamRequest, UpstreamResponse,
};
use ironwire_upstream::observe::Observation;
use tower::ServiceExt;

const BODY: &[u8] = br#"{"model":"claude-opus-4-6","system":"You are Claude Code","messages":[{"role":"user","content":"hi"}]}"#;

#[derive(Default)]
struct Checks {
    active: AtomicUsize,
    maximum: AtomicUsize,
    calls: AtomicUsize,
}
impl Checks {
    fn blocking_check(&self, delay: Duration) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(delay);
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Login {
    id: BackendId,
    caps: Capabilities,
    status_checks: Arc<Checks>,
    probe_checks: Arc<Checks>,
    sends: Arc<AtomicUsize>,
    delay: Duration,
    overloaded: bool,
}
#[async_trait::async_trait]
impl Backend for Login {
    fn id(&self) -> &BackendId {
        &self.id
    }
    fn name(&self) -> &str {
        self.id.as_str()
    }
    fn kind(&self) -> BackendKind {
        BackendKind::Subscription
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn models(&self) -> Vec<(String, ModelTier)> {
        anthropic_models()
    }
    fn required_client_identity(&self) -> Option<ClientIdentity> {
        Some(ClientIdentity::ClaudeCode)
    }
    fn login(&self) -> Option<SubscriptionLogin> {
        Some(SubscriptionLogin {
            product: SubscriptionProduct::Claude,
            location: Some(format!("/synthetic/{}", self.id)),
            config_dir: Some(format!("/synthetic/{}", self.id)),
            source_dir: None,
            default: false,
        })
    }
    async fn status(&self) -> BackendStatus {
        let checks = Arc::clone(&self.status_checks);
        let delay = self.delay;
        tokio::task::spawn_blocking(move || checks.blocking_check(delay))
            .await
            .expect("credential check");
        BackendStatus {
            id: self.id.clone(),
            name: self.name().to_string(),
            kind: self.kind(),
            authenticated: true,
            account_scope: None,
            detail: None,
            quota: self.quota(),
            models: self.models(),
        }
    }
    async fn send(&self, request: UpstreamRequest) -> Result<UpstreamResponse, UpstreamError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.body.as_ref(), BODY, "native bytes changed");
        if self.overloaded {
            return Err(UpstreamError::Upstream {
                backend: self.id.clone(),
                status: StatusCode::from_u16(529).unwrap(),
                body: Bytes::from_static(b"overloaded"),
            });
        }
        Ok(UpstreamResponse {
            status: StatusCode::OK,
            headers: vec![("content-type".into(), "application/json".into())],
            body: stream::once(async { Ok(Bytes::from_static(b"{\"content\":[]}")) }).boxed(),
        })
    }
    fn record(&self, _: &Observation) {}
    fn quota(&self) -> QuotaSnapshot {
        QuotaSnapshot::default()
    }
    async fn probe(&self) -> Result<(), UpstreamError> {
        let checks = Arc::clone(&self.probe_checks);
        let delay = self.delay;
        tokio::task::spawn_blocking(move || checks.blocking_check(delay))
            .await
            .expect("credential check");
        Ok(())
    }
}

fn setup(
    count: usize,
    overloaded: usize,
    delay: Duration,
) -> (AppState, Arc<Checks>, Arc<Checks>, Vec<Arc<AtomicUsize>>) {
    let status = Arc::new(Checks::default());
    let probe = Arc::new(Checks::default());
    let mut registry = BackendRegistry::new();
    let mut consent = ConsentLedger::default();
    let mut sends = Vec::new();
    for n in 0..count {
        let id = format!("claude-{n}");
        consent.grant_for(&id, Some(&format!("/synthetic/{id}")), chrono::Utc::now());
        let sent = Arc::new(AtomicUsize::new(0));
        registry.push(Arc::new(Login {
            id: BackendId::from(id.as_str()),
            caps: anthropic_capabilities(),
            status_checks: status.clone(),
            probe_checks: probe.clone(),
            sends: sent.clone(),
            delay,
            overloaded: n < overloaded,
        }));
        sends.push(sent);
    }
    let mut config = Config::default();
    config.subscriptions.cross_account_fallback = true;
    (
        AppState::new(registry, config, consent, "test-token".into()),
        status,
        probe,
        sends,
    )
}

async fn infer(state: AppState, login: usize) -> axum::response::Response {
    app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/anthropic/account/claude-{login}/v1/messages"))
                .header("content-type", "application/json")
                .body(Body::from(BODY))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_thousand_logins_keep_fresh_checks_bounded_across_concurrent_requests() {
    let (state, status, _, sends) = setup(1024, 0, Duration::from_millis(1));
    let started = Instant::now();
    let results = stream::iter(0..4)
        .map(|login| infer(state.clone(), login))
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
    for response in results {
        assert_eq!(response.status(), StatusCode::OK);
        axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
    }
    assert_eq!(
        status.calls.load(Ordering::SeqCst),
        4 * 1024,
        "every decision freshly checks all credentials"
    );
    assert!((2..=BACKEND_CHECK_CONCURRENCY).contains(&status.maximum.load(Ordering::SeqCst)));
    for sent in sends.iter().take(4) {
        assert_eq!(
            sent.load(Ordering::SeqCst),
            1,
            "a bound login's route was lost"
        );
    }
    eprintln!(
        "four concurrent requests with 1024 subscriptions: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_thousand_logins_preserve_control_order_and_bound_simultaneous_probes() {
    let (state, status, probe, _) = setup(1024, 0, Duration::from_millis(1));
    let statuses = state.backends.statuses().await;
    for (n, row) in statuses.iter().enumerate() {
        assert_eq!(row.id.as_str(), format!("claude-{n}"));
    }
    let candidates = state
        .backends
        .candidates(&statuses, &state.consent_snapshot());
    assert_eq!(candidates.len(), 1024);
    assert!(candidates.iter().all(|c| c.healthy && c.consented));
    let responses = stream::iter(0..2)
        .map(|_| {
            app(state.clone()).oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_ironwire/probe")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
        })
        .buffer_unordered(2)
        .collect::<Vec<_>>()
        .await;
    for response in responses {
        let response = response.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(rows.len(), 1024);
        for (n, row) in rows.iter().enumerate() {
            assert_eq!(row["id"], format!("claude-{n}"));
            assert_eq!(row["ok"], true);
        }
    }
    assert!(status.maximum.load(Ordering::SeqCst) <= BACKEND_CHECK_CONCURRENCY);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2048);
    assert!((2..=BACKEND_CHECK_CONCURRENCY).contains(&probe.maximum.load(Ordering::SeqCst)));
}

#[tokio::test(flavor = "current_thread")]
async fn blocking_credential_readers_leave_the_async_worker_available() {
    let (state, _, _, _) = setup(16, 0, Duration::from_millis(20));
    let checks = tokio::spawn(async move { state.backends.statuses().await });
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(
        !checks.is_finished(),
        "status polling blocked the only runtime worker"
    );
    assert_eq!(checks.await.unwrap().len(), 16);
}

#[tokio::test]
async fn overloaded_logins_do_not_spend_the_attempt_budget_before_a_healthy_sibling_is_tried() {
    let (state, _, _, sends) = setup(3, 2, Duration::ZERO);
    let response = infer(state, 0).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("x-ironwire-backend").unwrap(),
        "claude-2"
    );
    assert_eq!(sends[0].load(Ordering::SeqCst), 3);
    assert_eq!(sends[1].load(Ordering::SeqCst), 3);
    assert_eq!(sends[2].load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_bound_login_checks_only_its_own_credential_until_cross_account_fallback_is_enabled() {
    let (state, checks, _, sends) = setup(1024, 0, Duration::ZERO);
    state
        .policy
        .lock()
        .unwrap()
        .set_cross_account_fallback(false);
    let response = infer(state.clone(), 1023).await;
    assert_eq!(response.status(), StatusCode::OK);
    axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(checks.calls.load(Ordering::SeqCst), 1);
    assert_eq!(sends[1023].load(Ordering::SeqCst), 1);
    assert!(
        sends[..1023]
            .iter()
            .all(|sent| sent.load(Ordering::SeqCst) == 0)
    );
    let mut peek = ironwire_core::peek::RequestPeek::inspect(
        ironwire_core::protocol::Protocol::AnthropicMessages,
        &serde_json::from_slice(BODY).unwrap(),
        BODY.len(),
    );
    peek.client_login = Some(BackendId::from("claude-1023"));
    let statuses = state.backends.routing_statuses(&peek, false).await;
    assert_eq!(statuses.len(), 1);
    let candidates = state
        .backends
        .candidates(&statuses, &state.consent_snapshot());
    assert_eq!(
        candidates
            .iter()
            .filter(|candidate| candidate.healthy)
            .count(),
        1,
        "unobserved sibling credentials must not become authenticated candidates"
    );
    assert!(
        candidates
            .iter()
            .find(|candidate| candidate.id.as_str() == "claude-1023")
            .unwrap()
            .healthy
    );
    let statuses = state.backends.routing_statuses(&peek, true).await;
    assert_eq!(statuses.len(), 1024);
    assert!(
        state
            .backends
            .candidates(&statuses, &state.consent_snapshot())
            .iter()
            .all(|candidate| candidate.healthy)
    );
    assert_eq!(checks.calls.load(Ordering::SeqCst), 1026);
}
