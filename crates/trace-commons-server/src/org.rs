// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Organizations on the Commons: the server's rules over an org tenant
//! (docs/superpowers/specs/2026-11-11-enterprise-gateway-design.md).
//!
//! An org is one tenant in the `org-` namespace (V159). This module holds the
//! decisions the issuer, ingest and settlement make about one, as pure
//! functions over the records the store returns, so each rule is tested
//! without a database:
//!
//! - [`check_org_claim`]: whether a seat's device may get an upload claim
//!   (spec 5.3, 11 "Fail-closed rules", flow changes A and C).
//! - [`check_org_landing`]: the same facts re-checked when an upload lands,
//!   so nothing is credited after suspension, sharing off, removal or a
//!   lapsed membership, even from an upload already in flight.
//! - [`org_settlement_target`] and [`tenant_settlement_group_key`]: credit in
//!   an org tenant settles to the org's holder account (spec 6.1), and an
//!   org that cannot say where blocks settlement instead of settling to the
//!   seats.
//!
//! Every refusal is a fixed label. None carries a tenant, account, org or
//! person.

use chrono::{DateTime, Duration, Utc};
use trace_commons_protocol::org_policy::{
    ConsentBasis, OrgMemberAutomatic, OrgPolicy, OrgRole, OrgSeatKind, OrgSendMode, OrgSharing,
    OrgStatus, uses_org_namespace,
};
use trace_commons_protocol::trace_contribution::{ConsentScope, TraceAllowedUse};
use uuid::Uuid;

/// The HMAC domain of a seat's `identity_index`: the proof that admitted it
/// (a NEAR AI subject today). Distinct from the login anchor's domain, so a
/// person's org seat cannot be joined to their personal account.
pub const ORG_MEMBER_IDENTITY_DOMAIN: &[u8] = b"trace_commons.org_member_identity.v1\n";

/// The HMAC domain of `trace_orgs.near_ai_org_index`, the bound NEAR AI
/// organization. Distinct from the payout subject index.
pub const NEAR_AI_ORG_BINDING_DOMAIN: &[u8] = b"trace_commons.org_near_ai_binding.v1\n";

/// A new org tenant id: `org-` and 32 random bytes in lowercase hex, made the
/// way `nearai-` tenants are, so it is not computable from anything public.
pub fn random_org_tenant_id() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut aes_gcm::aead::OsRng, &mut bytes);
    format!(
        "{}{}",
        trace_commons_protocol::org_policy::ORG_TENANT_PREFIX,
        hex::encode(bytes)
    )
}

/// One `trace_orgs` row, with the facts about its holder account the rules
/// need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgRecord {
    pub tenant_id: String,
    pub status: OrgStatus,
    pub holder_account_id: Option<Uuid>,
    /// The holder account exists and is not closed.
    pub holder_account_open: bool,
    pub near_ai_org_bound: bool,
    pub policy: OrgPolicy,
    pub policy_version: i64,
    pub policy_digest: String,
    pub seat_limit: i32,
    pub server_limit: i32,
    pub settlement_cap_micros: Option<i64>,
}

/// One `trace_org_seats` row, as the claim and landing rules read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgSeatRecord {
    pub account_id: Uuid,
    pub kind: OrgSeatKind,
    pub org_role: Option<OrgRole>,
    pub membership_proved_at: Option<DateTime<Utc>>,
    /// Whether the seat must re-prove membership to keep sending: every
    /// member seat except one admitted by an enrolment token, which is "this
    /// Mac" and has no membership to re-prove (spec 5.3, path B).
    pub requires_membership_proof: bool,
    pub joined_at: DateTime<Utc>,
    pub removed_at: Option<DateTime<Utc>>,
}

impl OrgSeatRecord {
    pub fn is_active(&self) -> bool {
        self.removed_at.is_none()
    }

    pub fn is_org_admin(&self) -> bool {
        self.is_active() && self.org_role == Some(OrgRole::OrgAdmin)
    }

    /// Whether the seat's last membership proof is older than the org's
    /// member session lifetime (or missing), for a seat that must keep one.
    pub fn membership_is_stale(&self, policy: &OrgPolicy, now: DateTime<Utc>) -> bool {
        if self.kind != OrgSeatKind::Member || !self.requires_membership_proof {
            return false;
        }
        let lifetime = Duration::seconds(i64::from(policy.member_session_lifetime_seconds));
        match self.membership_proved_at {
            Some(proved_at) => now - proved_at > lifetime,
            None => true,
        }
    }
}

/// A seat in the admin's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgSeatListing {
    pub seat: OrgSeatRecord,
    pub active_devices: i64,
}

/// What a device asserted on its upload-claim request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrgClaimAssertion {
    pub consent_basis: Option<ConsentBasis>,
    pub policy_digest: Option<String>,
    pub send_mode: Option<OrgSendMode>,
}

/// Why an org tenant refused a claim or an upload. The label is the wire
/// value and the only thing logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgRefusal {
    RecordMissing,
    Suspended,
    Closing,
    SharingOff,
    SeatMissing,
    SeatRemoved,
    SeatCannotSend,
    ConsentBasisMissing,
    ConsentBasisMismatch,
    PolicyDigestMissing,
    PolicyDigestStale,
    SendModeMissing,
    AutomaticForbidden,
    MembershipStale,
    GrantMissing,
    ScopeNotAllowed,
    UseNotAllowed,
}

impl OrgRefusal {
    pub fn label(self) -> &'static str {
        match self {
            Self::RecordMissing => "org_record_missing",
            Self::Suspended => "org_suspended",
            Self::Closing => "org_closing",
            Self::SharingOff => "org_sharing_off",
            Self::SeatMissing => "org_seat_missing",
            Self::SeatRemoved => "org_seat_removed",
            Self::SeatCannotSend => "org_seat_cannot_send",
            Self::ConsentBasisMissing => "org_consent_basis_missing",
            Self::ConsentBasisMismatch => "org_consent_basis_mismatch",
            Self::PolicyDigestMissing => "org_policy_digest_missing",
            Self::PolicyDigestStale => "org_policy_digest_stale",
            Self::SendModeMissing => "org_send_mode_missing",
            Self::AutomaticForbidden => "org_automatic_forbidden",
            Self::MembershipStale => "org_membership_stale",
            Self::GrantMissing => "org_seat_grant_missing",
            Self::ScopeNotAllowed => "org_consent_scope_not_allowed",
            Self::UseNotAllowed => "org_allowed_use_not_allowed",
        }
    }
}

/// The org-wide facts every send needs: the record exists, the org is
/// active, sharing is on.
fn check_org_open(org: Option<&OrgRecord>) -> Result<&OrgRecord, OrgRefusal> {
    let org = org.ok_or(OrgRefusal::RecordMissing)?;
    match org.status {
        OrgStatus::Active => {}
        OrgStatus::Suspended => return Err(OrgRefusal::Suspended),
        OrgStatus::Closing => return Err(OrgRefusal::Closing),
    }
    if org.policy.sharing == OrgSharing::Off {
        return Err(OrgRefusal::SharingOff);
    }
    Ok(org)
}

/// The seat facts every send needs: present, not removed, a kind that sends,
/// and for a member a fresh membership proof.
fn check_seat_sends<'a>(
    org: &OrgRecord,
    seat: Option<&'a OrgSeatRecord>,
    now: DateTime<Utc>,
) -> Result<&'a OrgSeatRecord, OrgRefusal> {
    let seat = seat.ok_or(OrgRefusal::SeatMissing)?;
    if !seat.is_active() {
        return Err(OrgRefusal::SeatRemoved);
    }
    if seat.kind.consent_basis().is_none() {
        return Err(OrgRefusal::SeatCannotSend);
    }
    if seat.membership_is_stale(&org.policy, now) {
        return Err(OrgRefusal::MembershipStale);
    }
    Ok(seat)
}

/// Whether the device of `seat` may get an upload claim in `org`'s tenant,
/// given what it asserted. Returns the consent basis the claim records.
///
/// - The org must exist, be active, and have sharing on.
/// - The seat must exist, not be removed, be a member or server seat, and a
///   member seat must hold a membership proof younger than the org's member
///   session lifetime.
/// - The consent basis is required and must be the seat kind's: the issuer
///   refuses `org_policy` from a member and `member_grant` from a server.
/// - A member must name the org's current policy digest and how the trace
///   was sent, and an `automatic` send is refused when the org forbids
///   Automatic (flow change A). A server may omit the digest; one it names
///   must be current.
pub fn check_org_claim(
    org: Option<&OrgRecord>,
    seat: Option<&OrgSeatRecord>,
    assertion: &OrgClaimAssertion,
    now: DateTime<Utc>,
) -> Result<ConsentBasis, OrgRefusal> {
    let org = check_org_open(org)?;
    let seat = check_seat_sends(org, seat, now)?;
    let expected = seat
        .kind
        .consent_basis()
        .ok_or(OrgRefusal::SeatCannotSend)?;
    let asserted = assertion
        .consent_basis
        .ok_or(OrgRefusal::ConsentBasisMissing)?;
    if asserted != expected {
        return Err(OrgRefusal::ConsentBasisMismatch);
    }
    match (seat.kind, assertion.policy_digest.as_deref()) {
        (OrgSeatKind::Member, None) => return Err(OrgRefusal::PolicyDigestMissing),
        (_, Some(digest)) if digest != org.policy_digest => {
            return Err(OrgRefusal::PolicyDigestStale);
        }
        _ => {}
    }
    if seat.kind == OrgSeatKind::Member {
        let send_mode = assertion.send_mode.ok_or(OrgRefusal::SendModeMissing)?;
        if send_mode == OrgSendMode::Automatic
            && org.policy.member_automatic == OrgMemberAutomatic::Forbidden
        {
            return Err(OrgRefusal::AutomaticForbidden);
        }
    }
    Ok(expected)
}

/// Narrow a claim's ceiling to what the org's policy allows. The issuer has
/// already refused an org seat with no active grant, so `scopes` and `uses`
/// come from a grant, never from the issuer's floor.
pub fn org_policy_ceiling(
    policy: &OrgPolicy,
    scopes: Vec<ConsentScope>,
    uses: Vec<TraceAllowedUse>,
) -> (Vec<ConsentScope>, Vec<TraceAllowedUse>) {
    (
        scopes
            .into_iter()
            .filter(|scope| policy.allowed_consent_scopes.contains(scope))
            .collect(),
        uses.into_iter()
            .filter(|use_| policy.allowed_uses.contains(use_))
            .collect(),
    )
}

/// The checks that can change while an upload is in flight, made again when
/// it lands (spec 5.3): the org is active with sharing on, the seat that
/// sent it is still seated (and, for a member, still a member), and every
/// scope and use the envelope carries is one the org's current policy
/// allows.
pub fn check_org_landing(
    org: Option<&OrgRecord>,
    seat: Option<&OrgSeatRecord>,
    scopes: &[ConsentScope],
    uses: &[TraceAllowedUse],
    now: DateTime<Utc>,
) -> Result<(), OrgRefusal> {
    let org = check_org_open(org)?;
    check_seat_sends(org, seat, now)?;
    if scopes
        .iter()
        .any(|scope| !org.policy.allowed_consent_scopes.contains(scope))
    {
        return Err(OrgRefusal::ScopeNotAllowed);
    }
    if uses
        .iter()
        .any(|use_| !org.policy.allowed_uses.contains(use_))
    {
        return Err(OrgRefusal::UseNotAllowed);
    }
    Ok(())
}

/// Where a tenant's credit settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgSettlementTarget {
    /// Not an org tenant: each principal settles to its account, or to
    /// itself, as before.
    NotOrg,
    /// Every principal in the org settles to the holder account, under the
    /// org's own cap.
    Holder { account_id: Uuid, cap_micros: i64 },
    /// An org tenant that cannot say where its credit goes. Settlement for
    /// the tenant stops with this label; it never settles to the seats.
    Blocked(&'static str),
}

/// Decide [`OrgSettlementTarget`] for `tenant_id` from its org record (spec
/// 6.1, 11 "Fail-closed rules"). An `org-` tenant with no record, a
/// suspended org, no holder account or a closed one, or no org settlement cap
/// is blocked. A closing org still settles: closing is when it is paid out.
pub fn org_settlement_target(tenant_id: &str, org: Option<&OrgRecord>) -> OrgSettlementTarget {
    if !uses_org_namespace(tenant_id) {
        return OrgSettlementTarget::NotOrg;
    }
    let Some(org) = org else {
        return OrgSettlementTarget::Blocked("org_record_missing");
    };
    if org.status == OrgStatus::Suspended {
        return OrgSettlementTarget::Blocked("org_suspended");
    }
    let Some(account_id) = org.holder_account_id else {
        return OrgSettlementTarget::Blocked("org_holder_account_missing");
    };
    if !org.holder_account_open {
        return OrgSettlementTarget::Blocked("org_holder_account_closed");
    }
    let Some(cap_micros) = org.settlement_cap_micros else {
        return OrgSettlementTarget::Blocked("org_settlement_cap_missing");
    };
    OrgSettlementTarget::Holder {
        account_id,
        cap_micros,
    }
}

/// [`org_settlement_target`] for `tenant_id`, reading its org record only
/// when the tenant claims the org namespace. Every other tenant answers
/// [`OrgSettlementTarget::NotOrg`] without a database read.
pub async fn org_settlement_target_for(
    db: &dyn crate::db::Database,
    tenant_id: &str,
) -> Result<OrgSettlementTarget, crate::error::DatabaseError> {
    if !uses_org_namespace(tenant_id) {
        return Ok(OrgSettlementTarget::NotOrg);
    }
    let org = db.get_org(tenant_id).await?;
    Ok(org_settlement_target(tenant_id, org.as_ref()))
}

/// The settlement key for a principal's credit in a tenant whose target is
/// `target`: the holder account for an org, otherwise
/// [`crate::db::settlement_group_key`]. The one tenant-aware resolver every
/// settlement call site uses, so the grouping, the cap and the pipeline can
/// never disagree about an org line.
///
/// A blocked target has no key: callers refuse before asking. Asked anyway,
/// it answers the ordinary key, which the caller's refusal never lets settle.
pub fn tenant_settlement_group_key(
    auth_principal_ref: &str,
    principal_to_account: &std::collections::HashMap<String, Uuid>,
    target: &OrgSettlementTarget,
) -> String {
    match target {
        OrgSettlementTarget::Holder { account_id, .. } => {
            format!("{}{account_id}", crate::db::ACCOUNT_SETTLEMENT_KEY_PREFIX)
        }
        OrgSettlementTarget::NotOrg | OrgSettlementTarget::Blocked(_) => {
            crate::db::settlement_group_key(auth_principal_ref, principal_to_account)
        }
    }
}

/// A hashed reference to a seat for audit metadata: `sha256:` over a domain
/// tag, the tenant and the account, in the manner of `credit_account_hash`.
/// Never the raw tenant or account id.
pub fn org_seat_ref_hash(tenant_id: &str, account_id: Uuid) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(format!(
        "trace_commons.org_seat_ref.v1\n{tenant_id}\n{account_id}"
    ));
    format!("sha256:{}", hex::encode(digest))
}

/// A hashed actor reference for `removed_by_ref` and audit metadata.
pub fn org_actor_ref_hash(actor_ref: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(format!("trace_commons.org_actor_ref.v1\n{actor_ref}"));
    format!("sha256:{}", hex::encode(digest))
}

/// How long a removed member seat may still withdraw its own traces (spec
/// 5.6, decision 6): thirty days from removal. After it the member withdraws
/// through the operator.
pub const ORG_REMOVED_SEAT_WITHDRAW_WINDOW_DAYS: i64 = 30;

/// Whether a removed seat is still inside its withdraw-only window.
pub fn removed_seat_may_withdraw(seat: &OrgSeatRecord, now: DateTime<Utc>) -> bool {
    match seat.removed_at {
        Some(removed_at) => {
            seat.kind == OrgSeatKind::Member
                && now - removed_at <= Duration::days(ORG_REMOVED_SEAT_WITHDRAW_WINDOW_DAYS)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn org() -> OrgRecord {
        let policy = OrgPolicy::conservative_default();
        OrgRecord {
            tenant_id: format!("org-{}", "ab".repeat(32)),
            status: OrgStatus::Active,
            holder_account_id: Some(Uuid::from_u128(7)),
            holder_account_open: true,
            near_ai_org_bound: true,
            policy_digest: policy.digest(),
            policy,
            policy_version: 1,
            seat_limit: 10,
            server_limit: 2,
            settlement_cap_micros: Some(1_000_000),
        }
    }

    fn member(now: DateTime<Utc>) -> OrgSeatRecord {
        OrgSeatRecord {
            account_id: Uuid::from_u128(1),
            kind: OrgSeatKind::Member,
            org_role: Some(OrgRole::Member),
            membership_proved_at: Some(now - Duration::hours(1)),
            requires_membership_proof: true,
            joined_at: now - Duration::days(1),
            removed_at: None,
        }
    }

    fn server(now: DateTime<Utc>) -> OrgSeatRecord {
        OrgSeatRecord {
            account_id: Uuid::from_u128(2),
            kind: OrgSeatKind::Server,
            org_role: None,
            membership_proved_at: None,
            requires_membership_proof: false,
            joined_at: now - Duration::days(1),
            removed_at: None,
        }
    }

    fn member_assertion(org: &OrgRecord, mode: OrgSendMode) -> OrgClaimAssertion {
        OrgClaimAssertion {
            consent_basis: Some(ConsentBasis::MemberGrant),
            policy_digest: Some(org.policy_digest.clone()),
            send_mode: Some(mode),
        }
    }

    #[test]
    fn member_submit_under_current_policy_is_admitted() {
        let now = Utc::now();
        let org = org();
        let basis = check_org_claim(
            Some(&org),
            Some(&member(now)),
            &member_assertion(&org, OrgSendMode::Submit),
            now,
        );
        assert_eq!(basis, Ok(ConsentBasis::MemberGrant));
    }

    #[test]
    fn missing_or_inactive_org_refuses() {
        let now = Utc::now();
        let org_row = org();
        let assertion = member_assertion(&org_row, OrgSendMode::Submit);
        assert_eq!(
            check_org_claim(None, Some(&member(now)), &assertion, now),
            Err(OrgRefusal::RecordMissing)
        );
        for (status, refusal) in [
            (OrgStatus::Suspended, OrgRefusal::Suspended),
            (OrgStatus::Closing, OrgRefusal::Closing),
        ] {
            let mut org = org();
            org.status = status;
            assert_eq!(
                check_org_claim(Some(&org), Some(&member(now)), &assertion, now),
                Err(refusal)
            );
        }
    }

    /// Promise 4: sharing off means no new credit, for members and servers.
    #[test]
    fn sharing_off_refuses_every_seat() {
        let now = Utc::now();
        let mut org = org();
        org.policy.sharing = OrgSharing::Off;
        org.policy_digest = org.policy.digest();
        assert_eq!(
            check_org_claim(
                Some(&org),
                Some(&member(now)),
                &member_assertion(&org, OrgSendMode::Submit),
                now
            ),
            Err(OrgRefusal::SharingOff)
        );
        let server_assertion = OrgClaimAssertion {
            consent_basis: Some(ConsentBasis::OrgPolicy),
            ..Default::default()
        };
        assert_eq!(
            check_org_claim(Some(&org), Some(&server(now)), &server_assertion, now),
            Err(OrgRefusal::SharingOff)
        );
    }

    /// Promise 3 and flow change A.
    #[test]
    fn forbidden_automatic_and_stale_digest_are_refused() {
        let now = Utc::now();
        let org = org();
        assert_eq!(
            check_org_claim(
                Some(&org),
                Some(&member(now)),
                &member_assertion(&org, OrgSendMode::Automatic),
                now
            ),
            Err(OrgRefusal::AutomaticForbidden)
        );
        let mut stale = member_assertion(&org, OrgSendMode::Submit);
        stale.policy_digest = Some(format!("sha256:{}", "00".repeat(32)));
        assert_eq!(
            check_org_claim(Some(&org), Some(&member(now)), &stale, now),
            Err(OrgRefusal::PolicyDigestStale)
        );
        let mut missing = member_assertion(&org, OrgSendMode::Submit);
        missing.policy_digest = None;
        assert_eq!(
            check_org_claim(Some(&org), Some(&member(now)), &missing, now),
            Err(OrgRefusal::PolicyDigestMissing)
        );
        let mut no_mode = member_assertion(&org, OrgSendMode::Submit);
        no_mode.send_mode = None;
        assert_eq!(
            check_org_claim(Some(&org), Some(&member(now)), &no_mode, now),
            Err(OrgRefusal::SendModeMissing)
        );

        let mut allowing = org.clone();
        allowing.policy.member_automatic = OrgMemberAutomatic::Allowed;
        allowing.policy_digest = allowing.policy.digest();
        assert_eq!(
            check_org_claim(
                Some(&allowing),
                Some(&member(now)),
                &member_assertion(&allowing, OrgSendMode::Automatic),
                now
            ),
            Ok(ConsentBasis::MemberGrant)
        );
    }

    /// Promise 9: servers share only under the org's consent, and a member
    /// cannot claim it.
    #[test]
    fn consent_basis_must_match_the_seat_kind() {
        let now = Utc::now();
        let org = org();
        let mut as_org = member_assertion(&org, OrgSendMode::Submit);
        as_org.consent_basis = Some(ConsentBasis::OrgPolicy);
        assert_eq!(
            check_org_claim(Some(&org), Some(&member(now)), &as_org, now),
            Err(OrgRefusal::ConsentBasisMismatch)
        );
        let server_as_member = OrgClaimAssertion {
            consent_basis: Some(ConsentBasis::MemberGrant),
            ..Default::default()
        };
        assert_eq!(
            check_org_claim(Some(&org), Some(&server(now)), &server_as_member, now),
            Err(OrgRefusal::ConsentBasisMismatch)
        );
        let none = OrgClaimAssertion::default();
        assert_eq!(
            check_org_claim(Some(&org), Some(&server(now)), &none, now),
            Err(OrgRefusal::ConsentBasisMissing)
        );
        let server_ok = OrgClaimAssertion {
            consent_basis: Some(ConsentBasis::OrgPolicy),
            ..Default::default()
        };
        assert_eq!(
            check_org_claim(Some(&org), Some(&server(now)), &server_ok, now),
            Ok(ConsentBasis::OrgPolicy)
        );
    }

    /// Promises 5 and 6.
    #[test]
    fn removed_seat_and_stale_membership_are_refused() {
        let now = Utc::now();
        let org = org();
        let assertion = member_assertion(&org, OrgSendMode::Submit);
        let mut removed = member(now);
        removed.removed_at = Some(now);
        assert_eq!(
            check_org_claim(Some(&org), Some(&removed), &assertion, now),
            Err(OrgRefusal::SeatRemoved)
        );
        assert_eq!(
            check_org_claim(Some(&org), None, &assertion, now),
            Err(OrgRefusal::SeatMissing)
        );
        let mut stale = member(now);
        stale.membership_proved_at = Some(
            now - Duration::seconds(i64::from(org.policy.member_session_lifetime_seconds) + 1),
        );
        assert_eq!(
            check_org_claim(Some(&org), Some(&stale), &assertion, now),
            Err(OrgRefusal::MembershipStale)
        );
        let mut never = member(now);
        never.membership_proved_at = None;
        assert_eq!(
            check_org_claim(Some(&org), Some(&never), &assertion, now),
            Err(OrgRefusal::MembershipStale)
        );
        let mut by_token = never.clone();
        by_token.requires_membership_proof = false;
        assert_eq!(
            check_org_claim(Some(&org), Some(&by_token), &assertion, now),
            Ok(ConsentBasis::MemberGrant)
        );
    }

    #[test]
    fn holder_seat_never_sends() {
        let now = Utc::now();
        let org = org();
        let holder = OrgSeatRecord {
            account_id: Uuid::from_u128(7),
            kind: OrgSeatKind::Holder,
            org_role: None,
            membership_proved_at: None,
            requires_membership_proof: false,
            joined_at: now,
            removed_at: None,
        };
        for basis in [ConsentBasis::MemberGrant, ConsentBasis::OrgPolicy] {
            let assertion = OrgClaimAssertion {
                consent_basis: Some(basis),
                ..Default::default()
            };
            assert_eq!(
                check_org_claim(Some(&org), Some(&holder), &assertion, now),
                Err(OrgRefusal::SeatCannotSend)
            );
        }
    }

    #[test]
    fn landing_rechecks_scopes_uses_and_seat() {
        let now = Utc::now();
        let org = org();
        let seat = member(now);
        assert_eq!(
            check_org_landing(
                Some(&org),
                Some(&seat),
                &[ConsentScope::DebuggingEvaluation],
                &[TraceAllowedUse::Debugging],
                now
            ),
            Ok(())
        );
        assert_eq!(
            check_org_landing(
                Some(&org),
                Some(&seat),
                &[ConsentScope::ModelTraining],
                &[TraceAllowedUse::Debugging],
                now
            ),
            Err(OrgRefusal::ScopeNotAllowed)
        );
        assert_eq!(
            check_org_landing(
                Some(&org),
                Some(&seat),
                &[ConsentScope::DebuggingEvaluation],
                &[TraceAllowedUse::ModelTraining],
                now
            ),
            Err(OrgRefusal::UseNotAllowed)
        );
        let mut removed = seat.clone();
        removed.removed_at = Some(now);
        assert_eq!(
            check_org_landing(Some(&org), Some(&removed), &[], &[], now),
            Err(OrgRefusal::SeatRemoved)
        );
        let mut suspended = org.clone();
        suspended.status = OrgStatus::Suspended;
        assert_eq!(
            check_org_landing(Some(&suspended), Some(&seat), &[], &[], now),
            Err(OrgRefusal::Suspended)
        );
    }

    #[test]
    fn policy_ceiling_only_narrows() {
        let policy = OrgPolicy::conservative_default();
        let (scopes, uses) = org_policy_ceiling(
            &policy,
            vec![
                ConsentScope::DebuggingEvaluation,
                ConsentScope::ModelTraining,
            ],
            vec![TraceAllowedUse::Debugging, TraceAllowedUse::ModelTraining],
        );
        assert_eq!(scopes, vec![ConsentScope::DebuggingEvaluation]);
        assert_eq!(uses, vec![TraceAllowedUse::Debugging]);
    }

    #[test]
    fn settlement_target_fails_closed() {
        let tenant = org().tenant_id;
        assert_eq!(
            org_settlement_target("nearai-abc", None),
            OrgSettlementTarget::NotOrg
        );
        assert_eq!(
            org_settlement_target(&tenant, None),
            OrgSettlementTarget::Blocked("org_record_missing")
        );
        assert_eq!(
            org_settlement_target("ORG-not-a-hash", None),
            OrgSettlementTarget::Blocked("org_record_missing")
        );
        let mut no_holder = org();
        no_holder.holder_account_id = None;
        assert_eq!(
            org_settlement_target(&tenant, Some(&no_holder)),
            OrgSettlementTarget::Blocked("org_holder_account_missing")
        );
        let mut closed = org();
        closed.holder_account_open = false;
        assert_eq!(
            org_settlement_target(&tenant, Some(&closed)),
            OrgSettlementTarget::Blocked("org_holder_account_closed")
        );
        let mut no_cap = org();
        no_cap.settlement_cap_micros = None;
        assert_eq!(
            org_settlement_target(&tenant, Some(&no_cap)),
            OrgSettlementTarget::Blocked("org_settlement_cap_missing")
        );
        let mut suspended = org();
        suspended.status = OrgStatus::Suspended;
        assert_eq!(
            org_settlement_target(&tenant, Some(&suspended)),
            OrgSettlementTarget::Blocked("org_suspended")
        );
        let mut closing = org();
        closing.status = OrgStatus::Closing;
        assert_eq!(
            org_settlement_target(&tenant, Some(&closing)),
            OrgSettlementTarget::Holder {
                account_id: Uuid::from_u128(7),
                cap_micros: 1_000_000
            }
        );
    }

    #[test]
    fn org_credit_settles_to_the_holder() {
        let seat_account = Uuid::from_u128(1);
        let map = HashMap::from([("principal-a".to_string(), seat_account)]);
        let holder = OrgSettlementTarget::Holder {
            account_id: Uuid::from_u128(7),
            cap_micros: 1,
        };
        let holder_key = format!("account:{}", Uuid::from_u128(7));
        assert_eq!(
            tenant_settlement_group_key("principal-a", &map, &holder),
            holder_key
        );
        assert_eq!(
            tenant_settlement_group_key("principal-unlinked", &map, &holder),
            holder_key,
            "every principal in an org settles to the holder"
        );
        assert_eq!(
            tenant_settlement_group_key("principal-a", &map, &OrgSettlementTarget::NotOrg),
            format!("account:{seat_account}")
        );
        assert_eq!(
            tenant_settlement_group_key("principal-unlinked", &map, &OrgSettlementTarget::NotOrg),
            "principal-unlinked"
        );
    }

    #[test]
    fn removed_member_may_withdraw_for_thirty_days() {
        let now = Utc::now();
        let mut seat = member(now);
        assert!(!removed_seat_may_withdraw(&seat, now));
        seat.removed_at = Some(now - Duration::days(29));
        assert!(removed_seat_may_withdraw(&seat, now));
        seat.removed_at = Some(now - Duration::days(31));
        assert!(!removed_seat_may_withdraw(&seat, now));
        let mut gone_server = server(now);
        gone_server.removed_at = Some(now);
        assert!(!removed_seat_may_withdraw(&gone_server, now));
    }

    #[test]
    fn hashed_refs_are_domain_separated_and_shaped() {
        let account = Uuid::from_u128(1);
        let seat = org_seat_ref_hash("org-x", account);
        let actor = org_actor_ref_hash("org-x");
        assert!(seat.starts_with("sha256:") && seat.len() == 71);
        assert_ne!(seat, actor);
        assert_ne!(seat, org_seat_ref_hash("org-y", account));
    }

    #[test]
    fn random_org_tenant_is_in_the_namespace() {
        let id = random_org_tenant_id();
        assert!(trace_commons_protocol::org_policy::is_org_tenant(&id));
        assert_ne!(id, random_org_tenant_id());
    }
}
