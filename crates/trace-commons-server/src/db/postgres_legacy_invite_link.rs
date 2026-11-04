// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Legacy invite link storage (V81, V91, V104). Every statement runs in the NEAR
//! account's tenant context; the only cross-tenant read is inside
//! `trace_link_legacy_invite`, owned by a NOLOGIN, NOBYPASSRLS guard.

use trace_commons_protocol::legacy_invite_link::{
    LegacyInviteLinkRecord, LegacyInviteLinkRecordKind, LegacyInviteLinkStatement,
};

use super::PgBackend;
use crate::error::DatabaseError;
use crate::legacy_invite_link::{
    ChallengeWrite, LEGACY_INVITE_LINK_MAX_OPEN_CHALLENGES, LinkDbAttempt, LinkDbOutcome,
    LinkRefusal,
};

impl PgBackend {
    pub(super) async fn store_legacy_invite_link_challenge_in_tx(
        &self,
        challenge: &ChallengeWrite,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx =
            Self::begin_trace_tenant_transaction(&mut client, &challenge.account_tenant).await?;
        // Serialize this account's issuance so the cap cannot be raced.
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link_challenge:' || $1::TEXT || ':' || $2::UUID::TEXT, 0))",
            &[&challenge.account_tenant, &challenge.account_id],
        )
        .await?;
        let open: i64 = tx
            .query_one(
                "SELECT count(*) FROM trace_legacy_invite_link_challenges
                  WHERE tenant_id = $1 AND account_id = $2 AND consumed_at IS NULL
                    AND expires_at > $3",
                &[
                    &challenge.account_tenant,
                    &challenge.account_id,
                    &challenge.issued_at,
                ],
            )
            .await?
            .get(0);
        if open >= LEGACY_INVITE_LINK_MAX_OPEN_CHALLENGES {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO trace_legacy_invite_link_challenges
                (tenant_id, account_id, nonce_hash, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &challenge.account_tenant,
                &challenge.account_id,
                &challenge.nonce_hash,
                &challenge.issued_at,
                &challenge.expires_at,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub(super) async fn link_legacy_invite_in_tx(
        &self,
        attempt: &LinkDbAttempt,
    ) -> Result<LinkDbOutcome, DatabaseError> {
        let record = &attempt.record;
        let statement = &record.statement;
        let mut client = self.trace_pool().get().await?;
        let tx =
            Self::begin_trace_tenant_transaction(&mut client, &statement.account_tenant_id).await?;
        let params: [&(dyn tokio_postgres::types::ToSql + Sync); 13] = [
            &statement.account_tenant_id,
            &statement.account_id,
            &statement.legacy_tenant_id,
            &statement.device_key_id,
            &attempt.device_public_key,
            &statement.invite_subject_hash,
            &statement.nonce,
            &statement.issued_at,
            &record.link_id,
            &record.linked_at,
            &record.device_signature,
            &record.server_kid,
            &attempt.server_signature,
        ];
        let row = tx
            .query_one(
                "SELECT outcome, link_id, device_key_id, invite_subject_hash, nonce, issued_at,
                        device_signature, linked_at, server_kid, server_signature, trust_version
                   FROM trace_link_legacy_invite($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                &params,
            )
            .await?;
        let outcome: String = row.get(0);
        // V146, decision 3: an instance-enrolled device has no invite row, so
        // the V81 function answers `device_not_eligible` after spending the
        // challenge. Its sibling links it, in this transaction, without any
        // invite grant or trust change.
        if outcome == "device_not_eligible" {
            let instance = tx
                .query_one(
                    "SELECT outcome, link_id, device_key_id, invite_subject_hash, nonce,
                            issued_at, device_signature, linked_at, server_kid,
                            server_signature, trust_version, authority
                       FROM trace_link_legacy_instance_device($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                    &params,
                )
                .await?;
            let instance_outcome: String = instance.get(0);
            let refusal = match instance_outcome.as_str() {
                // Another device of an instance tenant gets no attestation:
                // only the device that linked it is proven.
                "already_linked"
                    if instance.get::<_, Option<String>>(2).as_deref()
                        != Some(statement.device_key_id.as_str()) =>
                {
                    Some(LinkRefusal::DeviceNotEligible)
                }
                "linked" | "already_linked" => None,
                "challenge_invalid" => Some(LinkRefusal::ChallengeInvalid),
                "account_ineligible" => Some(LinkRefusal::AccountIneligible),
                "device_not_eligible" => Some(LinkRefusal::DeviceNotEligible),
                "tenant_pooled" => Some(LinkRefusal::TenantPooled),
                "tenant_claimed" => Some(LinkRefusal::TenantClaimed),
                _ => Some(LinkRefusal::Unavailable),
            };
            if let Some(refusal) = refusal {
                // Refusals commit: the spent challenge stays spent, and a
                // conflicting claim stays recorded for the operator.
                tx.commit().await?;
                return Ok(LinkDbOutcome::Refused(refusal));
            }
            claim_device_principals(&tx, statement).await?;
            tx.commit().await?;
            return Ok(LinkDbOutcome::Linked {
                record: Box::new(LegacyInviteLinkRecord {
                    link_id: instance.get(1),
                    statement: LegacyInviteLinkStatement {
                        legacy_tenant_id: statement.legacy_tenant_id.clone(),
                        device_key_id: instance.get(2),
                        invite_subject_hash: instance.get(3),
                        account_tenant_id: statement.account_tenant_id.clone(),
                        account_id: statement.account_id,
                        nonce: instance.get(4),
                        issued_at: instance.get(5),
                    },
                    device_signature: instance.get(6),
                    linked_at: instance.get(7),
                    server_kid: instance.get(8),
                    kind: LegacyInviteLinkRecordKind::Link,
                }),
                server_signature: instance.get(9),
                trust_version: instance.get(10),
                authority: instance.get(11),
            });
        }
        // V91/V104: the same account, from another of the tenant's devices.
        // The link function answered with the first device's record, which
        // this device cannot verify; record and return its own attestation
        // instead, countersigned under the attestation domain, in the same
        // transaction that spent its challenge (V104 checks that it did).
        if outcome == "already_linked"
            && row.get::<_, Option<String>>(2).as_deref() != Some(statement.device_key_id.as_str())
        {
            let attested = tx
                .query_one(
                    "SELECT outcome, attestation_id, device_key_id, invite_subject_hash, nonce,
                            issued_at, device_signature, attested_at, server_kid, server_signature
                       FROM trace_attest_legacy_invite_device($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                    &[
                        &statement.account_tenant_id,
                        &statement.account_id,
                        &statement.legacy_tenant_id,
                        &statement.device_key_id,
                        &attempt.device_public_key,
                        &statement.invite_subject_hash,
                        &statement.nonce,
                        &statement.issued_at,
                        &record.link_id,
                        &record.linked_at,
                        &record.device_signature,
                        &record.server_kid,
                        &attempt.attestation_server_signature,
                    ],
                )
                .await?;
            let attested_outcome: String = attested.get(0);
            let refusal = match attested_outcome.as_str() {
                "attested" | "already_attested" => None,
                "challenge_invalid" => Some(LinkRefusal::ChallengeInvalid),
                "account_ineligible" => Some(LinkRefusal::AccountIneligible),
                "device_not_eligible" => Some(LinkRefusal::DeviceNotEligible),
                "tenant_pooled" => Some(LinkRefusal::TenantPooled),
                "invite_revoked" => Some(LinkRefusal::InviteRevoked),
                "tenant_claimed" => Some(LinkRefusal::TenantClaimed),
                "invite_not_linked" => Some(LinkRefusal::InviteNotLinked),
                _ => Some(LinkRefusal::Unavailable),
            };
            let trust_version = row.get::<_, Option<i64>>(10);
            if refusal.is_none() && trust_version.is_some() {
                // V146, decision 5: the attested device's upload credit.
                claim_device_principals(&tx, statement).await?;
            }
            // Refusals commit too: the spent challenge stays spent, and an
            // invite_not_linked refusal stays recorded for the operator.
            tx.commit().await?;
            if let Some(refusal) = refusal {
                return Ok(LinkDbOutcome::Refused(refusal));
            }
            let Some(trust_version) = trust_version else {
                return Ok(LinkDbOutcome::Refused(LinkRefusal::Unavailable));
            };
            return Ok(LinkDbOutcome::Linked {
                record: Box::new(LegacyInviteLinkRecord {
                    link_id: attested.get(1),
                    statement: LegacyInviteLinkStatement {
                        legacy_tenant_id: statement.legacy_tenant_id.clone(),
                        device_key_id: attested.get(2),
                        invite_subject_hash: attested.get(3),
                        account_tenant_id: statement.account_tenant_id.clone(),
                        account_id: statement.account_id,
                        nonce: attested.get(4),
                        issued_at: attested.get(5),
                    },
                    device_signature: attested.get(6),
                    linked_at: attested.get(7),
                    server_kid: attested.get(8),
                    kind: LegacyInviteLinkRecordKind::DeviceAttestation,
                }),
                server_signature: attested.get(9),
                trust_version,
                authority: INVITED.to_string(),
            });
        }
        let refusal = match outcome.as_str() {
            "linked" | "already_linked" => None,
            "challenge_invalid" => Some(LinkRefusal::ChallengeInvalid),
            "account_ineligible" => Some(LinkRefusal::AccountIneligible),
            "tenant_pooled" => Some(LinkRefusal::TenantPooled),
            "invite_revoked" => Some(LinkRefusal::InviteRevoked),
            "tenant_claimed" => Some(LinkRefusal::TenantClaimed),
            _ => Some(LinkRefusal::Unavailable),
        };
        let trust_version = row.get::<_, Option<i64>>(10);
        if refusal.is_none() && trust_version.is_some() {
            // V146, decision 5: the signing device's upload credit.
            claim_device_principals(&tx, statement).await?;
        }
        // Refusals commit too: a spent challenge stays spent, and a conflicting
        // claim stays recorded for the operator.
        tx.commit().await?;
        if let Some(refusal) = refusal {
            return Ok(LinkDbOutcome::Refused(refusal));
        }
        let Some(trust_version) = trust_version else {
            // The link stands but its trust row is gone: an operator change.
            return Ok(LinkDbOutcome::Refused(LinkRefusal::Unavailable));
        };
        Ok(LinkDbOutcome::Linked {
            record: Box::new(LegacyInviteLinkRecord {
                link_id: row.get(1),
                statement: LegacyInviteLinkStatement {
                    legacy_tenant_id: statement.legacy_tenant_id.clone(),
                    device_key_id: row.get(2),
                    invite_subject_hash: row.get(3),
                    account_tenant_id: statement.account_tenant_id.clone(),
                    account_id: statement.account_id,
                    nonce: row.get(4),
                    issued_at: row.get(5),
                },
                device_signature: row.get(6),
                linked_at: row.get(7),
                server_kid: row.get(8),
                kind: LegacyInviteLinkRecordKind::Link,
            }),
            server_signature: row.get(9),
            trust_version,
            authority: INVITED.to_string(),
        })
    }
}

/// The authority a V81 invite link confers.
const INVITED: &str = "invited";

/// V146, owner decision 5 (option A): link the device's signed-upload
/// principals in the legacy tenant to an account, so its past credit is
/// claimable. Runs in the link's transaction, under the NEAR AI account's
/// tenant context; the function restores that context before it returns.
async fn claim_device_principals(
    tx: &deadpool_postgres::Transaction<'_>,
    statement: &LegacyInviteLinkStatement,
) -> Result<i32, DatabaseError> {
    Ok(tx
        .query_one(
            "SELECT trace_legacy_link_claim_device_principals($1, $2, $3, $4)",
            &[
                &statement.account_tenant_id,
                &statement.account_id,
                &statement.legacy_tenant_id,
                &statement.device_key_id,
            ],
        )
        .await?
        .get(0))
}
