// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `MockJudgementModel`: a deterministic, hash-derived test double (spec
//! sections 5 and 10).
//!
//! Like the gate-enclave `Mock*` types it is derived from its input by
//! hashing and means nothing. It must never gate anything and is never on
//! the production allowlist ([`super::production::DEVELOPMENT_ONLY_MODEL_IDS`]
//! names it). This module compiles only under `cfg(test)` or the
//! `judgement-development-models` feature, which the production image is
//! built without.
//!
//! It pairs with [`MockReceiptSigner`], a fresh Ed25519 key that signs a
//! real three-part `provider_tee` receipt (`model:sha256(request):
//! sha256(response)`) over the exact bytes of each exchange. With that key
//! pinned in the test configuration, the whole request path, receipt check
//! included, runs in `cargo test` without NEAR AI.
//!
//! [`MockFault`] makes it misbehave in the ways the receipt check and the
//! limiter must refuse.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};
use trace_commons_attestation::receipt::{ReceiptAlgo, ReceiptPayload, ReceiptSignatureKind};
use trace_commons_protocol::insights_judgement::ConfidenceBasis;

use super::model::{
    DecisionCall, JudgementModel, ModelAnswer, ModelError, ParsedAnswer, RawDecision, TokenUsage,
};
use super::prompts::JUDGEMENT_TEMPERATURE;

/// The mock's model id.
pub const MOCK_MODEL_ID: &str = "trace-commons/mock-judgement-model";
/// Completion tokens the mock reports for every call.
pub const MOCK_COMPLETION_TOKENS: u64 = 12;
/// Prompt tokens the mock reports under [`MockFault::UsageAboveReservation`]:
/// more than any judgement request's reservation allows.
pub const MOCK_OVER_RESERVATION_TOKENS: u64 = 1_000_000;

/// A test-only Ed25519 receipt signer. Generated per instance; never used by
/// a deployment.
pub struct MockReceiptSigner {
    pair: Ed25519KeyPair,
}

impl std::fmt::Debug for MockReceiptSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MockReceiptSigner(..)")
    }
}

impl MockReceiptSigner {
    pub fn generate() -> Self {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).expect("ring generates an Ed25519 key");
        Self {
            pair: Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("ring parses its own key"),
        }
    }

    /// The public key, 64 lowercase hex: the value to pin.
    pub fn public_hex(&self) -> String {
        hex::encode(self.pair.public_key().as_ref())
    }

    /// A receipt over `text`, of the given kind.
    pub fn receipt(&self, text: String, kind: ReceiptSignatureKind) -> ReceiptPayload {
        let signature = hex::encode(self.pair.sign(text.as_bytes()).as_ref());
        ReceiptPayload {
            text,
            signature,
            signing_address: self.public_hex(),
            signing_algo: ReceiptAlgo::Ed25519,
            signature_kind: kind,
        }
    }
}

/// Ways the mock misbehaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockFault {
    /// No receipt comes back.
    NoReceipt,
    /// The receipt is a two-part receipt, binding no model.
    TwoPartReceipt,
    /// The receipt claims the `gateway` kind.
    GatewayReceipt,
    /// The receipt names another model.
    WrongModelReceipt,
    /// The receipt is over a different response than the one returned.
    ResponseSwapped,
    /// The response carries no `usage`.
    NoUsage,
    /// The body's answer does not parse. The receipt signs that body.
    Unparseable,
    /// The body's answer names a label outside the choices. The receipt
    /// signs that body.
    LabelOutsideChoices,
    /// The body names a different valid choice than [`MockJudgementModel::expected_answer`],
    /// and the receipt signs that body.
    BodyLabelDiffers,
    /// The receipt verifies and the body reports usage far above any
    /// reservation.
    UsageAboveReservation,
    /// The receipt is over a different response than the one returned, and
    /// the returned body reports `usage {0, 0}`.
    ResponseSwappedZeroUsage,
    /// The adapter reports the call timed out.
    Timeout,
    /// The call never returns: only the caller's timeout ends it.
    Hang,
    /// The call fails.
    Failed,
}

/// The mock model.
pub struct MockJudgementModel {
    signer: MockReceiptSigner,
    fault: Mutex<Option<MockFault>>,
    calls: AtomicUsize,
}

impl std::fmt::Debug for MockJudgementModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockJudgementModel")
            .field("calls", &self.calls())
            .finish_non_exhaustive()
    }
}

impl MockJudgementModel {
    pub fn new(signer: MockReceiptSigner) -> Self {
        Self {
            signer,
            fault: Mutex::new(None),
            calls: AtomicUsize::new(0),
        }
    }

    /// The signer's public key, to pin.
    pub fn signer_public_hex(&self) -> String {
        self.signer.public_hex()
    }

    /// Misbehave from the next call on; `None` behaves.
    pub fn set_fault(&self, fault: Option<MockFault>) {
        *self.fault.lock().expect("mock fault lock") = fault;
    }

    /// How many calls reached [`JudgementModel::decide`].
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// The deterministic answer for a call: a choice and a confidence picked
    /// by hashing the prompt version, the event index and the digest.
    pub fn expected_answer(call: &DecisionCall<'_>) -> (&'static str, &'static str) {
        let mut hasher = Sha256::new();
        hasher.update(call.prompt.prompt_version().as_bytes());
        hasher.update(call.event_index.unwrap_or(u32::MAX).to_le_bytes());
        hasher.update(call.digest.as_bytes());
        let hash = hasher.finalize();
        let choices = call.prompt.choices();
        let label = choices[usize::from(hash[0]) % choices.len()].as_wire();
        let confidence = ["high", "medium", "low"][usize::from(hash[1]) % 3];
        (label, confidence)
    }

    /// A valid choice for `call` other than `label`.
    pub fn other_choice(call: &DecisionCall<'_>, label: &str) -> &'static str {
        call.prompt
            .choices()
            .iter()
            .map(|c| c.as_wire())
            .find(|c| *c != label)
            .expect("every prompt has at least two choices")
    }
}

#[async_trait]
impl JudgementModel for MockJudgementModel {
    fn model_id(&self) -> &str {
        MOCK_MODEL_ID
    }

    fn request_body(&self, call: &DecisionCall<'_>) -> Vec<u8> {
        let body = serde_json::json!({
            "model": MOCK_MODEL_ID,
            "messages": [
                {"role": "system", "content": call.prompt.text()},
                {"role": "user", "content": call.prompt.user_turn(call.digest, call.event_index)},
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": call.prompt.name(),
                    "strict": true,
                    "schema": serde_json::from_str::<serde_json::Value>(&call.prompt.output_schema())
                        .expect("the output schema is JSON"),
                },
            },
            "max_tokens": call.max_tokens,
            "temperature": serde_json::from_str::<serde_json::Value>(JUDGEMENT_TEMPERATURE)
                .expect("the temperature is a JSON number"),
            "stream": false,
        });
        serde_json::to_vec(&body).expect("a request body is JSON serializable")
    }

    async fn decide(
        &self,
        call: &DecisionCall<'_>,
        request_body: &[u8],
    ) -> Result<RawDecision, ModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let fault = *self.fault.lock().expect("mock fault lock");
        match fault {
            Some(MockFault::Timeout) => return Err(ModelError::Timeout),
            Some(MockFault::Failed) => return Err(ModelError::Failed),
            Some(MockFault::Hang) => std::future::pending::<()>().await,
            _ => {}
        }

        let (label, confidence) = Self::expected_answer(call);
        let label = match fault {
            Some(MockFault::LabelOutsideChoices) => "definitely_not_a_choice",
            Some(MockFault::BodyLabelDiffers) => Self::other_choice(call, label),
            _ => label,
        };
        let content = if fault == Some(MockFault::Unparseable) {
            "I think it is probably a bug fix.".to_string()
        } else {
            serde_json::json!({"label": label, "confidence": confidence}).to_string()
        };
        let prompt_tokens = (request_body.len() as u64).div_ceil(3);
        let mut response = serde_json::json!({
            "id": format!("mock-{}", &hex::encode(Sha256::digest(request_body))[..16]),
            "object": "chat.completion",
            "model": MOCK_MODEL_ID,
            "choices": [{"index": 0, "finish_reason": "stop",
                         "message": {"role": "assistant", "content": content}}],
        });
        let usage = match fault {
            Some(MockFault::NoUsage) => None,
            Some(MockFault::ResponseSwappedZeroUsage) => Some((0, 0)),
            Some(MockFault::UsageAboveReservation) => {
                Some((MOCK_OVER_RESERVATION_TOKENS, MOCK_COMPLETION_TOKENS))
            }
            _ => Some((prompt_tokens, MOCK_COMPLETION_TOKENS)),
        };
        if let Some((prompt_tokens, completion_tokens)) = usage {
            response["usage"] = serde_json::json!({
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
            });
        }
        let response_body = serde_json::to_vec(&response).expect("JSON serializable");

        let request_hash = hex::encode(Sha256::digest(request_body));
        let signed_response = if matches!(
            fault,
            Some(MockFault::ResponseSwapped | MockFault::ResponseSwappedZeroUsage)
        ) {
            b"a different response".to_vec()
        } else {
            response_body.clone()
        };
        let response_hash = hex::encode(Sha256::digest(&signed_response));
        let receipt = match fault {
            Some(MockFault::NoReceipt) => None,
            Some(MockFault::TwoPartReceipt) => Some(self.signer.receipt(
                format!("{request_hash}:{response_hash}"),
                ReceiptSignatureKind::ProviderTee,
            )),
            Some(MockFault::GatewayReceipt) => Some(self.signer.receipt(
                format!("{MOCK_MODEL_ID}:{request_hash}:{response_hash}"),
                ReceiptSignatureKind::Gateway,
            )),
            Some(MockFault::WrongModelReceipt) => Some(self.signer.receipt(
                format!("some/other-model:{request_hash}:{response_hash}"),
                ReceiptSignatureKind::ProviderTee,
            )),
            _ => Some(self.signer.receipt(
                format!("{MOCK_MODEL_ID}:{request_hash}:{response_hash}"),
                ReceiptSignatureKind::ProviderTee,
            )),
        };
        Ok(RawDecision {
            response_body,
            receipt,
        })
    }

    /// Reads a Chat Completions body: `choices[0].message.content` as the
    /// structured `{label, confidence}`, and `usage`.
    fn parse_answer(&self, _call: &DecisionCall<'_>, response_body: &[u8]) -> ParsedAnswer {
        let unparseable = ModelAnswer::Unparseable {
            basis: ConfidenceBasis::SelfReported,
        };
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(response_body) else {
            return ParsedAnswer {
                answer: unparseable,
                usage: None,
            };
        };
        let usage = body.get("usage").and_then(|usage| {
            Some(TokenUsage {
                prompt_tokens: usage.get("prompt_tokens")?.as_u64()?,
                completion_tokens: usage.get("completion_tokens")?.as_u64()?,
            })
        });
        let answer = body["choices"][0]["message"]["content"]
            .as_str()
            .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
            .and_then(|content| {
                Some(ModelAnswer::SelfReported {
                    label: content.get("label")?.as_str()?.to_string(),
                    confidence: content.get("confidence")?.as_str()?.to_string(),
                })
            })
            .unwrap_or(unparseable);
        ParsedAnswer { answer, usage }
    }
}
