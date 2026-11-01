// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The PostgreSQL referral bonus store (V142;
//! docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections 4,
//! 6 and 7).
//!
//! Every tenant-scoped call opens a transaction in that tenant's context
//! first (`begin_trace_tenant_transaction`), so forced RLS bounds every
//! statement: the referee's rows are read under the referee's tenant and the
//! referrer's rows written under the referrer's, each in its own
//! transaction. The three deployment-wide reads take
//! `trace_referral_resolver` with `SET LOCAL ROLE`, which can read only who
//! owns a code index and whether it is live.
//!
//! Writes to one referrer account's bonus rows (pricing and voids) take a
//! transaction advisory lock on the account, so a void computing what is
//! unsent never races the pass adding to it.
//!
//! Amounts cross the wire as text and are parsed as `u128`, as the payout
//! store does.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use trace_commons_gate_api::RailId;
use trace_commons_gate_api::pipeline::InstrumentId;
use uuid::Uuid;

use super::postgres::PgBackend;
use super::{
    PayoutAccrualKind, ReferralAttributionRecord, ReferralBonusKind, ReferralBonusRecord,
    ReferralBonusStore, ReferralBonusWrite, ReferralCodeState, ReferralOwed,
    ReferralRefereeAccrual, ReferralRefereeFactsRow, ReferralReferrerView, ReferralSuspendedCode,
    ReferralTenantReport, ReferralVoidOutcome,
};
use crate::error::DatabaseError;
use crate::referral_abuse::{REFERRAL_SIGNAL_LABELS, RefereeMirror};

/// The label on a `code_resumed` audit row when the operator exempted
/// nothing.
pub const REFERRAL_OPERATOR_CLEAR: &str = "operator_clear";

/// The instrument a void row is written in when the referee has no bonus
/// row to copy it from (the `near_ai` rail's).
const NEAR_AI_INSTRUMENT: &str = crate::near_ai_rail::NEAR_AI_INSTRUMENT_ID;

fn parse_units(value: &str) -> Result<u128, DatabaseError> {
    value
        .parse::<u128>()
        .map_err(|_| DatabaseError::Query("referral_amount_out_of_range".into()))
}

/// A signed NUMERIC text, floored at zero.
fn parse_floor_zero(value: &str) -> Result<u128, DatabaseError> {
    if value.starts_with('-') {
        return Ok(0);
    }
    parse_units(value)
}

/// The per-account lock every write that changes a referrer's referral owed
/// or sent takes: bonus rows, voids, and the referral stream's line insert.
pub(super) async fn lock_account(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
) -> Result<(), DatabaseError> {
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended(
             'referral-bonus-v1:' || $1::TEXT || ':' || $2::UUID::TEXT, 0))",
        &[&tenant, &account],
    )
    .await?;
    Ok(())
}

async fn insert_referral_audit(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    action: &str,
    actor_ref_hash: &str,
    code_index: Option<&str>,
    referee_ref_hash: Option<&str>,
    label: Option<&str>,
) -> Result<(), DatabaseError> {
    tx.execute(
        "INSERT INTO trace_referral_audit (
             tenant_id, audit_id, action, actor_ref_hash, code_index, referee_ref_hash, label
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &tenant,
            &Uuid::new_v4(),
            &action,
            &actor_ref_hash,
            &code_index,
            &referee_ref_hash,
            &label,
        ],
    )
    .await?;
    Ok(())
}

const MIRROR_COLUMNS: &str = "referee_ref_hash, code_index, applied_at, signal_labels,
    recipient_index, subject_index, voided_at IS NOT NULL, signals_updated_at IS NOT NULL";

fn mirror_from_row(row: &tokio_postgres::Row) -> RefereeMirror {
    RefereeMirror {
        referee_ref_hash: row.get(0),
        code_index: row.get(1),
        applied_at: row.get(2),
        signal_labels: row.get(3),
        recipient_index: row.get(4),
        subject_index: row.get(5),
        voided: row.get(6),
        signals_seen: row.get(7),
    }
}

/// The account's referral stream on `rail`, inside `tx`.
pub(super) async fn owed_on_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
    rail: &str,
) -> Result<ReferralOwed, DatabaseError> {
    let row = tx
        .query_one(
            "SELECT
                 (SELECT COALESCE(SUM(bonus_units), 0)::TEXT FROM trace_referral_bonus_accruals
                   WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3),
                 (SELECT COALESCE(SUM(delta), 0)::TEXT FROM trace_payout_lines
                   WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                     AND stream = 'referral' AND state IN ('confirmed', 'pending'))",
            &[&tenant, &account, &rail],
        )
        .await?;
    let owed: String = row.get(0);
    let sent: String = row.get(1);
    Ok(ReferralOwed {
        owed: parse_floor_zero(&owed)?,
        sent: parse_units(&sent)?,
    })
}

/// What `referee` is owed and not yet sent on `rail`, inside `tx`.
///
/// The referral stream sends one account-wide amount, so what was sent is
/// allotted to the account's referees in the order each first earned
/// (oldest first, ties by referee hash), each up to their net bonus. A void
/// then takes back only the voided referee's unsent share: never what was
/// sent, and never another referee's unsent bonus. The order is stable
/// under voids (a void row is never a referee's first row), so voiding one
/// referee leaves every other referee's share where it was.
async fn referee_unsent_on_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
    rail: &str,
    referee_ref_hash: &str,
) -> Result<u128, DatabaseError> {
    let mut sent = owed_on_tx(tx, tenant, account, rail).await?.sent;
    let rows = tx
        .query(
            "SELECT referee_ref_hash, COALESCE(SUM(bonus_units), 0)::TEXT
               FROM trace_referral_bonus_accruals
              WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
              GROUP BY referee_ref_hash
              ORDER BY MIN(created_at), referee_ref_hash",
            &[&tenant, &account, &rail],
        )
        .await?;
    for row in &rows {
        let hash: String = row.get(0);
        let net = parse_floor_zero(&row.get::<_, String>(1))?;
        let covered = net.min(sent);
        sent -= covered;
        if hash == referee_ref_hash {
            return Ok(net - covered);
        }
    }
    Ok(0)
}

/// Writes one referee's void row, taking back that referee's unsent bonus
/// (`referee_unsent_on_tx`). Returns the units taken back.
async fn write_void_row(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
    referee_ref_hash: &str,
    rail: &str,
) -> Result<u128, DatabaseError> {
    let row = tx
        .query_one(
            "SELECT (SELECT rate_version FROM trace_referral_bonus_accruals
                      WHERE tenant_id = $1 AND rail_id = $2 AND referee_ref_hash = $3
                      ORDER BY created_at DESC, bonus_id LIMIT 1),
                    (SELECT instrument_id FROM trace_referral_bonus_accruals
                      WHERE tenant_id = $1 AND rail_id = $2 AND referee_ref_hash = $3
                      ORDER BY created_at DESC, bonus_id LIMIT 1),
                    EXISTS (SELECT 1 FROM trace_referral_bonus_accruals
                             WHERE tenant_id = $1 AND rail_id = $2 AND referee_ref_hash = $3
                               AND kind = 'void')",
            &[&tenant, &rail, &referee_ref_hash],
        )
        .await?;
    let rate_version: Option<String> = row.get(0);
    let instrument: Option<String> = row.get(1);
    let already: bool = row.get(2);
    // A referee who never earned has nothing to take back, and no rate
    // version to name; the voided mirror row alone stops new bonus.
    let Some(rate_version) = rate_version else {
        return Ok(0);
    };
    if already {
        return Ok(0);
    }
    let take = referee_unsent_on_tx(tx, tenant, account, rail, referee_ref_hash).await?;
    let bonus = format!("-{take}");
    tx.execute(
        "INSERT INTO trace_referral_bonus_accruals (
             tenant_id, bonus_id, rail_id, account_id, referee_ref_hash, kind,
             source_accrual_id, bps, base_units, bonus_units, rate_version, instrument_id
         ) VALUES ($1, $2, $3, $4, $5, 'void', NULL, 10000, 0, $6::TEXT::NUMERIC, $7, $8)
         ON CONFLICT DO NOTHING",
        &[
            &tenant,
            &Uuid::new_v4(),
            &rail,
            &account,
            &referee_ref_hash,
            &bonus,
            &rate_version,
            &instrument.unwrap_or_else(|| NEAR_AI_INSTRUMENT.to_owned()),
        ],
    )
    .await?;
    Ok(take)
}

impl PgBackend {
    async fn referral_resolver_query(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute(super::postgres_referral::REFERRAL_RESOLVER_ASSUME_SQL)
            .await?;
        let rows = tx.query(sql, params).await?;
        tx.commit().await?;
        Ok(rows)
    }
}

#[async_trait]
impl ReferralBonusStore for PgBackend {
    async fn referral_attributions(
        &self,
        tenant: &str,
    ) -> Result<Vec<ReferralAttributionRecord>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT account_id, code_index, referrer_tenant_id, referrer_account_id,
                        referee_ref_hash, referee_account_created_at, applied_at,
                        voided_at IS NOT NULL, earn_until
                   FROM trace_referral_attributions
                  WHERE tenant_id = $1
                  ORDER BY applied_at, account_id",
                &[&tenant],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .iter()
            .map(|row| ReferralAttributionRecord {
                account_id: row.get(0),
                code_index: row.get(1),
                referrer_tenant_id: row.get(2),
                referrer_account_id: row.get(3),
                referee_ref_hash: row.get(4),
                referee_account_created_at: row.get(5),
                applied_at: row.get(6),
                earn_until: row.get(8),
                voided: row.get(7),
            })
            .collect())
    }

    async fn referral_referee_accruals(
        &self,
        tenant: &str,
        rail: &RailId,
        account: Uuid,
    ) -> Result<Vec<ReferralRefereeAccrual>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT a.accrual_id, a.kind, a.settlement_batch_id, a.credit_account_hash,
                        a.account_id, abs(a.rail_units)::TEXT, a.rate_version, a.instrument_id,
                        b.created_at
                   FROM trace_payout_accruals a
                   JOIN trace_credit_settlement_batches b
                     ON b.tenant_id = a.tenant_id
                    AND b.settlement_batch_id = a.settlement_batch_id
                  WHERE a.tenant_id = $1 AND a.rail_id = $2 AND a.account_id = $3
                    AND b.status = 'finalized'
                  ORDER BY b.created_at, a.accrual_id",
                &[&tenant, &rail.as_str(), &account],
            )
            .await?;
        tx.commit().await?;
        rows.iter()
            .map(|row| {
                let kind: String = row.get(1);
                let kind = match kind.as_str() {
                    "credit" => PayoutAccrualKind::Credit,
                    "reversal" => PayoutAccrualKind::Reversal,
                    _ => return Err(DatabaseError::Query("payout_state_unknown".into())),
                };
                let units: String = row.get(5);
                let instrument: String = row.get(7);
                Ok(ReferralRefereeAccrual {
                    accrual_id: row.get(0),
                    kind,
                    settlement_batch_id: row.get(2),
                    credit_account_hash: row.get(3),
                    account_id: row.get(4),
                    rail_units: parse_units(&units)?,
                    rate_version: row.get(6),
                    instrument_id: InstrumentId::new(instrument)
                        .map_err(|_| DatabaseError::Query("payout_instrument_unknown".into()))?,
                    batch_created_at: row.get(8),
                })
            })
            .collect()
    }

    async fn referral_referee_facts(
        &self,
        tenant: &str,
        rail: &RailId,
        account: Uuid,
    ) -> Result<ReferralRefereeFactsRow, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_one(
                "WITH owned AS (
                     SELECT principal_ref FROM trace_account_owned_principals
                      WHERE tenant_id = $1 AND account_id = $2
                 ), mine AS (
                     SELECT s.status FROM trace_submissions s
                      WHERE s.tenant_id = $1
                        AND s.auth_principal_ref IN (SELECT principal_ref FROM owned)
                 )
                 SELECT (SELECT count(*) FROM mine),
                        (SELECT count(*) FROM mine WHERE status = 'rejected'),
                        EXISTS (SELECT 1 FROM trace_payout_accruals
                                 WHERE tenant_id = $1 AND rail_id = $3 AND account_id = $2
                                   AND kind = 'reversal'),
                        EXISTS (SELECT 1 FROM trace_payout_accruals
                                 WHERE tenant_id = $1 AND rail_id = $3 AND account_id = $2
                                   AND kind = 'credit')",
                &[&tenant, &account, &rail.as_str()],
            )
            .await?;
        tx.commit().await?;
        Ok(ReferralRefereeFactsRow {
            submissions: row.get::<_, i64>(0).max(0) as u64,
            refused: row.get::<_, i64>(1).max(0) as u64,
            reversed: row.get(2),
            credited: row.get(3),
        })
    }

    async fn referral_trace_clusters(
        &self,
        tenant: &str,
        account: Uuid,
        since: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<Uuid>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "WITH owned AS (
                     SELECT principal_ref FROM trace_account_owned_principals
                      WHERE tenant_id = $1 AND account_id = $2
                 ), recent AS (
                     SELECT s.submission_id FROM trace_submissions s
                      WHERE s.tenant_id = $1
                        AND s.auth_principal_ref IN (SELECT principal_ref FROM owned)
                        AND s.received_at >= $3
                      ORDER BY s.received_at DESC, s.submission_id
                      LIMIT $4
                 )
                 SELECT DISTINCT g.dedup_cluster_id FROM trace_gate_decisions g
                  WHERE g.tenant_id = $1
                    AND g.submission_id IN (SELECT submission_id FROM recent)
                    AND g.dedup_cluster_id IS NOT NULL",
                &[&tenant, &account, &since, &limit.max(0)],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    async fn referral_network_marks(
        &self,
        tenant: &str,
        account: Uuid,
        retention_days: u32,
        limit: i64,
    ) -> Result<Vec<String>, DatabaseError> {
        let retention_days = i32::try_from(retention_days).unwrap_or(i32::MAX);
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT mark FROM trace_referral_network_marks
                  WHERE tenant_id = $1 AND account_id = $2
                    AND last_seen_at >= clock_timestamp() - make_interval(days => $3)
                  ORDER BY last_seen_at DESC, mark
                  LIMIT $4",
                &[&tenant, &account, &retention_days, &limit.max(0)],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    async fn prune_referral_network_marks(
        &self,
        tenant: &str,
        retention_days: u32,
    ) -> Result<u64, DatabaseError> {
        let retention_days = i32::try_from(retention_days).unwrap_or(i32::MAX);
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let deleted = tx
            .execute(
                "DELETE FROM trace_referral_network_marks
                  WHERE tenant_id = $1
                    AND last_seen_at < clock_timestamp() - make_interval(days => $2)",
                &[&tenant, &retention_days],
            )
            .await?;
        tx.commit().await?;
        Ok(deleted)
    }

    async fn void_referral_attribution(
        &self,
        tenant: &str,
        account: Uuid,
        label: &str,
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                "UPDATE trace_referral_attributions
                    SET voided_at = clock_timestamp(), void_label = $3
                  WHERE tenant_id = $1 AND account_id = $2 AND voided_at IS NULL
                  RETURNING code_index, referee_ref_hash",
                &[&tenant, &account, &label],
            )
            .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let code_index: String = row.get(0);
        let referee_ref_hash: String = row.get(1);
        insert_referral_audit(
            &tx,
            tenant,
            "referral_voided",
            actor_ref_hash,
            Some(&code_index),
            Some(&referee_ref_hash),
            Some(label),
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn referral_referrer_view(
        &self,
        tenant: &str,
        referrer_account: Uuid,
        referee_ref_hash: &str,
        rail: &RailId,
    ) -> Result<ReferralReferrerView, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let referrer_exists: bool = tx
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM trace_accounts
                                 WHERE tenant_id = $1 AND account_id = $2)",
                &[&tenant, &referrer_account],
            )
            .await?
            .get(0);
        let mirror = tx
            .query_opt(
                &format!(
                    "SELECT {MIRROR_COLUMNS} FROM trace_referral_referees
                      WHERE tenant_id = $1 AND referee_ref_hash = $2"
                ),
                &[&tenant, &referee_ref_hash],
            )
            .await?
            .map(|row| mirror_from_row(&row));
        let rows = tx
            .query(
                "SELECT kind, source_accrual_id, bps, abs(base_units)::TEXT, abs(bonus_units)::TEXT
                   FROM trace_referral_bonus_accruals
                  WHERE tenant_id = $1 AND rail_id = $2 AND referee_ref_hash = $3
                  ORDER BY created_at, bonus_id",
                &[&tenant, &rail.as_str(), &referee_ref_hash],
            )
            .await?;
        tx.commit().await?;
        let bonus_rows = rows
            .iter()
            .map(|row| {
                let kind: String = row.get(0);
                let base: String = row.get(3);
                let bonus: String = row.get(4);
                Ok(ReferralBonusRecord {
                    kind: ReferralBonusKind::parse(&kind)
                        .ok_or_else(|| DatabaseError::Query("referral_state_unknown".into()))?,
                    source_accrual_id: row.get(1),
                    bps: u32::try_from(row.get::<_, i32>(2))
                        .map_err(|_| DatabaseError::Query("referral_bps_invalid".into()))?,
                    base_units: parse_units(&base)?,
                    bonus_units: parse_units(&bonus)?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        Ok(ReferralReferrerView {
            referrer_exists,
            mirror,
            bonus_rows,
        })
    }

    async fn insert_referral_bonus_rows(
        &self,
        tenant: &str,
        rows: &[ReferralBonusWrite],
    ) -> Result<u64, DatabaseError> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let accounts: BTreeSet<Uuid> = rows.iter().map(|row| row.account_id).collect();
        for account in accounts {
            lock_account(&tx, tenant, account).await?;
        }
        // The rows were planned from a view read in an earlier transaction.
        // A void that committed since then took back only the bonus that
        // existed at the time and stops every later plan, so a bonus or
        // reversal planned before it must not land after it: recheck under
        // the account lock the void takes, and drop those rows.
        let mut voided: BTreeSet<(&str, &str)> = BTreeSet::new();
        let referees: BTreeSet<(&str, &str)> = rows
            .iter()
            .map(|row| (row.referee_ref_hash.as_str(), row.rail_id.as_str()))
            .collect();
        for (referee_ref_hash, rail) in referees {
            let is_voided: bool = tx
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM trace_referral_referees
                                     WHERE tenant_id = $1 AND referee_ref_hash = $2
                                       AND voided_at IS NOT NULL)
                         OR EXISTS (SELECT 1 FROM trace_referral_bonus_accruals
                                     WHERE tenant_id = $1 AND referee_ref_hash = $2
                                       AND rail_id = $3 AND kind = 'void')",
                    &[&tenant, &referee_ref_hash, &rail],
                )
                .await?
                .get(0);
            if is_voided {
                voided.insert((referee_ref_hash, rail));
            }
        }
        let mut inserted = 0;
        for row in rows {
            if row.kind != ReferralBonusKind::Void
                && voided.contains(&(row.referee_ref_hash.as_str(), row.rail_id.as_str()))
            {
                continue;
            }
            let sign = match row.kind {
                ReferralBonusKind::Bonus => "",
                ReferralBonusKind::Reversal | ReferralBonusKind::Void => "-",
            };
            let base = if row.kind == ReferralBonusKind::Void {
                "0".to_owned()
            } else {
                format!("{sign}{}", row.base_units)
            };
            let bonus = format!("{sign}{}", row.bonus_units);
            let bps = i32::try_from(row.bps)
                .map_err(|_| DatabaseError::Query("referral_bps_invalid".into()))?;
            inserted += tx
                .execute(
                    "INSERT INTO trace_referral_bonus_accruals (
                         tenant_id, bonus_id, rail_id, account_id, referee_ref_hash, kind,
                         source_accrual_id, bps, base_units, bonus_units, rate_version,
                         instrument_id
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                               $9::TEXT::NUMERIC, $10::TEXT::NUMERIC, $11, $12)
                     ON CONFLICT DO NOTHING",
                    &[
                        &tenant,
                        &Uuid::new_v4(),
                        &row.rail_id.as_str(),
                        &row.account_id,
                        &row.referee_ref_hash,
                        &row.kind.as_str(),
                        &row.source_accrual_id,
                        &bps,
                        &base,
                        &bonus,
                        &row.rate_version,
                        &row.instrument_id.as_str(),
                    ],
                )
                .await?;
        }
        tx.commit().await?;
        Ok(inserted)
    }

    async fn refresh_referral_referee_signals(
        &self,
        tenant: &str,
        referee_ref_hash: &str,
        labels: &[String],
        recipient_index: Option<&str>,
        subject_index: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        tx.execute(
            "UPDATE trace_referral_referees
                SET signal_labels = $3, recipient_index = $4, subject_index = $5,
                    signals_updated_at = clock_timestamp()
              WHERE tenant_id = $1 AND referee_ref_hash = $2",
            &[
                &tenant,
                &referee_ref_hash,
                &labels,
                &recipient_index,
                &subject_index,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn referral_mirror_rows(
        &self,
        tenant: &str,
        account: Uuid,
    ) -> Result<Vec<RefereeMirror>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                &format!(
                    "SELECT {MIRROR_COLUMNS} FROM trace_referral_referees
                      WHERE tenant_id = $1 AND account_id = $2
                      ORDER BY applied_at, referee_ref_hash"
                ),
                &[&tenant, &account],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(mirror_from_row).collect())
    }

    async fn referral_codes(&self, tenant: &str) -> Result<Vec<ReferralCodeState>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT c.code_index, c.account_id, c.disabled_at IS NOT NULL, c.suspended_at,
                        c.suspension_labels, c.voided_at IS NOT NULL,
                        ARRAY(SELECT DISTINCT a.label FROM trace_referral_audit a
                               WHERE a.tenant_id = c.tenant_id AND a.action = 'code_resumed'
                                 AND a.code_index = c.code_index AND a.label IS NOT NULL)
                   FROM trace_referral_codes c
                  WHERE c.tenant_id = $1
                  ORDER BY c.created_at, c.code_id",
                &[&tenant],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .iter()
            .map(|row| ReferralCodeState {
                code_index: row.get(0),
                account_id: row.get(1),
                disabled: row.get(2),
                suspended_at: row.get(3),
                suspension_labels: row.get(4),
                voided: row.get(5),
                exempt_labels: row
                    .get::<_, Vec<String>>(6)
                    .into_iter()
                    .filter(|label| REFERRAL_SIGNAL_LABELS.contains(&label.as_str()))
                    .collect(),
            })
            .collect())
    }

    async fn suspend_referral_code(
        &self,
        tenant: &str,
        code_index: &str,
        labels: &[String],
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError> {
        if labels.is_empty() {
            return Ok(false);
        }
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let changed = tx
            .execute(
                "UPDATE trace_referral_codes
                    SET suspended_at = clock_timestamp(), suspension_labels = $3
                  WHERE tenant_id = $1 AND code_index = $2
                    AND suspended_at IS NULL AND voided_at IS NULL",
                &[&tenant, &code_index, &labels],
            )
            .await?;
        if changed == 0 {
            return Ok(false);
        }
        for label in labels {
            insert_referral_audit(
                &tx,
                tenant,
                "code_suspended",
                actor_ref_hash,
                Some(code_index),
                None,
                Some(label),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    async fn resume_referral_code(
        &self,
        tenant: &str,
        code_index: &str,
        exempt_labels: &[String],
        actor_ref_hash: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                "UPDATE trace_referral_codes
                    SET suspended_at = NULL, suspension_labels = '{}'
                  WHERE tenant_id = $1 AND code_index = $2
                    AND suspended_at IS NOT NULL AND voided_at IS NULL
                  RETURNING account_id",
                &[&tenant, &code_index],
            )
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let labels: Vec<&str> = if exempt_labels.is_empty() {
            vec![REFERRAL_OPERATOR_CLEAR]
        } else {
            exempt_labels.iter().map(String::as_str).collect()
        };
        for label in labels {
            insert_referral_audit(
                &tx,
                tenant,
                "code_resumed",
                actor_ref_hash,
                Some(code_index),
                None,
                Some(label),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(Some(row.get(0)))
    }

    async fn void_referral_code(
        &self,
        tenant: &str,
        code_index: &str,
        label: &str,
        rail: &RailId,
        actor_ref_hash: &str,
    ) -> Result<Option<ReferralVoidOutcome>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let Some(account) = tx
            .query_opt(
                "SELECT account_id FROM trace_referral_codes
                  WHERE tenant_id = $1 AND code_index = $2 AND voided_at IS NULL",
                &[&tenant, &code_index],
            )
            .await?
            .map(|row| row.get::<_, Uuid>(0))
        else {
            return Ok(None);
        };
        lock_account(&tx, tenant, account).await?;
        let changed = tx
            .execute(
                "UPDATE trace_referral_codes SET voided_at = clock_timestamp(), void_label = $3
                  WHERE tenant_id = $1 AND code_index = $2 AND voided_at IS NULL",
                &[&tenant, &code_index, &label],
            )
            .await?;
        if changed == 0 {
            return Ok(None);
        }
        insert_referral_audit(
            &tx,
            tenant,
            "code_voided",
            actor_ref_hash,
            Some(code_index),
            None,
            Some(label),
        )
        .await?;
        let referees: Vec<String> = tx
            .query(
                "UPDATE trace_referral_referees SET voided_at = clock_timestamp()
                  WHERE tenant_id = $1 AND code_index = $2 AND voided_at IS NULL
                  RETURNING referee_ref_hash",
                &[&tenant, &code_index],
            )
            .await?
            .iter()
            .map(|row| row.get(0))
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect();
        let mut outcome = ReferralVoidOutcome {
            account_id: Some(account),
            ..ReferralVoidOutcome::default()
        };
        for referee in &referees {
            outcome.taken_back +=
                write_void_row(&tx, tenant, account, referee, rail.as_str()).await?;
            outcome.referees_voided += 1;
            insert_referral_audit(
                &tx,
                tenant,
                "referral_voided",
                actor_ref_hash,
                Some(code_index),
                Some(referee),
                Some(label),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(Some(outcome))
    }

    async fn void_referral_referee(
        &self,
        tenant: &str,
        referee_ref_hash: &str,
        label: &str,
        rail: &RailId,
        actor_ref_hash: &str,
    ) -> Result<Option<ReferralVoidOutcome>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let Some(row) = tx
            .query_opt(
                "SELECT account_id, code_index, voided_at IS NOT NULL
                   FROM trace_referral_referees
                  WHERE tenant_id = $1 AND referee_ref_hash = $2",
                &[&tenant, &referee_ref_hash],
            )
            .await?
        else {
            return Ok(None);
        };
        let account: Uuid = row.get(0);
        let code_index: String = row.get(1);
        let already: bool = row.get(2);
        lock_account(&tx, tenant, account).await?;
        let mut outcome = ReferralVoidOutcome {
            account_id: Some(account),
            ..ReferralVoidOutcome::default()
        };
        if !already {
            tx.execute(
                "UPDATE trace_referral_referees SET voided_at = clock_timestamp()
                  WHERE tenant_id = $1 AND referee_ref_hash = $2 AND voided_at IS NULL",
                &[&tenant, &referee_ref_hash],
            )
            .await?;
            outcome.referees_voided = 1;
            insert_referral_audit(
                &tx,
                tenant,
                "referral_voided",
                actor_ref_hash,
                Some(&code_index),
                Some(referee_ref_hash),
                Some(label),
            )
            .await?;
        }
        outcome.taken_back =
            write_void_row(&tx, tenant, account, referee_ref_hash, rail.as_str()).await?;
        tx.commit().await?;
        Ok(Some(outcome))
    }

    async fn referral_bonus_owed(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<ReferralOwed, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let owed = owed_on_tx(&tx, tenant, account, rail.as_str()).await?;
        tx.commit().await?;
        Ok(owed)
    }

    async fn referral_suspended_codes(
        &self,
        tenant: &str,
        rail: &RailId,
    ) -> Result<Vec<ReferralSuspendedCode>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT c.code_index, c.suspension_labels, c.suspended_at,
                        (SELECT count(*) FROM trace_referral_referees r
                          WHERE r.tenant_id = c.tenant_id AND r.code_index = c.code_index
                            AND r.voided_at IS NULL),
                        (SELECT COALESCE(SUM(b.bonus_units), 0)::TEXT
                           FROM trace_referral_bonus_accruals b
                           JOIN trace_referral_referees r
                             ON r.tenant_id = b.tenant_id
                            AND r.referee_ref_hash = b.referee_ref_hash
                          WHERE r.tenant_id = c.tenant_id AND r.code_index = c.code_index
                            AND b.rail_id = $2)
                   FROM trace_referral_codes c
                  WHERE c.tenant_id = $1 AND c.suspended_at IS NOT NULL AND c.voided_at IS NULL
                  ORDER BY c.suspended_at, c.code_index",
                &[&tenant, &rail.as_str()],
            )
            .await?;
        tx.commit().await?;
        rows.iter()
            .map(|row| {
                let held: String = row.get(4);
                Ok(ReferralSuspendedCode {
                    code_index: row.get(0),
                    suspension_labels: row.get(1),
                    suspended_at: row.get(2),
                    referees: row.get::<_, i64>(3).max(0) as u64,
                    held_units: parse_floor_zero(&held)?,
                })
            })
            .collect()
    }

    async fn referral_tenant_report(
        &self,
        tenant: &str,
        rail: &RailId,
    ) -> Result<ReferralTenantReport, DatabaseError> {
        let codes = self.referral_codes(tenant).await?;
        let mut report = ReferralTenantReport::default();
        for code in &codes {
            *report
                .codes_by_state
                .entry(code.state_label().to_owned())
                .or_default() += 1;
        }
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let referees: i64 = tx
            .query_one(
                "SELECT count(*) FROM trace_referral_referees
                  WHERE tenant_id = $1 AND voided_at IS NULL",
                &[&tenant],
            )
            .await?
            .get(0);
        report.referees = referees.max(0) as u64;
        let signals = tx
            .query(
                "SELECT label, count(*) FROM trace_referral_referees r,
                        unnest(r.signal_labels) AS label
                  WHERE r.tenant_id = $1 AND r.voided_at IS NULL
                  GROUP BY label ORDER BY label",
                &[&tenant],
            )
            .await?;
        report.referees_by_signal = signals
            .iter()
            .map(|row| (row.get::<_, String>(0), row.get::<_, i64>(1).max(0) as u64))
            .collect::<BTreeMap<_, _>>();
        let accounts = tx
            .query(
                "SELECT DISTINCT b.account_id,
                        EXISTS (SELECT 1 FROM trace_payout_holds h
                                 WHERE h.tenant_id = b.tenant_id AND h.account_id = b.account_id
                                   AND h.rail_id = b.rail_id AND h.cleared_at IS NULL)
                        OR EXISTS (SELECT 1 FROM trace_accounts a
                                    WHERE a.tenant_id = b.tenant_id
                                      AND a.account_id = b.account_id
                                      AND a.closed_at IS NOT NULL)
                   FROM trace_referral_bonus_accruals b
                  WHERE b.tenant_id = $1 AND b.rail_id = $2",
                &[&tenant, &rail.as_str()],
            )
            .await?;
        for row in &accounts {
            let account: Uuid = row.get(0);
            let held: bool = row.get(1);
            let owed = owed_on_tx(&tx, tenant, account, rail.as_str()).await?;
            report.owed = report.owed.saturating_add(owed.owed);
            if held {
                report.held = report.held.saturating_add(owed.unsent());
            } else {
                report.sendable = report.sendable.saturating_add(owed.unsent());
            }
        }
        tx.commit().await?;
        Ok(report)
    }

    async fn referral_suspended_code_tenants(
        &self,
    ) -> Result<Vec<(String, String)>, DatabaseError> {
        let rows = self
            .referral_resolver_query(
                "SELECT tenant_id, code_index FROM trace_referral_codes
                  WHERE suspended_at IS NOT NULL AND voided_at IS NULL
                  ORDER BY tenant_id, code_index",
                &[],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
            .collect())
    }

    async fn referral_code_tenants(&self) -> Result<Vec<String>, DatabaseError> {
        let rows = self
            .referral_resolver_query(
                "SELECT DISTINCT tenant_id FROM trace_referral_codes ORDER BY tenant_id",
                &[],
            )
            .await?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    async fn referral_codes_suspended_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, DatabaseError> {
        let rows = self
            .referral_resolver_query(
                "SELECT count(*) FROM trace_referral_codes
                  WHERE suspended_at >= $1 AND voided_at IS NULL",
                &[&since],
            )
            .await?;
        Ok(rows
            .first()
            .map(|row| row.get::<_, i64>(0).max(0) as u64)
            .unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_totals_floor_at_zero_and_refuse_garbage() {
        assert_eq!(parse_floor_zero("-5").unwrap(), 0);
        assert_eq!(parse_floor_zero("0").unwrap(), 0);
        assert_eq!(parse_floor_zero("42").unwrap(), 42);
        assert!(parse_floor_zero("1.5").is_err());
        assert!(parse_units("340282366920938463463374607431768211456").is_err());
    }
}
