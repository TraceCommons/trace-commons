//! Non-redeeming invite lookup.
//!
//! A contributor pastes an invite link into the Join screen and wants to know
//! who issued it and what it pays before committing. `POST /v1/onboard` is
//! the only other validation and it spends a use, so the issuer exposes this
//! read-only route beside it. It mints nothing and consumes nothing.
//!
//! The invite code is a bearer secret, so it travels in the request body,
//! never the URL, and the answer is deliberately thin: an issuer display name
//! and a credit range on success, one coarse label on failure. It never says
//! how many uses an invite has, how many remain, or which operator issued it.

use serde::{Deserialize, Serialize};

/// Issuer route.
pub const INVITE_LOOKUP_PATH: &str = "/v1/invite/lookup";

pub const INVITE_LOOKUP_REQUEST_SCHEMA_VERSION: &str = "trace_commons.invite_lookup_request.v1";

/// The unit of [`CreditRange`] figures: whole credit points per accepted
/// trace, the same integer points the credit ledger and credit summary count
/// in. A client that shows fractional credit formats it from these.
pub const CREDIT_RANGE_UNIT_POINTS: &str = "points_per_accepted_trace";

/// Longest issuer display name, in Unicode scalar values.
pub const ISSUER_DISPLAY_NAME_MAX_CHARS: usize = 64;

/// Refusal labels carried in `reason_label` when `valid` is false.
pub const INVITE_LOOKUP_REASON_MALFORMED: &str = "malformed";
pub const INVITE_LOOKUP_REASON_NOT_FOUND: &str = "not_found";
pub const INVITE_LOOKUP_REASON_EXPIRED: &str = "expired";
pub const INVITE_LOOKUP_REASON_EXHAUSTED: &str = "exhausted";
pub const INVITE_LOOKUP_REASON_REVOKED: &str = "revoked";

/// `POST /v1/invite/lookup` body.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteLookupRequest {
    pub schema_version: String,
    pub invite_code: String,
}

impl std::fmt::Debug for InviteLookupRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The code is a bearer secret; a stray `{:?}` must not print it.
        f.debug_struct("InviteLookupRequest")
            .field("schema_version", &self.schema_version)
            .field("invite_code", &"<redacted>")
            .finish()
    }
}

/// Operator-set credit per accepted trace, inclusive on both ends.
///
/// This is an estimate, not a promise. It is set by the operator on the
/// invite, the credit ledger does not enforce it, settlement is disabled and
/// grading is in shadow mode. Clients MUST present it as "estimated credit per
/// accepted trace, not yet settled", and never as a payment promise or a
/// guaranteed rate.
///
/// Response types deliberately do not use `deny_unknown_fields`, so an issuer
/// can add a field without breaking clients already shipped. Only the request
/// does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditRange {
    pub min: i64,
    pub max: i64,
    /// Always [`CREDIT_RANGE_UNIT_POINTS`] today.
    pub unit: String,
}

impl CreditRange {
    /// A range in points per accepted trace.
    pub fn points(min: i64, max: i64) -> Self {
        Self {
            min,
            max,
            unit: CREDIT_RANGE_UNIT_POINTS.to_string(),
        }
    }
}

/// The answer. On `valid: true` the display name and range are present when
/// the operator set them; on `valid: false` only `reason_label` is.
///
/// Clients MUST present `credit_range` as "estimated credit per accepted
/// trace, not yet settled", never as a payment promise. See [`CreditRange`].
///
/// Unknown fields are ignored on purpose: a field added to the response later
/// must not break clients already shipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteLookupResponse {
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_range: Option<CreditRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_label: Option<String>,
}

impl InviteLookupResponse {
    pub fn invalid(reason_label: &str) -> Self {
        Self {
            valid: false,
            issuer_display_name: None,
            credit_range: None,
            reason_label: Some(reason_label.to_string()),
        }
    }
}

/// Whether `name` is acceptable as a public issuer display name: non-empty,
/// no leading or trailing whitespace, at most
/// [`ISSUER_DISPLAY_NAME_MAX_CHARS`] characters, and no control, bidi or
/// zero-width characters. It is shown verbatim to contributors, so it is
/// checked when stored.
pub fn valid_issuer_display_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= ISSUER_DISPLAY_NAME_MAX_CHARS
        && !name
            .chars()
            .any(|c| c.is_control() || is_bidi_or_invisible(c))
}

fn is_bidi_or_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// A credit range is storable when `0 <= min <= max`.
pub fn valid_credit_range(min: i64, max: i64) -> bool {
    0 <= min && min <= max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_response_with_an_extra_field_still_deserializes() {
        let parsed: InviteLookupResponse = serde_json::from_value(serde_json::json!({
            "valid": true,
            "issuer_display_name": "Pilot",
            "credit_range": {"min": 1, "max": 2, "unit": "points_per_accepted_trace", "tier": "x"},
            "a_future_field": {"nested": [1, 2]},
        }))
        .unwrap();
        assert!(parsed.valid);
        assert_eq!(parsed.credit_range, Some(CreditRange::points(1, 2)));
        // The request stays strict.
        assert!(
            serde_json::from_value::<InviteLookupRequest>(serde_json::json!({
                "schema_version": INVITE_LOOKUP_REQUEST_SCHEMA_VERSION,
                "invite_code": "x",
                "extra": 1,
            }))
            .is_err()
        );
    }

    #[test]
    fn display_name_rules() {
        assert!(valid_issuer_display_name("Trace Commons Pilot"));
        assert!(valid_issuer_display_name("Zaki's Lab (beta)"));
        assert!(!valid_issuer_display_name(""));
        assert!(!valid_issuer_display_name("  padded "));
        assert!(!valid_issuer_display_name("line\nbreak"));
        assert!(!valid_issuer_display_name("tab\there"));
        assert!(!valid_issuer_display_name("evil\u{202E}name"));
        assert!(!valid_issuer_display_name(&"x".repeat(65)));
        assert!(valid_issuer_display_name(&"x".repeat(64)));
    }

    #[test]
    fn credit_range_rules() {
        assert!(valid_credit_range(0, 0));
        assert!(valid_credit_range(1, 5));
        assert!(!valid_credit_range(5, 1));
        assert!(!valid_credit_range(-1, 3));
    }

    #[test]
    fn invalid_response_carries_only_a_label() {
        let json = serde_json::to_value(InviteLookupResponse::invalid("expired")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"valid": false, "reason_label": "expired"})
        );
    }

    #[test]
    fn request_debug_hides_the_code() {
        let req = InviteLookupRequest {
            schema_version: INVITE_LOOKUP_REQUEST_SCHEMA_VERSION.to_string(),
            invite_code: "ABCDEFGHJKLMNPQR".to_string(),
        };
        assert!(!format!("{req:?}").contains("ABCDEFGH"));
    }

    #[test]
    fn unknown_fields_are_refused() {
        assert!(
            serde_json::from_value::<InviteLookupRequest>(serde_json::json!({
                "schema_version": INVITE_LOOKUP_REQUEST_SCHEMA_VERSION,
                "invite_code": "X",
                "extra": 1
            }))
            .is_err()
        );
    }
}
