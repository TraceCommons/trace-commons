// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The receipt-checking call wrapper (spec sections 5 and 6): one judgement
//! call, from the production check to a [`JudgementResult`].
//!
//! The order is the design:
//!
//! 1. the production check: allowlisted, pinned, not latched, callable in
//!    the catalog now. A refusal here makes no call and charges nothing;
//! 2. build the exact request bytes, and **reserve** against the account
//!    and the operator total. A cap refusal makes no call;
//! 3. call the model, under a timeout. A call that fails or times out
//!    leaves the reservation as the charge;
//! 4. check the receipt ([`check_receipt`]) over the bytes held here and the
//!    bytes received;
//! 5. **settle**, only now that the receipt's outcome is known:
//!    - verified: to the usage in the verified body. A usage above the
//!      reservation is charged in full and latches the model, because the
//!      reservation's bound did not hold for it;
//!    - not verified: to `max(reservation, reported)`, because NEAR AI billed
//!      the call whatever the body says, so a swapped response claiming
//!      `usage {0, 0}` cannot lower spend. A rotation-shaped failure
//!      ([`ReceiptRefusal::latches`]) latches the model first. The label is
//!      discarded and the request refused with `judgement_model_unattested`;
//!    - no usage reported: the reservation stands;
//! 6. only then read the label out of the verified body
//!    ([`JudgementModel::parse_answer`]). An answer that does not parse, or
//!    names a value outside the choices, becomes the kind's fallback label
//!    at the fallback confidence; it was still receipt-checked and metered.
//!
//! On the failed path the wrapper reads only `usage` from the unverified
//! body, and only to raise the charge above the reservation, never to lower
//! it. It never reads an answer from a body whose receipt failed.
//!
//! Two log lines, label and model id only: `judgement_model_latched` when a
//! model is latched, and `judgement_usage_over_reservation` when a verified
//! usage passed its reservation.
//!
//! # The receipt check
//!
//! Body-free checks first, so a receipt of the wrong kind is named as that
//! rather than as a hash mismatch: `signing_algo` must be `ed25519` (an
//! ECDSA signer is not quote-bound), `signature_kind` must be
//! `provider_tee` (a gateway key is shared across every model), the text
//! must be three-part and its model part must be this model (a two-part
//! receipt binds no model), and the signer must be in this model's pin set.
//! Then `verify_receipt` checks the signature and both hashes over the exact
//! request bytes held here and the exact response bytes received.

use std::sync::Arc;
use std::time::Duration;

use trace_commons_attestation::receipt::{
    ReceiptAlgo, ReceiptError, ReceiptPayload, ReceiptSignatureKind, signer_is_attested_for_model,
    verify_receipt,
};
use trace_commons_protocol::insights_judgement::{
    ConfidenceBasis, ConfidenceBucket, JudgeVersion, JudgementConfidence, JudgementKind,
    JudgementLabel, JudgementReceipt, JudgementRefusal, JudgementResult,
};

use super::limiter::{
    AccountKey, ClaimTime, Limiter, Settled, charge_nanousd, reservation_nanousd,
};
use super::model::{DecisionCall, JudgementModel, ModelAnswer, ModelError};
use super::production::ProductionCheck;
use super::prompts::{FALLBACK_CONFIDENCE, bucket_for_probability, fallback_label, prompt_for};

/// The completion-token ceiling, defined with the prompts because it is
/// hashed into every `prompt_version`.
pub use super::prompts::JUDGEMENT_MAX_TOKENS;
/// How long one model call may take, receipt fetch included.
pub const JUDGEMENT_CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// The tracing target of the latch and over-reservation lines: the
/// service's one target.
const LOG_TARGET: &str = "trace_commons_judgement";

/// Why a receipt was refused. All map to `judgement_model_unattested`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptRefusal {
    #[error("judgement_receipt_absent")]
    Absent,
    #[error("judgement_receipt_not_ed25519")]
    NotEd25519,
    #[error("judgement_receipt_not_provider_tee")]
    NotProviderTee,
    #[error("judgement_receipt_binds_no_model")]
    BindsNoModel,
    #[error("judgement_receipt_wrong_model")]
    WrongModel,
    #[error("judgement_receipt_signer_unpinned")]
    SignerUnpinned,
    #[error("judgement_receipt_unverified")]
    Unverified(ReceiptError),
}

impl ReceiptRefusal {
    /// Whether this failure means the pins no longer match the model's
    /// signer, so the model is latched: an unpinned signer, a receipt that
    /// is not `provider_tee`, or a receipt for another model.
    pub fn latches(&self) -> bool {
        matches!(
            self,
            Self::SignerUnpinned | Self::NotProviderTee | Self::WrongModel
        )
    }
}

/// The charge a call settles to. `reported` is the usage's charge as the
/// body states it, `None` when it states none.
///
/// - receipt verified: `reported`, or the reservation when absent;
/// - receipt not verified: the larger of the reservation and `reported`.
///
/// Returns `None` when the reservation stands unchanged.
pub fn settle_charge(
    receipt_verified: bool,
    reservation: u64,
    reported: Option<u64>,
) -> Option<u64> {
    let reported = reported?;
    if receipt_verified {
        Some(reported)
    } else {
        Some(reported.max(reservation))
    }
}

/// Check one receipt for `model_id` over the exact exchange bytes, against
/// that model's pinned signers. Returns the evidence the response carries.
pub fn check_receipt(
    receipt: &ReceiptPayload,
    request_body: &[u8],
    response_body: &[u8],
    model_id: &str,
    pinned_signers: &[String],
) -> Result<JudgementReceipt, ReceiptRefusal> {
    if receipt.signing_algo != ReceiptAlgo::Ed25519 {
        return Err(ReceiptRefusal::NotEd25519);
    }
    if receipt.signature_kind != ReceiptSignatureKind::ProviderTee {
        return Err(ReceiptRefusal::NotProviderTee);
    }
    let parts: Vec<&str> = receipt.text.split(':').collect();
    match parts.as_slice() {
        [bound_model, _, _] if *bound_model == model_id => {}
        [_, _, _] => return Err(ReceiptRefusal::WrongModel),
        _ => return Err(ReceiptRefusal::BindsNoModel),
    }
    if !signer_is_attested_for_model(&receipt.signing_address, pinned_signers) {
        return Err(ReceiptRefusal::SignerUnpinned);
    }
    let verdict = verify_receipt(receipt, request_body, response_body, model_id)
        .map_err(ReceiptRefusal::Unverified)?;
    // Re-asserted on the verdict, which is what the signature established,
    // rather than on the pre-check's reading of the text.
    if verdict.model.as_deref() != Some(model_id) {
        return Err(ReceiptRefusal::BindsNoModel);
    }
    if verdict.signing_algo != ReceiptAlgo::Ed25519
        || verdict.signature_kind != ReceiptSignatureKind::ProviderTee
        || !signer_is_attested_for_model(&verdict.signing_address, pinned_signers)
    {
        return Err(ReceiptRefusal::SignerUnpinned);
    }
    JudgementReceipt::new(model_id.to_string(), verdict.signing_address)
        .map_err(|_| ReceiptRefusal::SignerUnpinned)
}

/// The label and confidence an answer gives for `kind`, with the fallback
/// rule for an answer that does not name one of the choices.
pub fn label_from_answer(
    kind: JudgementKind,
    answer: &ModelAnswer,
) -> (JudgementLabel, JudgementConfidence) {
    let choice = |label: &str| {
        kind.choices()
            .iter()
            .copied()
            .find(|c| c.as_wire() == label)
    };
    let fallback = |basis| {
        (
            fallback_label(kind),
            JudgementConfidence {
                bucket: FALLBACK_CONFIDENCE,
                basis,
            },
        )
    };
    match answer {
        ModelAnswer::SelfReported { label, confidence } => {
            let bucket = match confidence.as_str() {
                "high" => Some(ConfidenceBucket::High),
                "medium" => Some(ConfidenceBucket::Medium),
                "low" => Some(ConfidenceBucket::Low),
                _ => None,
            };
            match (choice(label), bucket) {
                (Some(label), Some(bucket)) => (
                    label,
                    JudgementConfidence {
                        bucket,
                        basis: ConfidenceBasis::SelfReported,
                    },
                ),
                _ => fallback(ConfidenceBasis::SelfReported),
            }
        }
        ModelAnswer::Probabilities(choices) => {
            let valid = choices
                .iter()
                .all(|(_, p)| p.is_finite() && (0.0..=1.0).contains(p));
            // The first of equal maxima, so the pick is deterministic.
            let top = choices
                .iter()
                .fold(None::<&(String, f64)>, |best, candidate| match best {
                    Some(best) if best.1 >= candidate.1 => Some(best),
                    _ => Some(candidate),
                });
            match top
                .filter(|_| valid)
                .and_then(|(label, p)| Some((choice(label)?, *p)))
            {
                Some((label, probability)) => (
                    label,
                    JudgementConfidence {
                        bucket: bucket_for_probability(probability),
                        basis: ConfidenceBasis::Probability,
                    },
                ),
                None => fallback(ConfidenceBasis::Probability),
            }
        }
        ModelAnswer::Unparseable { basis } => fallback(*basis),
    }
}

/// One model behind the production check and the limiter.
pub struct JudgementCaller {
    model: Arc<dyn JudgementModel>,
    check: Arc<ProductionCheck>,
    limiter: Arc<Limiter>,
    timeout: Duration,
}

impl std::fmt::Debug for JudgementCaller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JudgementCaller")
            .field("model", &self.model.model_id())
            .finish_non_exhaustive()
    }
}

impl JudgementCaller {
    pub fn new(
        model: Arc<dyn JudgementModel>,
        check: Arc<ProductionCheck>,
        limiter: Arc<Limiter>,
    ) -> Self {
        Self {
            model,
            check,
            limiter,
            timeout: JUDGEMENT_CALL_TIMEOUT,
        }
    }

    /// The same caller with another call timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn model_id(&self) -> &str {
        self.model.model_id()
    }

    pub fn limiter(&self) -> &Limiter {
        &self.limiter
    }

    /// Latch `model_id`, logging `judgement_model_latched` once, with the
    /// cause's label.
    fn latch(&self, model_id: &str, cause: &str) {
        if self.check.latch(model_id) {
            tracing::warn!(
                target: LOG_TARGET,
                model_id = %model_id,
                cause = %cause,
                "judgement_model_latched"
            );
        }
    }

    /// One judgement over `digest` for `account`, under the verified claim's
    /// `iat` and the service clock.
    pub async fn judge(
        &self,
        account: &AccountKey,
        digest: &str,
        kind: JudgementKind,
        event_index: Option<u32>,
        time: ClaimTime,
    ) -> Result<JudgementResult, JudgementRefusal> {
        let model_id = self.model.model_id();
        // 1. The production check: no call and no charge on a refusal.
        let prices = self.check.call_check(model_id, time.now)?;
        let pins = self
            .check
            .model_pins(model_id)
            .ok_or(JudgementRefusal::ModelUnattested)?;

        // 2. The exact request bytes, and the reservation over them.
        let call = DecisionCall {
            prompt: prompt_for(kind),
            digest,
            event_index,
            max_tokens: JUDGEMENT_MAX_TOKENS,
        };
        let request_body = self.model.request_body(&call);
        let amount = reservation_nanousd(request_body.len(), call.max_tokens, prices)
            .ok_or(JudgementRefusal::AccountBudgetExhausted)?;
        let reservation = self.limiter.reserve(account, amount, time)?;

        // 3. The call. On failure the reservation is dropped unsettled and
        //    stands as the charge: NEAR AI may have billed it.
        let raw = match tokio::time::timeout(self.timeout, self.model.decide(&call, &request_body))
            .await
        {
            Ok(Ok(raw)) => raw,
            Ok(Err(ModelError::Timeout)) | Err(_) => return Err(JudgementRefusal::Timeout),
            Ok(Err(ModelError::Failed)) => return Err(JudgementRefusal::ModelUnattested),
        };

        // 4. The receipt, over the bytes held here and the bytes received.
        let verdict = match raw.receipt.as_ref() {
            Some(receipt) => {
                check_receipt(receipt, &request_body, &raw.response_body, model_id, pins)
            }
            None => Err(ReceiptRefusal::Absent),
        };
        let parsed = self.model.parse_answer(&call, &raw.response_body);
        // An overflowing charge errs high.
        let reported = parsed
            .usage
            .map(|usage| charge_nanousd(usage, prices).unwrap_or(u64::MAX));

        // 5. Settle, now that the receipt's outcome is known.
        let evidence = match verdict {
            Err(refusal) => {
                if refusal.latches() {
                    self.latch(model_id, &refusal.to_string());
                }
                let charge = settle_charge(false, reservation.amount_nanousd(), reported);
                self.limiter.settle(reservation, charge)?;
                return Err(JudgementRefusal::ModelUnattested);
            }
            Ok(evidence) => evidence,
        };
        let charge = settle_charge(true, reservation.amount_nanousd(), reported);
        if self.limiter.settle(reservation, charge)? == Settled::OverReservation {
            tracing::warn!(
                target: LOG_TARGET,
                model_id = %model_id,
                "judgement_usage_over_reservation"
            );
            self.latch(model_id, "judgement_usage_over_reservation");
        }

        // 6. Only now, the label, from the verified body.
        let (label, confidence) = label_from_answer(kind, &parsed.answer);
        let judge_version = JudgeVersion::new(call.prompt.prompt_version(), model_id.to_string())
            .map_err(|_| JudgementRefusal::ModelUnattested)?;
        JudgementResult::judged(
            kind,
            event_index,
            label,
            confidence,
            judge_version,
            Some(evidence),
        )
        .map_err(|_| JudgementRefusal::RequestMalformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judgement_service::catalog::{CatalogEntry, TokenPrices};
    use crate::judgement_service::ledger::LedgerState;
    use crate::judgement_service::limiter::tests::{MemoryStore, NOW};
    use crate::judgement_service::limiter::{LimiterCaps, LimiterKeys};
    use crate::judgement_service::mock::{
        MOCK_MODEL_ID, MockFault, MockJudgementModel, MockReceiptSigner,
    };
    use crate::judgement_service::model::TokenUsage;
    use std::collections::BTreeMap;
    use trace_commons_protocol::insights_judgement::JudgementReceiptKind;

    const LIVE_RECEIPT: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_receipt_provider_tee_ed25519.json"
    );
    const LIVE_GATEWAY_RECEIPT: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_receipt_gateway_ed25519.json"
    );
    const LIVE_ECDSA_TRIPLE: &str = include_str!("../../tests/fixtures/near_ai_live_triple.json");
    const LIVE_MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";
    const LIVE_SIGNER: &str = "aba45f0b8f90869baab26db02e8b01354bb8f8730769c60650cb7a635da602d4";

    fn live_receipt(json: &str) -> ReceiptPayload {
        let v: serde_json::Value = serde_json::from_str(json).expect("fixture is JSON");
        let s = |k: &str| v[k].as_str().expect("a captured field").to_string();
        ReceiptPayload {
            text: s("text"),
            signature: s("signature"),
            signing_address: s("signing_address"),
            signing_algo: ReceiptAlgo::from_wire(&s("signing_algo")).expect("a known algo"),
            signature_kind: ReceiptSignatureKind::from_wire(
                v["signature_kind"].as_str().unwrap_or(""),
            ),
        }
    }

    // -- the receipt check, against captured live receipts ------------------

    #[test]
    fn a_live_provider_tee_receipt_passes_every_body_free_check() {
        // The capture carries no bodies, so the check reaches the hash
        // comparison and stops there: every check before it passed.
        let receipt = live_receipt(LIVE_RECEIPT);
        assert_eq!(
            check_receipt(
                &receipt,
                b"{}",
                b"{}",
                LIVE_MODEL,
                &[LIVE_SIGNER.to_string()]
            )
            .unwrap_err(),
            ReceiptRefusal::Unverified(ReceiptError::RequestHashMismatch)
        );
    }

    #[test]
    fn a_live_gateway_receipt_is_refused_as_binding_no_model() {
        let receipt = live_receipt(LIVE_GATEWAY_RECEIPT);
        let signer = receipt.signing_address.clone();
        assert_eq!(
            check_receipt(&receipt, b"{}", b"{}", LIVE_MODEL, &[signer]).unwrap_err(),
            ReceiptRefusal::NotProviderTee
        );
    }

    #[test]
    fn a_live_ecdsa_receipt_is_refused_as_not_quote_bound() {
        let v: serde_json::Value = serde_json::from_str(LIVE_ECDSA_TRIPLE).expect("JSON");
        let r = &v["receipt"];
        let receipt = ReceiptPayload {
            text: r["text"].as_str().expect("text").to_string(),
            signature: r["signature"].as_str().expect("sig").to_string(),
            signing_address: r["signing_address"].as_str().expect("addr").to_string(),
            signing_algo: ReceiptAlgo::Ecdsa,
            signature_kind: ReceiptSignatureKind::ProviderTee,
        };
        assert_eq!(
            check_receipt(
                &receipt,
                b"{}",
                b"{}",
                "Qwen/Qwen3.6-27B-FP8",
                std::slice::from_ref(&receipt.signing_address)
            )
            .unwrap_err(),
            ReceiptRefusal::NotEd25519
        );
    }

    #[test]
    fn a_live_receipt_for_another_model_is_refused() {
        let receipt = live_receipt(LIVE_RECEIPT);
        assert_eq!(
            check_receipt(
                &receipt,
                b"{}",
                b"{}",
                "Qwen/Qwen3.8-27B",
                &[LIVE_SIGNER.to_string()]
            )
            .unwrap_err(),
            ReceiptRefusal::WrongModel
        );
    }

    #[test]
    fn a_live_receipt_from_an_unpinned_signer_is_refused() {
        let receipt = live_receipt(LIVE_RECEIPT);
        assert_eq!(
            check_receipt(&receipt, b"{}", b"{}", LIVE_MODEL, &["cb".repeat(32)]).unwrap_err(),
            ReceiptRefusal::SignerUnpinned
        );
        assert_eq!(
            check_receipt(&receipt, b"{}", b"{}", LIVE_MODEL, &[]).unwrap_err(),
            ReceiptRefusal::SignerUnpinned
        );
    }

    #[test]
    fn a_receipt_over_the_exact_bytes_verifies_and_yields_the_evidence() {
        let signer = MockReceiptSigner::generate();
        let request = b"{\"model\":\"m\"}";
        let response = b"{\"id\":\"c\"}";
        let text = format!(
            "{MOCK_MODEL_ID}:{}:{}",
            hex::encode(sha2::Sha256::digest(request)),
            hex::encode(sha2::Sha256::digest(response))
        );
        use sha2::Digest as _;
        let receipt = signer.receipt(text, ReceiptSignatureKind::ProviderTee);
        let evidence = check_receipt(
            &receipt,
            request,
            response,
            MOCK_MODEL_ID,
            &[signer.public_hex()],
        )
        .expect("verifies");
        assert_eq!(evidence.signature_kind(), JudgementReceiptKind::ProviderTee);
        assert_eq!(evidence.model(), MOCK_MODEL_ID);
        assert_eq!(evidence.signer(), signer.public_hex());
        // One byte of either body changed: refused.
        assert!(matches!(
            check_receipt(
                &receipt,
                b"{\"model\":\"n\"}",
                response,
                MOCK_MODEL_ID,
                &[signer.public_hex()]
            ),
            Err(ReceiptRefusal::Unverified(
                ReceiptError::RequestHashMismatch
            ))
        ));
        assert!(matches!(
            check_receipt(
                &receipt,
                request,
                b"{\"id\":\"d\"}",
                MOCK_MODEL_ID,
                &[signer.public_hex()]
            ),
            Err(ReceiptRefusal::Unverified(
                ReceiptError::ResponseHashMismatch
            ))
        ));
    }

    #[test]
    fn a_failed_receipt_can_raise_the_charge_but_never_lower_it() {
        // Verified: the verified usage, up or down.
        assert_eq!(settle_charge(true, 100, Some(30)), Some(30));
        assert_eq!(settle_charge(true, 100, Some(300)), Some(300));
        // Not verified: never below the reservation.
        assert_eq!(settle_charge(false, 100, Some(0)), Some(100));
        assert_eq!(settle_charge(false, 100, Some(30)), Some(100));
        assert_eq!(settle_charge(false, 100, Some(300)), Some(300));
        // No usage: the reservation stands either way.
        assert_eq!(settle_charge(true, 100, None), None);
        assert_eq!(settle_charge(false, 100, None), None);
    }

    #[test]
    fn only_a_rotation_shaped_failure_latches() {
        for (refusal, latches) in [
            (ReceiptRefusal::SignerUnpinned, true),
            (ReceiptRefusal::NotProviderTee, true),
            (ReceiptRefusal::WrongModel, true),
            (ReceiptRefusal::Absent, false),
            (ReceiptRefusal::NotEd25519, false),
            (ReceiptRefusal::BindsNoModel, false),
            (
                ReceiptRefusal::Unverified(ReceiptError::ResponseHashMismatch),
                false,
            ),
        ] {
            assert_eq!(refusal.latches(), latches, "{refusal:?}");
        }
    }

    // -- answers ------------------------------------------------------------

    #[test]
    fn a_self_reported_answer_inside_the_choices_is_kept() {
        let (label, confidence) = label_from_answer(
            JudgementKind::Task,
            &ModelAnswer::SelfReported {
                label: "bug_fix".into(),
                confidence: "medium".into(),
            },
        );
        assert_eq!(label, JudgementLabel::BugFix);
        assert_eq!(
            confidence,
            JudgementConfidence {
                bucket: ConfidenceBucket::Medium,
                basis: ConfidenceBasis::SelfReported
            }
        );
    }

    #[test]
    fn an_answer_outside_the_choices_falls_back_at_low_confidence() {
        for (kind, label, fallback) in [
            (JudgementKind::Task, "yes", JudgementLabel::Other),
            (JudgementKind::Finished, "bug_fix", JudgementLabel::Unclear),
            (
                JudgementKind::EventKind,
                "unjudged",
                JudgementLabel::Unclear,
            ),
            (
                JudgementKind::EventKind,
                "Rejection",
                JudgementLabel::Unclear,
            ),
        ] {
            let (got, confidence) = label_from_answer(
                kind,
                &ModelAnswer::SelfReported {
                    label: label.into(),
                    confidence: "high".into(),
                },
            );
            assert_eq!(got, fallback, "{label}");
            assert_eq!(confidence.bucket, ConfidenceBucket::Low);
        }
        // A confidence outside the set falls back too.
        let (got, confidence) = label_from_answer(
            JudgementKind::Finished,
            &ModelAnswer::SelfReported {
                label: "yes".into(),
                confidence: "certain".into(),
            },
        );
        assert_eq!(got, JudgementLabel::Unclear);
        assert_eq!(confidence.bucket, ConfidenceBucket::Low);
        let (got, confidence) = label_from_answer(
            JudgementKind::Task,
            &ModelAnswer::Unparseable {
                basis: ConfidenceBasis::Probability,
            },
        );
        assert_eq!(got, JudgementLabel::Other);
        assert_eq!(
            confidence,
            JudgementConfidence {
                bucket: ConfidenceBucket::Low,
                basis: ConfidenceBasis::Probability
            }
        );
    }

    #[test]
    fn probabilities_pick_the_top_choice_and_bucket_it() {
        let (label, confidence) = label_from_answer(
            JudgementKind::Finished,
            &ModelAnswer::Probabilities(vec![
                ("yes".into(), 0.15),
                ("no".into(), 0.7),
                ("unclear".into(), 0.15),
            ]),
        );
        assert_eq!(label, JudgementLabel::No);
        assert_eq!(
            confidence,
            JudgementConfidence {
                bucket: ConfidenceBucket::Medium,
                basis: ConfidenceBasis::Probability
            }
        );
        // A top choice outside the set, or a value that is not a
        // probability, falls back.
        for choices in [
            vec![("maybe".to_string(), 0.9), ("yes".to_string(), 0.1)],
            vec![("yes".to_string(), f64::NAN)],
            vec![("yes".to_string(), 1.5)],
            vec![],
        ] {
            let (label, confidence) = label_from_answer(
                JudgementKind::Finished,
                &ModelAnswer::Probabilities(choices),
            );
            assert_eq!(label, JudgementLabel::Unclear);
            assert_eq!(confidence.bucket, ConfidenceBucket::Low);
        }
    }

    // -- the call wrapper, end to end over the mock ---------------------------

    const DIGEST: &str = r#"{"schema_version":1,"events":[]}"#;

    struct Harness {
        model: Arc<MockJudgementModel>,
        store: Arc<MemoryStore>,
        caller: JudgementCaller,
        account: AccountKey,
    }

    fn harness_with(pin: Option<String>, account_cap_usd: u64) -> Harness {
        let model = Arc::new(MockJudgementModel::new(MockReceiptSigner::generate()));
        let pin = pin.unwrap_or_else(|| model.signer_public_hex());
        let mut pins = BTreeMap::new();
        pins.insert(MOCK_MODEL_ID.to_string(), vec![pin]);
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
        let store = Arc::new(MemoryStore::default());
        let limiter = Arc::new(Limiter::new(
            LimiterCaps::from_usd(account_cap_usd, 100).expect("caps"),
            LedgerState::default(),
            store.clone(),
        ));
        let caller = JudgementCaller::new(model.clone(), Arc::new(check), limiter);
        Harness {
            model,
            store,
            caller,
            account: LimiterKeys::from_root(&[3u8; 32]).account_key("near-a"),
        }
    }

    fn harness() -> Harness {
        harness_with(None, 5)
    }

    #[tokio::test]
    async fn a_good_call_returns_a_receipted_versioned_label_and_settles_usage() {
        let h = harness();
        let result = h
            .caller
            .judge(
                &h.account,
                DIGEST,
                JudgementKind::Task,
                None,
                ClaimTime::at(NOW),
            )
            .await
            .expect("judged");
        let call = DecisionCall {
            prompt: prompt_for(JudgementKind::Task),
            digest: DIGEST,
            event_index: None,
            max_tokens: JUDGEMENT_MAX_TOKENS,
        };
        let (label, confidence) = MockJudgementModel::expected_answer(&call);
        assert_eq!(result.label().as_wire(), label);
        let expected_bucket = match confidence {
            "high" => ConfidenceBucket::High,
            "medium" => ConfidenceBucket::Medium,
            _ => ConfidenceBucket::Low,
        };
        assert_eq!(
            result.confidence(),
            Some(JudgementConfidence {
                bucket: expected_bucket,
                basis: ConfidenceBasis::SelfReported
            })
        );
        let version = result.judge_version().expect("judged");
        assert_eq!(version.model_id(), MOCK_MODEL_ID);
        assert_eq!(
            version.prompt_version(),
            prompt_for(JudgementKind::Task).prompt_version()
        );
        let receipt = result.receipt().expect("receipted");
        assert_eq!(receipt.signer(), h.model.signer_public_hex());
        // Settled to the reported usage, not the reservation.
        let request_len = h.model.request_body(&call).len() as u64;
        let charge = request_len.div_ceil(3) * 40 + 12 * 1350;
        assert_eq!(
            h.caller.limiter().account_spent_nanousd(&h.account, NOW),
            charge
        );
        assert_eq!(h.store.writes.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn every_receipt_fault_is_refused_but_still_metered() {
        for fault in [
            MockFault::NoReceipt,
            MockFault::TwoPartReceipt,
            MockFault::GatewayReceipt,
            MockFault::WrongModelReceipt,
            MockFault::ResponseSwapped,
        ] {
            let h = harness();
            h.model.set_fault(Some(fault));
            assert_eq!(
                h.caller
                    .judge(
                        &h.account,
                        DIGEST,
                        JudgementKind::Finished,
                        None,
                        ClaimTime::at(NOW)
                    )
                    .await
                    .unwrap_err(),
                JudgementRefusal::ModelUnattested,
                "{fault:?}"
            );
            assert_eq!(h.model.calls(), 1);
            assert!(
                h.caller.limiter().account_spent_nanousd(&h.account, NOW) > 0,
                "{fault:?} must still be metered"
            );
        }
    }

    #[tokio::test]
    async fn an_unpinned_signer_is_refused_but_still_metered() {
        let h = harness_with(Some("cb".repeat(32)), 5);
        assert_eq!(
            h.caller
                .judge(
                    &h.account,
                    DIGEST,
                    JudgementKind::Task,
                    None,
                    ClaimTime::at(NOW)
                )
                .await
                .unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
        let spent = h.caller.limiter().account_spent_nanousd(&h.account, NOW);
        assert!(spent > 0);
        assert_eq!(h.model.calls(), 1);
        // The model is latched: the next request makes no call and is
        // charged nothing.
        assert_eq!(
            h.caller
                .judge(
                    &h.account,
                    DIGEST,
                    JudgementKind::Task,
                    None,
                    ClaimTime::at(NOW)
                )
                .await
                .unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
        assert_eq!(h.model.calls(), 1);
        assert_eq!(
            h.caller.limiter().account_spent_nanousd(&h.account, NOW),
            spent
        );
    }

    async fn judge_task(h: &Harness) -> Result<JudgementResult, JudgementRefusal> {
        h.caller
            .judge(
                &h.account,
                DIGEST,
                JudgementKind::Task,
                None,
                ClaimTime::at(NOW),
            )
            .await
    }

    fn task_call() -> DecisionCall<'static> {
        DecisionCall {
            prompt: prompt_for(JudgementKind::Task),
            digest: DIGEST,
            event_index: None,
            max_tokens: JUDGEMENT_MAX_TOKENS,
        }
    }

    fn task_reservation(h: &Harness) -> u64 {
        reservation_nanousd(
            h.model.request_body(&task_call()).len(),
            JUDGEMENT_MAX_TOKENS,
            PRICES,
        )
        .expect("no overflow")
    }

    const PRICES: TokenPrices = TokenPrices {
        input_nanousd: 40,
        output_nanousd: 1350,
    };

    #[tokio::test]
    async fn every_rotation_shaped_receipt_failure_latches_the_model() {
        for fault in [MockFault::GatewayReceipt, MockFault::WrongModelReceipt] {
            let h = harness();
            h.model.set_fault(Some(fault));
            assert_eq!(
                judge_task(&h).await.unwrap_err(),
                JudgementRefusal::ModelUnattested
            );
            h.model.set_fault(None);
            // Even a good response is not asked for now.
            assert_eq!(
                judge_task(&h).await.unwrap_err(),
                JudgementRefusal::ModelUnattested,
                "{fault:?}"
            );
            assert_eq!(h.model.calls(), 1, "{fault:?}");
        }
        // A failure that does not mean the pins moved does not latch.
        for fault in [
            MockFault::NoReceipt,
            MockFault::TwoPartReceipt,
            MockFault::ResponseSwapped,
        ] {
            let h = harness();
            h.model.set_fault(Some(fault));
            judge_task(&h).await.unwrap_err();
            h.model.set_fault(None);
            judge_task(&h).await.expect("not latched");
            assert_eq!(h.model.calls(), 2, "{fault:?}");
        }
    }

    #[tokio::test]
    async fn the_label_follows_the_receipted_body_not_the_adapters_answer() {
        let h = harness();
        h.model.set_fault(Some(MockFault::BodyLabelDiffers));
        let result = judge_task(&h).await.expect("judged");
        let call = task_call();
        let (expected, _) = MockJudgementModel::expected_answer(&call);
        let in_body = MockJudgementModel::other_choice(&call, expected);
        assert_ne!(expected, in_body);
        assert_eq!(result.label().as_wire(), in_body);
    }

    #[tokio::test]
    async fn a_swapped_response_claiming_zero_usage_still_charges_the_reservation() {
        let h = harness();
        h.model.set_fault(Some(MockFault::ResponseSwappedZeroUsage));
        assert_eq!(
            judge_task(&h).await.unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
        assert_eq!(
            h.caller.limiter().account_spent_nanousd(&h.account, NOW),
            task_reservation(&h)
        );
        assert_eq!(
            h.caller.limiter().operator_spent_nanousd(NOW),
            task_reservation(&h)
        );
    }

    #[tokio::test]
    async fn verified_usage_above_the_reservation_is_charged_and_latches_the_model() {
        let h = harness_with(None, 100);
        h.model.set_fault(Some(MockFault::UsageAboveReservation));
        // The label stands: the receipt verified.
        judge_task(&h).await.expect("judged");
        let charged = charge_nanousd(
            TokenUsage {
                prompt_tokens: crate::judgement_service::mock::MOCK_OVER_RESERVATION_TOKENS,
                completion_tokens: crate::judgement_service::mock::MOCK_COMPLETION_TOKENS,
            },
            PRICES,
        )
        .expect("no overflow");
        assert!(charged > task_reservation(&h));
        assert_eq!(
            h.caller.limiter().account_spent_nanousd(&h.account, NOW),
            charged
        );
        h.model.set_fault(None);
        assert_eq!(
            judge_task(&h).await.unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
        assert_eq!(h.model.calls(), 1);
    }

    #[tokio::test]
    async fn the_call_timeout_ends_a_call_that_never_returns() {
        let model = harness();
        let h = Harness {
            caller: model.caller.with_timeout(Duration::from_millis(20)),
            ..model
        };
        h.model.set_fault(Some(MockFault::Hang));
        assert_eq!(judge_task(&h).await.unwrap_err(), JudgementRefusal::Timeout);
        // The reservation stands as the charge: NEAR AI may have billed it.
        assert_eq!(
            h.caller.limiter().account_spent_nanousd(&h.account, NOW),
            task_reservation(&h)
        );
    }

    #[tokio::test]
    async fn absent_usage_charges_the_reservation() {
        let h = harness();
        h.model.set_fault(Some(MockFault::NoUsage));
        h.caller
            .judge(
                &h.account,
                DIGEST,
                JudgementKind::Task,
                None,
                ClaimTime::at(NOW),
            )
            .await
            .expect("judged");
        let call = DecisionCall {
            prompt: prompt_for(JudgementKind::Task),
            digest: DIGEST,
            event_index: None,
            max_tokens: JUDGEMENT_MAX_TOKENS,
        };
        let request_len = h.model.request_body(&call).len();
        assert_eq!(
            h.caller.limiter().account_spent_nanousd(&h.account, NOW),
            reservation_nanousd(
                request_len,
                JUDGEMENT_MAX_TOKENS,
                TokenPrices {
                    input_nanousd: 40,
                    output_nanousd: 1350
                }
            )
            .expect("no overflow")
        );
    }

    #[tokio::test]
    async fn an_unparseable_answer_is_receipted_and_falls_back() {
        let h = harness();
        h.model.set_fault(Some(MockFault::Unparseable));
        let result = h
            .caller
            .judge(
                &h.account,
                DIGEST,
                JudgementKind::Task,
                None,
                ClaimTime::at(NOW),
            )
            .await
            .expect("judged");
        assert_eq!(result.label(), JudgementLabel::Other);
        assert_eq!(
            result.confidence().expect("judged").bucket,
            ConfidenceBucket::Low
        );
        assert!(result.is_attested());
        h.model.set_fault(Some(MockFault::LabelOutsideChoices));
        let result = h
            .caller
            .judge(
                &h.account,
                DIGEST,
                JudgementKind::EventKind,
                Some(0),
                ClaimTime::at(NOW),
            )
            .await
            .expect("judged");
        assert_eq!(result.label(), JudgementLabel::Unclear);
        assert_eq!(result.event_index(), Some(0));
    }

    #[tokio::test]
    async fn a_timeout_or_failure_refuses_and_leaves_the_reservation_charged() {
        for (fault, refusal) in [
            (MockFault::Timeout, JudgementRefusal::Timeout),
            (MockFault::Failed, JudgementRefusal::ModelUnattested),
        ] {
            let h = harness();
            h.model.set_fault(Some(fault));
            assert_eq!(
                h.caller
                    .judge(
                        &h.account,
                        DIGEST,
                        JudgementKind::Task,
                        None,
                        ClaimTime::at(NOW)
                    )
                    .await
                    .unwrap_err(),
                refusal
            );
            assert!(h.caller.limiter().account_spent_nanousd(&h.account, NOW) > 0);
        }
    }

    #[tokio::test]
    async fn a_budget_refusal_makes_no_call() {
        let h = harness_with(None, 0);
        assert_eq!(
            h.caller
                .judge(
                    &h.account,
                    DIGEST,
                    JudgementKind::Task,
                    None,
                    ClaimTime::at(NOW)
                )
                .await
                .unwrap_err(),
            JudgementRefusal::AccountBudgetExhausted
        );
        assert_eq!(h.model.calls(), 0);
    }

    #[tokio::test]
    async fn a_failed_production_check_makes_no_call_and_charges_nothing() {
        let h = harness();
        // The catalog has gone stale.
        let later = NOW + crate::judgement_service::catalog::CATALOG_STALE_AFTER_SECS + 1;
        assert_eq!(
            h.caller
                .judge(
                    &h.account,
                    DIGEST,
                    JudgementKind::Task,
                    None,
                    ClaimTime::at(later)
                )
                .await
                .unwrap_err(),
            JudgementRefusal::ModelUnattested
        );
        assert_eq!(h.model.calls(), 0);
        assert_eq!(
            h.caller.limiter().account_spent_nanousd(&h.account, later),
            0
        );
    }

    #[tokio::test]
    async fn a_ledger_that_cannot_be_written_refuses_before_the_call() {
        let h = harness();
        h.store
            .fail
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            h.caller
                .judge(
                    &h.account,
                    DIGEST,
                    JudgementKind::Task,
                    None,
                    ClaimTime::at(NOW)
                )
                .await
                .unwrap_err(),
            JudgementRefusal::Saturated
        );
        assert_eq!(h.model.calls(), 0);
    }
}
