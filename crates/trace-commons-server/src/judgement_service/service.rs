// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One judgement request end to end, without a socket.
//!
//! [`JudgementService::judge`] takes the bearer claim and a parsed request
//! and returns the unsigned results and the spend bucket. The HTTP surface,
//! the nonce header and the response signature belong to the binary (slice
//! 5), which wraps this.
//!
//! Order:
//!
//! 1. verify the claim offline; derive the keyed account digest;
//! 2. count the digest's marked events (a digest that is not a version-1
//!    digest is refused before any model call);
//! 3. run each requested judgement in request order: `task` and `finished`
//!    once, `event_kind` once per marked event for the first
//!    [`JUDGEMENT_EVENT_KIND_MAX`], and `unjudged` for the rest, so one
//!    session costs at most six model calls;
//! 4. any refusal discards every label of the request.
//!
//! One log line per request, hash-only: a random request id,
//! `digest_sha256`, the model id, `ok` or the refusal label, and a latency
//! bucket. Nothing else.

use std::time::Instant;

use trace_commons_protocol::insights_judgement::{
    JUDGEMENT_EVENT_KIND_MAX, JudgementKind, JudgementRefusal, JudgementRequest, JudgementResult,
    JudgementSpend,
};

use super::claim::{IssuerKeys, verify_judgement_claim};
use super::digest::marked_event_count;
use super::limiter::LimiterKeys;
use super::receipt_check::JudgementCaller;

/// The tracing target of the request log line.
pub const LOG_TARGET: &str = "trace_commons_judgement";

/// The unsigned outcome of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedDigest {
    pub digest_sha256: String,
    pub results: Vec<JudgementResult>,
    pub spend: JudgementSpend,
}

/// The service: the claim verifier, the limiter keys, and the caller.
pub struct JudgementService {
    issuer: IssuerKeys,
    limiter_keys: LimiterKeys,
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
            caller,
        }
    }

    /// Judge one request for the holder of `bearer` at `now`.
    pub async fn judge(
        &self,
        bearer: &str,
        request: &JudgementRequest,
        now: i64,
    ) -> Result<JudgedDigest, JudgementRefusal> {
        let started = Instant::now();
        let request_id = uuid::Uuid::new_v4();
        let digest_sha256 = request.digest_sha256();
        let outcome = self.judge_inner(bearer, request, &digest_sha256, now).await;
        tracing::info!(
            target: LOG_TARGET,
            request_id = %request_id.simple(),
            digest_sha256 = %digest_sha256,
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

    async fn judge_inner(
        &self,
        bearer: &str,
        request: &JudgementRequest,
        digest_sha256: &str,
        now: i64,
    ) -> Result<JudgedDigest, JudgementRefusal> {
        let account = verify_judgement_claim(&self.issuer, bearer, now)?;
        let account_key = self.limiter_keys.account_key(account.account_ref());
        // The raw account reference is not needed past this point.
        drop(account);
        let events = marked_event_count(request.digest())?;

        let mut results = Vec::new();
        for &kind in request.judgements() {
            match kind {
                JudgementKind::Task | JudgementKind::Finished => {
                    results.push(
                        self.caller
                            .judge(&account_key, request.digest(), kind, None, now)
                            .await?,
                    );
                }
                JudgementKind::EventKind => {
                    for index in 0..events {
                        let result = if index < JUDGEMENT_EVENT_KIND_MAX {
                            self.caller
                                .judge(&account_key, request.digest(), kind, Some(index), now)
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
        if results.is_empty() {
            // Only `event_kind` was asked for, over a digest with no marks:
            // there is nothing to judge and nothing to sign.
            return Err(JudgementRefusal::RequestMalformed);
        }
        Ok(JudgedDigest {
            digest_sha256: digest_sha256.to_string(),
            results,
            spend: JudgementSpend {
                account_month_remaining_bucket: self
                    .caller
                    .limiter()
                    .remaining_bucket(&account_key, now),
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
    use std::sync::{Arc, Mutex};
    use trace_commons_protocol::insights_judgement::{JudgementLabel, SpendBucket};

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

    fn all_three(digest: String) -> JudgementRequest {
        JudgementRequest::new(
            digest,
            vec![
                JudgementKind::Task,
                JudgementKind::EventKind,
                JudgementKind::Finished,
            ],
        )
        .expect("request")
    }

    #[tokio::test]
    async fn a_request_judges_each_judgement_in_order_with_receipts() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request = all_three(digest(2));
        let judged = h
            .service
            .judge(&h.issuer.valid_claim(), &request, NOW)
            .await
            .expect("judged");
        assert_eq!(judged.digest_sha256, request.digest_sha256());
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

    #[tokio::test]
    async fn event_kind_is_judged_for_at_most_four_marks() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request = all_three(digest(6));
        let judged = h
            .service
            .judge(&h.issuer.valid_claim(), &request, NOW)
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
        assert_eq!(
            h.service
                .judge(&other.valid_claim(), &all_three(digest(1)), NOW)
                .await
                .unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
        let pooled = h.issuer.claim_for("devfolio-pool", NOW, NOW + 60);
        assert_eq!(
            h.service
                .judge(&pooled, &all_three(digest(1)), NOW)
                .await
                .unwrap_err(),
            JudgementRefusal::AccountIneligible
        );
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn a_malformed_digest_makes_no_call() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request = JudgementRequest::new("not a digest".to_string(), vec![JudgementKind::Task])
            .expect("request");
        assert_eq!(
            h.service
                .judge(&h.issuer.valid_claim(), &request, NOW)
                .await
                .unwrap_err(),
            JudgementRefusal::RequestMalformed
        );
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn a_request_that_would_judge_nothing_is_malformed() {
        let _serial = LOG_LINE.lock().await;
        let h = harness();
        let request =
            JudgementRequest::new(digest(0), vec![JudgementKind::EventKind]).expect("request");
        assert_eq!(
            h.service
                .judge(&h.issuer.valid_claim(), &request, NOW)
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
        assert_eq!(
            h.service
                .judge(&h.issuer.valid_claim(), &all_three(digest(3)), NOW)
                .await
                .unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
        // It stopped at the first failure rather than spending more.
        assert_eq!(h.model.calls(), 1);
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
        let request = all_three(digest(5));
        let (judged, refused) = tracing::subscriber::with_default(subscriber, || {
            // Other tests hit the same callsite with no subscriber, and the
            // first hit caches its interest globally. Recompute it now that
            // this scope has a subscriber, or the line can be filtered out
            // when the tests run in parallel.
            tracing::callsite::rebuild_interest_cache();
            runtime.block_on(async {
                let judged = h
                    .service
                    .judge(&claim, &request, NOW)
                    .await
                    .expect("judged");
                h.model.set_fault(Some(MockFault::WrongModelReceipt));
                let refused = h.service.judge(&claim, &request, NOW).await.unwrap_err();
                (judged, refused)
            })
        });
        let logs = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("utf8");
        // The line is there, with what it may carry.
        assert_eq!(logs.matches(LOG_TARGET).count(), 2, "captured: {logs:?}");
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
