// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The model adapter seam (spec section 5).
//!
//! Every model the service can call sits behind [`JudgementModel`], held as
//! `Arc<dyn JudgementModel>`, so a different model substitutes at the seam
//! and nothing above it names a concrete adapter.
//!
//! # One deviation from the spec's sketch, on purpose
//!
//! The spec sketches a single `decide` that "returns the exact request
//! bytes sent and response bytes received". Here the request bytes come out
//! of [`JudgementModel::request_body`] **before** the call, and
//! [`JudgementModel::decide`] must put exactly those bytes on the wire. Two
//! reasons:
//!
//! - the limiter reserves `ceil(request_bytes / 3) * input_price + ...`
//!   before the call (section 6), so the bytes have to exist before the call;
//! - the receipt is then verified over bytes the caller holds, not bytes the
//!   adapter reports having sent. An adapter that re-serialized its request
//!   would fail [`RequestHashMismatch`] rather than being believed.
//!
//! This is the contract the NEAR AI drill's `AttestationClient::complete`
//! already uses.
//!
//! [`RequestHashMismatch`]: trace_commons_attestation::receipt::ReceiptError::RequestHashMismatch

use async_trait::async_trait;
use trace_commons_attestation::receipt::ReceiptPayload;
use trace_commons_protocol::insights_judgement::{ConfidenceBasis, JudgementKind};

use super::prompts::Prompt;

/// One judgement call: which prompt, over which digest, and for
/// `event_kind` which marked event.
#[derive(Clone, Copy)]
pub struct DecisionCall<'a> {
    pub prompt: &'static Prompt,
    /// The digest, as received. Scrubbed contributor text: never logged.
    pub digest: &'a str,
    /// Present exactly for `event_kind`: the zero-based index into the
    /// digest's `events` array.
    pub event_index: Option<u32>,
    /// The completion-token ceiling the adapter must request. The limiter's
    /// reservation assumes the model spends all of it.
    pub max_tokens: u32,
}

impl DecisionCall<'_> {
    pub fn judgement(&self) -> JudgementKind {
        self.prompt.kind()
    }
}

impl std::fmt::Debug for DecisionCall<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionCall")
            .field("prompt", &self.prompt.name())
            .field("event_index", &self.event_index)
            .field("max_tokens", &self.max_tokens)
            .finish_non_exhaustive()
    }
}

/// What the model answered, as the adapter read it out of the response.
///
/// Strings rather than [`JudgementLabel`]s: the wrapper, not the adapter,
/// decides whether an answer names one of the prompt's choices, so the
/// fallback rule (spec section 7) is applied in one place.
///
/// [`JudgementLabel`]: trace_commons_protocol::insights_judgement::JudgementLabel
#[derive(Clone, PartialEq)]
pub enum ModelAnswer {
    /// A chat model's structured output: a label and its own confidence.
    SelfReported { label: String, confidence: String },
    /// A decision model's choice probabilities.
    Probabilities(Vec<(String, f64)>),
    /// Nothing parseable came back. `basis` is the basis this adapter would
    /// have reported, so the fallback label carries an honest one.
    Unparseable { basis: ConfidenceBasis },
}

/// The answer text is model output over contributor text, so `Debug` names
/// the shape only.
impl std::fmt::Debug for ModelAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SelfReported { .. } => f.write_str("SelfReported(..)"),
            Self::Probabilities(choices) => write!(f, "Probabilities({} choices)", choices.len()),
            Self::Unparseable { basis } => write!(f, "Unparseable({basis:?})"),
        }
    }
}

/// Token usage as the response body reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    /// Reasoning tokens count here (spec section 6).
    pub completion_tokens: u64,
}

/// One completed call.
pub struct RawDecision {
    /// The response body exactly as received. The receipt hashes this, so
    /// nothing may re-serialize it.
    pub response_body: Vec<u8>,
    pub answer: ModelAnswer,
    /// `None` when the response carried no `usage`: the reservation then
    /// stands as the charge.
    pub usage: Option<TokenUsage>,
    /// The per-request receipt for this completion, as the provider returned
    /// it. `None` when none could be fetched; the wrapper refuses that.
    pub receipt: Option<ReceiptPayload>,
}

impl std::fmt::Debug for RawDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawDecision")
            .field("response_bytes", &self.response_body.len())
            .field("answer", &self.answer)
            .field("usage", &self.usage)
            .field("receipt", &self.receipt.is_some())
            .finish()
    }
}

/// Why a call produced no response. Carries nothing from the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// The call did not complete in time.
    #[error("judgement_model_timeout")]
    Timeout,
    /// The call failed: transport, status, or an unreadable body.
    #[error("judgement_model_failed")]
    Failed,
}

/// A model the service can ask for one typed choice.
#[async_trait]
pub trait JudgementModel: Send + Sync {
    /// The model id, exactly as the catalog, the receipt and the judge
    /// version spell it.
    fn model_id(&self) -> &str;

    /// The exact request body [`Self::decide`] will send for `call`. Pure:
    /// the same call gives the same bytes.
    fn request_body(&self, call: &DecisionCall<'_>) -> Vec<u8>;

    /// Send `request_body` verbatim and read the answer. "Choices in, one
    /// choice out, with probabilities if available."
    async fn decide(
        &self,
        call: &DecisionCall<'_>,
        request_body: &[u8],
    ) -> Result<RawDecision, ModelError>;
}
