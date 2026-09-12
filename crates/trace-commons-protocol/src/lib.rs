pub mod admission;
pub mod canonical_json;
pub mod community_handle;
pub mod insights;
pub mod insights_cards;
pub mod insights_pricing;
pub mod llm;
pub mod mission_draft;
pub mod onboarding;
pub mod public_run;
mod redaction;
pub mod trace_contribution;

/// Response header carrying a rotated bearer for native account clients.
pub const ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER: &str = "x-trace-commons-session-token";

#[cfg(feature = "near-ai-privacy-filter")]
pub mod privacy_filter_near_ai;

#[cfg(feature = "self-hosted-privacy-filter")]
pub mod privacy_filter_self_hosted;

#[cfg(any(
    feature = "near-ai-privacy-filter",
    feature = "self-hosted-privacy-filter"
))]
pub(crate) mod privacy_filter_spans;

pub mod evidence_import;
