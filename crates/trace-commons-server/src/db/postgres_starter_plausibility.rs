// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Storage for the NEAR AI starter advance's plausibility features (V131).
//! Both calls run in a transaction scoped to the tenant under forced RLS.
//! See `crate::starter_plausibility` and
//! `docs/superpowers/specs/2026-10-21-nearai-starter-advance-design.md`.

use uuid::Uuid;

use super::PgBackend;
use crate::account_trust::TrustAccount;
use crate::error::DatabaseError;
use crate::starter_plausibility::{MAX_EVALUATED_ENVELOPES, StarterEnvelopeFeatures};

/// An account's counted features: its own submissions (through a live
/// ownership relation, including verified device uploads), still `accepted`
/// and not withdrawn. Read newest first,
/// so the cap drops the oldest rather than the ones inside the age window;
/// the caller reverses them back to oldest first.
const ACCOUNT_FEATURES_SQL: &str = "SELECT f.features
   FROM trace_starter_plausibility_features f
   JOIN trace_submissions s
     ON s.tenant_id = f.tenant_id AND s.submission_id = f.submission_id
   JOIN trace_account_owned_principals p
     ON p.tenant_id = s.tenant_id AND p.principal_ref = s.auth_principal_ref
    AND p.account_id = $2
  WHERE f.tenant_id = $1 AND f.tenant_id = trace_current_tenant_id()
    AND s.status = 'accepted' AND s.withdrawn_at IS NULL
  ORDER BY s.received_at DESC, s.submission_id DESC
  LIMIT $3";

impl PgBackend {
    /// Records one submission's features once. Returns whether a row was
    /// written; a second record for the same submission is a no-op.
    pub async fn record_starter_plausibility_features(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        features: &StarterEnvelopeFeatures,
    ) -> Result<bool, DatabaseError> {
        let value = serde_json::to_value(features)
            .map_err(|_| DatabaseError::Pool("starter_features_unserializable".into()))?;
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let written = tx
            .execute(
                "INSERT INTO trace_starter_plausibility_features
                     (tenant_id, submission_id, features_version, features)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (tenant_id, submission_id) DO NOTHING",
                &[&tenant_id, &submission_id, &features.version, &value],
            )
            .await?;
        tx.commit().await?;
        Ok(written == 1)
    }

    /// The features the evaluator counts for one account. A stored record
    /// that no longer parses is skipped, not an error: the evaluator ignores
    /// unknown versions the same way.
    pub async fn starter_plausibility_features_for_account(
        &self,
        account: &TrustAccount,
    ) -> Result<Vec<StarterEnvelopeFeatures>, DatabaseError> {
        let tenant = account.tenant_id();
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                ACCOUNT_FEATURES_SQL,
                &[
                    &tenant,
                    &account.account_id(),
                    &(MAX_EVALUATED_ENVELOPES as i64),
                ],
            )
            .await?;
        tx.commit().await?;
        // Read newest first so the cap drops the oldest; returned oldest
        // first.
        Ok(rows
            .into_iter()
            .rev()
            .filter_map(|row| {
                let value: serde_json::Value = row.get(0);
                serde_json::from_value(value).ok()
            })
            .collect())
    }
}
