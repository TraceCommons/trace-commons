// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Organizations (V159): the org record, its seats and devices, the policy
//! with its tenant-policy mirror, seat removal, and the org's audit trail.
//!
//! Every read and write runs in one transaction under the org tenant's RLS
//! context. Reads never create a tenant row, so a probe for an unknown org
//! tenant leaves nothing behind. Audit rows are written in the same
//! transaction as the change they record, with hash-only metadata.

use chrono::{DateTime, Utc};
use tokio_postgres::Row;
use trace_commons_protocol::org_policy::{
    OrgAuditEntry, OrgPolicy, OrgRole, OrgSeatKind, OrgStatus,
};
use uuid::Uuid;

use super::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::org::{OrgRecord, OrgSeatListing, OrgSeatRecord, org_actor_ref_hash, org_seat_ref_hash};

/// A new org, as the operator creates it (spec 5.2, step 1).
#[derive(Debug, Clone)]
pub struct NewOrg {
    pub tenant_id: String,
    pub policy: OrgPolicy,
    pub seat_limit: i32,
    pub server_limit: i32,
    pub settlement_cap_micros: Option<i64>,
    /// Who created it, for the audit row's `actor_ref`.
    pub actor_ref: String,
}

/// The org a [`NewOrg`] became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedOrg {
    pub tenant_id: String,
    pub holder_account_id: Uuid,
    pub policy_digest: String,
}

/// A seat to admit (or find again by its `identity_index`).
#[derive(Debug, Clone)]
pub struct OrgSeatAdmission {
    pub tenant_id: String,
    pub kind: OrgSeatKind,
    pub org_role: Option<OrgRole>,
    pub identity_index: String,
    pub authority_id: Option<Uuid>,
    pub membership_proved_at: Option<DateTime<Utc>>,
    pub actor_ref: String,
}

/// What admitting a seat did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgSeatAdmitOutcome {
    /// The seat exists: newly created, or found again by its identity, with
    /// its membership proof refreshed when one was given.
    Admitted { account_id: Uuid, created: bool },
    /// No org, or an org that is not active.
    OrgUnavailable,
    /// The org's member or server limit is reached.
    SeatLimitReached,
    /// The identity's seat was removed. A removed seat is never revived; the
    /// admin admits the person again only by the operator.
    SeatRemoved,
    /// The identity holds a seat of another kind.
    SeatKindMismatch,
}

/// What a policy write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgPolicyUpdateOutcome {
    Updated {
        policy_version: i64,
        policy_digest: String,
        widened: bool,
    },
    /// `expected_version` is not the current version: someone else wrote
    /// first.
    VersionConflict {
        current_version: i64,
    },
    OrgMissing,
}

/// What removing a seat did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgSeatRemoveOutcome {
    Removed {
        devices_revoked: u64,
        grants_revoked: u64,
        sessions_revoked: u64,
    },
    /// No such seat, or one already removed. Answered like an unknown id.
    NotFound,
    /// The holder seat is not a person or a machine and cannot be removed.
    HolderSeat,
    /// The org's last active admin. A sole admin is replaced by the operator.
    LastAdmin,
}

const SEAT_COLUMNS: &str = "s.account_id, s.kind, s.org_role, s.membership_proved_at,
        s.joined_at, s.removed_at,
        (s.kind = 'member' AND au.kind IS DISTINCT FROM 'enrolment_token')
            AS requires_membership_proof";

const SEAT_FROM: &str = "trace_org_seats s
   LEFT JOIN trace_org_enrolment_authorities au
     ON au.tenant_id = s.tenant_id AND au.authority_id = s.authority_id";

fn seat_from_row(row: &Row) -> Result<OrgSeatRecord, DatabaseError> {
    let kind: String = row.get("kind");
    let kind = OrgSeatKind::parse(&kind)
        .ok_or_else(|| DatabaseError::Serialization("org seat kind unknown".into()))?;
    let org_role = row
        .get::<_, Option<String>>("org_role")
        .map(|role| {
            OrgRole::parse(&role)
                .ok_or_else(|| DatabaseError::Serialization("org seat role unknown".into()))
        })
        .transpose()?;
    Ok(OrgSeatRecord {
        account_id: row.get("account_id"),
        kind,
        org_role,
        membership_proved_at: row.get("membership_proved_at"),
        requires_membership_proof: row.get("requires_membership_proof"),
        joined_at: row.get("joined_at"),
        removed_at: row.get("removed_at"),
    })
}

fn org_from_row(tenant_id: &str, row: &Row) -> Result<OrgRecord, DatabaseError> {
    let status: String = row.get("status");
    let status = OrgStatus::parse(&status)
        .ok_or_else(|| DatabaseError::Serialization("org status unknown".into()))?;
    let policy: serde_json::Value = row.get("policy");
    // A stored policy this build cannot read refuses, never defaults.
    let policy: OrgPolicy = serde_json::from_value(policy)
        .map_err(|_| DatabaseError::Serialization("org policy unreadable".into()))?;
    Ok(OrgRecord {
        tenant_id: tenant_id.to_string(),
        status,
        holder_account_id: row.get("holder_account_id"),
        holder_account_open: row.get("holder_account_open"),
        near_ai_org_bound: row.get("near_ai_org_bound"),
        policy,
        policy_version: row.get("policy_version"),
        policy_digest: row.get("policy_digest"),
        seat_limit: row.get("seat_limit"),
        server_limit: row.get("server_limit"),
        settlement_cap_micros: row.get("settlement_cap_micros"),
    })
}

/// Write the parts of the org policy the submission path enforces into the
/// tenant policy, inside the caller's transaction. Sharing off keeps the
/// lists (an empty list there means "no restriction"); sharing is enforced by
/// the issuer and at landing instead.
async fn mirror_tenant_policy(
    tx: &deadpool_postgres::Transaction<'_>,
    policy: &OrgPolicy,
    policy_version: i64,
    actor_ref: &str,
) -> Result<(), DatabaseError> {
    let scopes = serde_json::to_value(&policy.allowed_consent_scopes)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let uses = serde_json::to_value(&policy.allowed_uses)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    tx.execute(
        "INSERT INTO trace_tenant_policies (
            tenant_id, policy_version, allowed_consent_scopes, allowed_uses,
            updated_by_principal_ref
         ) VALUES (trace_current_tenant_id(), $1, $2, $3, $4)
         ON CONFLICT (tenant_id) DO UPDATE SET
            policy_version = excluded.policy_version,
            allowed_consent_scopes = excluded.allowed_consent_scopes,
            allowed_uses = excluded.allowed_uses,
            updated_by_principal_ref = excluded.updated_by_principal_ref,
            updated_at = now()",
        &[
            &format!("org-policy-v{policy_version}"),
            &scopes,
            &uses,
            &actor_ref,
        ],
    )
    .await?;
    Ok(())
}

async fn append_org_audit(
    tx: &deadpool_postgres::Transaction<'_>,
    action: &str,
    actor_ref: &str,
    outcome: &str,
    safe_metadata: serde_json::Value,
) -> Result<(), DatabaseError> {
    debug_assert!(action.starts_with("org_"));
    tx.execute(
        "INSERT INTO trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
         VALUES (trace_current_tenant_id(), $1, $2, $3, $4)",
        &[&action, &actor_ref, &outcome, &safe_metadata],
    )
    .await?;
    Ok(())
}

async fn org_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant_id: &str,
    for_update: bool,
) -> Result<Option<OrgRecord>, DatabaseError> {
    let lock = if for_update { " FOR UPDATE OF o" } else { "" };
    let row = tx
        .query_opt(
            &format!(
                "SELECT o.status, o.holder_account_id,
                        (a.account_id IS NOT NULL AND a.closed_at IS NULL) AS holder_account_open,
                        o.near_ai_org_index IS NOT NULL AS near_ai_org_bound,
                        o.policy, o.policy_version, o.policy_digest,
                        o.seat_limit, o.server_limit, o.settlement_cap_micros
                   FROM trace_orgs o
                   LEFT JOIN trace_accounts a
                     ON a.tenant_id = o.tenant_id AND a.account_id = o.holder_account_id
                  WHERE o.tenant_id = trace_current_tenant_id(){lock}"
            ),
            &[],
        )
        .await?;
    row.map(|row| org_from_row(tenant_id, &row)).transpose()
}

impl PgBackend {
    async fn org_tx_client(&self) -> Result<deadpool_postgres::Client, DatabaseError> {
        Ok(self.trace_pool().get().await?)
    }

    pub async fn get_org(&self, tenant_id: &str) -> Result<Option<OrgRecord>, DatabaseError> {
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let org = org_in_tx(&tx, tenant_id, false).await?;
        tx.commit().await?;
        Ok(org)
    }

    pub async fn get_org_seat(
        &self,
        tenant_id: &str,
        account_id: Uuid,
    ) -> Result<Option<OrgSeatRecord>, DatabaseError> {
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                &format!(
                    "SELECT {SEAT_COLUMNS} FROM {SEAT_FROM}
                      WHERE s.tenant_id = trace_current_tenant_id() AND s.account_id = $1"
                ),
                &[&account_id],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(seat_from_row).transpose()
    }

    /// The seat a registered device key belongs to.
    pub async fn get_org_seat_for_device(
        &self,
        tenant_id: &str,
        device_key_id: &str,
    ) -> Result<Option<OrgSeatRecord>, DatabaseError> {
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                &format!(
                    "SELECT {SEAT_COLUMNS} FROM {SEAT_FROM}
                       JOIN trace_org_seat_devices d
                         ON d.tenant_id = s.tenant_id AND d.account_id = s.account_id
                      WHERE s.tenant_id = trace_current_tenant_id()
                        AND d.device_key_id = $1
                        AND (d.expires_at IS NULL OR d.expires_at > now())"
                ),
                &[&device_key_id],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(seat_from_row).transpose()
    }

    /// The seat whose account owns `principal_ref`: a device principal, or an
    /// upload principal bound to one (`trace_account_owned_principals`).
    pub async fn get_org_seat_for_principal(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> Result<Option<OrgSeatRecord>, DatabaseError> {
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                &format!(
                    "SELECT {SEAT_COLUMNS} FROM {SEAT_FROM}
                       JOIN trace_account_owned_principals p
                         ON p.tenant_id = s.tenant_id AND p.account_id = s.account_id
                      WHERE s.tenant_id = trace_current_tenant_id()
                        AND p.principal_ref = $1
                      LIMIT 1"
                ),
                &[&principal_ref],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(seat_from_row).transpose()
    }

    pub async fn list_org_seats(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<OrgSeatListing>, DatabaseError> {
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                &format!(
                    "SELECT {SEAT_COLUMNS},
                            (SELECT count(*) FROM trace_org_seat_devices d
                               JOIN device_keys k ON k.device_key_id = d.device_key_id
                              WHERE d.tenant_id = s.tenant_id
                                AND d.account_id = s.account_id
                                AND k.revoked_at IS NULL
                                AND (d.expires_at IS NULL OR d.expires_at > now()))
                                AS active_devices
                       FROM {SEAT_FROM}
                      WHERE s.tenant_id = trace_current_tenant_id()
                        AND s.kind <> 'holder'
                      ORDER BY s.joined_at, s.account_id"
                ),
                &[],
            )
            .await?;
        tx.commit().await?;
        rows.iter()
            .map(|row| {
                Ok(OrgSeatListing {
                    seat: seat_from_row(row)?,
                    active_devices: row.get("active_devices"),
                })
            })
            .collect()
    }

    /// Create an org: its tenant, the `trace_orgs` row, the holder account and
    /// seat, and the tenant-policy mirror, in one transaction. The tenant must
    /// be new and in the org namespace.
    pub async fn create_org(&self, new: NewOrg) -> Result<CreatedOrg, DatabaseError> {
        if !trace_commons_protocol::org_policy::is_org_tenant(&new.tenant_id) {
            return Err(DatabaseError::Constraint("org_tenant_id_invalid".into()));
        }
        new.policy
            .validate()
            .map_err(|invalid| DatabaseError::Constraint(invalid.label().into()))?;
        let policy = new.policy.normalized();
        let policy_json = serde_json::to_value(&policy)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let policy_digest = policy.digest();
        let holder_account_id = Uuid::new_v4();
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &new.tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES (trace_current_tenant_id())",
            &[],
        )
        .await?;
        tx.execute(
            "INSERT INTO trace_orgs (tenant_id, policy, policy_digest, seat_limit,
                                     server_limit, settlement_cap_micros)
             VALUES (trace_current_tenant_id(), $1, $2, $3, $4, $5)",
            &[
                &policy_json,
                &policy_digest,
                &new.seat_limit,
                &new.server_limit,
                &new.settlement_cap_micros,
            ],
        )
        .await?;
        tx.execute(
            "INSERT INTO trace_accounts (tenant_id, account_id)
             VALUES (trace_current_tenant_id(), $1)",
            &[&holder_account_id],
        )
        .await?;
        tx.execute(
            "INSERT INTO trace_org_seats (tenant_id, account_id, kind)
             VALUES (trace_current_tenant_id(), $1, 'holder')",
            &[&holder_account_id],
        )
        .await?;
        tx.execute(
            "UPDATE trace_orgs SET holder_account_id = $1, updated_at = now()
              WHERE tenant_id = trace_current_tenant_id()",
            &[&holder_account_id],
        )
        .await?;
        mirror_tenant_policy(&tx, &policy, 1, &new.actor_ref).await?;
        append_org_audit(
            &tx,
            "org_created",
            &new.actor_ref,
            "success",
            serde_json::json!({
                "policy_digest": policy_digest,
                "seat_limit": new.seat_limit,
                "server_limit": new.server_limit,
                "settlement_cap_configured": new.settlement_cap_micros.is_some(),
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(CreatedOrg {
            tenant_id: new.tenant_id,
            holder_account_id,
            policy_digest,
        })
    }

    /// Admit a member or server seat, or find it again by its identity. A new
    /// seat gets its own account. A found member seat's membership proof is
    /// refreshed when one is given.
    pub async fn admit_org_seat(
        &self,
        admission: OrgSeatAdmission,
    ) -> Result<OrgSeatAdmitOutcome, DatabaseError> {
        if admission.kind == OrgSeatKind::Holder
            || (admission.kind == OrgSeatKind::Member) != admission.org_role.is_some()
        {
            return Err(DatabaseError::Constraint("org_seat_shape_invalid".into()));
        }
        let consent_basis = admission
            .kind
            .consent_basis()
            .map(|basis| basis.as_str())
            .ok_or_else(|| DatabaseError::Constraint("org_seat_shape_invalid".into()))?;
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &admission.tenant_id).await?;
        let Some(org) = org_in_tx(&tx, &admission.tenant_id, true).await? else {
            tx.commit().await?;
            return Ok(OrgSeatAdmitOutcome::OrgUnavailable);
        };
        if org.status != OrgStatus::Active {
            tx.commit().await?;
            return Ok(OrgSeatAdmitOutcome::OrgUnavailable);
        }
        let existing = tx
            .query_opt(
                "SELECT account_id, kind, removed_at FROM trace_org_seats
                  WHERE tenant_id = trace_current_tenant_id() AND identity_index = $1
                  FOR UPDATE",
                &[&admission.identity_index],
            )
            .await?;
        if let Some(row) = existing {
            let account_id: Uuid = row.get("account_id");
            let kind: String = row.get("kind");
            let removed_at: Option<DateTime<Utc>> = row.get("removed_at");
            if removed_at.is_some() {
                tx.commit().await?;
                return Ok(OrgSeatAdmitOutcome::SeatRemoved);
            }
            if kind != admission.kind.as_str() {
                tx.commit().await?;
                return Ok(OrgSeatAdmitOutcome::SeatKindMismatch);
            }
            if let Some(proved_at) = admission.membership_proved_at {
                tx.execute(
                    "UPDATE trace_org_seats
                        SET membership_proved_at = GREATEST(membership_proved_at, $2)
                      WHERE tenant_id = trace_current_tenant_id()
                        AND account_id = $1 AND kind = 'member'",
                    &[&account_id, &proved_at],
                )
                .await?;
            }
            tx.commit().await?;
            return Ok(OrgSeatAdmitOutcome::Admitted {
                account_id,
                created: false,
            });
        }
        let limit = match admission.kind {
            OrgSeatKind::Member => org.seat_limit,
            _ => org.server_limit,
        };
        let seated: i64 = tx
            .query_one(
                "SELECT count(*) FROM trace_org_seats
                  WHERE tenant_id = trace_current_tenant_id()
                    AND kind = $1 AND removed_at IS NULL",
                &[&admission.kind.as_str()],
            )
            .await?
            .get(0);
        if seated >= i64::from(limit) {
            tx.commit().await?;
            return Ok(OrgSeatAdmitOutcome::SeatLimitReached);
        }
        let account_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO trace_accounts (tenant_id, account_id)
             VALUES (trace_current_tenant_id(), $1)",
            &[&account_id],
        )
        .await?;
        tx.execute(
            "INSERT INTO trace_org_seats (tenant_id, account_id, kind, org_role, consent_basis,
                                          identity_index, authority_id, membership_proved_at)
             VALUES (trace_current_tenant_id(), $1, $2, $3, $4, $5, $6, $7)",
            &[
                &account_id,
                &admission.kind.as_str(),
                &admission.org_role.map(OrgRole::as_str),
                &consent_basis,
                &admission.identity_index,
                &admission.authority_id,
                &admission.membership_proved_at,
            ],
        )
        .await?;
        if let Some(authority_id) = admission.authority_id {
            tx.execute(
                "UPDATE trace_org_enrolment_authorities
                    SET seats_admitted = seats_admitted + 1
                  WHERE tenant_id = trace_current_tenant_id() AND authority_id = $1",
                &[&authority_id],
            )
            .await?;
        }
        append_org_audit(
            &tx,
            "org_seat_admitted",
            &admission.actor_ref,
            "success",
            serde_json::json!({
                "seat_ref": org_seat_ref_hash(&admission.tenant_id, account_id),
                "kind": admission.kind.as_str(),
                "org_admin": admission.org_role == Some(OrgRole::OrgAdmin),
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(OrgSeatAdmitOutcome::Admitted {
            account_id,
            created: true,
        })
    }

    /// Record a fresh membership proof for a member seat (spec 5.3). Never
    /// moves the proof backwards. Returns whether an active member seat was
    /// updated.
    pub async fn record_org_membership_proof(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        proved_at: DateTime<Utc>,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let updated = tx
            .execute(
                "UPDATE trace_org_seats
                    SET membership_proved_at = GREATEST(membership_proved_at, $2)
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND kind = 'member'
                    AND removed_at IS NULL",
                &[&account_id, &proved_at],
            )
            .await?;
        tx.commit().await?;
        Ok(updated > 0)
    }

    /// Link a registered device key to an active seat, and its device
    /// principal to the seat's account, so the issuer finds the seat from the
    /// key and settlement and landing find it from the principal.
    pub async fn register_org_seat_device(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        device_key_id: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<bool, DatabaseError> {
        let principal_ref = crate::account_trust::device_principal_ref(tenant_id, device_key_id);
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // The key must be registered to this tenant and live, and the seat
        // must be active and able to send.
        let eligible = tx
            .query_opt(
                "SELECT 1 FROM device_keys k, trace_org_seats s
                  WHERE k.device_key_id = $2 AND k.tenant_id = trace_current_tenant_id()
                    AND k.revoked_at IS NULL
                    AND s.tenant_id = trace_current_tenant_id() AND s.account_id = $1
                    AND s.removed_at IS NULL AND s.kind <> 'holder'",
                &[&account_id, &device_key_id],
            )
            .await?;
        if eligible.is_none() {
            tx.commit().await?;
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO trace_org_seat_devices (tenant_id, device_key_id, account_id,
                                                 principal_ref, expires_at)
             VALUES (trace_current_tenant_id(), $1, $2, $3, $4)",
            &[&device_key_id, &account_id, &principal_ref, &expires_at],
        )
        .await?;
        tx.execute(
            "INSERT INTO trace_account_principals (tenant_id, account_id, principal_ref)
             VALUES (trace_current_tenant_id(), $1, $2)",
            &[&account_id, &principal_ref],
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Write a new policy version (spec 4.4, 8). Refuses a stale
    /// `expected_version`. Updates the tenant-policy mirror and appends one
    /// audit row, flagged when the change widens.
    pub async fn update_org_policy(
        &self,
        tenant_id: &str,
        expected_version: i64,
        policy: &OrgPolicy,
        actor_ref: &str,
    ) -> Result<OrgPolicyUpdateOutcome, DatabaseError> {
        policy
            .validate()
            .map_err(|invalid| DatabaseError::Constraint(invalid.label().into()))?;
        let policy = policy.clone().normalized();
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let Some(org) = org_in_tx(&tx, tenant_id, true).await? else {
            tx.commit().await?;
            return Ok(OrgPolicyUpdateOutcome::OrgMissing);
        };
        if org.policy_version != expected_version {
            tx.commit().await?;
            return Ok(OrgPolicyUpdateOutcome::VersionConflict {
                current_version: org.policy_version,
            });
        }
        let widened = policy.widens_from(&org.policy);
        let policy_digest = policy.digest();
        let policy_version = org.policy_version + 1;
        let policy_json = serde_json::to_value(&policy)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        tx.execute(
            "UPDATE trace_orgs
                SET policy = $1, policy_version = $2, policy_digest = $3, updated_at = now()
              WHERE tenant_id = trace_current_tenant_id()",
            &[&policy_json, &policy_version, &policy_digest],
        )
        .await?;
        mirror_tenant_policy(&tx, &policy, policy_version, actor_ref).await?;
        append_org_audit(
            &tx,
            "org_policy_changed",
            actor_ref,
            "success",
            serde_json::json!({
                "policy_version": policy_version,
                "policy_digest": policy_digest,
                "previous_policy_digest": org.policy_digest,
                "widened": widened,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(OrgPolicyUpdateOutcome::Updated {
            policy_version,
            policy_digest,
            widened,
        })
    }

    /// Remove a member or server seat (spec 5.6): mark it removed, revoke its
    /// device keys and its principals' active grants. A member seat's
    /// sessions stay, because a removed member keeps a withdraw-only window
    /// (`org::removed_seat_may_withdraw`); a server seat's sessions end.
    pub async fn remove_org_seat(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        actor_ref: &str,
    ) -> Result<OrgSeatRemoveOutcome, DatabaseError> {
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // Lock the org's admin seats first, so two admins removing each other
        // at once cannot both pass the last-admin check.
        let admins: Vec<Uuid> = tx
            .query(
                "SELECT account_id FROM trace_org_seats
                  WHERE tenant_id = trace_current_tenant_id()
                    AND org_role = 'org_admin' AND removed_at IS NULL
                  ORDER BY account_id
                  FOR UPDATE",
                &[],
            )
            .await?
            .iter()
            .map(|row| row.get(0))
            .collect();
        let seat = tx
            .query_opt(
                "SELECT kind FROM trace_org_seats
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1 AND removed_at IS NULL
                  FOR UPDATE",
                &[&account_id],
            )
            .await?;
        let Some(seat) = seat else {
            tx.commit().await?;
            return Ok(OrgSeatRemoveOutcome::NotFound);
        };
        let kind: String = seat.get(0);
        if kind == OrgSeatKind::Holder.as_str() {
            tx.commit().await?;
            return Ok(OrgSeatRemoveOutcome::HolderSeat);
        }
        if admins.contains(&account_id) && admins.len() == 1 {
            tx.commit().await?;
            return Ok(OrgSeatRemoveOutcome::LastAdmin);
        }
        let removed_by_ref = org_actor_ref_hash(actor_ref);
        tx.execute(
            "UPDATE trace_org_seats SET removed_at = now(), removed_by_ref = $2
              WHERE tenant_id = trace_current_tenant_id() AND account_id = $1",
            &[&account_id, &removed_by_ref],
        )
        .await?;
        let devices_revoked = tx
            .execute(
                "UPDATE device_keys k SET revoked_at = now()
                   FROM trace_org_seat_devices d
                  WHERE d.tenant_id = trace_current_tenant_id()
                    AND d.account_id = $1
                    AND k.device_key_id = d.device_key_id
                    AND k.tenant_id = d.tenant_id
                    AND k.revoked_at IS NULL",
                &[&account_id],
            )
            .await?;
        let grants_revoked = tx
            .execute(
                "UPDATE trace_tenant_access_grants
                    SET status = 'revoked', revoked_at = now(),
                        revoked_by_principal_ref = $2,
                        reason = 'org_seat_removed', updated_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND status = 'active'
                    AND principal_ref IN (
                        SELECT principal_ref FROM trace_account_principals
                         WHERE tenant_id = trace_current_tenant_id()
                           AND account_id = $1 AND unlinked_at IS NULL)",
                &[&account_id, &actor_ref],
            )
            .await?;
        let sessions_revoked = if kind == OrgSeatKind::Server.as_str() {
            tx.execute(
                "UPDATE trace_sessions SET revoked_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1 AND revoked_at IS NULL",
                &[&account_id],
            )
            .await?
        } else {
            0
        };
        append_org_audit(
            &tx,
            "org_seat_removed",
            actor_ref,
            "success",
            serde_json::json!({
                "seat_ref": org_seat_ref_hash(tenant_id, account_id),
                "kind": kind,
                "devices_revoked": devices_revoked,
                "grants_revoked": grants_revoked,
                "sessions_revoked": sessions_revoked,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(OrgSeatRemoveOutcome::Removed {
            devices_revoked,
            grants_revoked,
            sessions_revoked,
        })
    }

    /// Append an org audit row on its own (a read of the per-seat report, for
    /// one). `action` must be an `org_` label.
    pub async fn append_org_audit_row(
        &self,
        tenant_id: &str,
        action: &str,
        actor_ref: &str,
        outcome: &str,
        safe_metadata: serde_json::Value,
    ) -> Result<(), DatabaseError> {
        if !action.starts_with("org_") {
            return Err(DatabaseError::Constraint("org_audit_action_invalid".into()));
        }
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        append_org_audit(&tx, action, actor_ref, outcome, safe_metadata).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The org's own audit rows, newest first: `org_` actions only, without
    /// the actor reference.
    pub async fn list_org_audit(
        &self,
        tenant_id: &str,
        limit: i64,
    ) -> Result<Vec<OrgAuditEntry>, DatabaseError> {
        let mut client = self.org_tx_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT audit_sequence, action, outcome, safe_metadata, created_at
                   FROM trace_account_audit
                  WHERE tenant_id = trace_current_tenant_id()
                    AND action LIKE 'org\\_%'
                  ORDER BY audit_sequence DESC
                  LIMIT $1",
                &[&limit.clamp(1, 500)],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .iter()
            .map(|row| OrgAuditEntry {
                sequence: row.get("audit_sequence"),
                action: row.get("action"),
                outcome: row.get("outcome"),
                safe_metadata: row.get("safe_metadata"),
                created_at: row.get("created_at"),
            })
            .collect())
    }
}
