// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One judgement request end to end, without a socket.
//!
//! [`JudgementService::judge`] takes the opened request plaintext (the
//! claim and the request nonce travel inside it) and returns the unsigned
//! results, the echoed request nonce and the spend bucket. Reading the body,
//! opening the seal, signing and sealing the response, and the judgement
//! permits belong to the binary (slice 5), which wraps this.
//!
//! Order (spec section 3, "The service's check order", steps 3 to 7):
//!
//! 1. parse the plaintext: the wire version, the closed shape, the digest
//!    cap;
//! 2. verify the claim offline; derive the keyed account digest;
//! 3. count the digest's marked events (a digest that is not a version-1
//!    digest, or a request that would judge nothing, is refused here);
//! 4. record the request nonce in the replay set, refusing a repeat with
//!    `judgement_request_replayed`, or `judgement_saturated` when the set is
//!    full. Only a request whose claim verified gets this far, so an
//!    unauthenticated caller cannot fill the set;
//! 5. take a judgement permit from the caller's `acquire_permit`, held until
//!    the last model call returns. A caller cannot hold one before step 4;
//! 6. run each requested judgement in request order: `task` and `finished`
//!    once, `event_kind` once per marked event for the first
//!    [`JUDGEMENT_EVENT_KIND_MAX`], and `unjudged` for the rest, so one
//!    session costs at most six model calls;
//! 7. any refusal discards every label of the request.
//!
//! One log line per request, hash-only: a random request id,
//! `digest_sha256` (`none` when the plaintext did not parse), the model id,
//! `ok` or the refusal label, and a latency bucket. Nothing else.

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use trace_commons_protocol::insights_judgement::{
    JUDGEMENT_EVENT_KIND_MAX, JudgementKind, JudgementRefusal, JudgementRequestNonce,
    JudgementResult, JudgementSpend, TurnDigest, parse_judgement_request,
};

use super::claim::{IssuerKeys, verify_judgement_claim};
use super::digest::marked_event_count;
use super::limiter::{ClaimTime, LimiterKeys};
use super::receipt_check::JudgementCaller;

/// The tracing target of the request log line.
pub const LOG_TARGET: &str = "trace_commons_judgement";
/// How long an accepted request nonce is remembered: the claim's 15-minute
/// lifetime plus skew. A replay after this carries an expired claim, which
/// the claim check refuses.
pub const REPLAY_WINDOW_SECS: i64 = 20 * 60;
/// The most request nonces remembered at once. A full set refuses with
/// `judgement_saturated` rather than forgetting a nonce early.
pub const REPLAY_SET_MAX: usize = 65_536;

/// The unsigned outcome of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedDigest {
    /// Echoed from the request, for the signed response.
    pub request_nonce: JudgementRequestNonce,
    pub digest_sha256: String,
    pub results: Vec<JudgementResult>,
    pub spend: JudgementSpend,
}

/// The request nonces accepted in the last [`REPLAY_WINDOW_SECS`], in
/// memory only. A restart empties it, and also changes the per-boot sealing
/// key, so a request sealed before the restart no longer opens.
pub struct ReplaySet {
    capacity: usize,
    inner: Mutex<ReplayInner>,
}

#[derive(Default)]
struct ReplayInner {
    seen: HashSet<[u8; 32]>,
    /// Insertion order with the time of each insertion, oldest first.
    order: VecDeque<(i64, [u8; 32])>,
}

impl std::fmt::Debug for ReplaySet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplaySet")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl Default for ReplaySet {
    fn default() -> Self {
        Self::with_capacity(REPLAY_SET_MAX)
    }
}

impl ReplaySet {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            inner: Mutex::new(ReplayInner::default()),
        }
    }

    /// Record `nonce` as accepted at `now`.
    ///
    /// [`JudgementRefusal::RequestReplayed`] if it was accepted within the
    /// window; [`JudgementRefusal::Saturated`] if the set is full of
    /// unexpired nonces. A clock that steps back only keeps entries longer.
    pub fn record(&self, nonce: &JudgementRequestNonce, now: i64) -> Result<(), JudgementRefusal> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        while let Some(&(at, oldest)) = inner.order.front() {
            if now.saturating_sub(at) < REPLAY_WINDOW_SECS {
                break;
            }
            inner.order.pop_front();
            inner.seen.remove(&oldest);
        }
        let nonce = *nonce.as_bytes();
        if inner.seen.contains(&nonce) {
            return Err(JudgementRefusal::RequestReplayed);
        }
        if inner.seen.len() >= self.capacity {
            return Err(JudgementRefusal::Saturated);
        }
        inner.seen.insert(nonce);
        inner.order.push_back((now, nonce));
        Ok(())
    }

    /// How many nonces are remembered. For tests and the drill.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .seen
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The service: the claim verifier, the limiter keys, the replay set and the
/// caller.
pub struct JudgementService {
    issuer: IssuerKeys,
    limiter_keys: LimiterKeys,
    replay: ReplaySet,
    caller: JudgementCaller,
}

impl std::fmt::Debug for JudgementService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JudgementService")
            .field("caller", &self.caller)
            .finish_non_exhaustive()
    }
}

/// A coarse latency bucket for the log line.
pub fn latency_bucket(elapsed_ms: u128) -> &'static str {
    match elapsed_ms {
        0..1_000 => "lt_1s",
        1_000..5_000 => "1s_5s",
        5_000..30_000 => "5s_30s",
        _ => "ge_30s",
    }
}

impl JudgementService {
    pub fn new(issuer: IssuerKeys, limiter_keys: LimiterKeys, caller: JudgementCaller) -> Self {
        Self {
            issuer,
            limiter_keys,
            replay: ReplaySet::default(),
            caller,
        }
    }

    /// Judge one opened request plaintext at `now`.
    ///
    /// `acquire_permit` is called once, after the claim verified and the
    /// request nonce was recorded and before any model call; its permit is
    /// held until the last call returns. The binary passes its judgement
    /// semaphore's `try_acquire` mapped to `judgement_saturated`.
    pub async fn judge<P>(
        &self,
        plaintext: &[u8],
        now: i64,
        acquire_permit: impl FnOnce() -> Result<P, JudgementRefusal>,
    ) -> Result<JudgedDigest, JudgementRefusal> {
        let started = Instant::now();
        let request_id = uuid::Uuid::new_v4();
        let mut digest_sha256 = None;
        let outcome = self
            .judge_inner(plaintext, now, acquire_permit, &mut digest_sha256)
            .await;
        tracing::info!(
            target: LOG_TARGET,
            request_id = %request_id.simple(),
            digest_sha256 = %digest_sha256.as_deref().unwrap_or("none"),
            model_id = %self.caller.model_id(),
            outcome = match &outcome {
                Ok(_) => "ok",
                Err(refusal) => refusal.label(),
            },
            latency_bucket = latency_bucket(started.elapsed().as_millis()),
            "judgement_request"
        );
        outcome
    }

    async fn judge_inner<P>(
        &self,
        plaintext: &[u8],
        now: i64,
        acquire_permit: impl FnOnce() -> Result<P, JudgementRefusal>,
        digest_sha256: &mut Option<String>,
    ) -> Result<JudgedDigest, JudgementRefusal> {
        let request = parse_judgement_request(plaintext)?;
        *digest_sha256 = Some(request.digest_sha256());
        let account = verify_judgement_claim(&self.issuer, request.claim(), now)?;
        let account_key = self.limiter_keys.account_key(account.account_ref());
        let time = ClaimTime {
            iat: account.iat(),
            now,
        };
        // The raw account reference is not needed past this point.
        drop(account);
        // The digest goes to the model as text, so it must be UTF-8.
        let digest = std::str::from_utf8(request.digest())
            .map_err(|_| JudgementRefusal::RequestMalformed)?;
        // `stuck` comes alone with a turn digest, which parses closed: it
        // must carry no contributor text. The others read a session digest.
        let stuck = request.judgements() == [JudgementKind::Stuck];
        let events = if stuck {
            TurnDigest::parse(request.digest()).map_err(|_| JudgementRefusal::RequestMalformed)?;
            0
        } else {
            marked_event_count(digest)?
        };
        if events == 0
            && request
                .judgements()
                .iter()
                .all(|kind| *kind == JudgementKind::EventKind)
        {
            // Only `event_kind` was asked for, over a digest with no marks:
            // there is nothing to judge and nothing to sign.
            return Err(JudgementRefusal::RequestMalformed);
        }
        self.replay.record(request.request_nonce(), now)?;
        let _permit = acquire_permit()?;

        let mut results = Vec::new();
        for &kind in request.judgements() {
            match kind {
                JudgementKind::Task | JudgementKind::Finished | JudgementKind::Stuck => {
                    results.push(
                        self.caller
                            .judge(&account_key, digest, kind, None, time)
                            .await?,
                    );
                }
                JudgementKind::EventKind => {
                    for index in 0..events {
                        let result = if index < JUDGEMENT_EVENT_KIND_MAX {
                            self.caller
                                .judge(&account_key, digest, kind, Some(index), time)
                                .await?
                        } else {
                            JudgementResult::unjudged_event(index)
                                .map_err(|_| JudgementRefusal::RequestMalformed)?
                        };
                        results.push(result);
                    }
                }
            }
        }
        Ok(JudgedDigest {
            request_nonce: *request.request_nonce(),
            digest_sha256: request.digest_sha256(),
            results,
            spend: JudgementSpend {
                account_month_remaining_bucket: self
                    .caller
                    .limiter()
                    .remaining_bucket(&account_key, time),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judgement_service::catalog::{CatalogEntry, TokenPrices};
    use crate::judgement_service::claim::tests::{ACCOUNT, NOW, TestIssuer};
    use crate::judgement_service::ledger::LedgerState;
    use crate::judgement_service::limiter::tests::MemoryStore;
    use crate::judgement_service::limiter::{Limiter, LimiterCaps};
    use crate::judgement_service::mock::{
        MOCK_MODEL_ID, MockFault, MockJudgementModel, MockReceiptSigner,
    };
    use crate::judgement_service::production::ProductionCheck;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Arc, Mutex};
    use trace_commons_protocol::insights_judgement::{
        JudgementLabel, JudgementRequest, SpendBucket,
    };

    const ROOT: [u8; 32] = [5u8; 32];

    /// Held by every test that emits the request log line. With the tests
    /// running in parallel, another thread's first hit on that callsite
    /// can leave it filtered for the scoped subscriber the log-capture test
    /// installs (observed: an empty capture under the default test
    /// threads, a full one with `--test-threads=1`). Serialising the
    /// emitters removes the race without weakening the assertion.
    static LOG_LINE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    /// A string that appears only in the digest's excerpt, so its presence
    /// anywhere in the logs means digest text leaked.
    const CANARY: &str = "zebra-canary-excerpt-4417";

    fn digest(events: usize) -> String {
        let events: Vec<serde_json::Value> = (0..events)
            .map(|i| serde_json::json!({"prompt_ix": i, "kind": "interrupt"}))
            .collect();
        serde_json::json!({
            "schema_version": 1,
            "events": events,
            "excerpts": [{"ix": 0, "text": CANARY}],
        })
        .to_string()
    }

    struct Harness {
        issuer: TestIssuer,
        model: Arc<MockJudgementModel>,
        limiter: Arc<Limiter>,
        service: JudgementService,
    }

    fn harness() -> Harness {
        let issuer = TestIssuer::new();
        let model = Arc::new(MockJudgementModel::new(MockReceiptSigner::generate()));
        let mut pins = BTreeMap::new();
        pins.insert(MOCK_MODEL_ID.to_string(), vec![model.signer_public_hex()]);
        let check = ProductionCheck::for_tests(&[MOCK_MODEL_ID], pins);
        let mut entries = BTreeMap::new();
        entries.insert(
            MOCK_MODEL_ID.to_string(),
            CatalogEntry {
                model_id: MOCK_MODEL_ID.to_string(),
                verifiable: true,
                attestation_supported: true,
                prices: Some(TokenPrices {
                    input_nanousd: 40,
                    output_nanousd: 1350,
                }),
            },
        );
        check
            .catalog()
            .write()
            .expect("lock")
            .record_refresh(NOW, entries);
        let limiter = Arc::new(Limiter::new(
            LimiterCaps::from_usd(5, 100).expect("caps"),
            LedgerState::default(),
            Arc::new(MemoryStore::default()),
        ));
        let caller = JudgementCaller::new(model.clone(), Arc::new(check), limiter.clone());
        let service = JudgementService::new(issuer.keys(), LimiterKeys::from_root(&ROOT), caller);
        Harness {
            issuer,
            model,
            limiter,
            service,
        }
    }

    static NEXT_NONCE: AtomicU8 = AtomicU8::new(1);

    /// A fresh request nonce per call, as a daemon draws one per send.
    fn fresh_nonce() -> JudgementRequestNonce {
        let mut bytes = [0u8; 32];
        bytes[0] = NEXT_NONCE.fetch_add(1, Ordering::SeqCst);
        bytes[1..17].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        JudgementRequestNonce::from_bytes(bytes)
    }

    fn request(claim: String, digest: String, judgements: Vec<JudgementKind>) -> JudgementRequest {
        JudgementRequest::new(fresh_nonce(), claim, digest.into_bytes(), judgements)
            .expect("request")
    }

    fn all_three(claim: String, digest: String) -> JudgementRequest {
        request(
            claim,
            digest,
            vec![
                JudgementKind::Task,
                JudgementKind::EventKind,
                JudgementKind::Finished,
            ],
        )
    }

    fn plaintext(request: &JudgementRequest) -> Vec<u8> {
        request.to_json().into_bytes()
    }

    fn no_permit() -> Result<(), JudgementRefusal> {
        Ok(())
    }

    #[tokio::test]
    async fn a_request_judges_each_judgement_in_order_with_receipts() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request = all_three(h.issuer.valid_claim(), digest(2));
        let judged = h
            .service
            .judge(&plaintext(&request), NOW, no_permit)
            .await
            .expect("judged");
        assert_eq!(judged.digest_sha256, request.digest_sha256());
        assert_eq!(&judged.request_nonce, request.request_nonce());
        let shape: Vec<_> = judged
            .results
            .iter()
            .map(|r| (r.judgement(), r.event_index()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (JudgementKind::Task, None),
                (JudgementKind::EventKind, Some(0)),
                (JudgementKind::EventKind, Some(1)),
                (JudgementKind::Finished, None),
            ]
        );
        assert!(judged.results.iter().all(JudgementResult::is_attested));
        assert_eq!(h.model.calls(), 4);
        assert_eq!(
            judged.spend.account_month_remaining_bucket,
            SpendBucket::OverHalf
        );
        let account = LimiterKeys::from_root(&ROOT).account_key(ACCOUNT);
        assert!(h.limiter.account_spent_nanousd(&account, NOW) > 0);
    }

    fn turn_digest() -> String {
        serde_json::json!({
            "digest_version": 1, "calls": 24, "edits": 9, "files_edited": 1,
            "most_edits_to_one_file": 9, "failures": 9, "repeated_failures": 7,
            "checks": 9, "last_check_failed": true, "usual_calls": 15,
            "recent": [{"kind": "check", "failed": true, "repeated": true}],
        })
        .to_string()
    }

    /// `stuck` judges a turn digest, once. A session digest, or a turn
    /// digest carrying anything beyond counts, is refused before any call.
    #[tokio::test]
    async fn stuck_judges_a_turn_digest_and_nothing_else() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let stuck = request(
            h.issuer.valid_claim(),
            turn_digest(),
            vec![JudgementKind::Stuck],
        );
        let judged = h
            .service
            .judge(&plaintext(&stuck), NOW, no_permit)
            .await
            .expect("judged");
        assert_eq!(judged.results.len(), 1);
        assert_eq!(judged.results[0].judgement(), JudgementKind::Stuck);
        assert!(
            JudgementKind::Stuck
                .choices()
                .contains(&judged.results[0].label())
        );
        assert_eq!(h.model.calls(), 1);

        let mut with_text: serde_json::Value = serde_json::from_str(&turn_digest()).unwrap();
        with_text["prompt"] = serde_json::json!(CANARY);
        for digest in [digest(1), with_text.to_string()] {
            // The protocol will not build this request, so a client bug or a
            // hostile client has to write it by hand: a `task` request with
            // its judgements rewritten to `stuck`.
            let task = request(h.issuer.valid_claim(), digest, vec![JudgementKind::Task]);
            let mut refused: serde_json::Value = serde_json::from_str(&task.to_json()).unwrap();
            refused["judgements"] = serde_json::json!(["stuck"]);
            assert_eq!(
                h.service
                    .judge(refused.to_string().as_bytes(), NOW, no_permit)
                    .await
                    .err(),
                Some(JudgementRefusal::RequestMalformed)
            );
        }
        assert_eq!(h.model.calls(), 1, "no call for a refused digest");
    }

    #[tokio::test]
    async fn event_kind_is_judged_for_at_most_four_marks() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request = all_three(h.issuer.valid_claim(), digest(6));
        let judged = h
            .service
            .judge(&plaintext(&request), NOW, no_permit)
            .await
            .expect("judged");
        let events: Vec<_> = judged
            .results
            .iter()
            .filter(|r| r.judgement() == JudgementKind::EventKind)
            .collect();
        assert_eq!(events.len(), 6);
        for (i, result) in events.iter().enumerate() {
            assert_eq!(result.event_index(), Some(i as u32));
            assert_eq!(result.label() == JudgementLabel::Unjudged, i >= 4);
        }
        // Six model calls at most: task, four marks, finished.
        assert_eq!(h.model.calls(), 6);
    }

    #[tokio::test]
    async fn a_refused_claim_makes_no_call() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let other = TestIssuer::new();
        let refused = all_three(other.valid_claim(), digest(1));
        assert_eq!(
            h.service
                .judge(&plaintext(&refused), NOW, no_permit)
                .await
                .unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
        let pooled = h.issuer.claim_for("devfolio-pool", NOW, NOW + 60);
        assert_eq!(
            h.service
                .judge(&plaintext(&all_three(pooled, digest(1))), NOW, no_permit)
                .await
                .unwrap_err(),
            JudgementRefusal::AccountIneligible
        );
        assert_eq!(h.model.calls(), 0);
        // A refused claim records no nonce, so it cannot fill the set.
        assert!(h.service.replay.is_empty());
    }

    #[tokio::test]
    async fn a_malformed_digest_makes_no_call() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let not_a_digest = request(
            h.issuer.valid_claim(),
            "not a digest".to_string(),
            vec![JudgementKind::Task],
        );
        let mut not_utf8 = request(h.issuer.valid_claim(), digest(1), vec![JudgementKind::Task]);
        not_utf8 = JudgementRequest::new(
            *not_utf8.request_nonce(),
            not_utf8.claim().to_string(),
            vec![0xff, 0xfe],
            vec![JudgementKind::Task],
        )
        .expect("request");
        for malformed in [
            plaintext(&not_a_digest),
            plaintext(&not_utf8),
            b"{}".to_vec(),
        ] {
            assert_eq!(
                h.service
                    .judge(&malformed, NOW, no_permit)
                    .await
                    .unwrap_err(),
                JudgementRefusal::RequestMalformed
            );
        }
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn a_request_that_would_judge_nothing_is_malformed() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request = request(
            h.issuer.valid_claim(),
            digest(0),
            vec![JudgementKind::EventKind],
        );
        assert_eq!(
            h.service
                .judge(&plaintext(&request), NOW, no_permit)
                .await
                .unwrap_err(),
            JudgementRefusal::RequestMalformed
        );
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn one_failed_call_discards_every_label() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        h.model.set_fault(Some(MockFault::GatewayReceipt));
        let request = all_three(h.issuer.valid_claim(), digest(3));
        assert_eq!(
            h.service
                .judge(&plaintext(&request), NOW, no_permit)
                .await
                .unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
        // It stopped at the first failure rather than spending more.
        assert_eq!(h.model.calls(), 1);
    }

    #[tokio::test]
    async fn a_replayed_request_nonce_is_refused_before_any_call() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request = all_three(h.issuer.valid_claim(), digest(1));
        h.service
            .judge(&plaintext(&request), NOW, no_permit)
            .await
            .expect("judged");
        let calls = h.model.calls();
        assert_eq!(
            h.service
                .judge(&plaintext(&request), NOW + 60, no_permit)
                .await
                .unwrap_err(),
            JudgementRefusal::RequestReplayed
        );
        assert_eq!(h.model.calls(), calls);
    }

    #[tokio::test]
    async fn the_permit_is_taken_after_the_claim_and_the_nonce_and_before_any_call() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        // A refused claim never asks for a permit.
        let other = TestIssuer::new();
        let asked = std::cell::Cell::new(false);
        let refused = all_three(other.valid_claim(), digest(1));
        assert_eq!(
            h.service
                .judge(&plaintext(&refused), NOW, || {
                    asked.set(true);
                    Ok(())
                })
                .await
                .unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
        assert!(!asked.get());
        // No permit free: saturated, no model call, and the nonce is spent,
        // because a daemon never resends one.
        let request = all_three(h.issuer.valid_claim(), digest(1));
        assert_eq!(
            h.service
                .judge(&plaintext(&request), NOW, || {
                    Err::<(), _>(JudgementRefusal::Saturated)
                })
                .await
                .unwrap_err(),
            JudgementRefusal::Saturated
        );
        assert_eq!(h.model.calls(), 0);
        assert_eq!(h.service.replay.len(), 1);
    }

    #[test]
    fn the_replay_set_forgets_after_twenty_minutes_and_refuses_when_full() {
        assert_eq!(ReplaySet::default().capacity, 65_536);
        let set = ReplaySet::with_capacity(2);
        let a = fresh_nonce();
        let b = fresh_nonce();
        let c = fresh_nonce();
        set.record(&a, NOW).expect("first");
        assert_eq!(
            set.record(&a, NOW + REPLAY_WINDOW_SECS - 1).unwrap_err(),
            JudgementRefusal::RequestReplayed
        );
        set.record(&b, NOW + 1).expect("second");
        // Full of unexpired nonces: refused, not forgotten early.
        assert_eq!(
            set.record(&c, NOW + 2).unwrap_err(),
            JudgementRefusal::Saturated
        );
        // `a` expires at the window, which frees one slot.
        set.record(&c, NOW + REPLAY_WINDOW_SECS).expect("a expired");
        assert_eq!(set.len(), 2);
        set.record(&a, NOW + REPLAY_WINDOW_SECS).unwrap_err();
        // A clock that steps back keeps every entry.
        assert_eq!(
            set.record(&b, NOW - 3600).unwrap_err(),
            JudgementRefusal::RequestReplayed
        );
    }

    #[test]
    fn latency_buckets_are_coarse() {
        assert_eq!(latency_bucket(0), "lt_1s");
        assert_eq!(latency_bucket(999), "lt_1s");
        assert_eq!(latency_bucket(1_000), "1s_5s");
        assert_eq!(latency_bucket(5_000), "5s_30s");
        assert_eq!(latency_bucket(30_000), "ge_30s");
    }

    /// A `MakeWriter` that appends every log line to a shared buffer.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// No log line carries the account, the account key, a label, a
    /// confidence, a prompt, the digest or the claim.
    #[test]
    fn no_log_line_carries_the_account_a_label_or_digest_text() {
        let _serial = LOG_LINE.blocking_lock();
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let h = harness();
        let claim = h.issuer.valid_claim();
        let request = all_three(claim.clone(), digest(5));
        let refused_request = all_three(claim.clone(), digest(5));
        let (judged, refused) = tracing::subscriber::with_default(subscriber, || {
            // Other tests hit the same callsite with no subscriber, and the
            // first hit caches its interest globally. Recompute it now that
            // this scope has a subscriber, or the line can be filtered out
            // when the tests run in parallel.
            tracing::callsite::rebuild_interest_cache();
            runtime.block_on(async {
                let judged = h
                    .service
                    .judge(&plaintext(&request), NOW, no_permit)
                    .await
                    .expect("judged");
                h.model.set_fault(Some(MockFault::WrongModelReceipt));
                let refused = h
                    .service
                    .judge(&plaintext(&refused_request), NOW, no_permit)
                    .await
                    .unwrap_err();
                (judged, refused)
            })
        });
        let logs = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("utf8");
        // The request line is there twice, with what it may carry, and the
        // wrong-model receipt latched the model, once.
        assert_eq!(
            logs.matches("judgement_request").count(),
            2,
            "captured: {logs:?}"
        );
        assert_eq!(logs.matches("judgement_model_latched").count(), 1);
        assert_eq!(logs.matches(LOG_TARGET).count(), 3, "captured: {logs:?}");
        assert!(logs.contains(&request.digest_sha256()));
        assert!(logs.contains(MOCK_MODEL_ID));
        assert!(logs.contains("outcome=ok") || logs.contains("outcome=\"ok\""));
        assert!(logs.contains(refused.label()));
        // And nothing else.
        let account_key = LimiterKeys::from_root(&ROOT).account_key(ACCOUNT);
        for forbidden in [
            ACCOUNT.to_string(),
            ACCOUNT["near-".len()..].to_string(),
            hex::encode(account_key.as_bytes()),
            CANARY.to_string(),
            request.request_nonce().to_hex(),
            claim.clone(),
            claim.split('.').nth(1).expect("payload").to_string(),
            crate::judgement_service::prompts::TASK_V1.text()[..40].to_string(),
        ] {
            assert!(
                !logs.contains(&forbidden),
                "a log line leaked {forbidden:?}"
            );
        }
        for result in &judged.results {
            let label = result.label().as_wire();
            for spelled in [format!("={label}"), format!("\"{label}\"")] {
                assert!(!logs.contains(&spelled), "a log line leaked label {label}");
            }
        }
        for confidence in ["high", "medium", "low", "self_reported", "probability"] {
            assert!(!logs.contains(confidence), "a log line leaked {confidence}");
        }
    }
}
