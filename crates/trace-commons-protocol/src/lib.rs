pub mod community_handle;
pub mod llm;
pub mod onboarding;
mod redaction;
pub mod trace_contribution;

#[cfg(feature = "near-ai-privacy-filter")]
pub mod privacy_filter_near_ai;

// Widened to `any(near-ai, self-hosted)` when the self-hosted adapter lands
// and becomes a second consumer. Gated on near-ai alone until then, so no
// feature combination compiles this module without a caller.
#[cfg(feature = "near-ai-privacy-filter")]
pub(crate) mod privacy_filter_spans;
