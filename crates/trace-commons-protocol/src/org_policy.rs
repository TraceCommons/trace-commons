//! Organizations on the Commons: the org policy document, its digest, and the
//! labels both sides of the wire share
//! (docs/superpowers/specs/2026-11-11-enterprise-gateway-design.md, sections
//! 4 and 5).
//!
//! An organization is one tenant in the `org-` namespace. Its admin sets a
//! small versioned policy. The server mirrors the parts it enforces into the
//! tenant's ceilings; enrolled devices fetch the whole document and act on
//! it. Policy only narrows what a person does: no field here can arm a
//! folder, add a data use a member did not choose, or turn on Automatic for a
//! person.
//!
//! The digest is what ties a send to the policy it was made under. A member
//! device puts the digest it acted on in its upload-claim request, and the
//! issuer refuses one that is not the org's current digest (spec 5.3, flow
//! change A), so a policy change cannot race a send.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::trace_contribution::{ConsentScope, TraceAllowedUse};

pub const ORG_POLICY_SCHEMA_VERSION: &str = "trace_commons.org_policy.v1";

/// The tenant namespace for an organization. Not an admission anchor
/// namespace: an org tenant has its own admission arm, which refuses when the
/// org's record is missing.
pub const ORG_TENANT_PREFIX: &str = "org-";

/// The shortest org member session an org may set: fifteen minutes.
pub const ORG_SESSION_LIFETIME_MIN_SECONDS: u32 = 15 * 60;
/// The operator ceiling on an org member session: one day. A member seat's
/// session does not slide; it is replaced only by re-proving membership, so
/// this bounds how long a person removed from the NEAR AI organization keeps
/// uploading.
pub const ORG_SESSION_LIFETIME_MAX_SECONDS: u32 = 24 * 60 * 60;
/// The lifetime the spec suggests (5.3): twelve hours.
pub const ORG_SESSION_LIFETIME_DEFAULT_SECONDS: u32 = 12 * 60 * 60;

/// Whether `tenant_id` is in the org namespace: `org-` followed by 64
/// lowercase hex characters, the shape [`ORG_TENANT_PREFIX`] tenants are made
/// in.
pub fn is_org_tenant(tenant_id: &str) -> bool {
    tenant_id
        .strip_prefix(ORG_TENANT_PREFIX)
        .is_some_and(crate::admission::is_hash)
}

/// Whether `tenant_id` claims the org namespace at all, matched ASCII
/// case-insensitively and with any suffix.
///
/// Broader than [`is_org_tenant`] on purpose. The fail-closed rules key on
/// this one: a tenant that looks like an org but has no org record must be
/// refused, never treated as an ordinary tenant.
pub fn uses_org_namespace(tenant_id: &str) -> bool {
    tenant_id
        .get(..ORG_TENANT_PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(ORG_TENANT_PREFIX))
}

/// Whether members may share at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgSharing {
    Allowed,
    /// Every member device applies its global Never override, and the issuer
    /// refuses every claim from the org's seats.
    Off,
}

/// Whether a member may grant Automatic to a folder. There is no `required`
/// value: the org cannot select Automatic for a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgMemberAutomatic {
    Allowed,
    Forbidden,
}

/// The scrub check before a send. An org can make review stricter, never
/// looser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgScrubCheck {
    Automatic,
    /// Every session waits for the member.
    Manual,
}

/// The NEAR AI spend organization. The only value pins it to the org's bound
/// NEAR AI organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgSpendOrganization {
    Pinned,
}

/// Private AI (NEAR AI routing) on member devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgPrivateAi {
    Allowed,
    RequiredForListedTools,
    Off,
}

/// The org policy document (spec 4.4).
///
/// Server-enforced: `sharing`, `allowed_consent_scopes`, `allowed_uses`,
/// `member_automatic` (through the claim's send mode) and
/// `member_session_lifetime_seconds` (through membership freshness). The
/// rest is applied by the client. The two `member_may_*` flags are a
/// convenience lock, not a control.
///
/// `upstreams`, `server_sources` and `server_roots` (spec 4.4, 7.4) are not
/// in this version. When they arrive they are added with
/// `skip_serializing_if` on their empty value, so no digest of a policy
/// written before them moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgPolicy {
    pub schema_version: String,
    pub sharing: OrgSharing,
    pub allowed_consent_scopes: Vec<ConsentScope>,
    pub allowed_uses: Vec<TraceAllowedUse>,
    pub member_automatic: OrgMemberAutomatic,
    pub scrub_check: OrgScrubCheck,
    pub near_ai_spend_org: OrgSpendOrganization,
    pub private_ai: OrgPrivateAi,
    pub member_session_lifetime_seconds: u32,
    pub member_may_sign_out: bool,
    pub member_may_unenrol: bool,
}

/// Why a policy document was refused. The label is the wire value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgPolicyInvalid {
    SchemaVersion,
    NoConsentScope,
    NoAllowedUse,
    SessionLifetime,
}

impl OrgPolicyInvalid {
    pub fn label(self) -> &'static str {
        match self {
            Self::SchemaVersion => "org_policy_schema_version",
            Self::NoConsentScope => "org_policy_no_consent_scope",
            Self::NoAllowedUse => "org_policy_no_allowed_use",
            Self::SessionLifetime => "org_policy_session_lifetime",
        }
    }
}

impl OrgPolicy {
    /// The policy a new org starts with: sharing allowed for debugging and
    /// evaluation only, Automatic forbidden, scrub check automatic, the spend
    /// org pinned, and the suggested twelve-hour member session. An admin
    /// widens it on purpose, and a widening voids standing member grants.
    pub fn conservative_default() -> Self {
        Self {
            schema_version: ORG_POLICY_SCHEMA_VERSION.to_string(),
            sharing: OrgSharing::Allowed,
            allowed_consent_scopes: vec![
                ConsentScope::DebuggingEvaluation,
                ConsentScope::PublicAttribution,
            ],
            allowed_uses: vec![
                TraceAllowedUse::Debugging,
                TraceAllowedUse::Evaluation,
                TraceAllowedUse::AggregateAnalytics,
            ],
            member_automatic: OrgMemberAutomatic::Forbidden,
            scrub_check: OrgScrubCheck::Automatic,
            near_ai_spend_org: OrgSpendOrganization::Pinned,
            private_ai: OrgPrivateAi::Allowed,
            member_session_lifetime_seconds: ORG_SESSION_LIFETIME_DEFAULT_SECONDS,
            member_may_sign_out: true,
            member_may_unenrol: false,
        }
    }

    /// Sort and de-duplicate the two lists, so equal policies have equal
    /// bytes and equal digests whatever order an admin wrote them in.
    pub fn normalized(mut self) -> Self {
        self.allowed_consent_scopes.sort();
        self.allowed_consent_scopes.dedup();
        self.allowed_uses.sort();
        self.allowed_uses.dedup();
        self
    }

    /// Refuse a document the server cannot enforce. `PublicAttribution` alone
    /// is not a consent scope for traces, so a policy that allows sharing must
    /// name at least one other scope; a policy with sharing off may name none.
    pub fn validate(&self) -> Result<(), OrgPolicyInvalid> {
        if self.schema_version != ORG_POLICY_SCHEMA_VERSION {
            return Err(OrgPolicyInvalid::SchemaVersion);
        }
        if !(ORG_SESSION_LIFETIME_MIN_SECONDS..=ORG_SESSION_LIFETIME_MAX_SECONDS)
            .contains(&self.member_session_lifetime_seconds)
        {
            return Err(OrgPolicyInvalid::SessionLifetime);
        }
        if self.sharing == OrgSharing::Allowed {
            if !self
                .allowed_consent_scopes
                .iter()
                .any(|scope| *scope != ConsentScope::PublicAttribution)
            {
                return Err(OrgPolicyInvalid::NoConsentScope);
            }
            if self.allowed_uses.is_empty() {
                return Err(OrgPolicyInvalid::NoAllowedUse);
            }
        }
        Ok(())
    }

    /// The canonical bytes: the normalized document, serialized field by
    /// field in declaration order. A derived struct needs no
    /// [`crate::canonical_json::canonicalize`] (see that module).
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.clone().normalized()).expect("an OrgPolicy always serializes")
    }

    /// `sha256:` and the lowercase hex SHA-256 of [`Self::canonical_bytes`].
    pub fn digest(&self) -> String {
        format!(
            "sha256:{}",
            hex::encode(Sha256::digest(self.canonical_bytes()))
        )
    }

    /// Whether moving from `previous` to `self` widens what a member's
    /// standing grant could send (spec 4.4): a consent scope or a data use
    /// added, sharing turned back on, Automatic allowed again, or the scrub
    /// check relaxed to automatic. A widening voids every member's standing
    /// grants; a narrowing takes effect without voiding.
    pub fn widens_from(&self, previous: &OrgPolicy) -> bool {
        let scope_added = self
            .allowed_consent_scopes
            .iter()
            .any(|scope| !previous.allowed_consent_scopes.contains(scope));
        let use_added = self
            .allowed_uses
            .iter()
            .any(|use_| !previous.allowed_uses.contains(use_));
        let sharing_on = previous.sharing == OrgSharing::Off && self.sharing == OrgSharing::Allowed;
        let automatic_on = previous.member_automatic == OrgMemberAutomatic::Forbidden
            && self.member_automatic == OrgMemberAutomatic::Allowed;
        let scrub_relaxed = previous.scrub_check == OrgScrubCheck::Manual
            && self.scrub_check == OrgScrubCheck::Automatic;
        scope_added || use_added || sharing_on || automatic_on || scrub_relaxed
    }
}

/// On what authority a trace is sent (spec 5.3, 7.2). A member's own grant,
/// or, for a server seat only, the org's policy. Carried on the upload-claim
/// request and the minted claim; never read from the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentBasis {
    MemberGrant,
    OrgPolicy,
}

impl ConsentBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MemberGrant => "member_grant",
            Self::OrgPolicy => "org_policy",
        }
    }
}

/// How a member's trace was sent: by the member's own Submit, or by a folder
/// the member armed for Automatic. Asserted by the client (spec 3.1 says
/// what that does and does not prove).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgSendMode {
    Submit,
    Automatic,
}

/// What a seat is. A holder seat owns the org's settled credit and has no
/// principal, session or device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgSeatKind {
    Member,
    Server,
    Holder,
}

impl OrgSeatKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Server => "server",
            Self::Holder => "holder",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "member" => Some(Self::Member),
            "server" => Some(Self::Server),
            "holder" => Some(Self::Holder),
            _ => None,
        }
    }

    /// The only consent basis a seat of this kind may send under. A holder
    /// seat sends nothing.
    pub fn consent_basis(self) -> Option<ConsentBasis> {
        match self {
            Self::Member => Some(ConsentBasis::MemberGrant),
            Self::Server => Some(ConsentBasis::OrgPolicy),
            Self::Holder => None,
        }
    }
}

/// A member seat's role in the org. Not a bearer-token role: it is read from
/// the seat on every request to the org console.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgRole {
    Member,
    OrgAdmin,
}

impl OrgRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::OrgAdmin => "org_admin",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "member" => Some(Self::Member),
            "org_admin" => Some(Self::OrgAdmin),
            _ => None,
        }
    }
}

/// An org's lifecycle. Only `active` admits claims and uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgStatus {
    Active,
    Suspended,
    Closing,
}

impl OrgStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Closing => "closing",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "suspended" => Some(Self::Suspended),
            "closing" => Some(Self::Closing),
            _ => None,
        }
    }
}

/// `GET /v1/org/policy`, and the answer to `PUT`.
pub const ORG_POLICY_PATH: &str = "/v1/org/policy";
/// `GET /v1/org`.
pub const ORG_SUMMARY_PATH: &str = "/v1/org";
/// `GET /v1/org/seats`.
pub const ORG_SEATS_PATH: &str = "/v1/org/seats";
/// `GET /v1/org/audit`.
pub const ORG_AUDIT_PATH: &str = "/v1/org/audit";
/// `GET /v1/account/org-policy`: the policy an enrolled member or server
/// device acts on. Any seat of the org may read it.
pub const ACCOUNT_ORG_POLICY_PATH: &str = "/v1/account/org-policy";

/// The org as its admin sees it. Counts and states only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgSummaryResponse {
    pub status: OrgStatus,
    pub policy_version: i64,
    pub policy_digest: String,
    pub seat_limit: i32,
    pub server_limit: i32,
    pub member_seats: i64,
    pub server_seats: i64,
    pub near_ai_org_bound: bool,
}

/// An org's current policy, its version and digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgPolicyResponse {
    pub policy: OrgPolicy,
    pub policy_version: i64,
    pub policy_digest: String,
}

/// `PUT /v1/org/policy`. `expected_version` is the version the admin edited,
/// so two admins cannot silently overwrite each other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgPolicyUpdateRequest {
    pub policy: OrgPolicy,
    pub expected_version: i64,
}

/// The answer to `PUT /v1/org/policy`. `widened` says the change voids
/// every member's standing grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgPolicyUpdateResponse {
    pub policy_version: i64,
    pub policy_digest: String,
    pub widened: bool,
}

/// One seat in the admin's list. No label in this version: labels are
/// sealed at rest and arrive with the sealing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgSeatSummary {
    pub account_id: uuid::Uuid,
    pub kind: OrgSeatKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_role: Option<OrgRole>,
    pub active: bool,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub active_devices: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgSeatsResponse {
    pub seats: Vec<OrgSeatSummary>,
}

/// One row of the org's own audit trail: an action label, an outcome and
/// hash-only metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgAuditEntry {
    pub sequence: i64,
    pub action: String,
    pub outcome: String,
    pub safe_metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgAuditResponse {
    pub entries: Vec<OrgAuditEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_namespace_is_prefix_and_hash() {
        let id = format!("org-{}", "ab".repeat(32));
        assert!(is_org_tenant(&id));
        assert!(uses_org_namespace(&id));
        assert!(!is_org_tenant("org-short"));
        assert!(uses_org_namespace("org-short"));
        assert!(uses_org_namespace("ORG-anything"));
        assert!(!uses_org_namespace("nearai-abc"));
        assert!(!uses_org_namespace("or"));
        assert!(!uses_org_namespace("organic-tenant"));
    }

    #[test]
    fn org_namespace_is_not_an_admission_anchor() {
        let id = format!("org-{}", "ab".repeat(32));
        assert!(!crate::admission::is_anchored_tenant(&id));
        assert!(!crate::admission::uses_anchor_namespace(&id));
    }

    #[test]
    fn default_policy_is_valid_and_forbids_automatic() {
        let policy = OrgPolicy::conservative_default();
        assert_eq!(policy.validate(), Ok(()));
        assert_eq!(policy.member_automatic, OrgMemberAutomatic::Forbidden);
    }

    #[test]
    fn digest_ignores_list_order_and_duplicates() {
        let a = OrgPolicy::conservative_default();
        let mut b = a.clone();
        b.allowed_uses.reverse();
        b.allowed_uses.push(TraceAllowedUse::Debugging);
        b.allowed_consent_scopes.reverse();
        assert_eq!(a.digest(), b.digest());
        assert!(a.digest().starts_with("sha256:"));
        assert_eq!(a.digest().len(), "sha256:".len() + 64);
    }

    #[test]
    fn digest_moves_with_every_field() {
        let base = OrgPolicy::conservative_default();
        let mut changed = Vec::new();
        let mut p = base.clone();
        p.sharing = OrgSharing::Off;
        changed.push(p);
        let mut p = base.clone();
        p.member_automatic = OrgMemberAutomatic::Allowed;
        changed.push(p);
        let mut p = base.clone();
        p.scrub_check = OrgScrubCheck::Manual;
        changed.push(p);
        let mut p = base.clone();
        p.private_ai = OrgPrivateAi::Off;
        changed.push(p);
        let mut p = base.clone();
        p.member_session_lifetime_seconds += 1;
        changed.push(p);
        let mut p = base.clone();
        p.member_may_sign_out = !p.member_may_sign_out;
        changed.push(p);
        let mut p = base.clone();
        p.member_may_unenrol = !p.member_may_unenrol;
        changed.push(p);
        let mut p = base.clone();
        p.allowed_uses.push(TraceAllowedUse::ModelTraining);
        changed.push(p);
        let mut p = base.clone();
        p.allowed_consent_scopes.push(ConsentScope::ModelTraining);
        changed.push(p);
        for policy in changed {
            assert_ne!(policy.digest(), base.digest(), "{policy:?}");
        }
    }

    /// The digest of the default policy is pinned: a change to the field
    /// order, a rename, or a new serialized field moves it, and every member
    /// device's claims would then be refused until it fetched again.
    #[test]
    fn default_policy_bytes_are_pinned() {
        let bytes = OrgPolicy::conservative_default().canonical_bytes();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            concat!(
                r#"{"schema_version":"trace_commons.org_policy.v1","sharing":"allowed","#,
                r#""allowed_consent_scopes":["debugging_evaluation","public_attribution"],"#,
                r#""allowed_uses":["debugging","evaluation","aggregate_analytics"],"#,
                r#""member_automatic":"forbidden","scrub_check":"automatic","#,
                r#""near_ai_spend_org":"pinned","private_ai":"allowed","#,
                r#""member_session_lifetime_seconds":43200,"#,
                r#""member_may_sign_out":true,"member_may_unenrol":false}"#
            )
        );
    }

    #[test]
    fn widening_follows_the_spec_list() {
        let base = OrgPolicy::conservative_default();
        assert!(!base.widens_from(&base));

        let mut more_uses = base.clone();
        more_uses.allowed_uses.push(TraceAllowedUse::ModelTraining);
        assert!(more_uses.widens_from(&base));
        assert!(!base.widens_from(&more_uses), "removing a use narrows");

        let mut more_scopes = base.clone();
        more_scopes
            .allowed_consent_scopes
            .push(ConsentScope::BenchmarkOnly);
        assert!(more_scopes.widens_from(&base));

        let mut off = base.clone();
        off.sharing = OrgSharing::Off;
        assert!(!off.widens_from(&base));
        assert!(base.widens_from(&off));

        let mut automatic = base.clone();
        automatic.member_automatic = OrgMemberAutomatic::Allowed;
        assert!(automatic.widens_from(&base));
        assert!(!base.widens_from(&automatic));

        let mut manual = base.clone();
        manual.scrub_check = OrgScrubCheck::Manual;
        assert!(!manual.widens_from(&base));
        assert!(base.widens_from(&manual));

        let mut shorter = base.clone();
        shorter.member_session_lifetime_seconds = ORG_SESSION_LIFETIME_MIN_SECONDS;
        assert!(!shorter.widens_from(&base));
    }

    #[test]
    fn validation_refuses_what_the_server_cannot_enforce() {
        let mut p = OrgPolicy::conservative_default();
        p.schema_version = "v0".into();
        assert_eq!(p.validate(), Err(OrgPolicyInvalid::SchemaVersion));

        let mut p = OrgPolicy::conservative_default();
        p.allowed_consent_scopes = vec![ConsentScope::PublicAttribution];
        assert_eq!(p.validate(), Err(OrgPolicyInvalid::NoConsentScope));
        p.sharing = OrgSharing::Off;
        assert_eq!(p.validate(), Ok(()), "sharing off needs no scope");

        let mut p = OrgPolicy::conservative_default();
        p.allowed_uses.clear();
        assert_eq!(p.validate(), Err(OrgPolicyInvalid::NoAllowedUse));

        for lifetime in [
            ORG_SESSION_LIFETIME_MIN_SECONDS - 1,
            ORG_SESSION_LIFETIME_MAX_SECONDS + 1,
        ] {
            let mut p = OrgPolicy::conservative_default();
            p.member_session_lifetime_seconds = lifetime;
            assert_eq!(p.validate(), Err(OrgPolicyInvalid::SessionLifetime));
        }
    }

    #[test]
    fn policy_refuses_unknown_fields_and_a_required_automatic() {
        let mut value = serde_json::to_value(OrgPolicy::conservative_default()).unwrap();
        value["ingest_url"] = serde_json::json!("https://elsewhere.example");
        assert!(serde_json::from_value::<OrgPolicy>(value).is_err());

        let mut value = serde_json::to_value(OrgPolicy::conservative_default()).unwrap();
        value["member_automatic"] = serde_json::json!("required");
        assert!(serde_json::from_value::<OrgPolicy>(value).is_err());
    }

    #[test]
    fn seat_kinds_send_under_one_basis_each() {
        assert_eq!(
            OrgSeatKind::Member.consent_basis(),
            Some(ConsentBasis::MemberGrant)
        );
        assert_eq!(
            OrgSeatKind::Server.consent_basis(),
            Some(ConsentBasis::OrgPolicy)
        );
        assert_eq!(OrgSeatKind::Holder.consent_basis(), None);
        for kind in [
            OrgSeatKind::Member,
            OrgSeatKind::Server,
            OrgSeatKind::Holder,
        ] {
            assert_eq!(OrgSeatKind::parse(kind.as_str()), Some(kind));
        }
        for role in [OrgRole::Member, OrgRole::OrgAdmin] {
            assert_eq!(OrgRole::parse(role.as_str()), Some(role));
        }
        for status in [OrgStatus::Active, OrgStatus::Suspended, OrgStatus::Closing] {
            assert_eq!(OrgStatus::parse(status.as_str()), Some(status));
        }
    }
}
