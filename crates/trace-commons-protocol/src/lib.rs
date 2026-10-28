pub mod account_device_code;
pub mod activity_missions;
pub mod admission;
pub mod canonical_json;
pub mod community_handle;
pub mod device_invite_subject;
pub mod first_run_estimate;
pub mod inference_connection;
pub mod insights;
pub mod insights_cards;
pub mod insights_pricing;
pub mod insights_usage_series;
pub mod invite_lookup;
pub mod legacy_invite_link;
pub mod llm;
pub mod local_credit_estimate;
pub mod mission_catalog;
pub mod mission_draft;
pub mod mission_evaluation;
pub mod model_training_consent;
pub mod near_ai_measurements;
pub mod onboarding;
pub mod payout_link;
pub mod privacy;
pub mod public_run;
mod redaction;
pub mod settlement_estimate;
pub mod trace_contribution;
pub mod upload_allowance;
pub mod witness_pacing;
pub mod witness_provenance;

/// Response header carrying a rotated bearer for native account clients.
pub const ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER: &str = "x-trace-commons-session-token";

/// Response header carrying the new expiry of a rotated native session, as an
/// RFC 3339 UTC timestamp. Sent only beside
/// [`ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER`]: a rotation extends the session, and
/// the client stores this with the new token. A server that does not extend
/// sends no such header, and the client keeps the expiry it had.
pub const ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER: &str = "x-trace-commons-session-expires-at";

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

pub mod token_distribution;

pub mod token_distribution_chat;

pub mod private_edit_map;
