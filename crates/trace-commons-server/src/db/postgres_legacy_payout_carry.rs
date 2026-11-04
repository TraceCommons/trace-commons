// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Carrying a NEAR AI account's payout link to the legacy accounts it has
//! linked (V146; docs/superpowers/specs/2026-11-03-legacy-account-payout-migration-design.md,
//! section 3.3).
//!
//! The source is the account's live link on the rail, proved by its own
//! `near_ai_login` anchor, on an open account. For each legacy tenant the
//! account has linked through V81, each open account a signed device proved
//! (V148 `trace_legacy_payout_proven_accounts`) gets a `waiting` link to the
//! same sealed recipient (the seal is bound to the rail and recipient, not
//! to a tenant), recorded in `trace_legacy_payout_carries`. Each legacy
//! account is then paid its own credit from its own tenant by the ordinary
//! pass.
//!
//! Carries run only at events (owner, 2026-11-03): the NEAR AI account's
//! confirm and a tenant link ([`carry_in_tx`]), and an account opening in a
//! linked legacy tenant ([`carry_opened_account_in_tx`]). The pass never
//! carries.
//!
//! Locks, in one global order: the source's confirm locks, by rail; then the
//! tenant-link locks, by legacy tenant. The revocation and close triggers
//! take only tenant-link locks. Every check that decides a carry (the tenant
//! link live, the source open) is re-read after the tenant-link lock.
//!
//! Runs inside the caller's transaction, setting the tenant context to each
//! tenant it touches (forced RLS applies to every statement) and restoring
//! it before returning. Nothing here writes the source's tenant except one
//! hash-only account audit row.

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
    let source = source_link(tx, tenant, account, rail).await?;
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
        // The source, too, is re-read under the lock: a close that committed
        // in between released its carries, and they must not come back
        // (review of #1879). A close that commits after this waits on the
        // lock in its trigger and then releases what this writes.
        if !still_live || !source_open(tx, tenant, account).await? {
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
            None,
            &mut report,
        )
        .await;
        set_tenant(tx, tenant).await?;
        outcome?;
    }
    audit_carry(tx, tenant, actor_ref_hash, &report).await?;
    Ok(report)
}

/// The NEAR AI tenant's account audit row for a carry, written only when
/// it changed something (review of #1879): an account keeping its own link
/// is skipped at every event, and that is no news. `tx` is under `tenant`.
async fn audit_carry(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    actor_ref_hash: &str,
    report: &LegacyCarryReport,
) -> Result<(), DatabaseError> {
    if report.carried + report.released > 0 {
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
    Ok(())
}

/// The newest live generation of the account's link on `rail` not being
/// unlinked, on an open account with a recorded NEAR AI login: the gate
/// proved it, and nothing else may be carried. `tx` is under `tenant`.
async fn source_link(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
    rail: &str,
) -> Result<Option<Source>, DatabaseError> {
    Ok(tx
        .query_opt(
            "SELECT l.link_id, l.recipient_kind, l.sealed_recipient, l.recipient_index,
                    l.recipient_key_ref, l.index_pepper_ref, l.subject_index, l.recipient_ref
               FROM trace_payout_links l
              WHERE l.tenant_id = $1 AND l.account_id = $2 AND l.rail_id = $3
                AND l.state IN ('waiting', 'active') AND l.unlink_requested_at IS NULL
                AND EXISTS (SELECT 1 FROM trace_accounts a
                             WHERE a.tenant_id = l.tenant_id AND a.account_id = l.account_id
                               AND a.closed_at IS NULL)
                AND EXISTS (SELECT 1 FROM trace_near_account_anchors n
                             WHERE n.tenant_id = l.tenant_id AND n.account_id = l.account_id
                               AND n.identity_source = 'near_ai_login'
                               AND n.retired_at IS NULL)
              ORDER BY (l.state = 'waiting') DESC, l.created_at DESC
              LIMIT 1",
            &[&tenant, &account, &rail],
        )
        .await?
        .map(|row| Source {
            link_id: row.get(0),
            row,
        }))
}

/// Whether the source account is open. `tx` is under `tenant`.
async fn source_open(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
) -> Result<bool, DatabaseError> {
    Ok(tx
        .query_opt(
            "SELECT 1 FROM trace_accounts
              WHERE tenant_id = $1 AND account_id = $2 AND closed_at IS NULL",
            &[&tenant, &account],
        )
        .await?
        .is_some())
}

/// The account-opening event (#1873; owner, 2026-11-03): `account` was just
/// created in legacy tenant `legacy`. Carry the linked NEAR AI account's
/// live link on each of `rails` to that account alone, if a signed device
/// proved it. `tx` is under `legacy`'s context and is left under it. Locks
/// in the global order: the source's confirm locks by rail, then this
/// tenant's link lock.
pub(super) async fn carry_opened_account_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    legacy: &str,
    account: Uuid,
    opened: bool,
    rails: &[(trace_commons_gate_api::RailId, CarryPrograms)],
    actor_ref_hash: &str,
) -> Result<LegacyCarryReport, DatabaseError> {
    let mut report = LegacyCarryReport::default();
    if rails.is_empty() || anchored_tenant(legacy) {
        return Ok(report);
    }
    // A returning user's sign-in (the account was reused, not opened) is a
    // repair event only (review of #1879): it does anything at all only for
    // a rail with no carry in the tenant, checked here without a lock so
    // the common sign-in pays one read.
    if !opened {
        let mut any_uncarried = false;
        for (rail, _) in rails {
            let carried: bool = tx
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM trace_legacy_payout_carries
                                     WHERE tenant_id = $1 AND rail_id = $2)",
                    &[&legacy, &rail.as_str()],
                )
                .await?
                .get(0);
            any_uncarried |= !carried;
        }
        if !any_uncarried {
            return Ok(report);
        }
    }
    let Some((source_tenant, source_account)) = tx
        .query_opt(
            "SELECT tenant_id, account_id FROM trace_legacy_payout_source_account($1)",
            &[&legacy],
        )
        .await?
        .map(|row| (row.get::<_, String>(0), row.get::<_, Uuid>(1)))
    else {
        return Ok(report);
    };
    let mut rails: Vec<&(trace_commons_gate_api::RailId, CarryPrograms)> = rails.iter().collect();
    rails.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    for (rail, _) in &rails {
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended(
                 'payout-link-confirm-v1:' || $1 || ':' || $2::UUID::TEXT || ':' || $3, 0))",
            &[&source_tenant, &source_account, &rail.as_str()],
        )
        .await?;
    }
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || $1, 0))",
        &[&legacy],
    )
    .await?;
    // Under the lock: the tenant link still live, the source still open,
    // and its live link on each rail.
    set_tenant(tx, &source_tenant).await?;
    let read = async {
        let tenant_link: Option<Uuid> = tx
            .query_opt(
                "SELECT link_id FROM trace_legacy_invite_links
                  WHERE tenant_id = $1 AND account_id = $2 AND legacy_tenant_id = $3
                    AND revoked_at IS NULL",
                &[&source_tenant, &source_account, &legacy],
            )
            .await?
            .map(|row| row.get(0));
        let Some(tenant_link) = tenant_link else {
            return Ok::<_, DatabaseError>(None);
        };
        if !source_open(tx, &source_tenant, source_account).await? {
            return Ok(None);
        }
        let mut sources = Vec::with_capacity(rails.len());
        for (rail, _) in &rails {
            sources.push(source_link(tx, &source_tenant, source_account, rail.as_str()).await?);
        }
        Ok(Some((tenant_link, sources)))
    }
    .await;
    set_tenant(tx, legacy).await?;
    let Some((tenant_link, sources)) = read? else {
        return Ok(report);
    };
    let legacy_hash = legacy_link_hash(tenant_link);
    for ((rail, programs), source) in rails.iter().zip(sources.iter()) {
        // The repair path (review of #1879): a tenant link with no carry at
        // all on this rail, while its source has a live link, means its
        // best-effort carry after the tenant link failed or never ran. This
        // event then carries the whole tenant. A returning sign-in does
        // nothing else; an opening otherwise carries only its account.
        let uncarried: bool = tx
            .query_one(
                "SELECT NOT EXISTS (SELECT 1 FROM trace_legacy_payout_carries
                                     WHERE tenant_id = $1 AND legacy_link_hash = $2
                                       AND rail_id = $3)",
                &[&legacy, &legacy_hash, &rail.as_str()],
            )
            .await?
            .get(0);
        let repair = uncarried && source.is_some();
        if !opened && !repair {
            continue;
        }
        carry_tenant(
            tx,
            legacy,
            &legacy_hash,
            source.as_ref(),
            rail.as_str(),
            programs,
            (!repair).then_some(account),
            &mut report,
        )
        .await?;
    }
    if report.carried + report.released > 0 {
        set_tenant(tx, &source_tenant).await?;
        let audited = audit_carry(tx, &source_tenant, actor_ref_hash, &report).await;
        set_tenant(tx, legacy).await?;
        audited?;
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
    only: Option<Uuid>,
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
    // Owner, 2026-11-03 (#1879): carry only to accounts a signed device
    // proved (the linking device or a V91-attested one). Any other account
    // waits, and a live carry on one that no longer qualifies is released
    // here; the send-time standing holds it until then. Asked only when
    // something could be carried: with no source or a pooled tenant every
    // carry is released whatever the answer.
    let proven: std::collections::HashSet<Uuid> = if source.is_some() && !pooled {
        tx.query(
            "SELECT * FROM trace_legacy_payout_proven_accounts($1)",
            &[&legacy],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect()
    } else {
        std::collections::HashSet::new()
    };
    let accounts = tx
        .query(
            "SELECT account_id, closed_at IS NOT NULL FROM trace_accounts
              WHERE tenant_id = $1 AND ($2::UUID IS NULL OR account_id = $2)
              ORDER BY account_id",
            &[&legacy, &only],
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
        let (Some(source), Some(source_hash), false, false, true) = (
            source,
            source_hash.as_deref(),
            pooled,
            closed,
            proven.contains(&account),
        ) else {
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
        // A link of the account's own (decision 6): never replaced. One
        // already ending is not kept: an active link with an unlink request
        // (V147 revokes a legacy account's own links that way) or a closing
        // one. The carry then lands as its waiting successor.
        let own: bool = tx
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM trace_payout_links l
                      WHERE l.tenant_id = $1 AND l.account_id = $2 AND l.rail_id = $3
                        AND (l.state = 'waiting'
                             OR (l.state = 'active' AND l.unlink_requested_at IS NULL))
                        AND NOT EXISTS (SELECT 1 FROM trace_legacy_payout_carries c
                                         WHERE c.tenant_id = l.tenant_id
                                           AND c.link_id = l.link_id))",
                &[&legacy, &account, &rail],
            )
            .await?
            .get(0);
        if own {
            report.skipped += 1;
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
