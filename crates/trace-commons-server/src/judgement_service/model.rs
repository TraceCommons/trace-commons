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
//! - the limiter reserves `(request_bytes + 512) * input_price + ...`
//!   before the call (section 6), so the bytes have to exist before the call;
//! - the receipt is then verified over bytes the caller holds, not bytes the
//!   adapter reports having sent. An adapter that re-serialized its request
//!   would fail [`RequestHashMismatch`] rather than being believed.
//!
//! This is the contract the NEAR AI drill's `AttestationClient::complete`
//! already uses.
//!
//! # The answer comes from the receipted bytes
//!
//! [`RawDecision`] carries the response body and the receipt, and no answer
//! and no usage. The wrapper verifies the receipt over `response_body`
//! first, and only then reads the label out of those same bytes with the
//! pure [`JudgementModel::parse_answer`]. An adapter therefore cannot pair a
//! receipt for one response with an answer read from another (a retry that
//! kept attempt 1's body with attempt 2's answer, or a wrong JSON path).
//!
//! `parse_answer` returns a [`ParsedAnswer`], not a `Result` as the spec's
//! sketch has it: an answer that does not parse is
//! [`ModelAnswer::Unparseable`], which the wrapper turns into the fallback
//! label, and a body without `usage` has `usage: None`.
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
    /// reservation assumes the model spends all of it, and one input token
    /// per request byte; it is an upper bound only under that assumption.
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

/// One completed call: the bytes and the receipt, nothing read out of them.
pub struct RawDecision {
    /// The response body exactly as received. The receipt hashes this, so
    /// nothing may re-serialize it, and the answer is read from it.
    pub response_body: Vec<u8>,
    /// The per-request receipt for this completion, as the provider returned
    /// it. `None` when none could be fetched; the wrapper refuses that.
    pub receipt: Option<ReceiptPayload>,
}

impl std::fmt::Debug for RawDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawDecision")
            .field("response_bytes", &self.response_body.len())
            .field("receipt", &self.receipt.is_some())
            .finish()
    }
}

/// What [`JudgementModel::parse_answer`] reads out of a response body.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAnswer {
    pub answer: ModelAnswer,
    /// `None` when the body carried no `usage`: the reservation then stands
    /// as the charge.
    pub usage: Option<TokenUsage>,
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

    /// Send `request_body` verbatim and return the response bytes and the
    /// receipt. "Choices in, one choice out, with probabilities if
    /// available." It returns no answer and no usage.
    async fn decide(
        &self,
        call: &DecisionCall<'_>,
        request_body: &[u8],
    ) -> Result<RawDecision, ModelError>;

    /// Pure: the answer and the usage in `response_body`. The wrapper takes
    /// a label from it only after the receipt over these bytes verified.
    fn parse_answer(&self, call: &DecisionCall<'_>, response_body: &[u8]) -> ParsedAnswer;
}
