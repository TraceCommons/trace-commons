// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! PostgreSQL state for settlement rails (V127) and the per-tenant dispatch
//! and reconcile ticks that drive a [`SettlementRail`].
//!
//! Every function runs in a transaction scoped to one tenant through
//! `trace_commons.trace_tenant_id`, so RLS confines it. Amounts cross the
//! driver as decimal text, because `NUMERIC(39,0)` holds the whole `u128`
//! range and this crate has no decimal type.
//!
//! # Serialisation
//!
//! A tenant has at most one active NEAR AI recipient and an organization at
//! most one active tenant (V127's unique indexes), and [`lease_next`] leases
//! at most one row per `(tenant, rail)` at a time under a transaction
//! advisory lock. Together these make every absolute write to one external
//! organization strictly sequential, which the rail's precondition relies on.

use crate::near_ai_admin::NearAiOrgId;
use crate::settlement_rail::{
    ConversionRate, PriorWrite, RailDelivery, RailErrorKind, RailOutcome, RailReceipt,
    RailVerification, RecipientRef, SettlementRail, delivery_hash, is_safe_label,
};
use chrono::{DateTime, Utc};
use deadpool_postgres::{Pool, Transaction};
use std::time::Duration;
use uuid::Uuid;

/// A store failure, by label.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("settlement_rail_store_database")]
    Database,
    #[error("{0}")]
    Refused(&'static str),
}

impl From<tokio_postgres::Error> for StoreError {
    fn from(err: tokio_postgres::Error) -> Self {
        if err.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) {
            return Self::Refused("settlement_rail_unique_violation");
        }
        Self::Database
    }
}

impl From<deadpool_postgres::PoolError> for StoreError {
    fn from(_: deadpool_postgres::PoolError) -> Self {
        Self::Database
    }
}

async fn scope(tx: &Transaction<'_>, tenant: &str) -> Result<(), StoreError> {
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await?;
    Ok(())
}

fn parse_units(text: &str) -> Result<u128, StoreError> {
    text.parse::<u128>()
        .map_err(|_| StoreError::Refused("settlement_rail_units_unreadable"))
}

/// Record that `tenant` elects `org` to receive NEAR AI inference credit,
/// revoking any earlier election. `org` and `role` must come from
/// [`crate::near_ai_recipient::verify_recipient`], which checks them with the
/// contributor's own NEAR AI token; this function re-checks only the role's
/// shape.
pub async fn bind_near_ai_recipient(
    pool: &Pool,
    tenant: &str,
    org: NearAiOrgId,
    role: &str,
) -> Result<Uuid, StoreError> {
    if !matches!(role, "owner" | "admin") {
        return Err(StoreError::Refused("near_ai_org_role_insufficient"));
    }
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    tx.execute(
        "UPDATE trace_near_ai_credit_recipients SET revoked_at = NOW()
          WHERE tenant_id = $1 AND revoked_at IS NULL",
        &[&tenant],
    )
    .await?;
    let id = Uuid::new_v4();
    tx.execute(
        "INSERT INTO trace_near_ai_credit_recipients
            (tenant_id, recipient_id, org_id, org_ref_hash, role_at_bind)
         VALUES ($1, $2, $3, $4, $5)",
        &[&tenant, &id, &org.as_uuid(), &org.ref_hash(), &role],
    )
    .await
    .map_err(|e| match StoreError::from(e) {
        StoreError::Refused(_) => StoreError::Refused("near_ai_org_bound_elsewhere"),
        other => other,
    })?;
    tx.commit().await?;
    Ok(id)
}

/// What enqueueing a batch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Queued {
        outbox_id: Uuid,
        units: u128,
    },
    /// The conversion floored to zero; the value is in the carry.
    Carried {
        outbox_id: Uuid,
    },
    AlreadyQueued,
    NoRecipient,
    BatchNotFinalized,
}

/// Convert one finalized settlement batch into a rail outbox row.
pub async fn enqueue_batch(
    pool: &Pool,
    tenant: &str,
    settlement_batch_id: Uuid,
    rail: &dyn SettlementRail,
    rate: &ConversionRate,
) -> Result<EnqueueOutcome, StoreError> {
    let rail_id = rail.rail_id();
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    let Some(batch) = tx
        .query_opt(
            "SELECT status, settled_credit_micros FROM trace_credit_settlement_batches
              WHERE tenant_id = $1 AND settlement_batch_id = $2",
            &[&tenant, &settlement_batch_id],
        )
        .await?
    else {
        return Ok(EnqueueOutcome::BatchNotFinalized);
    };
    let status: String = batch.get(0);
    let micros: i64 = batch.get(1);
    if status != "finalized" {
        return Ok(EnqueueOutcome::BatchNotFinalized);
    }
    let micros =
        u128::try_from(micros).map_err(|_| StoreError::Refused("settlement_batch_negative"))?;
    let existing = tx
        .query_opt(
            "SELECT 1 FROM trace_settlement_rail_outbox
              WHERE tenant_id = $1 AND settlement_batch_id = $2 AND rail_id = $3",
            &[&tenant, &settlement_batch_id, &rail_id],
        )
        .await?;
    if existing.is_some() {
        return Ok(EnqueueOutcome::AlreadyQueued);
    }
    let Some(recipient) = tx
        .query_opt(
            "SELECT recipient_id FROM trace_near_ai_credit_recipients
              WHERE tenant_id = $1 AND revoked_at IS NULL",
            &[&tenant],
        )
        .await?
    else {
        return Ok(EnqueueOutcome::NoRecipient);
    };
    let recipient_id: Uuid = recipient.get(0);

    let rate_hash = rate.ref_hash();
    tx.execute(
        "INSERT INTO trace_settlement_rail_carry (tenant_id, rail_id, rate_ref_hash)
         VALUES ($1, $2, $3) ON CONFLICT (tenant_id, rail_id) DO NOTHING",
        &[&tenant, &rail_id, &rate_hash],
    )
    .await?;
    let carry_row = tx
        .query_one(
            "SELECT rate_ref_hash, carry_units::text FROM trace_settlement_rail_carry
              WHERE tenant_id = $1 AND rail_id = $2 FOR UPDATE",
            &[&tenant, &rail_id],
        )
        .await?;
    let carry_hash: String = carry_row.get(0);
    let carry = if carry_hash == rate_hash {
        parse_units(&carry_row.get::<_, String>(1))?
    } else {
        0
    };
    let (units, next_carry) = rate
        .convert(micros, carry)
        .map_err(|e| StoreError::Refused(e.label))?;

    let outbox_id = Uuid::new_v4();
    let dhash = delivery_hash(rail_id, rail.instrument_id(), outbox_id);
    let state = if units == 0 { "carried" } else { "pending" };
    tx.execute(
        "INSERT INTO trace_settlement_rail_outbox
            (tenant_id, outbox_id, settlement_batch_id, rail_id, instrument_id,
             instrument_decimals, source_micros, atomic_units, rate_text, rate_ref_hash,
             delivery_hash, recipient_id, state)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::numeric, $9, $10, $11, $12, $13)",
        &[
            &tenant,
            &outbox_id,
            &settlement_batch_id,
            &rail_id,
            &rail.instrument_id(),
            &i16::from(rail.instrument_decimals()),
            &(micros as i64),
            &units.to_string(),
            &rate.canonical(),
            &rate_hash,
            &dhash,
            &recipient_id,
            &state,
        ],
    )
    .await?;
    tx.execute(
        "UPDATE trace_settlement_rail_carry
            SET rate_ref_hash = $3, carry_units = $4::text::numeric, updated_at = NOW()
          WHERE tenant_id = $1 AND rail_id = $2",
        &[&tenant, &rail_id, &rate_hash, &next_carry.to_string()],
    )
    .await?;
    tx.commit().await?;
    Ok(if units == 0 {
        EnqueueOutcome::Carried { outbox_id }
    } else {
        EnqueueOutcome::Queued { outbox_id, units }
    })
}

/// A leased row, ready to hand to the rail.
#[derive(Debug, Clone)]
pub struct LeasedRow {
    pub outbox_id: Uuid,
    pub lease_token: Uuid,
    pub recipient_id: Uuid,
    pub delivery: RailDelivery,
}

/// The receipt we last recorded for a recipient on a rail, which the next
/// absolute write must build on.
async fn expected_prior(
    tx: &Transaction<'_>,
    tenant: &str,
    rail_id: &str,
    recipient_id: Uuid,
) -> Result<Option<PriorWrite>, StoreError> {
    let row = tx
        .query_opt(
            "SELECT external_receipt_hash, written_units::text
               FROM trace_settlement_rail_outbox
              WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3
                AND external_receipt_hash IS NOT NULL
              ORDER BY delivered_at DESC, updated_at DESC
              LIMIT 1",
            &[&tenant, &rail_id, &recipient_id],
        )
        .await?;
    row.map(|r| {
        Ok(PriorWrite {
            external_receipt_hash: r.get(0),
            written_units: parse_units(&r.get::<_, String>(1))?,
        })
    })
    .transpose()
}

/// Lease the oldest due row for `(tenant, rail)`, unless one is already in
/// flight. Rows whose recipient is revoked or frozen are not leased.
pub async fn lease_next(
    pool: &Pool,
    tenant: &str,
    rail_id: &str,
    lease_for: Duration,
) -> Result<Option<LeasedRow>, StoreError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended('trace-settlement-rail:' || $1 || ':' || $2, 0))",
        &[&tenant, &rail_id],
    )
    .await?;
    let in_flight = tx
        .query_opt(
            "SELECT 1 FROM trace_settlement_rail_outbox
              WHERE tenant_id = $1 AND rail_id = $2 AND state = 'leased'
                AND lease_expires_at > NOW()",
            &[&tenant, &rail_id],
        )
        .await?;
    if in_flight.is_some() {
        return Ok(None);
    }
    let Some(row) = tx
        .query_opt(
            "SELECT o.outbox_id, o.recipient_id, o.delivery_hash, o.atomic_units::text,
                    r.org_id
               FROM trace_settlement_rail_outbox o
               JOIN trace_near_ai_credit_recipients r
                 ON r.tenant_id = o.tenant_id AND r.recipient_id = o.recipient_id
              WHERE o.tenant_id = $1 AND o.rail_id = $2
                AND r.revoked_at IS NULL AND r.frozen_label IS NULL
                AND (
                    (o.state IN ('pending', 'retry') AND o.next_attempt_at <= NOW())
                    OR (o.state = 'leased' AND o.lease_expires_at <= NOW())
                )
              ORDER BY o.created_at, o.outbox_id
              LIMIT 1
              FOR UPDATE OF o",
            &[&tenant, &rail_id],
        )
        .await?
    else {
        return Ok(None);
    };
    let outbox_id: Uuid = row.get(0);
    let recipient_id: Uuid = row.get(1);
    let dhash: String = row.get(2);
    let units = parse_units(&row.get::<_, String>(3))?;
    let org_id: Uuid = row.get(4);
    let prior = expected_prior(&tx, tenant, rail_id, recipient_id).await?;
    let token = Uuid::new_v4();
    let expires: DateTime<Utc> =
        Utc::now() + chrono::Duration::from_std(lease_for).unwrap_or(chrono::Duration::minutes(5));
    tx.execute(
        "UPDATE trace_settlement_rail_outbox
            SET state = 'leased', lease_token = $3, lease_expires_at = $4,
                attempt_count = attempt_count + 1, updated_at = NOW()
          WHERE tenant_id = $1 AND outbox_id = $2",
        &[&tenant, &outbox_id, &token, &expires],
    )
    .await?;
    tx.commit().await?;
    Ok(Some(LeasedRow {
        outbox_id,
        lease_token: token,
        recipient_id,
        delivery: RailDelivery {
            delivery_hash: dhash,
            recipient: RecipientRef::new(org_id.to_string()),
            units,
            expected_prior: prior,
        },
    }))
}

/// Backoff for the `n`th failed attempt: 30s doubling, capped at an hour.
pub fn backoff(attempt: i32) -> Duration {
    let exp = attempt.clamp(1, 8) as u32 - 1;
    Duration::from_secs((30u64 << exp).min(3600))
}

/// The recorded disposition of one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    Delivered,
    DeliveredFrozen,
    Retry,
    Frozen,
    DryRun,
    /// The lease was lost; nothing was written.
    LeaseLost,
}

/// Record the result of `deliver` for a leased row.
pub async fn record_attempt(
    pool: &Pool,
    tenant: &str,
    leased: &LeasedRow,
    result: &Result<RailOutcome, crate::settlement_rail::RailError>,
) -> Result<Recorded, StoreError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    let Some(row) = tx
        .query_opt(
            "SELECT attempt_count FROM trace_settlement_rail_outbox
              WHERE tenant_id = $1 AND outbox_id = $2 AND state = 'leased'
                AND lease_token = $3 FOR UPDATE",
            &[&tenant, &leased.outbox_id, &leased.lease_token],
        )
        .await?
    else {
        return Ok(Recorded::LeaseLost);
    };
    let attempts: i32 = row.get(0);
    let recorded = match result {
        Ok(outcome) => {
            let r = &outcome.receipt;
            let (state, label, recorded) = match outcome.anomaly {
                None => ("delivered", None, Recorded::Delivered),
                Some(label) => ("frozen", Some(label), Recorded::DeliveredFrozen),
            };
            tx.execute(
                "UPDATE trace_settlement_rail_outbox
                    SET state = $3, lease_token = NULL, lease_expires_at = NULL,
                        prior_units = $4::text::numeric, written_units = $5::text::numeric,
                        external_receipt_hash = $6, last_error_label = $7,
                        delivered_at = NOW(), updated_at = NOW()
                  WHERE tenant_id = $1 AND outbox_id = $2",
                &[
                    &tenant,
                    &leased.outbox_id,
                    &state,
                    &r.prior_units.to_string(),
                    &r.written_units.to_string(),
                    &r.external_receipt_hash,
                    &label,
                ],
            )
            .await?;
            recorded
        }
        Err(err) => {
            let label = if is_safe_label(err.label) {
                err.label
            } else {
                "settlement_rail_label_invalid"
            };
            let (state, delay, recorded) = match err.kind {
                RailErrorKind::Retry | RailErrorKind::Refused => {
                    ("retry", backoff(attempts), Recorded::Retry)
                }
                RailErrorKind::DryRun => ("pending", Duration::from_secs(3600), Recorded::DryRun),
                RailErrorKind::Frozen => ("frozen", Duration::ZERO, Recorded::Frozen),
            };
            let next: DateTime<Utc> = Utc::now()
                + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::hours(1));
            tx.execute(
                "UPDATE trace_settlement_rail_outbox
                    SET state = $3, lease_token = NULL, lease_expires_at = NULL,
                        last_error_label = $4, next_attempt_at = $5, updated_at = NOW()
                  WHERE tenant_id = $1 AND outbox_id = $2",
                &[&tenant, &leased.outbox_id, &state, &label, &next],
            )
            .await?;
            recorded
        }
    };
    tx.commit().await?;
    Ok(recorded)
}

/// Operator action: release a frozen row. A row whose effect exists returns
/// to `delivered` (to be reconciled); one without returns to `retry`.
pub async fn unfreeze(pool: &Pool, tenant: &str, outbox_id: Uuid) -> Result<bool, StoreError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    let n = tx
        .execute(
            "UPDATE trace_settlement_rail_outbox
                SET state = CASE WHEN external_receipt_hash IS NULL THEN 'retry' ELSE 'delivered' END,
                    next_attempt_at = NOW(), last_error_label = NULL, updated_at = NOW()
              WHERE tenant_id = $1 AND outbox_id = $2 AND state = 'frozen'",
            &[&tenant, &outbox_id],
        )
        .await?;
    tx.commit().await?;
    Ok(n == 1)
}

/// Operator action: cancel a row whose effect does not exist. The batch is
/// then free to pay out on another path.
pub async fn cancel(pool: &Pool, tenant: &str, outbox_id: Uuid) -> Result<bool, StoreError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    let n = tx
        .execute(
            "UPDATE trace_settlement_rail_outbox
                SET state = 'cancelled', updated_at = NOW()
              WHERE tenant_id = $1 AND outbox_id = $2
                AND state IN ('pending', 'retry', 'frozen')
                AND external_receipt_hash IS NULL",
            &[&tenant, &outbox_id],
        )
        .await?;
    tx.commit().await?;
    Ok(n == 1)
}

/// Counts from one dispatch tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatchSummary {
    pub delivered: u32,
    pub frozen: u32,
    pub retry: u32,
    pub dry_run: u32,
}

/// Deliver up to `limit` rows for one tenant, one at a time. A `Refused`
/// error (a missing or invalid control) ends the tick: every later row
/// would fail the same way.
pub async fn dispatch_tick(
    pool: &Pool,
    tenant: &str,
    rail: &dyn SettlementRail,
    limit: u32,
    lease_for: Duration,
) -> Result<DispatchSummary, StoreError> {
    let mut summary = DispatchSummary::default();
    for _ in 0..limit {
        let Some(leased) = lease_next(pool, tenant, rail.rail_id(), lease_for).await? else {
            break;
        };
        let result = rail.deliver(&leased.delivery).await;
        let stop = matches!(&result, Err(e) if e.kind == RailErrorKind::Refused);
        match record_attempt(pool, tenant, &leased, &result).await? {
            Recorded::Delivered => summary.delivered += 1,
            Recorded::DeliveredFrozen | Recorded::Frozen => summary.frozen += 1,
            Recorded::Retry => summary.retry += 1,
            Recorded::DryRun => summary.dry_run += 1,
            Recorded::LeaseLost => {}
        }
        if stop || matches!(&result, Err(e) if e.kind == RailErrorKind::DryRun) {
            break;
        }
    }
    Ok(summary)
}

/// Counts from one reconcile tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileSummary {
    pub reconciled: u32,
    pub overwritten: u32,
    pub missing: u32,
}

/// Verify delivered rows against the external system. An overwritten
/// recipient is frozen, so nothing further builds on a base that is no
/// longer ours; a missing effect freezes its row.
pub async fn reconcile_tick(
    pool: &Pool,
    tenant: &str,
    rail: &dyn SettlementRail,
    limit: i64,
) -> Result<ReconcileSummary, StoreError> {
    let rows = {
        let mut client = pool.get().await?;
        let tx = client.transaction().await?;
        scope(&tx, tenant).await?;
        let rows = tx
            .query(
                "SELECT o.outbox_id, o.recipient_id, o.delivery_hash, o.atomic_units::text,
                        o.prior_units::text, o.written_units::text, o.external_receipt_hash,
                        r.org_id
                   FROM trace_settlement_rail_outbox o
                   JOIN trace_near_ai_credit_recipients r
                     ON r.tenant_id = o.tenant_id AND r.recipient_id = o.recipient_id
                  WHERE o.tenant_id = $1 AND o.rail_id = $2 AND o.state = 'delivered'
                  ORDER BY o.delivered_at
                  LIMIT $3",
                &[&tenant, &rail.rail_id(), &limit],
            )
            .await?;
        tx.commit().await?;
        rows
    };
    let mut summary = ReconcileSummary::default();
    for row in rows {
        let outbox_id: Uuid = row.get(0);
        let recipient_id: Uuid = row.get(1);
        let org_id: Uuid = row.get(7);
        let delivery = RailDelivery {
            delivery_hash: row.get(2),
            recipient: RecipientRef::new(org_id.to_string()),
            units: parse_units(&row.get::<_, String>(3))?,
            expected_prior: None,
        };
        let receipt = RailReceipt {
            prior_units: parse_units(&row.get::<_, String>(4))?,
            written_units: parse_units(&row.get::<_, String>(5))?,
            external_receipt_hash: row.get(6),
            replayed: false,
        };
        let verdict = match rail.verify(&delivery, &receipt).await {
            Ok(v) => v,
            // A transient read failure is retried next tick.
            Err(e) if e.kind == RailErrorKind::Retry => continue,
            Err(_) => RailVerification::Missing,
        };
        let mut client = pool.get().await?;
        let tx = client.transaction().await?;
        scope(&tx, tenant).await?;
        match verdict {
            RailVerification::Current | RailVerification::Superseded => {
                tx.execute(
                    "UPDATE trace_settlement_rail_outbox
                        SET state = 'reconciled', reconciled_at = NOW(), updated_at = NOW()
                      WHERE tenant_id = $1 AND outbox_id = $2 AND state = 'delivered'",
                    &[&tenant, &outbox_id],
                )
                .await?;
                summary.reconciled += 1;
            }
            RailVerification::Overwritten => {
                tx.execute(
                    "UPDATE trace_near_ai_credit_recipients
                        SET frozen_label = 'near_ai_grant_bucket_foreign_write', frozen_at = NOW()
                      WHERE tenant_id = $1 AND recipient_id = $2 AND frozen_label IS NULL",
                    &[&tenant, &recipient_id],
                )
                .await?;
                tx.execute(
                    "UPDATE trace_settlement_rail_outbox
                        SET last_error_label = 'near_ai_grant_bucket_foreign_write', updated_at = NOW()
                      WHERE tenant_id = $1 AND outbox_id = $2",
                    &[&tenant, &outbox_id],
                )
                .await?;
                summary.overwritten += 1;
            }
            RailVerification::Missing => {
                tx.execute(
                    "UPDATE trace_settlement_rail_outbox
                        SET state = 'frozen', last_error_label = 'settlement_rail_effect_missing',
                            updated_at = NOW()
                      WHERE tenant_id = $1 AND outbox_id = $2 AND state = 'delivered'",
                    &[&tenant, &outbox_id],
                )
                .await?;
                summary.missing += 1;
            }
        }
        tx.commit().await?;
    }
    Ok(summary)
}
