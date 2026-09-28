// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Decision logic for the non-redeeming invite lookup
//! (`POST /v1/invite/lookup`, #1118 Z1).
//!
//! Pure: no I/O. The issuer route reads the invite row and hands it here, so
//! what a lookup may disclose is decided in exactly one place. The answer
//! carries the public issuer name and the credit range and nothing else: never
//! `policy_label`, `issued_by_label`, `note_label`, `max_uses` or the use
//! count.

use chrono::{DateTime, Utc};
use trace_commons_protocol::invite_lookup::{
    CreditRange, INVITE_LOOKUP_REASON_EXHAUSTED, INVITE_LOOKUP_REASON_EXPIRED,
    INVITE_LOOKUP_REASON_NOT_FOUND, INVITE_LOOKUP_REASON_REVOKED, InviteLookupResponse,
};

use crate::db::postgres::InvitePeek;

/// Classify an invite for a lookup. Revocation outranks expiry, and expiry
/// outranks exhaustion, so a caller is told the most durable reason.
pub fn classify_invite(peek: Option<&InvitePeek>, now: DateTime<Utc>) -> InviteLookupResponse {
    let Some(peek) = peek else {
        return InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_NOT_FOUND);
    };
    let entry = &peek.entry;
    if entry.revoked_at.is_some() {
        return InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_REVOKED);
    }
    if entry.expires_at.is_some_and(|exp| exp <= now) {
        return InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_EXPIRED);
    }
    if peek.consumed_uses >= entry.max_uses {
        return InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_EXHAUSTED);
    }
    InviteLookupResponse {
        valid: true,
        issuer_display_name: entry.issuer_display_name.clone(),
        credit_range: entry
            .credit_range
            .map(|(min, max)| CreditRange::points(min, max)),
        reason_label: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_invite_registry::{InviteEntry, InviteTenantMode};
    use chrono::Duration;

    fn peek(consumed: u32) -> InvitePeek {
        InvitePeek {
            entry: InviteEntry {
                invite_subject_hash: format!("sha256:{}", "a".repeat(64)),
                policy_label: "SECRET-POLICY".to_string(),
                tenant_mode: InviteTenantMode::Derived,
                fixed_tenant_id: None,
                tenant_template_id: Some("tmpl".to_string()),
                policy_version: "v1".to_string(),
                allowed_consent_scopes: Vec::new(),
                allowed_uses: Vec::new(),
                max_uses: 3,
                expires_at: None,
                issuance_source: "operator".to_string(),
                issued_by_label: Some("SECRET-ISSUER".to_string()),
                credential_binding_hash: None,
                note_label: Some("SECRET-NOTE".to_string()),
                issuer_display_name: Some("Trace Commons Pilot".to_string()),
                credit_range: Some((2, 6)),
                revoked_at: None,
            },
            consumed_uses: consumed,
        }
    }

    #[test]
    fn a_live_invite_names_the_issuer_and_range() {
        let response = classify_invite(Some(&peek(1)), Utc::now());
        assert!(response.valid);
        assert_eq!(
            response.issuer_display_name.as_deref(),
            Some("Trace Commons Pilot")
        );
        assert_eq!(response.credit_range, Some(CreditRange::points(2, 6)));
        assert_eq!(response.reason_label, None);
    }

    #[test]
    fn a_live_invite_without_public_fields_is_valid_and_bare() {
        let mut p = peek(0);
        p.entry.issuer_display_name = None;
        p.entry.credit_range = None;
        let response = classify_invite(Some(&p), Utc::now());
        assert!(response.valid);
        assert_eq!(response.issuer_display_name, None);
        assert_eq!(response.credit_range, None);
    }

    #[test]
    fn refusals_are_labelled() {
        let now = Utc::now();
        assert_eq!(
            classify_invite(None, now).reason_label.as_deref(),
            Some("not_found")
        );
        let mut revoked = peek(0);
        revoked.entry.revoked_at = Some(now);
        assert_eq!(
            classify_invite(Some(&revoked), now).reason_label.as_deref(),
            Some("revoked")
        );
        let mut expired = peek(0);
        expired.entry.expires_at = Some(now - Duration::seconds(1));
        assert_eq!(
            classify_invite(Some(&expired), now).reason_label.as_deref(),
            Some("expired")
        );
        assert_eq!(
            classify_invite(Some(&peek(3)), now).reason_label.as_deref(),
            Some("exhausted")
        );
        // Exactly at the boundary the last use is still available.
        assert!(classify_invite(Some(&peek(2)), now).valid);
    }

    #[test]
    fn revoked_outranks_expired_outranks_exhausted() {
        let now = Utc::now();
        let mut p = peek(3);
        p.entry.expires_at = Some(now - Duration::seconds(1));
        assert_eq!(
            classify_invite(Some(&p), now).reason_label.as_deref(),
            Some("expired")
        );
        p.entry.revoked_at = Some(now);
        assert_eq!(
            classify_invite(Some(&p), now).reason_label.as_deref(),
            Some("revoked")
        );
    }

    #[test]
    fn no_response_leaks_operator_only_fields() {
        let now = Utc::now();
        let mut revoked = peek(0);
        revoked.entry.revoked_at = Some(now);
        for p in [peek(0), peek(3), revoked] {
            let body = serde_json::to_string(&classify_invite(Some(&p), now)).unwrap();
            for secret in ["SECRET-POLICY", "SECRET-ISSUER", "SECRET-NOTE", "max_uses"] {
                assert!(!body.contains(secret), "{secret} leaked in {body}");
            }
        }
    }
}
