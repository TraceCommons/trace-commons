// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Carrying a NEAR AI account's payout link to the legacy accounts it has
//! linked (V146; docs/superpowers/specs/2026-11-03-legacy-account-payout-migration-design.md,
//! section 3.3).
//!
//! The source is the account's live link on the rail, proved by its own
//! `near_ai_login` anchor. For each legacy tenant the account has linked
//! through V81, every open account there gets a `waiting` link to the same
//! sealed recipient (the seal is bound to the rail and recipient, not to a
//! tenant), recorded in `trace_legacy_payout_carries`. Each legacy account
//! is then paid its own credit from its own tenant by the ordinary pass.
//!
//! Runs inside the caller's transaction. It reads the account's tenant
//! links under the account's own tenant context, then sets the context to
//! each legacy tenant in turn (forced RLS applies to every statement) and
//! restores it before returning. Nothing here writes the account's tenant
//! except one hash-only account audit row.

use uuid::Uuid;

use super::postgres_payout::{insert_audit, link_id_hash};
use super::{CarryPrograms, LegacyCarryReport, PayoutAuditAction, PayoutLinkState};
use crate::error::DatabaseError;

/// The audit label of a carried link and of a released carry.
pub const CARRIED_LABEL: &str = "legacy_tenant_link";
pub const RELEASED_LABEL: &str = "legacy_tenant_link_released";

/// A carry's hash of the V81 tenant link it rides on. Mirrored in SQL by
/// the V146 revocation trigger; keep the two together.
pub fn legacy_link_hash(link_id: Uuid) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("legacy-invite-link-v1:{link_id}").as_bytes())
    )
}

/// The actor hash written on the legacy tenant's audit rows for one
/// carried link: derived from that link's own random id, never from the
/// NEAR AI account, so the legacy tenant's rows cannot be joined to the
/// NEAR AI tenant's or to another legacy tenant's (spec section 5; an
/// unkeyed re-hash of the account's actor could be, review of #1865).
pub fn carry_actor_hash(carried_link_id: Uuid) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("legacy-payout-carry-actor-v1:{carried_link_id}").as_bytes())
    )
}

fn anchored_tenant(tenant: &str) -> bool {
    trace_commons_protocol::admission::is_anchored_tenant(tenant)
}

struct Source {
    link_id: Uuid,
    row: tokio_postgres::Row,
}

async fn set_tenant(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
) -> Result<(), DatabaseError> {
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await?;
    Ok(())
}

/// See [`super::PayoutStore::carry_legacy_payout_links`]. `tx` is under
/// `tenant`'s context and is left under it.
pub(super) async fn carry_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
    rail: &str,
    programs: &CarryPrograms,
    actor_ref_hash: &str,
) -> Result<LegacyCarryReport, DatabaseError> {
    let mut report = LegacyCarryReport::default();
    if !anchored_tenant(tenant) {
        return Ok(report);
    }
    // The newest live generation not being unlinked, on an account with a
    // recorded NEAR AI login: the gate proved it, and nothing else may be
    // carried.
    let source = tx
        .query_opt(
            "SELECT l.link_id, l.recipient_kind, l.sealed_recipient, l.recipient_index,
                    l.recipient_key_ref, l.index_pepper_ref, l.subject_index, l.recipient_ref
               FROM trace_payout_links l
              WHERE l.tenant_id = $1 AND l.account_id = $2 AND l.rail_id = $3
                AND l.state IN ('waiting', 'active') AND l.unlink_requested_at IS NULL
                AND EXISTS (SELECT 1 FROM trace_near_account_anchors n
                             WHERE n.tenant_id = l.tenant_id AND n.account_id = l.account_id
                               AND n.identity_source = 'near_ai_login')
              ORDER BY (l.state = 'waiting') DESC, l.created_at DESC
              LIMIT 1",
            &[&tenant, &account, &rail],
        )
        .await?
        .map(|row| Source {
            link_id: row.get(0),
            row,
        });
    let tenant_links = tx
        .query(
            "SELECT link_id, legacy_tenant_id FROM trace_legacy_invite_links
              WHERE tenant_id = $1 AND account_id = $2 AND revoked_at IS NULL
              ORDER BY legacy_tenant_id",
            &[&tenant, &account],
        )
        .await?;
    for tenant_link in tenant_links {
        let tenant_link_id: Uuid = tenant_link.get(0);
        let legacy: String = tenant_link.get(1);
        if anchored_tenant(&legacy) {
            continue;
        }
        // The tenant link's own lock, which its revocation trigger takes
        // too; then re-read it, so a revocation that committed meanwhile
        // is seen and nothing is carried on a revoked link.
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || $1, 0))",
            &[&legacy],
        )
        .await?;
        let still_live = tx
            .query_opt(
                "SELECT 1 FROM trace_legacy_invite_links
                  WHERE link_id = $1 AND revoked_at IS NULL",
                &[&tenant_link_id],
            )
            .await?
            .is_some();
        if !still_live {
            continue;
        }
        let legacy_hash = legacy_link_hash(tenant_link_id);
        set_tenant(tx, &legacy).await?;
        let outcome = carry_tenant(
            tx,
            &legacy,
            &legacy_hash,
            source.as_ref(),
            rail,
            programs,
            &mut report,
        )
        .await;
        set_tenant(tx, tenant).await?;
        outcome?;
    }
    if report != LegacyCarryReport::default() {
        tx.execute(
            "INSERT INTO trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
             VALUES ($1, 'legacy_payout_carried', $2, 'success', $3)",
            &[
                &tenant,
                &actor_ref_hash,
                &serde_json::json!({
                    "carried": report.carried,
                    "released": report.released,
                    "skipped": report.skipped,
                }),
            ],
        )
        .await?;
        tracing::info!(
            carried = report.carried,
            released = report.released,
            skipped = report.skipped,
            "legacy payout carry"
        );
    }
    Ok(report)
}

async fn release(
    tx: &deadpool_postgres::Transaction<'_>,
    link_id: Uuid,
) -> Result<(), DatabaseError> {
    tx.execute(
        "SELECT trace_legacy_payout_release_carry($1, $2, $3)",
        &[&link_id, &carry_actor_hash(link_id), &RELEASED_LABEL],
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn carry_tenant(
    tx: &deadpool_postgres::Transaction<'_>,
    legacy: &str,
    legacy_hash: &str,
    source: Option<&Source>,
    rail: &str,
    programs: &CarryPrograms,
    report: &mut LegacyCarryReport,
) -> Result<(), DatabaseError> {
    // Re-checked on every call: a tenant can be marked pooled after it was
    // linked, and a pooled tenant is never paid (spec section 7).
    let pooled: bool = tx
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM trace_legacy_invite_pooled_tenants WHERE tenant_id = $1)",
            &[&legacy],
        )
        .await?
        .get(0);
    let source_hash = source.map(|source| link_id_hash(source.link_id));
    let accounts = tx
        .query(
            "SELECT account_id, closed_at IS NOT NULL FROM trace_accounts
              WHERE tenant_id = $1 ORDER BY account_id",
            &[&legacy],
        )
        .await?;
    for row in accounts {
        let account: Uuid = row.get(0);
        let closed: bool = row.get(1);
        let live = tx
            .query(
                "SELECT c.link_id, c.source_link_hash, l.state
                   FROM trace_legacy_payout_carries c
                   JOIN trace_payout_links l ON l.tenant_id = c.tenant_id AND l.link_id = c.link_id
                  WHERE c.tenant_id = $1 AND c.account_id = $2 AND c.rail_id = $3
                    AND c.legacy_link_hash = $4 AND c.released_at IS NULL
                  ORDER BY c.carried_at
                  FOR UPDATE OF c",
                &[&legacy, &account, &rail, &legacy_hash],
            )
            .await?;
        let (Some(source), Some(source_hash), false, false) =
            (source, source_hash.as_deref(), pooled, closed)
        else {
            for carry in &live {
                release(tx, carry.get(0)).await?;
                report.released += 1;
            }
            continue;
        };
        if live
            .iter()
            .any(|carry| carry.get::<_, String>(1) == source_hash)
        {
            continue;
        }
        // A link of the account's own (decision 6): never replaced.
        let own: bool = tx
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM trace_payout_links l
                      WHERE l.tenant_id = $1 AND l.account_id = $2 AND l.rail_id = $3
                        AND l.state IN ('waiting', 'active', 'closing')
                        AND NOT EXISTS (SELECT 1 FROM trace_legacy_payout_carries c
                                         WHERE c.tenant_id = l.tenant_id
                                           AND c.link_id = l.link_id))",
                &[&legacy, &account, &rail],
            )
            .await?
            .get(0);
        if own {
            report.skipped += 1;
            tracing::warn!(
                label = "legacy_carry_skipped_own_link",
                "legacy payout carry skipped an account with its own link"
            );
            continue;
        }
        // An older generation is released outright: a waiting one never
        // paid and closes at zero; an active one gets an unlink request, so
        // it stops even if the new carry never activates (a later
        // revocation, unlink or pooled mark finds only the new one). The
        // pass still closes the old one and activates the new.
        for carry in &live {
            release(tx, carry.get(0)).await?;
            report.released += 1;
        }
        let link_id = Uuid::new_v4();
        let program = (programs.0)(link_id);
        let row = &source.row;
        tx.execute(
            "INSERT INTO trace_payout_links (
                 tenant_id, account_id, rail_id, link_id, recipient_kind, state,
                 sealed_recipient, recipient_index, recipient_key_ref,
                 index_pepper_ref, subject_index, program, recipient_ref
             ) VALUES ($1, $2, $3, $4, $5, 'waiting', $6, $7, $8, $9, $10, $11, $12)",
            &[
                &legacy,
                &account,
                &rail,
                &link_id,
                &row.get::<_, String>(1),
                &row.get::<_, serde_json::Value>(2),
                &row.get::<_, String>(3),
                &row.get::<_, String>(4),
                &row.get::<_, String>(5),
                &row.get::<_, String>(6),
                &program,
                &row.get::<_, Option<String>>(7),
            ],
        )
        .await?;
        tx.execute(
            "INSERT INTO trace_legacy_payout_carries
                 (tenant_id, account_id, rail_id, link_id, legacy_link_hash, source_link_hash)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &legacy,
                &account,
                &rail,
                &link_id,
                &legacy_hash,
                &source_hash,
            ],
        )
        .await?;
        insert_audit(
            tx,
            legacy,
            rail,
            PayoutAuditAction::LinkCreated,
            &carry_actor_hash(link_id),
            Some(account),
            Some(link_id),
            None,
            Some(PayoutLinkState::Waiting),
            Some(CARRIED_LABEL),
        )
        .await?;
        report.carried += 1;
    }
    Ok(())
}
