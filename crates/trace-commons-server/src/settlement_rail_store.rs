// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! PostgreSQL state for settlement rails (V129) and the per-tenant publish
//! and reconcile ticks that drive a [`SettlementRail`].
//!
//! The ledger is cumulative: each finalized settlement batch adds once
//! (`trace_settlement_rail_credits` is keyed by batch and rail) to a
//! per-recipient total that never decreases, and a rail publishes that
//! total. Because the external side keeps the maximum it has seen,
//! publishing is idempotent: a retry, a duplicate worker or a late publish
//! of an older total cannot double-pay or lower anything. That is why this
//! module needs no per-delivery outbox, no compare-and-swap and no
//! read-before-write.
//!
//! Every function runs in a transaction scoped to one tenant through
//! `trace_commons.trace_tenant_id`, so RLS confines it. Amounts cross the
//! driver as decimal text: `NUMERIC(39,0)` holds the whole `u128` range.

use crate::near_ai_admin::NearAiOrgId;
use crate::near_ai_credit_statement::{CreditStatement, SignedCreditStatement, StatementSigner};
use crate::settlement_rail::{
    Comparison, ConversionRate, CumulativeStatement, Published, RailError, RailErrorKind,
    RecipientRef, SettlementRail, compare, is_safe_label, statement_ref,
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
///
/// NEAR AI keeps one never-decreasing total per organization, so an
/// organization belongs to one tenant and one ledger for good: rebinding an
/// organization this tenant used before reactivates its recipient (and so
/// its ledger), and an organization another tenant ever bound is refused.
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
    let org_ref_hash = org.ref_hash();
    tx.execute(
        "UPDATE trace_near_ai_credit_recipients SET revoked_at = NOW()
          WHERE tenant_id = $1 AND revoked_at IS NULL AND org_ref_hash <> $2",
        &[&tenant, &org_ref_hash],
    )
    .await?;
    if let Some(row) = tx
        .query_opt(
            "SELECT recipient_id, revoked_at IS NOT NULL FROM trace_near_ai_credit_recipients
              WHERE tenant_id = $1 AND org_ref_hash = $2",
            &[&tenant, &org_ref_hash],
        )
        .await?
    {
        let id: Uuid = row.get(0);
        if row.get::<_, bool>(1) {
            tx.execute(
                "UPDATE trace_near_ai_credit_recipients SET revoked_at = NULL
                  WHERE tenant_id = $1 AND recipient_id = $2",
                &[&tenant, &id],
            )
            .await?;
        }
        tx.commit().await?;
        return Ok(id);
    }
    let id = Uuid::new_v4();
    tx.execute(
        "INSERT INTO trace_near_ai_credit_recipients
            (tenant_id, recipient_id, org_id, org_ref_hash, role_at_bind)
         VALUES ($1, $2, $3, $4, $5)",
        &[&tenant, &id, &org.as_uuid(), &org_ref_hash, &role],
    )
    .await
    .map_err(|e| match StoreError::from(e) {
        StoreError::Refused(_) => StoreError::Refused("near_ai_org_bound_elsewhere"),
        other => other,
    })?;
    tx.commit().await?;
    Ok(id)
}

/// What accruing a batch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccrueOutcome {
    /// Added `units`; the recipient's total is now `cumulative_units`.
    Accrued {
        units: u128,
        cumulative_units: u128,
    },
    AlreadyAccrued,
    NoRecipient,
    BatchNotFinalized,
}

/// Add one finalized settlement batch to the active recipient's cumulative
/// total, once.
pub async fn accrue_batch(
    pool: &Pool,
    tenant: &str,
    settlement_batch_id: Uuid,
    rail: &dyn SettlementRail,
    rate: &ConversionRate,
) -> Result<AccrueOutcome, StoreError> {
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
        return Ok(AccrueOutcome::BatchNotFinalized);
    };
    if batch.get::<_, String>(0) != "finalized" {
        return Ok(AccrueOutcome::BatchNotFinalized);
    }
    let micros_i64: i64 = batch.get(1);
    let micros =
        u128::try_from(micros_i64).map_err(|_| StoreError::Refused("settlement_batch_negative"))?;
    if tx
        .query_opt(
            "SELECT 1 FROM trace_settlement_rail_credits
              WHERE tenant_id = $1 AND settlement_batch_id = $2 AND rail_id = $3",
            &[&tenant, &settlement_batch_id, &rail_id],
        )
        .await?
        .is_some()
    {
        return Ok(AccrueOutcome::AlreadyAccrued);
    }
    let Some(recipient) = tx
        .query_opt(
            "SELECT recipient_id FROM trace_near_ai_credit_recipients
              WHERE tenant_id = $1 AND revoked_at IS NULL",
            &[&tenant],
        )
        .await?
    else {
        return Ok(AccrueOutcome::NoRecipient);
    };
    let recipient_id: Uuid = recipient.get(0);

    let rate_hash = rate.ref_hash();
    tx.execute(
        "INSERT INTO trace_settlement_rail_ledgers
            (tenant_id, rail_id, recipient_id, instrument_id, instrument_decimals, rate_ref_hash)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (tenant_id, rail_id, recipient_id) DO NOTHING",
        &[
            &tenant,
            &rail_id,
            &recipient_id,
            &rail.instrument_id(),
            &i16::from(rail.instrument_decimals()),
            &rate_hash,
        ],
    )
    .await?;
    let ledger = tx
        .query_one(
            "SELECT rate_ref_hash, carry_units::text, cumulative_units::text
               FROM trace_settlement_rail_ledgers
              WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3
              FOR UPDATE",
            &[&tenant, &rail_id, &recipient_id],
        )
        .await?;
    let carry = if ledger.get::<_, String>(0) == rate_hash {
        parse_units(&ledger.get::<_, String>(1))?
    } else {
        0
    };
    let previous = parse_units(&ledger.get::<_, String>(2))?;
    let (units, next_carry) = rate
        .convert(micros, carry)
        .map_err(|e| StoreError::Refused(e.label))?;
    let cumulative = previous
        .checked_add(units)
        .ok_or(StoreError::Refused("settlement_rail_cumulative_overflow"))?;

    tx.execute(
        "INSERT INTO trace_settlement_rail_credits
            (tenant_id, settlement_batch_id, rail_id, recipient_id, source_micros,
             atomic_units, rate_text, rate_ref_hash)
         VALUES ($1, $2, $3, $4, $5, $6::text::numeric, $7, $8)",
        &[
            &tenant,
            &settlement_batch_id,
            &rail_id,
            &recipient_id,
            &micros_i64,
            &units.to_string(),
            &rate.canonical(),
            &rate_hash,
        ],
    )
    .await?;
    tx.execute(
        "UPDATE trace_settlement_rail_ledgers
            SET cumulative_units = $4::text::numeric,
                cumulative_micros = cumulative_micros + $5::bigint,
                rate_ref_hash = $6, carry_units = $7::text::numeric,
                publish_state = CASE
                    WHEN publish_state IN ('idle', 'published')
                         AND $4::text::numeric > confirmed_units THEN 'due'
                    ELSE publish_state END,
                next_attempt_at = CASE
                    WHEN publish_state IN ('idle', 'published') THEN NOW()
                    ELSE next_attempt_at END,
                updated_at = NOW()
          WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3",
        &[
            &tenant,
            &rail_id,
            &recipient_id,
            &cumulative.to_string(),
            &micros_i64,
            &rate_hash,
            &next_carry.to_string(),
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(AccrueOutcome::Accrued {
        units,
        cumulative_units: cumulative,
    })
}

/// A leased ledger, ready to publish.
#[derive(Debug, Clone)]
pub struct LeasedLedger {
    pub recipient_id: Uuid,
    pub lease_token: Uuid,
    pub statement: CumulativeStatement,
}

/// Lease the oldest due ledger for `(tenant, rail)`. Concurrent workers
/// skip each other's rows; even without that, two publishes of one
/// cumulative total are harmless.
pub async fn lease_due(
    pool: &Pool,
    tenant: &str,
    rail_id: &str,
    lease_for: Duration,
) -> Result<Option<LeasedLedger>, StoreError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    let Some(row) = tx
        .query_opt(
            "SELECT l.recipient_id, l.cumulative_units::text, r.org_id, r.org_ref_hash
               FROM trace_settlement_rail_ledgers l
               JOIN trace_near_ai_credit_recipients r
                 ON r.tenant_id = l.tenant_id AND r.recipient_id = l.recipient_id
              WHERE l.tenant_id = $1 AND l.rail_id = $2 AND r.revoked_at IS NULL
                AND (
                    (l.publish_state = 'due' AND l.next_attempt_at <= NOW())
                    OR (l.publish_state = 'leased' AND l.lease_expires_at <= NOW())
                )
              ORDER BY l.next_attempt_at
              LIMIT 1
              FOR UPDATE OF l SKIP LOCKED",
            &[&tenant, &rail_id],
        )
        .await?
    else {
        return Ok(None);
    };
    let recipient_id: Uuid = row.get(0);
    let cumulative = parse_units(&row.get::<_, String>(1))?;
    let org_id: Uuid = row.get(2);
    let org_ref_hash: String = row.get(3);
    let token = Uuid::new_v4();
    let expires: DateTime<Utc> =
        Utc::now() + chrono::Duration::from_std(lease_for).unwrap_or(chrono::Duration::minutes(5));
    tx.execute(
        "UPDATE trace_settlement_rail_ledgers
            SET publish_state = 'leased', lease_token = $4, lease_expires_at = $5,
                attempt_count = attempt_count + 1, updated_at = NOW()
          WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3",
        &[&tenant, &rail_id, &recipient_id, &token, &expires],
    )
    .await?;
    tx.commit().await?;
    Ok(Some(LeasedLedger {
        recipient_id,
        lease_token: token,
        statement: CumulativeStatement {
            recipient: RecipientRef::new(org_id.to_string()),
            statement_ref: statement_ref(rail_id, &org_ref_hash, cumulative),
            recipient_ref_hash: org_ref_hash,
            cumulative_units: cumulative,
        },
    }))
}

/// Backoff for the `n`th failed attempt: 30s doubling, capped at an hour.
pub fn backoff(attempt: i32) -> Duration {
    let exp = attempt.clamp(1, 8) as u32 - 1;
    Duration::from_secs((30u64 << exp).min(3600))
}

/// The recorded disposition of one publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// The external side holds our whole total.
    Confirmed,
    /// It holds part of it (our total grew meanwhile); publish again.
    Partial,
    /// Pull: the statement is servable; NEAR AI applies it on its schedule.
    Deferred,
    Retry,
    Frozen,
    DryRun,
    LeaseLost,
}

/// Record the result of `publish` for a leased ledger.
pub async fn record_publish(
    pool: &Pool,
    tenant: &str,
    rail_id: &str,
    leased: &LeasedLedger,
    result: &Result<Published, RailError>,
) -> Result<Recorded, StoreError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    let Some(row) = tx
        .query_opt(
            "SELECT attempt_count, cumulative_units::text, confirmed_units::text
               FROM trace_settlement_rail_ledgers
              WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3
                AND publish_state = 'leased' AND lease_token = $4
              FOR UPDATE",
            &[&tenant, &rail_id, &leased.recipient_id, &leased.lease_token],
        )
        .await?
    else {
        return Ok(Recorded::LeaseLost);
    };
    let attempts: i32 = row.get(0);
    let cumulative = parse_units(&row.get::<_, String>(1))?;
    let confirmed = parse_units(&row.get::<_, String>(2))?;
    let published = leased.statement.cumulative_units;

    // (state, new confirmed, label, delay, disposition)
    let (state, new_confirmed, label, delay, recorded): (
        &str,
        u128,
        Option<&'static str>,
        Duration,
        Recorded,
    ) = match result {
        Ok(Published::Recorded {
            effective_units, ..
        }) => {
            let effective = *effective_units;
            if effective < published {
                // A monotonic max of our total cannot be below our total.
                (
                    "frozen",
                    confirmed,
                    Some("settlement_rail_external_refused_total"),
                    Duration::ZERO,
                    Recorded::Frozen,
                )
            } else {
                match compare(effective, cumulative, confirmed) {
                    Comparison::Current => {
                        ("idle", effective, None, Duration::ZERO, Recorded::Confirmed)
                    }
                    Comparison::Behind => {
                        ("due", effective, None, Duration::ZERO, Recorded::Partial)
                    }
                    Comparison::Ahead => (
                        "frozen",
                        confirmed,
                        Some("settlement_rail_external_ahead"),
                        Duration::ZERO,
                        Recorded::Frozen,
                    ),
                    Comparison::Regressed => (
                        "frozen",
                        confirmed,
                        Some("settlement_rail_external_regressed"),
                        Duration::ZERO,
                        Recorded::Frozen,
                    ),
                }
            }
        }
        Ok(Published::Deferred) => (
            "published",
            confirmed,
            None,
            Duration::ZERO,
            Recorded::Deferred,
        ),
        Err(err) => {
            let label = if is_safe_label(err.label) {
                err.label
            } else {
                "settlement_rail_label_invalid"
            };
            match err.kind {
                RailErrorKind::Retry | RailErrorKind::Refused => (
                    "due",
                    confirmed,
                    Some(label),
                    backoff(attempts),
                    Recorded::Retry,
                ),
                RailErrorKind::DryRun => (
                    "due",
                    confirmed,
                    Some(label),
                    Duration::from_secs(3600),
                    Recorded::DryRun,
                ),
                RailErrorKind::Frozen => (
                    "frozen",
                    confirmed,
                    Some(label),
                    Duration::ZERO,
                    Recorded::Frozen,
                ),
            }
        }
    };
    let next: DateTime<Utc> =
        Utc::now() + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::hours(1));
    let confirmed_now = new_confirmed > confirmed;
    tx.execute(
        "UPDATE trace_settlement_rail_ledgers
            SET publish_state = $4, lease_token = NULL, lease_expires_at = NULL,
                confirmed_units = $5::text::numeric, last_error_label = $6,
                next_attempt_at = $7,
                confirmed_at = CASE WHEN $8 THEN NOW() ELSE confirmed_at END,
                updated_at = NOW()
          WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3",
        &[
            &tenant,
            &rail_id,
            &leased.recipient_id,
            &state,
            &new_confirmed.to_string(),
            &label,
            &next,
            &confirmed_now,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(recorded)
}

/// Counts from one publish tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublishSummary {
    pub confirmed: u32,
    pub partial: u32,
    pub deferred: u32,
    pub retry: u32,
    pub frozen: u32,
    pub dry_run: u32,
}

/// Publish up to `limit` due ledgers for one tenant. A `Refused` error (a
/// missing or invalid control) ends the tick.
pub async fn publish_tick(
    pool: &Pool,
    tenant: &str,
    rail: &dyn SettlementRail,
    limit: u32,
    lease_for: Duration,
) -> Result<PublishSummary, StoreError> {
    let mut summary = PublishSummary::default();
    for _ in 0..limit {
        let Some(leased) = lease_due(pool, tenant, rail.rail_id(), lease_for).await? else {
            break;
        };
        let result = rail.publish(&leased.statement).await;
        let stop = matches!(&result, Err(e) if matches!(e.kind, RailErrorKind::Refused | RailErrorKind::DryRun));
        match record_publish(pool, tenant, rail.rail_id(), &leased, &result).await? {
            Recorded::Confirmed => summary.confirmed += 1,
            Recorded::Partial => summary.partial += 1,
            Recorded::Deferred => summary.deferred += 1,
            Recorded::Retry => summary.retry += 1,
            Recorded::Frozen => summary.frozen += 1,
            Recorded::DryRun => summary.dry_run += 1,
            Recorded::LeaseLost => {}
        }
        if stop {
            break;
        }
    }
    Ok(summary)
}

/// Counts from one reconcile tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileSummary {
    pub verified: u32,
    pub behind: u32,
    pub frozen: u32,
    pub unobservable: u32,
}

/// Compare what the external side holds with each ledger. Anything above
/// our total, or below what it confirmed before, freezes the ledger.
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
                "SELECT l.recipient_id, l.cumulative_units::text, l.confirmed_units::text,
                        r.org_id
                   FROM trace_settlement_rail_ledgers l
                   JOIN trace_near_ai_credit_recipients r
                     ON r.tenant_id = l.tenant_id AND r.recipient_id = l.recipient_id
                  WHERE l.tenant_id = $1 AND l.rail_id = $2 AND r.revoked_at IS NULL
                    AND l.publish_state IN ('idle', 'due', 'published')
                  ORDER BY l.verified_at NULLS FIRST
                  LIMIT $3",
                &[&tenant, &rail.rail_id(), &limit],
            )
            .await?;
        tx.commit().await?;
        rows
    };
    let mut summary = ReconcileSummary::default();
    for row in rows {
        let recipient_id: Uuid = row.get(0);
        let org_id: Uuid = row.get(3);
        let observed = match rail.observe(&RecipientRef::new(org_id.to_string())).await {
            Ok(Some(v)) => v,
            Ok(None) => {
                summary.unobservable += 1;
                continue;
            }
            Err(e) if e.kind == RailErrorKind::Retry => continue,
            Err(e) => {
                freeze(pool, tenant, rail.rail_id(), recipient_id, e.label).await?;
                summary.frozen += 1;
                continue;
            }
        };
        let mut client = pool.get().await?;
        let tx = client.transaction().await?;
        scope(&tx, tenant).await?;
        // Re-read under lock: an accrual or a publish may have moved it.
        let Some(current) = tx
            .query_opt(
                "SELECT cumulative_units::text, confirmed_units::text, publish_state
                   FROM trace_settlement_rail_ledgers
                  WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3
                  FOR UPDATE",
                &[&tenant, &rail.rail_id(), &recipient_id],
            )
            .await?
        else {
            continue;
        };
        let cumulative = parse_units(&current.get::<_, String>(0))?;
        let confirmed = parse_units(&current.get::<_, String>(1))?;
        let state: String = current.get(2);
        if state == "leased" || state == "frozen" {
            continue;
        }
        let (next_state, label) = match compare(observed, cumulative, confirmed) {
            Comparison::Current => {
                summary.verified += 1;
                ("idle", None)
            }
            Comparison::Behind => {
                summary.behind += 1;
                (
                    if state == "published" {
                        "published"
                    } else {
                        "due"
                    },
                    None,
                )
            }
            Comparison::Ahead => {
                summary.frozen += 1;
                ("frozen", Some("settlement_rail_external_ahead"))
            }
            Comparison::Regressed => {
                summary.frozen += 1;
                ("frozen", Some("settlement_rail_external_regressed"))
            }
        };
        let new_confirmed = if label.is_none() { observed } else { confirmed };
        tx.execute(
            "UPDATE trace_settlement_rail_ledgers
                SET publish_state = $4, confirmed_units = $5::text::numeric,
                    last_error_label = COALESCE($6, last_error_label),
                    verified_at = NOW(), updated_at = NOW()
              WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3",
            &[
                &tenant,
                &rail.rail_id(),
                &recipient_id,
                &next_state,
                &new_confirmed.to_string(),
                &label,
            ],
        )
        .await?;
        tx.commit().await?;
    }
    Ok(summary)
}

async fn freeze(
    pool: &Pool,
    tenant: &str,
    rail_id: &str,
    recipient_id: Uuid,
    label: &'static str,
) -> Result<(), StoreError> {
    let label = if is_safe_label(label) {
        label
    } else {
        "settlement_rail_label_invalid"
    };
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    tx.execute(
        "UPDATE trace_settlement_rail_ledgers
            SET publish_state = 'frozen', last_error_label = $4, updated_at = NOW()
          WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3
            AND publish_state <> 'leased'",
        &[&tenant, &rail_id, &recipient_id, &label],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Operator action: release a frozen ledger to be published again.
pub async fn unfreeze(
    pool: &Pool,
    tenant: &str,
    rail_id: &str,
    recipient_id: Uuid,
) -> Result<bool, StoreError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    let n = tx
        .execute(
            "UPDATE trace_settlement_rail_ledgers
                SET publish_state = 'due', next_attempt_at = NOW(), last_error_label = NULL,
                    updated_at = NOW()
              WHERE tenant_id = $1 AND rail_id = $2 AND recipient_id = $3
                AND publish_state = 'frozen'",
            &[&tenant, &rail_id, &recipient_id],
        )
        .await?;
    tx.commit().await?;
    Ok(n == 1)
}

/// Pull mode: the signed statement of the active recipient's cumulative
/// total, for NEAR AI to fetch. `None` when the tenant has no active
/// recipient or its ledger is frozen (a frozen total is not served).
pub async fn signed_statement(
    pool: &Pool,
    tenant: &str,
    rail_id: &str,
    signer: &StatementSigner,
    now: DateTime<Utc>,
) -> Result<Option<SignedCreditStatement>, StoreError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    scope(&tx, tenant).await?;
    let row = tx
        .query_opt(
            "SELECT r.org_id, r.org_ref_hash, l.cumulative_units::text
               FROM trace_near_ai_credit_recipients r
               JOIN trace_settlement_rail_ledgers l
                 ON l.tenant_id = r.tenant_id AND l.recipient_id = r.recipient_id
              WHERE r.tenant_id = $1 AND r.revoked_at IS NULL AND l.rail_id = $2
                AND l.publish_state <> 'frozen'",
            &[&tenant, &rail_id],
        )
        .await?;
    tx.commit().await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let org_id: Uuid = row.get(0);
    let org_ref_hash: String = row.get(1);
    let cumulative = parse_units(&row.get::<_, String>(2))?;
    Ok(Some(signer.sign(CreditStatement {
        org_id: org_id.to_string(),
        cumulative_nano_usd: cumulative.to_string(),
        statement_ref: statement_ref(rail_id, &org_ref_hash, cumulative),
        issued_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    })))
}
