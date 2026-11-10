// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `PgPipelineStore`: the pipeline's PostgreSQL store.

use super::*;

impl PgPipelineStore {
    pub fn new(backend: Arc<PgBackend>) -> Self {
        Self {
            backend,
            lost_follow_up_cursors: Arc::default(),
        }
    }

    pub(super) async fn tenant_transaction<'a>(
        client: &'a mut deadpool_postgres::Client,
        tenant_id: &str,
    ) -> Result<Transaction<'a>, DatabaseError> {
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await?;
        Ok(tx)
    }

    /// `tenant_transaction` that states its isolation level, READ COMMITTED,
    /// when it begins (`START TRANSACTION ISOLATION LEVEL READ COMMITTED`),
    /// whatever `default_transaction_isolation` the database or the role
    /// sets (final fix wave G14). For a transaction whose correctness rests
    /// on a statement that reads what committed while an earlier statement
    /// waited for a lock: the receipt's staging and commit transactions (the
    /// routing read after the shared routing lock), the index rebuild's
    /// re-check of a run after it locks the run row, and the invalidation
    /// claim. Under REPEATABLE READ such a statement would read the snapshot
    /// of the transaction's first statement, taken before the wait, and no
    /// error would say so: an advisory lock has no row version.
    pub(crate) async fn read_committed_tenant_transaction<'a>(
        client: &'a mut deadpool_postgres::Client,
        tenant_id: &str,
    ) -> Result<Transaction<'a>, DatabaseError> {
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
            .start()
            .await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await?;
        Ok(tx)
    }

    pub async fn register_bundle(
        &self,
        tenant_id: &str,
        package: &BundlePackage,
    ) -> Result<(), DatabaseError> {
        package
            .validate()
            .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
        let package_json = serde_json::to_value(package)
            .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
        let format_version = i32::try_from(package.manifest.format_version)
            .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)
             ON CONFLICT (tenant_id) DO NOTHING",
            &[&tenant_id],
        )
        .await?;
        // Serialize the descriptor-conflict check below against a concurrent
        // registration for this tenant, so two packages racing to pin a new
        // instrument cannot both read past each other and both commit.
        let lock_key = format!("pipeline-bundle-registry:{tenant_id}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 1))",
            &[&lock_key],
        )
        .await?;
        let registered = tx
            .query(
                "SELECT package FROM pipeline_bundle_packages WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        for row in &registered {
            let registered_package: BundlePackage = serde_json::from_value(
                row.get::<_, serde_json::Value>("package"),
            )
            .map_err(|_| DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string()))?;
            for (instrument_id, descriptor) in &package.manifest.instruments {
                if let Some(registered_descriptor) =
                    registered_package.manifest.instruments.get(instrument_id)
                {
                    if registered_descriptor != descriptor {
                        return Err(DatabaseError::Constraint(
                            "bundle_instrument_conflict".to_string(),
                        ));
                    }
                }
            }
        }
        tx.execute(
            "INSERT INTO pipeline_bundle_packages (
                tenant_id, bundle_id, manifest_format_version, package
             ) VALUES ($1,$2,$3,$4)
             ON CONFLICT (tenant_id, bundle_id) DO NOTHING",
            &[
                &tenant_id,
                &package.bundle_id,
                &format_version,
                &package_json,
            ],
        )
        .await?;
        let stored: serde_json::Value = tx
            .query_one(
                "SELECT package FROM pipeline_bundle_packages
                 WHERE tenant_id = $1 AND bundle_id = $2",
                &[&tenant_id, &package.bundle_id],
            )
            .await?
            .get("package");
        if stored != package_json {
            return Err(DatabaseError::Constraint(
                "bundle identifier already has different package bytes".to_string(),
            ));
        }
        for phase in [Phase::Admission, Phase::Review, Phase::Score, Phase::Settle] {
            tx.execute(
                "INSERT INTO pipeline_bundle_policy_status (
                    tenant_id, bundle_id, phase, runnable
                 ) VALUES ($1,$2,$3,TRUE)
                 ON CONFLICT (tenant_id, bundle_id, phase) DO NOTHING",
                &[&tenant_id, &package.bundle_id, &phase_as_db(Some(phase))],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Selects `bundle_id` as the tenant's active bundle only when the tenant
    /// has none (an `INSERT ... ON CONFLICT DO NOTHING`, which needs no
    /// `UPDATE`): the default bundle's registration at start. It never
    /// replaces a selection, so a start after an activation or a rollback
    /// leaves the selected bundle. The ingest runtime switches a tenant's
    /// bundle only through the qualified activation gate
    /// (`PipelineQualificationStore::activate_qualified_bundle_in`, V112).
    pub async fn activate_bundle_if_none(
        &self,
        tenant_id: &str,
        bundle_id: &str,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO pipeline_active_bundles (tenant_id, bundle_id)
             SELECT $1, bundle_id
             FROM pipeline_bundle_packages
             WHERE tenant_id = $1 AND bundle_id = $2
             ON CONFLICT (tenant_id) DO NOTHING",
            &[&tenant_id, &bundle_id],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The bundles a worker may run for `tenant_id`: its active bundle and
    /// the bundle of each run not yet `complete` or `failed` (a tenant keeps
    /// both when the default package changes). Sorted, without repeats.
    pub async fn runnable_bundle_ids(&self, tenant_id: &str) -> Result<Vec<String>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1
                 UNION
                 SELECT bundle_id FROM pipeline_runs
                  WHERE tenant_id = $1
                    AND state IN ('pending', 'leased', 'retry', 'awaiting_review',
                                  'awaiting_operator')
                 ORDER BY 1",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(|row| row.get("bundle_id")).collect())
    }

    pub async fn active_bundle_id(&self, tenant_id: &str) -> Result<Option<String>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        Ok(row.map(|row| row.get("bundle_id")))
    }

    pub async fn load_bundle(
        &self,
        tenant_id: &str,
        bundle_id: &str,
    ) -> Result<Option<BundlePackage>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let package = load_bundle_from_transaction(&tx, tenant_id, bundle_id).await?;
        tx.commit().await?;
        Ok(package)
    }

    pub async fn policy_is_runnable(
        &self,
        tenant_id: &str,
        bundle_id: &str,
        phase: Phase,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let runnable = tx
            .query_opt(
                "SELECT runnable FROM pipeline_bundle_policy_status
                 WHERE tenant_id = $1 AND bundle_id = $2 AND phase = $3",
                &[&tenant_id, &bundle_id, &phase_as_db(Some(phase))],
            )
            .await?
            .map(|row| row.get("runnable"))
            .unwrap_or(false);
        tx.commit().await?;
        Ok(runnable)
    }

    /// An operator suspends or resumes one policy of a bound bundle (port
    /// `ef97a459` lines 678 to 829, with `terminate` refused). The status row
    /// is the only row it locks (`FOR UPDATE`), so it takes no lock a phase
    /// commit takes before the policy row, and it waits for every transaction
    /// that holds the row `FOR SHARE` (`lock_runnable_policy`): a transaction
    /// that passed its guard lands before the suspension returns, and one that
    /// starts after it is refused.
    ///
    /// What a suspension stops, once it has returned: each phase's commit
    /// (Admission at the receipt, Review, Score, Settle); in Settle, a new
    /// selection, the index write, each leg's external call, and the credit
    /// event with its batch; and a payout dispatch (a submit to the NEAR
    /// adapter, never a confirmation lookup). A run bound to the policy waits
    /// in `retry`, uncharged, under the same bundle, and continues from what it
    /// has stored after the resume.
    ///
    /// What it does not stop: an external call that is already in flight (a
    /// guard does not claim to retract an accepted operation: an index write
    /// that holds the policy row finishes first, a leg's adapter call that was
    /// accepted stays accepted, and a NEAR submit already made stays made); a
    /// payout confirmation; and the failure and withdrawal paths, which write
    /// no outcome and run in every policy state. A `resume` does not wake the
    /// released runs at once: each waits for its retry backoff (the age of its
    /// phase, at most one hour).
    ///
    /// How long a `suspend` can wait: for the commit transactions, a moment;
    /// for the credit transaction of a Settle leg, its own few statements; and
    /// for a Settle index dispatch, which holds the run row, the submission
    /// row, and the policy row through every upsert to the commit of
    /// `index_write_state`, at most the earlier of the Settle lease's end and
    /// `PIPELINE_INDEX_DISPATCH_BUDGET_SECONDS` (30 seconds): past that the
    /// dispatch stops writing and rolls back. The intervention itself waits
    /// at most 5 seconds for the row (`PIPELINE_ADMIN_LOCK_TIMEOUT_SQL`), then
    /// fails with `policy_intervention_busy` and writes nothing: the operator
    /// repeats it (an index dispatch can hold the row for longer than that).
    ///
    /// Only `suspend` of a runnable policy and `resume` of a suspended one
    /// are transitions. `terminate` is `policy_intervention_not_supported`;
    /// any other action, a `bundle_id` that is not `sha256:` and 64 lowercase
    /// hex digits, and an actor or reason that fails `validate_actor`, are
    /// `policy_intervention_invalid` (no text of the caller is in the error);
    /// and a transition that does not exist is
    /// `policy_intervention_no_transition`. Each refusal, and a well-formed
    /// bundle the tenant has no policy row for (`NotFound`), writes nothing.
    ///
    /// A suspension keeps `runnable` and `operational_status` equal (V111's
    /// check) and puts the reason in `error_label`, which a resume clears.
    /// It changes no run and no bundle. The record's `recorded_at` is the
    /// database clock at the write, not the transaction's start, so a resume
    /// that waited for a suspension is never listed before it.
    pub async fn intervene_policy(
        &self,
        tenant_id: &str,
        bundle_id: &str,
        phase: Phase,
        action: &str,
        actor_principal_ref: &str,
        reason_code: &str,
    ) -> Result<PipelinePolicyInterventionRecord, DatabaseError> {
        validate_actor(
            actor_principal_ref,
            reason_code,
            PIPELINE_POLICY_INTERVENTION_INVALID_LABEL,
        )?;
        // The shape the table checks, refused before the id can reach a
        // query or an error string (`NotFound` names the bundle).
        if !is_bundle_id(bundle_id) {
            return Err(DatabaseError::Constraint(
                PIPELINE_POLICY_INTERVENTION_INVALID_LABEL.to_string(),
            ));
        }
        let resulting = match action {
            "suspend" => PolicyOperationalStatus::Suspended,
            "resume" => PolicyOperationalStatus::Runnable,
            "terminate" => {
                return Err(DatabaseError::Constraint(
                    PIPELINE_POLICY_INTERVENTION_NOT_SUPPORTED_LABEL.to_string(),
                ));
            }
            _ => {
                return Err(DatabaseError::Constraint(
                    PIPELINE_POLICY_INTERVENTION_INVALID_LABEL.to_string(),
                ));
            }
        };
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        // The wait for the policy row is bounded (G15); a wait that runs out
        // is `policy_intervention_busy`, and nothing is written.
        tx.batch_execute(PIPELINE_ADMIN_LOCK_TIMEOUT_SQL).await?;
        let row = tx
            .query_opt(
                "SELECT operational_status
                   FROM pipeline_bundle_policy_status
                  WHERE tenant_id = $1 AND bundle_id = $2 AND phase = $3
                  FOR UPDATE",
                &[&tenant_id, &bundle_id, &phase_as_db(Some(phase))],
            )
            .await
            .map_err(|error| {
                lock_timeout_as(error.into(), PIPELINE_POLICY_INTERVENTION_BUSY_LABEL)
            })?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "pipeline_bundle_policy".to_string(),
                id: format!("{bundle_id}:{}", phase_as_db(Some(phase))),
            })?;
        let previous =
            PolicyOperationalStatus::from_db(row.get::<_, String>("operational_status").as_str())?;
        if !matches!(
            (previous, resulting),
            (
                PolicyOperationalStatus::Runnable,
                PolicyOperationalStatus::Suspended
            ) | (
                PolicyOperationalStatus::Suspended,
                PolicyOperationalStatus::Runnable
            )
        ) {
            return Err(DatabaseError::Constraint(
                PIPELINE_POLICY_INTERVENTION_NO_TRANSITION_LABEL.to_string(),
            ));
        }
        let evidence_hash = sha256_prefixed(
            format!(
                "trace-commons-policy-intervention\0{tenant_id}\0{bundle_id}\0{}\0{action}\0{actor_principal_ref}\0{reason_code}\0{}\0{}",
                phase_as_db(Some(phase)),
                previous.as_db(),
                resulting.as_db()
            )
            .as_bytes(),
        );
        let runnable = resulting == PolicyOperationalStatus::Runnable;
        let error_label = (!runnable).then_some(reason_code);
        tx.execute(
            "UPDATE pipeline_bundle_policy_status
                SET runnable = $4, operational_status = $5, error_label = $6,
                    updated_at = clock_timestamp()
              WHERE tenant_id = $1 AND bundle_id = $2 AND phase = $3",
            &[
                &tenant_id,
                &bundle_id,
                &phase_as_db(Some(phase)),
                &runnable,
                &resulting.as_db(),
                &error_label,
            ],
        )
        .await?;
        let intervention_id = Uuid::new_v4();
        let recorded_at: DateTime<Utc> = tx
            .query_one(
                "INSERT INTO pipeline_policy_interventions (
                    tenant_id, intervention_id, bundle_id, phase, action,
                    actor_principal_ref, reason_code, previous_status,
                    resulting_status, evidence_hash, recorded_at
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10, clock_timestamp())
                 RETURNING recorded_at",
                &[
                    &tenant_id,
                    &intervention_id,
                    &bundle_id,
                    &phase_as_db(Some(phase)),
                    &action,
                    &actor_principal_ref,
                    &reason_code,
                    &previous.as_db(),
                    &resulting.as_db(),
                    &evidence_hash,
                ],
            )
            .await?
            .get("recorded_at");
        tx.commit().await?;
        Ok(PipelinePolicyInterventionRecord {
            intervention_id,
            bundle_id: bundle_id.to_string(),
            phase,
            action: action.to_string(),
            actor_principal_ref: actor_principal_ref.to_string(),
            reason_code: reason_code.to_string(),
            previous_status: previous,
            resulting_status: resulting,
            evidence_hash,
            recorded_at,
        })
    }

    /// The interventions on `bundle_id`'s policies, oldest first.
    pub async fn list_policy_interventions(
        &self,
        tenant_id: &str,
        bundle_id: &str,
    ) -> Result<Vec<PipelinePolicyInterventionRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT intervention_id, bundle_id, phase, action, actor_principal_ref,
                        reason_code, previous_status, resulting_status, evidence_hash,
                        recorded_at
                   FROM pipeline_policy_interventions
                  WHERE tenant_id = $1 AND bundle_id = $2
                  ORDER BY recorded_at ASC, intervention_id ASC",
                &[&tenant_id, &bundle_id],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(policy_intervention_from_row).collect()
    }

    pub async fn get_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT * FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(pipeline_run_from_row).transpose()
    }

    pub async fn list_outcomes(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Vec<PhaseOutcomeRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT * FROM phase_outcomes
                 WHERE tenant_id = $1 AND run_id = $2
                 ORDER BY CASE phase
                    WHEN 'admission' THEN 1
                    WHEN 'review' THEN 2
                    WHEN 'score' THEN 3
                    WHEN 'settle' THEN 4
                 END",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(phase_outcome_from_row).collect()
    }

    /// Loads the single committed outcome for `phase`, or `None` if the run
    /// has not reached it yet -- one targeted query instead of listing every
    /// phase outcome and filtering in memory.
    pub async fn outcome_for_phase(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        phase: Phase,
    ) -> Result<Option<PhaseOutcomeRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT * FROM phase_outcomes
                 WHERE tenant_id = $1 AND run_id = $2 AND phase = $3",
                &[&tenant_id, &run_id, &phase_as_db(Some(phase))],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(phase_outcome_from_row).transpose()
    }

    /// Claims the next due run for `tenant_id` across every phase, sizing
    /// the lease this claim grants by the claimed row's own `next_phase`:
    /// the CASE below picks Review's, Score's, or Settle's configured lease
    /// for the one row the candidate CTE selects, in the same statement
    /// that claims it, so a Score run never gets Review's shorter lease or
    /// vice versa.
    ///
    /// First it sweeps: a run still `leased` after its lease expired with
    /// its attempts exhausted (its worker is gone) is failed as
    /// `attempts_exhausted`. In the same transaction, every open
    /// leg of a swept Settle run is resolved without an adapter call
    /// (`resolve_open_settlement_legs_on_tx`) -- there is no worker to make
    /// one -- and the `received` submission of a swept run that Review never
    /// decided moves to `rejected`
    /// (`reject_received_submissions_of_failed_runs_on_tx`, #1346).
    pub async fn claim_next(
        &self,
        tenant_id: &str,
        lease_config: PipelineLeaseConfig,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        // #1313 (b): an expired, exhausted Settle run that can finish
        // (`finishable_settle_run_sql!`) is not failed by the sweep below.
        // It gets its one extra attempt back, due now, so this claim or a
        // later one commits it: it ends `complete`, and N-7 keeps its
        // revision. Same lock order and skip rule as the sweep.
        tx.execute(
            concat!(
                "WITH finishable AS MATERIALIZED (
                    SELECT p.run_id FROM pipeline_runs p
                     WHERE p.tenant_id = $1
                       AND p.state = 'leased'
                       AND p.lease_expires_at <= NOW()
                       AND p.attempt_count >= p.max_attempts
                       AND ",
                finishable_settle_run_sql!(),
                "
                     ORDER BY p.run_id
                     FOR UPDATE OF p SKIP LOCKED
                 )
                 UPDATE pipeline_runs r
                 SET state = 'retry', lease_token = NULL, lease_expires_at = NULL,
                     attempt_count = GREATEST(r.max_attempts - 1, 0),
                     next_attempt_at = NOW(),
                     settle_finish_granted_at = clock_timestamp(),
                     last_error_label = $2, updated_at = NOW()
                 FROM finishable
                 WHERE r.tenant_id = $1 AND r.run_id = finishable.run_id"
            ),
            &[&tenant_id, &PIPELINE_SETTLE_FINISH_GRANTED_LABEL],
        )
        .await?;
        // #1413 review: an expired, exhausted run that was held before (an
        // operator released it, and its attempt crashed) is held again
        // rather than failed, so N-7 does not remove the paid revision the
        // operator chose to keep. Same lock order and skip rule as the sweep.
        let reheld = tx
            .execute(
                concat!(
                    "WITH reheld AS MATERIALIZED (
                        SELECT p.run_id FROM pipeline_runs p
                         WHERE p.tenant_id = $1
                           AND p.state = 'leased'
                           AND p.lease_expires_at <= NOW()
                           AND p.attempt_count >= p.max_attempts
                           AND p.operator_held_at IS NOT NULL
                           AND ",
                    settle_run_is_finishable_sql!(),
                    " AND ",
                    settle_run_has_paid_leg_sql!(),
                    "
                         ORDER BY p.run_id
                         FOR UPDATE OF p SKIP LOCKED
                     )
                     UPDATE pipeline_runs r
                     SET state = 'awaiting_operator', lease_token = NULL,
                         lease_expires_at = NULL,
                         operator_held_at = clock_timestamp(),
                         review_audit_pending_at = clock_timestamp(),
                         last_error_label = $2, updated_at = NOW()
                     FROM reheld
                     WHERE r.tenant_id = $1 AND r.run_id = reheld.run_id"
                ),
                &[&tenant_id, &PIPELINE_ATTEMPTS_EXHAUSTED_LABEL],
            )
            .await?;
        if reheld > 0 {
            tracing::warn!(
                label = PIPELINE_SETTLE_HELD_FOR_OPERATOR_LABEL,
                reason_label = PIPELINE_ATTEMPTS_EXHAUSTED_LABEL,
                tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id).as_str(),
                held_runs = reheld,
                "released pipeline runs are held for an operator again"
            );
        }
        // Multi-lens review L4-1 and L4-6(a): the sweep locks in run id order
        // and skips a row another transaction holds (a live dispatch, a
        // withdrawal), so a claim never waits behind one and two sweeps
        // cannot deadlock with a withdrawal; a later claim sweeps the row.
        let swept = tx
            .query(
                "WITH expired AS MATERIALIZED (
                    SELECT run_id FROM pipeline_runs
                     WHERE tenant_id = $1
                       AND state = 'leased'
                       AND lease_expires_at <= NOW()
                       AND attempt_count >= max_attempts
                     ORDER BY run_id
                     FOR UPDATE SKIP LOCKED
                 )
                 UPDATE pipeline_runs p
                 SET state = 'failed', lease_token = NULL, lease_expires_at = NULL,
                     last_error_label = $2, updated_at = NOW()
                 FROM expired
                 WHERE p.tenant_id = $1 AND p.run_id = expired.run_id
                 RETURNING p.run_id, p.next_phase",
                &[&tenant_id, &PIPELINE_ATTEMPTS_EXHAUSTED_LABEL],
            )
            .await?;
        let swept_settle_runs = swept
            .iter()
            .filter(|row| row.get::<_, &str>("next_phase") == phase_as_db(Some(Phase::Settle)))
            .map(|row| row.get::<_, Uuid>("run_id"))
            .collect::<Vec<_>>();
        resolve_open_settlement_legs_on_tx(&tx, tenant_id, &swept_settle_runs).await?;
        invalidate_index_writes_of_failed_runs_on_tx(&tx, tenant_id, &swept_settle_runs).await?;
        let swept_runs = swept
            .iter()
            .map(|row| row.get::<_, Uuid>("run_id"))
            .collect::<Vec<_>>();
        reject_received_submissions_of_failed_runs_on_tx(&tx, tenant_id, &swept_runs).await?;
        let lease_token = Uuid::new_v4();
        let review_milliseconds = lease_config.review().num_milliseconds();
        let score_milliseconds = lease_config.score().num_milliseconds();
        let settle_milliseconds = lease_config.settle().num_milliseconds();
        let row = tx
            .query_opt(
                "WITH candidate AS (
                    SELECT run_id FROM pipeline_runs
                    WHERE tenant_id = $1
                      AND next_phase <> 'none'
                      AND attempt_count < max_attempts
                      AND (
                          (state IN ('pending', 'retry') AND next_attempt_at <= NOW())
                          OR (state = 'leased' AND lease_expires_at <= NOW())
                      )
                    ORDER BY next_attempt_at ASC, created_at ASC, run_id ASC
                    FOR UPDATE SKIP LOCKED
                    LIMIT 1
                 )
                 UPDATE pipeline_runs p
                 SET state = 'leased',
                     lease_token = $2,
                     lease_expires_at = NOW() + (
                         CASE p.next_phase
                             WHEN 'review' THEN $3::bigint
                             WHEN 'score' THEN $4::bigint
                             WHEN 'settle' THEN $5::bigint
                             ELSE $3::bigint
                         END * INTERVAL '1 millisecond'
                     ),
                     attempt_count = p.attempt_count + 1,
                     last_error_label = NULL,
                     updated_at = NOW()
                 FROM candidate
                 WHERE p.tenant_id = $1 AND p.run_id = candidate.run_id
                 RETURNING p.*",
                &[
                    &tenant_id,
                    &lease_token,
                    &review_milliseconds,
                    &score_milliseconds,
                    &settle_milliseconds,
                ],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(pipeline_run_from_row).transpose()
    }

    /// Claims a specific run with an explicit lease length, kept for tests
    /// that need precise control over a claim's own lease (many call this
    /// directly with `chrono::Duration::seconds(30)`).
    /// `PipelineService::process_run` uses `claim_run_with_lease_config`
    /// instead, so it never has to read the run's phase before claiming it.
    /// The bound was 5 minutes; it is now 2 hours so a caller
    /// can hand this the same lease `PipelineLeaseConfig` allows.
    pub async fn claim_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        lease_duration: Duration,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        if lease_duration <= Duration::zero() || lease_duration > Duration::hours(2) {
            return Err(DatabaseError::Constraint(
                "pipeline lease duration is invalid".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let lease_token = Uuid::new_v4();
        let lease_milliseconds = lease_duration.num_milliseconds();
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET state = 'leased',
                     lease_token = $3,
                     lease_expires_at = NOW() + ($4::bigint * INTERVAL '1 millisecond'),
                     attempt_count = attempt_count + 1,
                     last_error_label = NULL,
                     updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND next_phase <> 'none'
                   AND attempt_count < max_attempts
                   AND (
                       (state IN ('pending', 'retry') AND next_attempt_at <= NOW())
                       OR (state = 'leased' AND lease_expires_at <= NOW())
                   )
                 RETURNING *",
                &[&tenant_id, &run_id, &lease_token, &lease_milliseconds],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(pipeline_run_from_row).transpose()
    }

    /// Claims a specific run, picking the granted lease by its own
    /// `next_phase` in the claiming SQL itself -- the same `CASE`
    /// `claim_next` uses, scoped to one `run_id` instead
    /// of scanning the whole tenant queue. `PipelineService::process_run`
    /// uses this rather than reading the run's phase first and handing
    /// `claim_run` an explicit duration: a phase commit landing between an
    /// earlier read and a later claim could otherwise hand out a lease sized
    /// for the phase the row was in at read time, not the one it is
    /// actually claimed for.
    pub async fn claim_run_with_lease_config(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        lease_config: PipelineLeaseConfig,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let lease_token = Uuid::new_v4();
        let review_milliseconds = lease_config.review().num_milliseconds();
        let score_milliseconds = lease_config.score().num_milliseconds();
        let settle_milliseconds = lease_config.settle().num_milliseconds();
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs p
                 SET state = 'leased',
                     lease_token = $3,
                     lease_expires_at = NOW() + (
                         CASE p.next_phase
                             WHEN 'review' THEN $4::bigint
                             WHEN 'score' THEN $5::bigint
                             WHEN 'settle' THEN $6::bigint
                             ELSE $4::bigint
                         END * INTERVAL '1 millisecond'
                     ),
                     attempt_count = p.attempt_count + 1,
                     last_error_label = NULL,
                     updated_at = NOW()
                 WHERE p.tenant_id = $1 AND p.run_id = $2
                   AND next_phase <> 'none'
                   AND attempt_count < max_attempts
                   AND (
                       (state IN ('pending', 'retry') AND next_attempt_at <= NOW())
                       OR (state = 'leased' AND lease_expires_at <= NOW())
                   )
                 RETURNING p.*",
                &[
                    &tenant_id,
                    &run_id,
                    &lease_token,
                    &review_milliseconds,
                    &score_milliseconds,
                    &settle_milliseconds,
                ],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(pipeline_run_from_row).transpose()
    }

    pub async fn release_claim(&self, run: &PipelineRunRecord) -> Result<(), DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let updated = tx
            .execute(
                "UPDATE pipeline_runs
             SET state = 'pending', lease_token = NULL, lease_expires_at = NULL,
                 attempt_count = GREATEST(attempt_count - 1, 0), updated_at = NOW()
             WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
               AND lease_token = $3 AND lease_expires_at > NOW()",
                &[&run.tenant_id, &run.run_id, &lease_token],
            )
            .await?;
        if updated != 1 {
            return Err(stale_lease_error());
        }
        tx.commit().await?;
        Ok(())
    }

    /// Extends a lease this claim still holds. Never shortens it, never
    /// extends past `lease_cap`, and changes nothing when the token no longer
    /// matches or the lease already expired -- the same token-plus-liveness
    /// fence every other lease-checked write uses, so a lost lease is simply
    /// never renewed rather than renewed onto a run another worker now holds.
    /// Called only by `PipelineLeaseRenewal`'s background task, on its own
    /// connection checkout, never inside another transaction.
    ///
    /// Liveness is decided only once the run row is held (wave 2; rebase 10
    /// review, M7; fix round 1, review M4): the transaction first locks the
    /// row (`SELECT ... FOR NO KEY UPDATE`, the mode the UPDATE itself takes,
    /// so it does not also wait on a child row's foreign-key `FOR KEY SHARE`;
    /// fix round 2), waiting out any holder, and the UPDATE
    /// that follows compares `lease_expires_at` with `clock_timestamp()`,
    /// the time it runs, not `NOW()`, the transaction's start. A lease that
    /// expired while the renewal waited is then not renewed, whether the
    /// holder updated the row or only locked it (a lock-only holder gives
    /// no re-check of a waiting UPDATE's `WHERE`, which is why the lock comes
    /// first). The new expiry still counts from `NOW()`, which can only make
    /// it earlier. Lock order: the run row only, as every lease-checked
    /// write takes it.
    pub async fn renew_lease(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        lease_token: Uuid,
        extension: Duration,
        lease_cap: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let extension_ms = extension.num_milliseconds();
        tx.query_opt(
            "SELECT 1 FROM pipeline_runs
              WHERE tenant_id = $1 AND run_id = $2 FOR NO KEY UPDATE",
            &[&tenant_id, &run_id],
        )
        .await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                    SET lease_expires_at = GREATEST(
                            lease_expires_at,
                            LEAST(NOW() + ($4::bigint * INTERVAL '1 millisecond'), $5)
                        ),
                        updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $3 AND lease_expires_at > clock_timestamp()
                  RETURNING lease_expires_at",
                &[&tenant_id, &run_id, &lease_token, &extension_ms, &lease_cap],
            )
            .await?;
        tx.commit().await?;
        Ok(row.map(|row| row.get(0)))
    }

    /// Commits the Review outcome together with its approved-content
    /// provenance and the phase transition, in one transaction (decision
    /// D7). On approval: the approved object ref, its `trace_derived_records`
    /// row, the submission's `accepted` status, the outcome row, and the
    /// run's `next_phase = score` / `approved_*` columns all commit or none
    /// do. On rejection: only the submission's `rejected` status, the
    /// outcome row, and the run's terminal transition commit -- the run's
    /// `approved_*` columns stay NULL, satisfying
    /// `pipeline_runs_approved_content_shape`.
    ///
    /// An approval also resets `attempt_count` to 0: each phase has the
    /// run's whole `max_attempts` budget (multi-lens review L2-1, owner
    /// decision of 2026-10-01), so a Review that commits on its last
    /// attempt leaves Score a claimable run rather than a `pending` one no
    /// claim can select. A terminal commit keeps its count.
    ///
    /// The submission-operability
    /// guard that gated the source read at the start of Review
    /// (`load_object_bytes`) was checked in an earlier, already-committed
    /// transaction. A withdrawal (or an expiry/purge) landing after that
    /// read and before this commit -- for example while this attempt was
    /// writing the approved object -- must never be reversed by an approval
    /// or rejection racing in behind it. This re-checks operability here,
    /// under the same lock this commit already takes, and refuses the whole
    /// commit (`PIPELINE_SUBMISSION_INOPERABLE_LABEL`) rather than resurrect
    /// the trace: no object ref, no derived record, no status change, no
    /// outcome, no run update.
    ///
    /// The same holds for the Review policy (GRD-004): the commit locks its
    /// status row `FOR SHARE` after those two guards and refuses with
    /// `PIPELINE_POLICY_NOT_RUNNABLE_LABEL`, writing nothing, when an operator
    /// suspended the policy while the attempt ran. The worker releases the run
    /// uncharged, and it commits after the resume.
    ///
    /// An approval of a run that needs a privacy pass (`privacy_pass_required`,
    /// every run received from V117 on) and has none recorded is refused with
    /// `PIPELINE_PRIVACY_PASS_MISSING_LABEL`, writing nothing (critique 0);
    /// the CHECK `pipeline_runs_privacy_pass_before_approval` backs it. The
    /// derived record names the privacy pass object as its input when a pass
    /// is recorded, since that is the object whose hash it stores.
    ///
    /// `pass_approval` is the human assessment that approved a run the
    /// privacy pass escalated (Q2). On an approval of such a run its
    /// `evidence_hash` and resolved reasons are written to the run's
    /// `privacy_pass_approval_*` columns in this transaction, so the approved
    /// outcome is linked to the human decision. It is ignored on a rejection
    /// and on a run whose pass did not escalate (both columns stay NULL).
    pub async fn commit_review(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        approved: Option<ApprovedRevision>,
        pass_approval: Option<&HumanReviewAssessment>,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if outcome.phase != Phase::Review || run.next_phase != Some(Phase::Review) {
            return Err(DatabaseError::Constraint(
                "phase does not match run transition".to_string(),
            ));
        }
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        // Lock order: the run row (`ensure_current_lease`), then the
        // submission row (below) -- held the same way everywhere both are
        // touched -- then the policy row (`lock_runnable_policy`).
        ensure_current_lease(&tx, run, lease_token).await?;

        // Fix round 1, finding I2 / Ruling T3-8: this re-check used to
        // inline its own copy of the operability predicate; it now calls
        // the same `review_submission_is_operable` helper `claim_review`
        // and `record_review_assessment` use, so the predicate exists once.
        // Behaviour is unchanged (`!operable || withdrawn`): the helper
        // returns `Ok(false)` for exactly the same two conditions, just
        // short-circuiting the `trace_withdrawals` lookup when the
        // `trace_submissions` row alone already refuses.
        if !review_submission_is_operable(&tx, &run.tenant_id, run.submission_id).await? {
            return Err(DatabaseError::Constraint(
                PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
            ));
        }
        // GRD-004: the Review policy must still be runnable, and stays so
        // until this commit ends (`lock_runnable_policy`, the last lock before
        // the first write). A policy suspended while this attempt ran refuses
        // the commit as the checks above do: no object ref, no derived record,
        // no status change, no outcome, no run update.
        if !lock_runnable_policy(&tx, &run.tenant_id, &run.bundle_id, Phase::Review).await? {
            return Err(DatabaseError::Constraint(
                PIPELINE_POLICY_NOT_RUNNABLE_LABEL.to_string(),
            ));
        }

        // The derived record names the object Review read, whose hash it
        // stores in `input_hash`: the privacy pass's output once a pass is
        // recorded, else (a run received before V117) the source.
        let derived_input_object_ref_id = run
            .privacy_pass_object_ref_id
            .unwrap_or(run.source_object_ref_id);
        let (approved_revision_id, approved_object_ref_id, approved_content_hash) =
            if let Some(approved) = &approved {
                tx.execute(
                    "INSERT INTO trace_object_refs (
                        tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
                        object_key, content_sha256, encryption_key_ref, size_bytes, compression,
                        created_by_job_id
                     ) VALUES ($1,$2,$3,'review_snapshot',$4,$5,$6,$7,$8,$9,$10)
                     ON CONFLICT (tenant_id, submission_id, object_ref_id) DO NOTHING",
                    &[
                        &approved.object_ref.tenant_id,
                        &approved.object_ref.submission_id,
                        &approved.object_ref.object_ref_id,
                        &approved.object_ref.object_store,
                        &approved.object_ref.object_key,
                        &approved.object_ref.content_sha256,
                        &approved.object_ref.encryption_key_ref,
                        &approved.object_ref.size_bytes,
                        &approved.object_ref.compression,
                        &approved.object_ref.created_by_job_id,
                    ],
                )
                .await?;
                tx.execute(
                    "INSERT INTO trace_derived_records (
                        tenant_id, derived_id, submission_id, trace_id, status,
                        worker_kind, worker_version, input_object_ref_id, input_hash,
                        output_object_ref_id, summary_model,
                        canonical_summary, canonical_summary_hash
                     ) VALUES ($1,$2,$3,$4,'current','summary',$5,$6,$7,$8,$5,$9,$10)
                     ON CONFLICT (tenant_id, derived_id) DO NOTHING",
                    &[
                        &run.tenant_id,
                        &approved.revision_id,
                        &run.submission_id,
                        &run.trace_id,
                        &approved.worker_identity,
                        &derived_input_object_ref_id,
                        &approved.source_content_hash,
                        &approved.object_ref.object_ref_id,
                        &approved.canonical_summary,
                        &approved.canonical_summary_hash,
                    ],
                )
                .await?;
                // Defense in depth: the status update stays
                // conditional on the same pre-review states the gate above
                // just re-checked, and must touch exactly one row. The
                // whole transaction has not committed yet, so refusing here
                // still rolls back the two inserts just above.
                let updated_submission = tx
                    .execute(
                        "UPDATE trace_submissions
                         SET status = 'accepted', reviewed_at = NOW(), updated_at = NOW()
                         WHERE tenant_id = $1 AND submission_id = $2
                           AND status IN ('received', 'quarantined')",
                        &[&run.tenant_id, &run.submission_id],
                    )
                    .await?;
                if updated_submission != 1 {
                    return Err(DatabaseError::Constraint(
                        PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
                    ));
                }
                (
                    Some(approved.revision_id),
                    Some(approved.object_ref.object_ref_id),
                    Some(approved.content_hash.clone()),
                )
            } else {
                let updated_submission = tx
                    .execute(
                        "UPDATE trace_submissions
                         SET status = 'rejected', reviewed_at = NOW(), updated_at = NOW()
                         WHERE tenant_id = $1 AND submission_id = $2
                           AND status IN ('received', 'quarantined')",
                        &[&run.tenant_id, &run.submission_id],
                    )
                    .await?;
                if updated_submission != 1 {
                    return Err(DatabaseError::Constraint(
                        PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
                    ));
                }
                (None, None, None)
            };

        insert_outcome(
            &tx,
            &run.tenant_id,
            run.run_id,
            run.trace_id,
            &run.bundle_id,
            Uuid::new_v4(),
            outcome,
        )
        .await?;
        let next_phase = approved_revision_id.map(|_| Phase::Score);
        let terminal = next_phase.is_none();
        let state = if terminal {
            PipelineRunState::Complete
        } else {
            PipelineRunState::Pending
        };
        // Q2: the approving assessment of an escalated run, linked on the
        // pass record. Only on an approval; the UPDATE writes it only when
        // the recorded pass escalated (`privacy_pass_outcome = 'escalated'`,
        // which `pipeline_runs_privacy_pass_approval_shape` also requires).
        let (pass_approval_hash, pass_approval_reasons) =
            match pass_approval.filter(|_| approved.is_some()) {
                Some(assessment) => {
                    let reasons: Vec<&str> = assessment
                        .resolved_quarantine_reasons
                        .iter()
                        .map(ReasonCode::as_str)
                        .collect();
                    let reasons = serde_json::to_value(reasons).map_err(|_| {
                        DatabaseError::Serialization(
                            "privacy pass approval reasons encode failed".to_string(),
                        )
                    })?;
                    (Some(assessment.evidence_hash.clone()), Some(reasons))
                }
                None => (None, None),
            };
        // Critique 0: an approval of a run that needs a privacy pass
        // requires one recorded. `ensure_current_lease` already passed under
        // the run lock, so no row here means the pass is missing; the
        // refusal carries a safe label, never the raw CHECK violation
        // (`pipeline_runs_privacy_pass_before_approval`), which stays as the
        // backstop for a binary without this predicate. Returning drops the
        // transaction, so the inserts above roll back.
        let Some(row) = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET next_phase = $3, state = $4,
                     attempt_count = CASE WHEN $3 = 'none' THEN attempt_count ELSE 0 END,
                     approved_revision_id = $5,
                     approved_object_ref_id = $6,
                     approved_content_hash = $7,
                     review_audit_pending_at = clock_timestamp(),
                     privacy_pass_approval_assessment_hash = CASE
                         WHEN privacy_pass_outcome = 'escalated' THEN $9::text
                         ELSE privacy_pass_approval_assessment_hash END,
                     privacy_pass_approval_resolved_reasons = CASE
                         WHEN privacy_pass_outcome = 'escalated' THEN $10::jsonb
                         ELSE privacy_pass_approval_resolved_reasons END,
                     lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = NOW(), phase_started_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $8 AND lease_expires_at > NOW()
                   AND ($6::uuid IS NULL OR NOT privacy_pass_required
                        OR privacy_pass_object_ref_id IS NOT NULL)
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &phase_as_db(next_phase),
                    &state.as_db(),
                    &approved_revision_id,
                    &approved_object_ref_id,
                    &approved_content_hash,
                    &lease_token,
                    &pass_approval_hash,
                    &pass_approval_reasons,
                ],
            )
            .await?
        else {
            return Err(DatabaseError::Constraint(
                PIPELINE_PRIVACY_PASS_MISSING_LABEL.to_string(),
            ));
        };
        let updated = pipeline_run_from_row(&row)?;
        // PR 4: the attempt's staged object -- the approved content, when
        // this commit approved one -- moves to `committed` in this same
        // transaction, the one that records it as an object ref of the
        // submission; from here on the withdrawal, not the attempt sweep,
        // owns its deletion. A refused commit above already returned without
        // reaching here, so this never runs for one.
        //
        // An approval must move exactly the one `approved` row that names
        // the object it records (its key, and its hash); a rejection staged
        // no object and must move none (wave 2, fix round 1; review I3).
        // Anything else is `pipeline_attempt_artifact_missing`, which rolls
        // this transaction back; the caller deletes the approved object
        // (`phase_commit_refused`), as for any refused commit.
        let moved = match &approved {
            Some(approved) => {
                let ciphertext_sha256 = approved
                    .object_ref
                    .content_sha256
                    .strip_prefix("sha256:")
                    .unwrap_or(&approved.object_ref.content_sha256);
                tx.execute(
                    "UPDATE pipeline_attempt_artifacts
                        SET state = 'committed', committed_at = NOW()
                      WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3
                        AND artifact = 'approved' AND state = 'staged'
                        AND object_key = $4
                        AND (ciphertext_sha256 IS NULL OR ciphertext_sha256 = $5)",
                    &[
                        &run.tenant_id,
                        &run.run_id,
                        &lease_token,
                        &approved.object_ref.object_key,
                        &ciphertext_sha256,
                    ],
                )
                .await?
            }
            None => {
                tx.execute(
                    "UPDATE pipeline_attempt_artifacts
                        SET state = 'committed', committed_at = NOW()
                      WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3
                        AND state = 'staged'",
                    &[&run.tenant_id, &run.run_id, &lease_token],
                )
                .await?
            }
        };
        if moved != u64::from(approved.is_some()) {
            return Err(DatabaseError::Constraint(
                PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL.to_string(),
            ));
        }
        tx.commit().await?;
        Ok(updated)
    }

    /// Records the Review-start privacy pass on `run`, in one tenant
    /// transaction under the run's lease, modelled on `commit_review`: the
    /// pass object's ref, the post-classifier privacy values on the
    /// submission row (and, for an escalated pass, its status moved from
    /// `received` to `quarantined`, #1326), the six `privacy_pass_*` columns
    /// on the run, and the attempt's staged `privacy-pass` row moved to
    /// `committed`.
    ///
    /// Lock order as `commit_review`: the run row (`ensure_current_lease`),
    /// then the submission row (`review_submission_is_operable`). No policy
    /// lock: the pass is a server control, not a bundle policy. The run's
    /// phase, state, lease and `attempt_count` are left as they are; the
    /// Review policy runs next under the same lease.
    ///
    /// `trace_submissions.redaction_hash` is never written: a withdrawal's
    /// tombstone is matched on the deterministic envelope's hash, and V68
    /// revokes every token bundle of a submission whose hash changes
    /// (decision P5).
    ///
    /// Refusals write nothing: a stale lease; an inoperable submission
    /// (`PIPELINE_SUBMISSION_INOPERABLE_LABEL`); a pass already recorded
    /// (`PIPELINE_PRIVACY_PASS_ALREADY_RECORDED_LABEL`); and a staged row
    /// that is missing or names another object key or hash
    /// (`PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL`). Without the last, a
    /// `privacy-pass` row left `staged` under the lease would make a
    /// same-dispatch rejection fail, because `commit_review`'s rejection
    /// branch requires no staged row of the lease to remain.
    pub async fn record_privacy_pass(
        &self,
        run: &PipelineRunRecord,
        pass: PrivacyPassRecord<'_>,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if run.next_phase != Some(Phase::Review) {
            return Err(DatabaseError::Constraint(
                "phase does not match run transition".to_string(),
            ));
        }
        let lease_token = required_lease_token(run)?;
        let ciphertext_sha256 = pass
            .ciphertext_sha256
            .strip_prefix("sha256:")
            .unwrap_or(pass.ciphertext_sha256);
        if pass.object_ref.content_sha256 != format!("sha256:{ciphertext_sha256}")
            || pass.object_ref.tenant_id != run.tenant_id
            || pass.object_ref.submission_id != run.submission_id
        {
            return Err(DatabaseError::Constraint(
                PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL.to_string(),
            ));
        }
        let privacy_risk = enum_string(&pass.residual_pii_risk).map_err(|_| {
            DatabaseError::Serialization("privacy pass residual risk encode failed".to_string())
        })?;
        let residual_risk_basis = serde_json::to_value(pass.basis_labels).map_err(|_| {
            DatabaseError::Serialization(
                "privacy pass residual risk basis encode failed".to_string(),
            )
        })?;
        let redaction_counts = serde_json::to_value(pass.redaction_counts).map_err(|_| {
            DatabaseError::Serialization("privacy pass redaction counts encode failed".to_string())
        })?;

        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
        if !review_submission_is_operable(&tx, &run.tenant_id, run.submission_id).await? {
            return Err(DatabaseError::Constraint(
                PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
            ));
        }

        tx.execute(
            "INSERT INTO trace_object_refs (
                tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
                object_key, content_sha256, encryption_key_ref, size_bytes, compression,
                created_by_job_id
             ) VALUES ($1,$2,$3,'review_snapshot',$4,$5,$6,$7,$8,$9,$10)
             ON CONFLICT (tenant_id, submission_id, object_ref_id) DO NOTHING",
            &[
                &pass.object_ref.tenant_id,
                &pass.object_ref.submission_id,
                &pass.object_ref.object_ref_id,
                &pass.object_ref.object_store,
                &pass.object_ref.object_key,
                &pass.object_ref.content_sha256,
                &pass.object_ref.encryption_key_ref,
                &pass.object_ref.size_bytes,
                &pass.object_ref.compression,
                &pass.object_ref.created_by_job_id,
            ],
        )
        .await?;
        // #1326: an escalated pass holds the run for a human, so an
        // Admission-admitted run's `received` row moves to `quarantined`, as
        // `main`'s PII backstop quarantines the equal state. `main`'s record
        // readers (the assessment route's consent check among them) read no
        // `received` row, and the contributor and `main`'s review counts see
        // the hold. An Admission-quarantined row is already `quarantined`; a
        // cleared pass leaves the status as it is.
        let updated_submission = tx
            .execute(
                "UPDATE trace_submissions
                 SET privacy_risk = $3, residual_risk_basis = $4, redaction_counts = $5,
                     redaction_pipeline_version = $6,
                     status = CASE WHEN $7::text = 'escalated' AND status = 'received'
                                   THEN 'quarantined' ELSE status END,
                     updated_at = NOW()
                 WHERE tenant_id = $1 AND submission_id = $2",
                &[
                    &run.tenant_id,
                    &run.submission_id,
                    &privacy_risk,
                    &residual_risk_basis,
                    &redaction_counts,
                    &pass.redaction_pipeline_version,
                    &pass.outcome.as_db(),
                ],
            )
            .await?;
        if updated_submission != 1 {
            return Err(DatabaseError::Constraint(
                PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
            ));
        }
        // V118: an escalated pass sets the review audit marker, so the
        // worker's review audit pass appends the hold's audit event.
        let Some(row) = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET privacy_pass_object_ref_id = $4,
                     privacy_pass_content_hash = $5,
                     privacy_pass_source_hash = $6,
                     privacy_pass_residual_risk_basis = $7,
                     privacy_pass_outcome = $8,
                     privacy_pass_recorded_at = NOW(),
                     review_audit_pending_at = CASE WHEN $8 = 'escalated'
                         THEN clock_timestamp() ELSE review_audit_pending_at END,
                     updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND next_phase = 'review'
                   AND lease_token = $3 AND lease_expires_at > NOW()
                   AND privacy_pass_object_ref_id IS NULL
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &lease_token,
                    &pass.object_ref.object_ref_id,
                    &pass.content_hash,
                    &pass.source_hash,
                    &residual_risk_basis,
                    &pass.outcome.as_db(),
                ],
            )
            .await?
        else {
            // `ensure_current_lease` passed under the run lock, so the
            // lease holds: a missing row is the fence.
            return Err(DatabaseError::Constraint(
                PIPELINE_PRIVACY_PASS_ALREADY_RECORDED_LABEL.to_string(),
            ));
        };
        let updated = pipeline_run_from_row(&row)?;
        let moved = tx
            .execute(
                "UPDATE pipeline_attempt_artifacts
                    SET state = 'committed', committed_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3
                    AND artifact = 'privacy-pass' AND state = 'staged'
                    AND object_key = $4
                    AND (ciphertext_sha256 IS NULL OR ciphertext_sha256 = $5)",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &lease_token,
                    &pass.object_ref.object_key,
                    &ciphertext_sha256,
                ],
            )
            .await?;
        if moved != 1 {
            return Err(DatabaseError::Constraint(
                PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL.to_string(),
            ));
        }
        tx.commit().await?;
        Ok(updated)
    }

    /// The receipt-time privacy values of `submission_id`, raw as the
    /// receipt stored them (`trace_submissions.privacy_risk` and
    /// `residual_risk_basis`), read in a tenant transaction. The privacy pass
    /// reads them before it overwrites them (`record_privacy_pass`) and maps
    /// them through `pipeline_privacy_risk` itself (decision P3).
    pub async fn load_submission_receipt_privacy(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<SubmissionReceiptPrivacy, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT privacy_risk, residual_risk_basis FROM trace_submissions
                 WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "trace_submission".to_string(),
                id: submission_id.to_string(),
            })?;
        tx.commit().await?;
        let privacy_risk: String = row.get("privacy_risk");
        let residual_pii_risk: ResidualPiiRisk =
            serde_json::from_value(serde_json::Value::String(privacy_risk)).map_err(|_| {
                DatabaseError::Serialization("submission privacy risk is unknown".to_string())
            })?;
        let residual_risk_basis = row
            .get::<_, Option<serde_json::Value>>("residual_risk_basis")
            .map(serde_json::from_value::<Vec<String>>)
            .transpose()
            .map_err(|_| {
                DatabaseError::Serialization(
                    "submission residual risk basis is not an array of labels".to_string(),
                )
            })?
            .unwrap_or_default();
        Ok(SubmissionReceiptPrivacy {
            residual_pii_risk,
            residual_risk_basis,
        })
    }

    /// A reviewer's exclusive claim on a run quarantined at Review (port
    /// `ef97a459` lines 832 to 892), adapted for Ruling T3-6: before
    /// attempting the claim, this locks the run row and refuses -- releasing
    /// an `awaiting_review` run back to `pending` in the same transaction --
    /// when the run's submission is no longer operable, the same predicate
    /// `commit_review` re-checks (`review_submission_is_operable`). The port
    /// folded its own narrower `trace_withdrawals` check into the claiming
    /// `INSERT ... SELECT`; that check is now this broader one, done first.
    ///
    /// Fix round 1, finding I1: the locking `SELECT` also requires `state IN
    /// ('pending', 'retry', 'awaiting_review')`, `admission_decision =
    /// 'quarantine'`, and no assessment already recorded. Without the state
    /// predicate, a reviewer could claim a run a worker currently holds
    /// `leased` (and then, worse, a run that has already failed -- `failed`
    /// never clears `next_phase`); without the decision predicate, a
    /// reviewer could claim an `Admit` run that is simply waiting its own
    /// automatic Review turn (an `Admit` run the privacy pass escalated is
    /// the exception: the server holds it for a human, so it is claimable,
    /// and a run that needs the pass is claimable only once its pass is
    /// recorded); without the assessment predicate, a reviewer
    /// could re-claim a run that already has one, and a later
    /// `record_review_assessment` would then hit
    /// `pipeline_review_assessments`'s `UNIQUE (tenant_id, run_id, phase)` (V130) instead
    /// of refusing cleanly. `FOR UPDATE` on this row also now does the
    /// concurrency work: a worker's `claim_run` (its own `UPDATE ...
    /// RETURNING *`) and this `SELECT` take the same row lock, so whichever
    /// transaction commits first is the one the other sees -- a worker that
    /// already claimed the run makes this `SELECT` see `state = 'leased'`
    /// and refuse; a reviewer's claim or assessment that commits first
    /// makes the worker's `claim_run` (which re-reads state in its own
    /// `WHERE`) see the post-assessment row once it acquires the lock.
    ///
    /// The outcome tells the cases apart under the run's lock: `Ineligible`
    /// when the run is not waiting for review (it does not exist, is not at
    /// Review, is not `pending`/`retry`/`awaiting_review`, is neither a
    /// quarantine nor escalated by the privacy pass, needs a privacy pass it
    /// does not have yet, or already has an assessment), and
    /// `HeldByAnotherReviewer` when another reviewer holds a live claim on
    /// an otherwise eligible run (Ruling T3-9(b): ingest answers the first
    /// `404` and only the second `409`).
    ///
    /// A run its Score policy held for operator review
    /// (`run_held_at_score_sql!`) is claimable the same way, under the same
    /// claim table and lease. Its assessment is its own (`phase` `score`,
    /// V130), so an assessment the run got at Review does not make it
    /// ineligible, and its submission must still be the accepted one Score
    /// needs (`hold_submission_is_operable`).
    pub async fn claim_review(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        reviewer_principal_ref: &str,
        lease_duration: Duration,
    ) -> Result<PipelineReviewClaimOutcome, DatabaseError> {
        if !reviewer_principal_ref.starts_with("reviewer_sha256:")
            || lease_duration <= Duration::zero()
            || lease_duration > Duration::minutes(30)
        {
            return Err(DatabaseError::Constraint(
                "invalid review claim".to_string(),
            ));
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let run_row = tx
            .query_opt(
                concat!(
                    "SELECT state, submission_id, next_phase FROM pipeline_runs p
                      WHERE p.tenant_id = $1 AND p.run_id = $2 AND ((",
                    run_waiting_for_review_sql!(),
                    ") OR (",
                    run_held_at_score_sql!(),
                    ")) FOR UPDATE"
                ),
                &[&tenant_id, &run_id],
            )
            .await?;
        let Some(run_row) = run_row else {
            tx.commit().await?;
            return Ok(PipelineReviewClaimOutcome::Ineligible);
        };
        // Zaki review 1, round 2, finding 8: the assessment check is a
        // statement of its own, run after the run's lock is held, so it reads
        // an assessment committed while this claim waited for that lock. A
        // check inside the locking statement would read the snapshot taken
        // before the wait.
        let at_score = run_row.get::<_, Option<String>>("next_phase").as_deref() == Some("score");
        let assessed: bool = tx
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pipeline_review_assessments
                                 WHERE tenant_id = $1 AND run_id = $2 AND phase = $3)",
                &[&tenant_id, &run_id, &assessment_phase(at_score)],
            )
            .await?
            .get(0);
        if assessed {
            tx.commit().await?;
            return Ok(PipelineReviewClaimOutcome::Ineligible);
        }
        let run_state: String = run_row.get("state");
        let submission_id: Uuid = run_row.get("submission_id");
        if !hold_submission_is_operable(&tx, tenant_id, submission_id, at_score).await? {
            if run_state == "awaiting_review" {
                release_awaiting_review(&tx, tenant_id, run_id).await?;
            }
            tx.commit().await?;
            return Err(DatabaseError::Constraint(
                "review claim is stale or inoperable".to_string(),
            ));
        }
        let lease_token = Uuid::new_v4();
        let lease_milliseconds = lease_duration.num_milliseconds();
        let row = tx
            .query_opt(
                "INSERT INTO pipeline_review_claims (
                    tenant_id, run_id, reviewer_principal_ref, lease_token, lease_expires_at
                 ) VALUES ($1, $2, $3, $4, NOW() + ($5::bigint * INTERVAL '1 millisecond'))
                 ON CONFLICT (tenant_id, run_id) DO UPDATE
                 SET reviewer_principal_ref = EXCLUDED.reviewer_principal_ref,
                     lease_token = EXCLUDED.lease_token,
                     lease_expires_at = EXCLUDED.lease_expires_at,
                     claimed_at = NOW()
                 WHERE pipeline_review_claims.lease_expires_at <= NOW()
                    OR pipeline_review_claims.reviewer_principal_ref = $3
                 RETURNING *",
                &[
                    &tenant_id,
                    &run_id,
                    &reviewer_principal_ref,
                    &lease_token,
                    &lease_milliseconds,
                ],
            )
            .await?;
        tx.commit().await?;
        // The run is eligible and locked, so a claim row the upsert did not
        // take over is another reviewer's live claim.
        Ok(match row {
            Some(row) => PipelineReviewClaimOutcome::Claimed(PipelineReviewClaim {
                tenant_id: row.get("tenant_id"),
                run_id: row.get("run_id"),
                reviewer_principal_ref: row.get("reviewer_principal_ref"),
                lease_token: row.get("lease_token"),
                lease_expires_at: row.get("lease_expires_at"),
            }),
            None => PipelineReviewClaimOutcome::HeldByAnotherReviewer,
        })
    }

    /// Records a reviewer's Approve/Reject assessment of a quarantined run
    /// (port `ef97a459` lines 893 to 1006): the assessment row and the
    /// spent claim's deletion commit together, in the same transaction as
    /// (Ruling T3-3) releasing an `awaiting_review` run back to `pending`,
    /// due at once with its parking label cleared, so the next claim
    /// reaches it without waiting out any backoff. A `pending` or `retry`
    /// run needs no such release; the update is a harmless no-op for it.
    ///
    /// Ruling T3-6 adds the same submission-operability re-check
    /// `claim_review` takes, under the same row locks this already holds
    /// (the run's, then the claim's): an inoperable submission refuses with the
    /// port's stale-claim label and releases an `awaiting_review` run back
    /// to `pending` in this same transaction, so the run still ends under
    /// `submission_inoperable` rather than staying parked with a claim that
    /// can never be assessed.
    ///
    /// Fix round 1, finding I1: the initial lookup also requires `p.state IN
    /// ('pending', 'retry', 'awaiting_review')` and `p.admission_decision =
    /// 'quarantine'` (not the assessment-exists check `claim_review` adds --
    /// a claim already excludes an assessed run, so this is defense against
    /// a stale claim object, not a first line of defense). Without the
    /// state predicate, a worker that leased the run between the claim and
    /// the assessment (or that failed it, which leaves `next_phase =
    /// 'review'`) would never block this call, and the assessment could
    /// land after -- or racing -- the worker's own Review attempt. The run's
    /// `FOR UPDATE` makes this the same lock-and-recheck pattern
    /// `claim_review` uses: a worker's `claim_run` and this lookup take the
    /// same row lock, so the loser re-reads the winner's committed state.
    ///
    /// An `Approve` recommendation refuses
    /// (`quarantine reason is unresolved`) unless `resolved_quarantine_reasons`
    /// names every reason that holds the run: its own `admission_reason`, if
    /// it has one, and `privacy_pass_review_required`, if the privacy pass
    /// escalated it. A run held for no logged reason at all needs at least
    /// one resolved reason, as in the port.
    ///
    /// A run its Score policy held for operator review is assessed the same
    /// way, under the same claim, and its assessment is recorded with
    /// `phase` `score` and the policy's hold label (`hold_label`, V130),
    /// read from the run before the release clears it. An approval must
    /// resolve that label. The release sends the run back to Score, which
    /// reads the assessment (`load_score_hold_assessment`): an approval
    /// reaches the Score policy on its input (`ScoreInput::operator_review`),
    /// and a rejection makes the server commit a zero Score outcome itself
    /// (`ScoreOutput::rejected_after_hold`), so the run settles with no
    /// credit and no index entry.
    pub async fn record_review_assessment(
        &self,
        claim: &PipelineReviewClaim,
        recommendation: ReviewRecommendation,
        reason: ReasonCode,
        resolved_quarantine_reasons: Vec<ReasonCode>,
    ) -> Result<HumanReviewAssessment, DatabaseError> {
        self.record_review_assessment_for_hold(
            claim,
            recommendation,
            reason,
            resolved_quarantine_reasons,
        )
        .await
        .map(|(assessment, _)| assessment)
    }

    /// `record_review_assessment`, also answering whether the assessment
    /// resolved a Score hold (`true`) or a Review hold (`false`), as read
    /// under the run's lock.
    pub async fn record_review_assessment_for_hold(
        &self,
        claim: &PipelineReviewClaim,
        recommendation: ReviewRecommendation,
        reason: ReasonCode,
        resolved_quarantine_reasons: Vec<ReasonCode>,
    ) -> Result<(HumanReviewAssessment, bool), DatabaseError> {
        let mut resolved = resolved_quarantine_reasons;
        resolved.sort();
        resolved.dedup();
        let recommendation_label = match recommendation {
            ReviewRecommendation::Approve => "approve",
            ReviewRecommendation::Reject => "reject",
        };
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &claim.tenant_id).await?;
        // Zaki review 1, round 2, finding 7: the run row first, then the
        // claim row, in two statements -- the order `claim_review` takes
        // them in (the run's `FOR UPDATE`, then the claim's upsert) -- so
        // a claim and an assessment of one run never wait on each other in
        // opposite orders.
        let row = tx
            .query_opt(
                concat!(
                    "SELECT p.admission_reason, p.privacy_pass_outcome, p.state,
                            p.submission_id, p.next_phase, p.last_error_label
                     FROM pipeline_runs p
                     WHERE p.tenant_id = $1 AND p.run_id = $2 AND ((",
                    run_waiting_for_review_sql!(),
                    ") OR (",
                    run_held_at_score_sql!(),
                    ")) FOR UPDATE"
                ),
                &[&claim.tenant_id, &claim.run_id],
            )
            .await?;
        let claim_held = match row {
            Some(_) => tx
                .query_opt(
                    "SELECT 1 FROM pipeline_review_claims c
                     WHERE c.tenant_id = $1 AND c.run_id = $2
                       AND c.lease_token = $3
                       AND c.reviewer_principal_ref = $4
                       AND c.lease_expires_at > NOW()
                     FOR UPDATE",
                    &[
                        &claim.tenant_id,
                        &claim.run_id,
                        &claim.lease_token,
                        &claim.reviewer_principal_ref,
                    ],
                )
                .await?
                .is_some(),
            None => false,
        };
        let Some(row) = row.filter(|_| claim_held) else {
            return Err(DatabaseError::Constraint(
                "review claim is stale or inoperable".to_string(),
            ));
        };
        let admission_reason: Option<String> = row.get("admission_reason");
        let pass_escalated = row
            .get::<_, Option<String>>("privacy_pass_outcome")
            .as_deref()
            == Some(PrivacyPassOutcome::Escalated.as_db());
        let run_state: String = row.get("state");
        let submission_id: Uuid = row.get("submission_id");
        let at_score = row.get::<_, Option<String>>("next_phase").as_deref() == Some("score");
        // The label a Score policy held the run under; a Score hold always
        // parks under one (`hold_score_for_review_on_tx`).
        let hold_label: Option<String> = if at_score {
            Some(
                row.get::<_, Option<String>>("last_error_label")
                    .filter(|label| ReasonCode::new(label.clone()).is_ok())
                    .ok_or_else(|| {
                        DatabaseError::Constraint("review claim is stale or inoperable".to_string())
                    })?,
            )
        } else {
            None
        };
        if !hold_submission_is_operable(&tx, &claim.tenant_id, submission_id, at_score).await? {
            if run_state == "awaiting_review" {
                release_awaiting_review(&tx, &claim.tenant_id, claim.run_id).await?;
            }
            tx.commit().await?;
            return Err(DatabaseError::Constraint(
                "review claim is stale or inoperable".to_string(),
            ));
        }
        if recommendation == ReviewRecommendation::Approve {
            // Every reason that holds the run must be resolved: at Score,
            // the policy's hold label; at Review, Admission's, if it gave
            // one, and the privacy pass's, if it escalated.
            let required: Vec<&str> = if at_score {
                hold_label.as_deref().into_iter().collect()
            } else {
                admission_reason
                    .as_deref()
                    .into_iter()
                    .chain(pass_escalated.then_some(PIPELINE_PRIVACY_PASS_REVIEW_REQUIRED_LABEL))
                    .collect()
            };
            let unresolved = if required.is_empty() {
                resolved.is_empty()
            } else {
                !required
                    .iter()
                    .all(|reason| resolved.iter().any(|item| item.as_str() == *reason))
            };
            if unresolved {
                return Err(DatabaseError::Constraint(
                    "quarantine reason is unresolved".to_string(),
                ));
            }
        }
        let resolved_json = serde_json::to_value(&resolved).map_err(|_| {
            DatabaseError::Serialization("review assessment encode failed".to_string())
        })?;
        // A Review assessment hashes exactly what it hashed before V130; a
        // Score assessment also binds its phase and the hold it resolves.
        let mut evidence = serde_json::json!({
            "schema": "trace_commons.pipeline_review_assessment.v1",
            "run_id": claim.run_id,
            "recommendation": recommendation_label,
            "reason": reason.as_str(),
            "resolved_quarantine_reasons": resolved,
        });
        if let Some(hold_label) = &hold_label {
            evidence["phase"] = serde_json::json!(assessment_phase(true));
            evidence["hold_label"] = serde_json::json!(hold_label);
        }
        let evidence_hash = sha256_prefixed(
            serde_json::to_vec(&evidence)
                .map_err(|_| {
                    DatabaseError::Serialization("review assessment encode failed".to_string())
                })?
                .as_slice(),
        );
        let assessment_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!(
                "tracecommons:pipeline-review-assessment:{}:{evidence_hash}",
                claim.run_id
            )
            .as_bytes(),
        );
        tx.execute(
            "INSERT INTO pipeline_review_assessments (
                tenant_id, assessment_id, run_id, reviewer_principal_ref,
                recommendation, reason_code, resolved_quarantine_reasons, evidence_hash,
                phase, hold_label
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[
                &claim.tenant_id,
                &assessment_id,
                &claim.run_id,
                &claim.reviewer_principal_ref,
                &recommendation_label,
                &reason.as_str(),
                &resolved_json,
                &evidence_hash,
                &assessment_phase(at_score),
                &hold_label,
            ],
        )
        .await?;
        // V118: the worker's review audit pass appends this assessment's
        // audit event when the route's own append missed it.
        tx.execute(
            "UPDATE pipeline_runs SET review_audit_pending_at = clock_timestamp()
             WHERE tenant_id = $1 AND run_id = $2",
            &[&claim.tenant_id, &claim.run_id],
        )
        .await?;
        tx.execute(
            "DELETE FROM pipeline_review_claims
             WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3",
            &[&claim.tenant_id, &claim.run_id, &claim.lease_token],
        )
        .await?;
        // Ruling T3-3: unparks the run so the worker's ordinary claim query
        // reaches it right away, whether this assessment approved (Review
        // now resolves) or rejected (Review now ends the run) it.
        release_awaiting_review(&tx, &claim.tenant_id, claim.run_id).await?;
        tx.commit().await?;
        Ok((
            HumanReviewAssessment {
                assessment_id,
                recommendation,
                reason,
                resolved_quarantine_reasons: resolved,
                evidence_hash,
            },
            at_score,
        ))
    }

    /// The stored assessment for `run_id`, if a reviewer has recorded one
    /// (port `ef97a459` lines 1007 to 1052), with the time it was recorded.
    /// Read into `ReviewInput.human_assessment` before every Review attempt;
    /// `recorded_at` lets the Review arm ignore an assessment recorded
    /// before a privacy pass that escalated the run (critique 1 (2)).
    pub async fn load_review_assessment(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Option<StoredReviewAssessment>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT assessment_id, recommendation, reason_code,
                        resolved_quarantine_reasons, evidence_hash, recorded_at
                 FROM pipeline_review_assessments
                 WHERE tenant_id = $1 AND run_id = $2 AND phase = 'review'",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        row.map(|row| {
            let recommendation = match row.get::<_, String>("recommendation").as_str() {
                "approve" => Ok(ReviewRecommendation::Approve),
                "reject" => Ok(ReviewRecommendation::Reject),
                _ => Err(DatabaseError::Serialization(
                    "unknown review recommendation".to_string(),
                )),
            }?;
            let reason = ReasonCode::new(row.get::<_, String>("reason_code")).map_err(|_| {
                DatabaseError::Serialization("invalid review assessment reason".to_string())
            })?;
            let resolved_quarantine_reasons = serde_json::from_value(
                row.get::<_, serde_json::Value>("resolved_quarantine_reasons"),
            )
            .map_err(|_| {
                DatabaseError::Serialization("invalid resolved quarantine reasons".to_string())
            })?;
            Ok(StoredReviewAssessment {
                assessment: HumanReviewAssessment {
                    assessment_id: row.get("assessment_id"),
                    recommendation,
                    reason,
                    resolved_quarantine_reasons,
                    evidence_hash: row.get("evidence_hash"),
                },
                recorded_at: row.get("recorded_at"),
            })
        })
        .transpose()
    }

    /// The operator's resolution of `run_id`'s Score hold, if one is
    /// recorded (`record_review_assessment` on a run held at Score): the
    /// recommendation and the label the policy held the run under. Read
    /// before every Score attempt (`commit_score_phase`). Nothing that names
    /// the reviewer is returned.
    pub async fn load_score_hold_assessment(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Option<ScoreHoldReview>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT recommendation, hold_label
                 FROM pipeline_review_assessments
                 WHERE tenant_id = $1 AND run_id = $2 AND phase = 'score'",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        row.map(|row| {
            let decision = match row.get::<_, String>("recommendation").as_str() {
                "approve" => Ok(ReviewRecommendation::Approve),
                "reject" => Ok(ReviewRecommendation::Reject),
                _ => Err(DatabaseError::Serialization(
                    "unknown review recommendation".to_string(),
                )),
            }?;
            let hold_label = row
                .get::<_, Option<String>>("hold_label")
                .and_then(|label| ReasonCode::new(label).ok())
                .ok_or_else(|| {
                    DatabaseError::Serialization("invalid score hold label".to_string())
                })?;
            Ok(ScoreHoldReview {
                decision,
                hold_label,
            })
        })
        .transpose()
    }

    /// Releases every run of `tenant_id` parked in `awaiting_review` whose
    /// submission is no longer operable -- the rule `review_submission_is_operable`
    /// applies: status `received` or `quarantined`, never revoked, purged or
    /// withdrawn, not expired by the database clock, no `trace_withdrawals`
    /// row -- back to `pending`, due at once (`release_awaiting_review`), so
    /// Review ends it under `submission_inoperable`, as `claim_review` and
    /// `record_review_assessment` would (Zaki review 1, round 2, finding 9).
    /// Locks only the run rows it releases, skipping any another session
    /// holds, at most `limit` of them. Returns how many it released.
    pub async fn release_inoperable_parked_runs(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> Result<u64, DatabaseError> {
        let limit = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        // At most `limit` per call, in run id order (Zaki review 3, Z3-L4), as
        // a materialized CTE: an `IN (SELECT ... LIMIT ...)` subquery may be
        // evaluated more than once.
        let released = tx
            .execute(
                concat!(
                    "WITH parked AS MATERIALIZED (
                        SELECT p.run_id FROM pipeline_runs p
                         WHERE p.tenant_id = $1 AND p.state = 'awaiting_review'
                           AND NOT EXISTS (
                               SELECT 1 FROM trace_submissions s
                                WHERE s.tenant_id = p.tenant_id
                                  AND s.submission_id = p.submission_id
                                  AND ",
                    parked_run_submission_sql!(),
                    "
                           )
                         ORDER BY p.run_id
                         LIMIT $2
                         FOR UPDATE OF p SKIP LOCKED
                    )
                    UPDATE pipeline_runs r
                       SET state = 'pending', next_attempt_at = NOW(), last_error_label = NULL,
                           updated_at = NOW()
                      FROM parked
                     WHERE r.tenant_id = $1 AND r.run_id = parked.run_id
                       AND r.state = 'awaiting_review'"
                ),
                &[&tenant_id, &limit],
            )
            .await?;
        tx.commit().await?;
        Ok(released)
    }

    /// Runs waiting for a human assessment, oldest first, label-only fields
    /// (brief Step 3). Each entry carries the reason that holds the run
    /// (`privacy_pass_review_required` when the privacy pass escalated it,
    /// else Admission's reason) and whether its assessment is superseded: a
    /// run whose only assessment was recorded before the pass escalated it
    /// is listed with `assessment_superseded`, since the Review arm ignores
    /// that assessment and the run cannot be assessed again (decision P9);
    /// every other assessed run is left out. Ruling T3-2 adds `awaiting_review` to the states
    /// selected -- the port's `list_policy_interventions`-adjacent draft
    /// predates that state. Ruling T3-6 excludes a run whose submission is
    /// no longer operable, the same predicate `review_submission_is_operable`
    /// checks (Ruling T3-8 names it as the one rule both this query and that
    /// helper mirror), applied here as a join and an anti-join rather than a
    /// per-row lock (this is a plain read, not a claim, so it must filter in
    /// SQL rather than call the helper row by row).
    ///
    /// A run whose Score policy held it for operator review
    /// (`hold_score_for_review_on_tx`) is listed too, with `run.next_phase`
    /// Score and the policy's label as its hold reason, whether or not an
    /// earlier Review assessment exists for it, and never as superseded.
    /// It is claimed and assessed as a Review hold is (`claim_review`,
    /// `record_review_assessment`), and leaves the queue once its own
    /// assessment (`phase` `score`) is recorded. Only a Review assessment
    /// (`phase` `review`) bears on a run held at Review.
    pub async fn list_review_queue(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> Result<Vec<PipelineReviewQueueEntry>, DatabaseError> {
        let limit = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                concat!(
                    "SELECT p.*,
                            CASE WHEN p.next_phase = 'score' THEN p.last_error_label
                                 WHEN p.privacy_pass_outcome = 'escalated'
                                 THEN 'privacy_pass_review_required'
                                 ELSE p.admission_reason
                            END AS queue_hold_reason,
                            (a.run_id IS NOT NULL AND p.next_phase = 'review')
                                AS queue_assessment_superseded
                       FROM pipeline_runs p
                      JOIN trace_submissions s
                        ON s.tenant_id = p.tenant_id AND s.submission_id = p.submission_id
                      LEFT JOIN pipeline_review_assessments a
                        ON a.tenant_id = p.tenant_id AND a.run_id = p.run_id
                       AND a.phase = 'review'
                      WHERE p.tenant_id = $1 AND ((",
                    run_waiting_for_review_sql!(),
                    " AND (a.run_id IS NULL
                           OR (p.privacy_pass_outcome = 'escalated'
                               AND a.recorded_at < p.privacy_pass_recorded_at)))
                        OR (",
                    run_held_at_score_sql!(),
                    " AND NOT EXISTS (
                               SELECT 1 FROM pipeline_review_assessments sa
                                WHERE sa.tenant_id = p.tenant_id AND sa.run_id = p.run_id
                                  AND sa.phase = 'score')))
                      AND ",
                    parked_run_submission_sql!(),
                    " ORDER BY p.created_at, p.run_id
                      LIMIT $2"
                ),
                &[&tenant_id, &limit],
            )
            .await?;
        tx.commit().await?;
        rows.iter()
            .map(|row| {
                Ok(PipelineReviewQueueEntry {
                    run: pipeline_run_from_row(row)?,
                    hold_reason: row.get("queue_hold_reason"),
                    assessment_superseded: row.get("queue_assessment_superseded"),
                })
            })
            .collect()
    }

    /// Commits the Score outcome together with the exact index command it
    /// proposed (if any) and one pending settlement row per award, in one
    /// transaction (decision D5). `command`/`neighbor` are each `(stored
    /// ref, hash, object ref)`, already stored by the caller before this call
    /// opens its transaction. Each object is also recorded as an object ref
    /// of the submission (`score_object_ref`), as Review records its
    /// approved object, so everything that deletes a submission's payloads
    /// -- a withdrawal's queued follow-up above all -- deletes these too.
    /// Every award's settlement row starts `operation_state =
    /// 'pending'` (the column default) with `result_ref_hash` NULL --
    /// Settle records a result only once a leg completes. An award naming
    /// an instrument that `payout_rails` does not cover fails the whole
    /// commit with the safe label `settlement_adapter_missing`.
    ///
    /// A leg's payout starts `pending` only on the `near` rail of a service
    /// whose NEAR payout is enabled (`payout_enabled`); every other leg
    /// starts `disabled` (Ruling T10-2), so a payout that nothing will ever
    /// make never reads as pending. A leg seeded `disabled` is not paid
    /// later, even if payout is enabled afterwards. The Trace Credit leg of a
    /// bundle whose Trace Credit event is `NoveltyUtility` (the compatibility
    /// family, `trace_credit_event`) is never batched or paid, so it starts
    /// `disabled` on any rail (Ruling T10-9).
    ///
    /// Score read its input (`load_approved_bytes`) in an earlier
    /// transaction that has committed, so a withdrawal can land between
    /// that read and this commit. This re-checks the submission under this
    /// transaction (`submission_guard_on_tx`, after the run row's lock) and
    /// refuses the whole commit with `PIPELINE_SUBMISSION_INOPERABLE_LABEL`:
    /// no outcome, no settlement rows, no run update.
    ///
    /// The Score policy gets the same guard (GRD-004): after the submission
    /// guard, the commit locks the policy's status row `FOR SHARE` and refuses
    /// with `PIPELINE_POLICY_NOT_RUNNABLE_LABEL`, writing nothing, when the
    /// policy is no longer runnable.
    ///
    /// The commit resets `attempt_count` to 0, so Settle has the run's whole
    /// `max_attempts` budget (multi-lens review L2-1; see `commit_review`).
    #[allow(clippy::too_many_arguments)]
    pub async fn commit_score(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        awards: &InstrumentAwards,
        command: Option<(&str, &str, &TraceObjectRefWrite)>,
        neighbor: Option<(&str, &str, &TraceObjectRefWrite)>,
        payout_rails: &BTreeMap<String, String>,
        payout_enabled: bool,
        trace_credit_event: PipelineTraceCreditEvent,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        Self::commit_score_in(
            tx,
            run,
            outcome,
            awards,
            command,
            neighbor,
            payout_rails,
            payout_enabled,
            trace_credit_event,
            &[],
        )
        .await
    }

    /// `commit_score`, on `tx`, a tenant transaction it commits: a
    /// compatibility Score commits on the transaction that holds its
    /// tenant's Score lock (`PipelineService::commit_score_phase`), so the
    /// lock is released only with the commit.
    ///
    /// `score_bases` are the `(run id, revision id)` of the unapplied index
    /// commands the Score counted (`unapplied_index_commands_on_tx`), which
    /// the caller passes only for a Score that passed quality and failed
    /// novelty; they are recorded in `pipeline_score_bases` (#1313 case (a),
    /// V128) on this transaction, after the outcome.
    #[allow(clippy::too_many_arguments)]
    pub async fn commit_score_in(
        tx: Transaction<'_>,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        awards: &InstrumentAwards,
        command: Option<(&str, &str, &TraceObjectRefWrite)>,
        neighbor: Option<(&str, &str, &TraceObjectRefWrite)>,
        payout_rails: &BTreeMap<String, String>,
        payout_enabled: bool,
        trace_credit_event: PipelineTraceCreditEvent,
        score_bases: &[(Uuid, Uuid)],
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if outcome.phase != Phase::Score || run.next_phase != Some(Phase::Score) {
            return Err(DatabaseError::Constraint(
                "phase does not match run transition".to_string(),
            ));
        }
        let lease_token = required_lease_token(run)?;
        ensure_current_lease(&tx, run, lease_token).await?;
        if !Self::submission_guard_on_tx(&tx, run).await?.operable {
            // Dropping the transaction rolls it back: nothing was written.
            return Err(DatabaseError::Constraint(
                PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
            ));
        }
        // GRD-004: the Score policy must still be runnable, and stays so
        // until this commit ends. The policy row comes after the run row, the
        // submission row, and (for a compatibility Score) the tenant's Score
        // lock, and before the first write; a refusal writes nothing.
        if !lock_runnable_policy(&tx, &run.tenant_id, &run.bundle_id, Phase::Score).await? {
            return Err(DatabaseError::Constraint(
                PIPELINE_POLICY_NOT_RUNNABLE_LABEL.to_string(),
            ));
        }

        insert_outcome(
            &tx,
            &run.tenant_id,
            run.run_id,
            run.trace_id,
            &run.bundle_id,
            Uuid::new_v4(),
            outcome,
        )
        .await?;

        for (basis_run_id, basis_revision_id) in score_bases {
            tx.execute(
                "INSERT INTO pipeline_score_bases (
                     tenant_id, run_id, basis_run_id, basis_revision_id)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT DO NOTHING",
                &[&run.tenant_id, &run.run_id, basis_run_id, basis_revision_id],
            )
            .await?;
        }

        for award in awards.iter() {
            let instrument_id = award.instrument_id().as_str();
            let payout_rail = payout_rails.get(instrument_id).ok_or_else(|| {
                DatabaseError::Constraint(PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL.to_string())
            })?;
            // Zaki review 1, round 2, finding 16: only a Trace Credit leg
            // that settles into a batch is ever paid, so only it is seeded
            // `pending`; any other instrument on the `near` rail, and a
            // `NoveltyUtility` leg, is seeded `disabled`. A seeded leg is
            // marked payout-eligible: Settle writes its batch line under the
            // account's settlement key and hold.
            //
            // This seeds the NEP-141 outbox payout only. A graded-rule leg
            // (`SettledNoveltyUtility`) is batched and paid by the NEAR AI
            // rail, which reads finalized batches, not this state; it keeps
            // the NEP-141 outbox off, as every `NoveltyUtility` leg does.
            let paid = instrument_id == InstrumentId::trace_credit().as_str()
                && trace_credit_event == PipelineTraceCreditEvent::PipelineScore;
            let payout_eligible = payout_rail == "near" && payout_enabled && paid;
            let payout_state = if payout_eligible {
                "pending"
            } else {
                "disabled"
            };
            let atomic_units = award.atomic_units().to_string();
            let operation_ref_hash = pipeline_operation_ref(run.run_id, award);
            tx.execute(
                "INSERT INTO pipeline_run_settlements (
                    tenant_id, run_id, instrument_id, atomic_units,
                    operation_ref_hash, result_ref_hash, payout_rail, payout_state,
                    payout_eligible
                 ) VALUES ($1,$2,$3,$4::TEXT::NUMERIC,$5,$6,$7,$8,$9)",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &instrument_id,
                    &atomic_units,
                    &operation_ref_hash,
                    &Option::<&str>::None,
                    &payout_rail.as_str(),
                    &payout_state,
                    &payout_eligible,
                ],
            )
            .await?;
        }

        for (_, _, object_ref) in command.iter().chain(neighbor.iter()) {
            insert_score_object_ref_on_tx(&tx, object_ref).await?;
        }
        let (command_ref, command_hash) = match command {
            Some((stored_ref, hash, _)) => (Some(stored_ref), Some(hash)),
            None => (None, None),
        };
        let (neighbor_ref, neighbor_hash) = match neighbor {
            Some((stored_ref, hash, _)) => (Some(stored_ref), Some(hash)),
            None => (None, None),
        };
        let row = tx
            .query_opt(
                // Multi-lens review L2-5: `clock_timestamp()`, not `NOW()`.
                // A compatibility Score commits on the transaction it opened
                // before its scorer ran, whose `NOW()` is that start: the
                // stamps and the lease check must read the commit's time.
                // The lease can therefore end after `ensure_current_lease`
                // above; no row is then a stale lease, as for Settle's
                // selection, and the caller deletes the attempt's objects.
                "UPDATE pipeline_runs
                 SET next_phase = 'settle', state = 'pending', attempt_count = 0,
                     index_command_ref = $3, index_command_hash = $4,
                     score_neighbor_ref = $5, score_neighbor_hash = $6,
                     lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = clock_timestamp(),
                     phase_started_at = clock_timestamp(),
                     updated_at = clock_timestamp()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $7 AND lease_expires_at > clock_timestamp()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &command_ref,
                    &command_hash,
                    &neighbor_ref,
                    &neighbor_hash,
                    &lease_token,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        // PR 4: the attempt's staged objects -- `index-command` and/or
        // `score-neighbors`, whichever this commit wrote -- move to
        // `committed` in this same transaction, the one that records each as
        // an object ref of the submission (above); from here on the
        // withdrawal, not the attempt sweep, owns their deletion. A refused
        // commit above already returned without reaching here.
        //
        // Rebase 10, option D: a compatibility Score staged both rows before
        // its tenant lock, with no hash. The commit sets a written row's
        // hash from the object ref it records (`object_key#ciphertext_sha256`;
        // a row staged with its hash keeps it, which V108's guard enforces),
        // and deletes the row of an artifact this Score did not write, since
        // no object exists for it. Only a row with no hash is deleted: a row
        // with a hash was staged just before its object was published.
        //
        // Each written artifact must move exactly one row, the one that names
        // its object: the same key, and either no hash yet or the same hash
        // (wave 2; rebase 10 review, M4; fix round 1, review I2). An object
        // no committed row names would be the attempt sweep's to delete
        // although the run's object ref names it. The lease was found live
        // above, and the sweep removes a `staged` row only once its
        // `cleanup_after`, past any lease this attempt can hold, has passed,
        // so a missing or mismatched row is an out-of-band change or a
        // defect: `pipeline_attempt_artifact_missing`, the label the Review
        // commit and the receipt use, which rolls this transaction back; the
        // caller deletes the attempt's objects (`phase_commit_refused`).
        for (artifact, stored) in [("index-command", command), ("score-neighbors", neighbor)] {
            match stored {
                Some((stored_ref, _, _)) => {
                    let ciphertext_sha256 = stored_ref
                        .rsplit_once('#')
                        .map(|(_, ciphertext_sha256)| ciphertext_sha256)
                        .ok_or_else(|| {
                            DatabaseError::Constraint("score_outcome_invalid".to_string())
                        })?;
                    let object_key = stored_ref
                        .rsplit_once('#')
                        .map(|(object_key, _)| object_key)
                        .ok_or_else(|| {
                            DatabaseError::Constraint("score_outcome_invalid".to_string())
                        })?;
                    let committed = tx
                        .execute(
                            "UPDATE pipeline_attempt_artifacts
                                SET ciphertext_sha256 = COALESCE(ciphertext_sha256, $5),
                                    state = 'committed', committed_at = NOW()
                              WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3
                                AND artifact = $4 AND state = 'staged'
                                AND object_key = $6
                                AND (ciphertext_sha256 IS NULL OR ciphertext_sha256 = $5)",
                            &[
                                &run.tenant_id,
                                &run.run_id,
                                &lease_token,
                                &artifact,
                                &ciphertext_sha256,
                                &object_key,
                            ],
                        )
                        .await?;
                    if committed != 1 {
                        return Err(DatabaseError::Constraint(
                            PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL.to_string(),
                        ));
                    }
                }
                None => {
                    tx.execute(
                        "DELETE FROM pipeline_attempt_artifacts
                          WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3
                            AND artifact = $4 AND state = 'staged'
                            AND ciphertext_sha256 IS NULL",
                        &[&run.tenant_id, &run.run_id, &lease_token, &artifact],
                    )
                    .await?;
                }
            }
        }
        tx.commit().await?;
        Ok(updated)
    }

    /// The index commands a compatibility Score counts as neighbours with
    /// the live index (Zaki review 1, round 2, finding 12): those the
    /// tenant's other runs committed at Score and Settle has not applied,
    /// read on `tx`, which holds the tenant's Score lock. A run's command
    /// counts while its Settle is still to run or to retry (`next_phase =
    /// 'settle'`, in `pending`, `leased` or `retry`), its index write is not
    /// done (`index_write_state` `none` or `pending`: a `complete` write is
    /// in the live index, and a `failed` or `cancelled` one is never
    /// applied), its membership is not `excluded`, no invalidation of its
    /// revision is queued, and its submission is operable (the predicate of
    /// `submission_guard_on_tx`: not withdrawn, revoked, purged or expired).
    /// A run no claim can select (not `leased`, its attempts spent) is left
    /// out: it would never apply its command, so it must not take a later
    /// near-duplicate's award (multi-lens review L2-1's backstop).
    /// Each row is `(run id, stored ref, command hash, revision id)`, by
    /// `run_id`. A run whose command was removed on purpose -- its
    /// submission withdrawn, revoked, purged or expired, its revision's
    /// invalidation queued, or the run failed for good -- is never selected
    /// (Zaki review 3, Z3-L1). `idx_pipeline_runs_work` leads with
    /// `(tenant_id, state)`, so the read covers the tenant's runs in flight,
    /// not its history.
    pub async fn unapplied_index_commands_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
    ) -> Result<Vec<(Uuid, String, String, Uuid)>, DatabaseError> {
        let rows = tx
            .query(
                concat!(
                    "SELECT r.run_id, r.index_command_ref, r.index_command_hash,
                        r.approved_revision_id
                   FROM pipeline_runs r
                   JOIN trace_submissions s
                     ON s.tenant_id = r.tenant_id AND s.submission_id = r.submission_id
                  WHERE r.tenant_id = $1 AND r.run_id <> $2
                    AND r.state IN ('pending', 'leased', 'retry')
                    AND (r.state = 'leased' OR r.attempt_count < r.max_attempts)
                    AND r.next_phase = 'settle'
                    AND r.index_command_ref IS NOT NULL
                    AND r.index_command_hash IS NOT NULL
                    AND r.approved_revision_id IS NOT NULL
                    AND r.index_membership <> 'excluded'
                    AND r.index_write_state IN ('none', 'pending')
                    AND NOT EXISTS (
                        SELECT 1 FROM pipeline_index_invalidations i
                         WHERE i.tenant_id = r.tenant_id AND i.run_id = r.run_id
                    )
                    AND ",
                    accepted_submission_sql!(),
                    " ORDER BY r.run_id"
                ),
                &[&run.tenant_id, &run.run_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| {
                (
                    row.get::<_, Uuid>(0),
                    row.get::<_, String>(1),
                    row.get::<_, String>(2),
                    row.get::<_, Uuid>(3),
                )
            })
            .collect())
    }

    /// The credit events the tenant's Trace Credit legs wrote to `main`'s
    /// ledger that have no `CreditMutate` audit event yet (Zaki review 1,
    /// round 2, N-5): `main` appends that hash-only event after each credit
    /// event it writes, through its mirrored audit log, which only ingest
    /// can append to. Settle records a leg's `credit_event_id` in its credit
    /// transaction, and ingest's worker appends the event and marks the leg
    /// (`mark_credit_audited`). Oldest first, at most `limit`; the work
    /// index holds only the legs still to audit.
    pub async fn list_unaudited_credit_events(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> Result<Vec<PipelineCreditAuditItem>, DatabaseError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT s.run_id, s.instrument_id, l.credit_event_id, l.submission_id,
                        l.event_type, l.points_delta::TEXT AS points_delta, l.reason,
                        l.external_ref, l.actor_principal_ref, l.actor_role
                   FROM pipeline_run_settlements s
                   JOIN trace_credit_ledger l
                     ON l.tenant_id = s.tenant_id AND l.credit_event_id = s.credit_event_id
                  WHERE s.tenant_id = $1
                    AND s.credit_event_id IS NOT NULL AND s.credit_audited_at IS NULL
                  ORDER BY l.occurred_at, s.run_id
                  LIMIT $2",
                &[&tenant_id, &limit],
            )
            .await?;
        tx.commit().await?;
        rows.iter()
            .map(|row| {
                let event_type: String = row.get("event_type");
                Ok(PipelineCreditAuditItem {
                    run_id: row.get("run_id"),
                    instrument_id: row.get("instrument_id"),
                    credit_event_id: row.get("credit_event_id"),
                    submission_id: row.get("submission_id"),
                    event_type: serde_json::from_value(serde_json::Value::String(event_type))
                        .map_err(|_| {
                            DatabaseError::Serialization(
                                "unknown pipeline credit event type".to_string(),
                            )
                        })?,
                    points_delta: row.get("points_delta"),
                    reason: row.get("reason"),
                    external_ref: row.get("external_ref"),
                    actor_principal_ref: row.get("actor_principal_ref"),
                    actor_role: row.get("actor_role"),
                })
            })
            .collect()
    }

    /// Marks `item`'s leg audited once its `CreditMutate` audit event is
    /// appended. Idempotent: a leg already marked keeps its first time.
    pub async fn mark_credit_audited(
        &self,
        tenant_id: &str,
        item: &PipelineCreditAuditItem,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "UPDATE pipeline_run_settlements
                SET credit_audited_at = COALESCE(credit_audited_at, NOW())
              WHERE tenant_id = $1 AND run_id = $2 AND instrument_id = $3
                AND credit_event_id = $4",
            &[
                &tenant_id,
                &item.run_id,
                &item.instrument_id,
                &item.credit_event_id,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The runs of the tenant with a review audit marker, oldest marker
    /// first, at most `limit`: a Review decision or a human assessment
    /// committed, the privacy pass escalated the run, or the run failed at
    /// its privacy classification, and its audit event is not known to be
    /// appended. The work index (V118) holds only the marked runs.
    pub async fn list_pending_review_audits(
        &self,
        tenant_id: &str,
        limit: i64,
    ) -> Result<Vec<PipelineReviewAuditItem>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT p.run_id, p.submission_id, p.review_audit_pending_at,
                        o.outcome_id, o.decision ->> 'kind' AS decision_kind,
                        p.privacy_pass_outcome IS NOT DISTINCT FROM 'escalated'
                            AS privacy_pass_escalated,
                        (p.state = 'failed' AND p.last_error_label IS NOT DISTINCT FROM $3)
                            AS privacy_classification_failed,
                        p.submission_rejected_at IS NOT NULL AS submission_rejected,
                        p.operator_held_at AS settle_held_for_operator_at
                   FROM pipeline_runs p
                   LEFT JOIN phase_outcomes o
                     ON o.tenant_id = p.tenant_id AND o.run_id = p.run_id
                    AND o.phase = 'review'
                  WHERE p.tenant_id = $1 AND p.review_audit_pending_at IS NOT NULL
                  ORDER BY p.review_audit_pending_at, p.run_id
                  LIMIT $2",
                &[
                    &tenant_id,
                    &limit,
                    &PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL,
                ],
            )
            .await?;
        // A run holds at most one assessment per phase (V130), and the one
        // marker covers both: every assessment of a marked run is read.
        let run_ids: Vec<Uuid> = rows.iter().map(|row| row.get("run_id")).collect();
        let assessment_rows = tx
            .query(
                "SELECT run_id, assessment_id, recommendation, reason_code,
                        reviewer_principal_ref, phase
                   FROM pipeline_review_assessments
                  WHERE tenant_id = $1 AND run_id = ANY($2)
                  ORDER BY recorded_at, assessment_id",
                &[&tenant_id, &run_ids],
            )
            .await?;
        tx.commit().await?;
        let mut assessments: std::collections::HashMap<Uuid, Vec<PipelineReviewAuditAssessment>> =
            std::collections::HashMap::new();
        for row in &assessment_rows {
            let approved = match row.get::<_, String>("recommendation").as_str() {
                "approve" => true,
                "reject" => false,
                _ => {
                    return Err(DatabaseError::Serialization(
                        "unknown review recommendation".to_string(),
                    ));
                }
            };
            let at_score = match row.get::<_, String>("phase").as_str() {
                "score" => true,
                "review" => false,
                _ => {
                    return Err(DatabaseError::Serialization(
                        "unknown review assessment phase".to_string(),
                    ));
                }
            };
            assessments
                .entry(row.get("run_id"))
                .or_default()
                .push(PipelineReviewAuditAssessment {
                    assessment_id: row.get("assessment_id"),
                    approved,
                    reason_code: row.get("reason_code"),
                    reviewer_principal_ref: row.get("reviewer_principal_ref"),
                    at_score,
                });
        }
        rows.iter()
            .map(|row| {
                let outcome = match row.get::<_, Option<Uuid>>("outcome_id") {
                    Some(outcome_id) => {
                        let approved =
                            match row.get::<_, Option<String>>("decision_kind").as_deref() {
                                Some("approved") => true,
                                Some("rejected") => false,
                                _ => {
                                    return Err(DatabaseError::Serialization(
                                        "unknown pipeline review decision".to_string(),
                                    ));
                                }
                            };
                        Some(PipelineReviewAuditOutcome {
                            outcome_id,
                            approved,
                        })
                    }
                    None => None,
                };
                Ok(PipelineReviewAuditItem {
                    run_id: row.get("run_id"),
                    submission_id: row.get("submission_id"),
                    pending_at: row.get("review_audit_pending_at"),
                    outcome,
                    assessments: assessments
                        .get(&row.get::<_, Uuid>("run_id"))
                        .cloned()
                        .unwrap_or_default(),
                    privacy_pass_escalated: row.get("privacy_pass_escalated"),
                    privacy_classification_failed: row.get("privacy_classification_failed"),
                    submission_rejected: row.get("submission_rejected"),
                    settle_held_for_operator_at: row.get("settle_held_for_operator_at"),
                })
            })
            .collect()
    }

    /// Clears `item`'s review audit marker once each of its audit events
    /// exists. A marker set again since the list (a newer time) stays.
    pub async fn clear_review_audit_pending(
        &self,
        tenant_id: &str,
        item: &PipelineReviewAuditItem,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "UPDATE pipeline_runs SET review_audit_pending_at = NULL
              WHERE tenant_id = $1 AND run_id = $2 AND review_audit_pending_at = $3",
            &[&tenant_id, &item.run_id, &item.pending_at],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Persists the Settle policy's raw result before any external effect:
    /// `settle_selection`, its hash, the decided
    /// `index_membership`, and -- only for an include -- `index_write_state
    /// = 'pending'`, all in one transaction, and only the first time (the
    /// `settle_selection IS NULL` guard). A later call for the same run
    /// finds the guard already tripped, matches no row, and returns the
    /// stale-lease error; the caller checks `load_settle_selection` first
    /// and skips this call entirely on a retry, so a retry never reaches
    /// that error.
    pub async fn persist_settle_selection(
        &self,
        run: &PipelineRunRecord,
        selection: &StoredPhaseResult,
        selection_hash: &str,
        membership: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let selection_json = serde_json::to_value(selection).map_err(|_| {
            DatabaseError::Serialization("settle selection encode failed".to_string())
        })?;
        let index_write_state = if membership == "included" {
            "pending"
        } else {
            "none"
        };
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET settle_selection = $3, settle_selection_hash = $4,
                     index_membership = $5, index_write_state = $6, updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $7 AND lease_expires_at > NOW()
                   AND settle_selection IS NULL
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &selection_json,
                    &selection_hash,
                    &membership,
                    &index_write_state,
                    &lease_token,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Reloads a persisted Settle selection, if this run has one. `None`
    /// before the first attempt has run the Settle policy.
    pub async fn load_settle_selection(
        &self,
        run: &PipelineRunRecord,
    ) -> Result<Option<StoredPhaseResult>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT settle_selection FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&run.tenant_id, &run.run_id],
            )
            .await?;
        tx.commit().await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let value: Option<serde_json::Value> = row.get("settle_selection");
        value
            .map(|value| {
                serde_json::from_value::<StoredPhaseResult>(value).map_err(|_| {
                    DatabaseError::Serialization("malformed settle selection".to_string())
                })
            })
            .transpose()
    }

    /// Records progress on the run's index dispatch (port 2258 to 2285), in
    /// a transaction of its own: `pending` after `persist_settle_selection`
    /// decides an include, `complete` once every entry has been applied,
    /// `failed` on a content conflict, `cancelled` when the submission
    /// stopped being operable before dispatch could run. The dispatch
    /// records `complete` and `cancelled` on the transaction that holds the
    /// submission guard (`set_index_write_state_on_tx`); it calls this for
    /// `failed`, after it has dropped that transaction.
    pub async fn mark_index_write_state(
        &self,
        run: &PipelineRunRecord,
        index_write_state: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let updated = Self::set_index_write_state_on_tx(&tx, run, index_write_state).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// `mark_index_write_state`'s update, with the same lease check, on the
    /// caller's transaction: Settle's index dispatch records `complete` or
    /// `cancelled` in the transaction that holds the submission guard
    /// (`submission_guard_on_tx`), so the record commits together with the
    /// release of that lock. That transaction is held across the index
    /// write, so the lease is compared with `clock_timestamp()`, not with
    /// `NOW()` (the transaction's start), as the Score commit's is (PR #1283
    /// review, finding 7).
    pub async fn set_index_write_state_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        index_write_state: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                 SET index_write_state = $3, updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $4 AND lease_expires_at > clock_timestamp()
                 RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &index_write_state,
                    &lease_token,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        pipeline_run_from_row(&row)
    }

    /// Queues the removal of `run`'s revision from the vector index, on the
    /// caller's transaction: one `pipeline_index_invalidations` row, due at
    /// once. The row's `state` is the run's invalidation state; a run with
    /// no row has none (readers `LEFT JOIN` the queue).
    /// `reason_code` must match `^[a-z0-9_]{1,64}$` (the table's CHECK).
    ///
    /// The rule: an `index_write_state` of `pending` may be partly written.
    /// A dispatch attempt can apply some or all of its entries and then roll
    /// its transaction back -- an `Uncertain` or `Failed` answer after
    /// earlier entries were applied, a crash after the last upsert, a failed
    /// commit -- so `pending` stays while entries are live. Whoever stops a
    /// `pending` write because the submission is no longer operable must
    /// therefore queue an invalidation, as for a `complete` one. Removing a
    /// revision that has no entries changes nothing
    /// (`VectorIndexWriter::invalidate_revision` returns `Ok(false)`).
    ///
    /// Idempotent: a run that already has an invalidation row keeps that
    /// row as it is, except that a `failed` one runs again from the start.
    pub async fn enqueue_index_invalidation_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        reason_code: &str,
        not_before: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        let revision_id = run.approved_revision_id.ok_or_else(|| {
            DatabaseError::Constraint("index_invalidation_revision_missing".to_string())
        })?;
        // `not_before` (multi-lens review L4-3): an index write that may
        // still be in flight ends by then, so the invalidation is not due
        // earlier, and a pending row already queued is not due earlier
        // either.
        tx.execute(
            "INSERT INTO pipeline_index_invalidations (
                tenant_id, run_id, submission_id, registry_revision_id, reason_code,
                next_attempt_at
             ) VALUES ($1,$2,$3,$4,$5, GREATEST(NOW(), COALESCE($6::timestamptz, NOW())))
             ON CONFLICT (tenant_id, run_id) DO UPDATE
                SET next_attempt_at = GREATEST(
                        pipeline_index_invalidations.next_attempt_at,
                        EXCLUDED.next_attempt_at
                    )
              WHERE pipeline_index_invalidations.state = 'pending'
                AND $6::timestamptz IS NOT NULL",
            &[
                &run.tenant_id,
                &run.run_id,
                &run.submission_id,
                &revision_id,
                &reason_code,
                &not_before,
            ],
        )
        .await?;
        // Zaki review 1, item 6: queuing a revision whose invalidation
        // `failed` runs it again, from the start (and no earlier than
        // `not_before`).
        reset_failed_index_invalidations_on_tx(tx, &run.tenant_id, Some(run.run_id), not_before)
            .await?;
        Ok(())
    }

    /// Re-enqueues every `failed` index invalidation of `tenant_id`, in one
    /// tenant transaction: each goes back to `pending`, with no attempt
    /// charged and due at once, so the worker's next pass tries it again
    /// from the start.
    /// Returns how many it re-enqueued. The operator route behind `main`'s
    /// admin credential calls this once the fault that failed them is fixed
    /// (Zaki review 1, item 6).
    pub async fn requeue_failed_index_invalidations(
        &self,
        tenant_id: &str,
    ) -> Result<u64, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let requeued = reset_failed_index_invalidations_on_tx(&tx, tenant_id, None, None).await?;
        tx.commit().await?;
        Ok(requeued)
    }

    /// Sets, or extends, the index rebuild fence `fence_id` of `tenant_id`
    /// (`pipeline_index_rebuild_fences`, V113) to end `duration` from now:
    /// one statement in a tenant transaction of its own, committed before
    /// this returns. While any fence of the tenant is unexpired, the
    /// invalidation claim takes none of the tenant's invalidations
    /// (`claim_due_index_invalidations`). There is one row per rebuild,
    /// keyed by `fence_id`: a later set of the same fence never moves its
    /// end earlier (`GREATEST`), and a set never touches another rebuild's
    /// row. The database computes the end from its own clock
    /// (`clock_timestamp()` plus `duration`), the clock the claim compares it
    /// with (`NOW()`), so a skew between the application's clock and the
    /// database's cannot end a fence early.
    pub async fn set_index_rebuild_fence(
        &self,
        tenant_id: &str,
        fence_id: Uuid,
        duration: std::time::Duration,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        Self::set_index_rebuild_fence_on(&mut client, tenant_id, fence_id, duration).await
    }

    /// `set_index_rebuild_fence` on a connection the caller already holds,
    /// outside any transaction of its own: the rebuild sets each run's fence
    /// on the connection that then opens the run's transaction, so it never
    /// holds two pooled connections at once.
    pub(super) async fn set_index_rebuild_fence_on(
        client: &mut deadpool_postgres::Client,
        tenant_id: &str,
        fence_id: Uuid,
        duration: std::time::Duration,
    ) -> Result<(), DatabaseError> {
        let milliseconds = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
        let tx = Self::tenant_transaction(client, tenant_id).await?;
        tx.execute(
            "INSERT INTO pipeline_index_rebuild_fences (tenant_id, fence_id, fenced_until)
             VALUES ($1, $2, clock_timestamp() + ($3::bigint * INTERVAL '1 millisecond'))
             ON CONFLICT (tenant_id, fence_id) DO UPDATE
                SET fenced_until = GREATEST(pipeline_index_rebuild_fences.fenced_until,
                                            EXCLUDED.fenced_until)",
            &[&tenant_id, &fence_id, &milliseconds],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Deletes the index rebuild fence `fence_id` of `tenant_id`, and the
    /// tenant's expired fences, which hold nothing back, so the row a lost
    /// rebuild leaves does not stay for ever: one statement in a tenant
    /// transaction of its own. It never deletes another rebuild's unexpired
    /// fence. A rebuild calls it once no write of its own is still running
    /// (`PipelineService::rebuild_index_from_authoritative_commands`).
    pub async fn clear_index_rebuild_fence(
        &self,
        tenant_id: &str,
        fence_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "DELETE FROM pipeline_index_rebuild_fences
              WHERE tenant_id = $1
                AND (fence_id = $2 OR fenced_until <= clock_timestamp())",
            &[&tenant_id, &fence_id],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Claims up to `limit` (clamped to 1..=500) of `tenant_id`'s index
    /// invalidations that are due -- `pending`, not held by a claim's lease
    /// or waiting out a retry's backoff, and with an attempt left, oldest due
    /// first -- each for one attempt, and holds them for `lease`. Listing and
    /// claiming are one statement (Zaki review 1, round 2, item 4): it
    /// selects the due rows `FOR UPDATE SKIP LOCKED` and moves their
    /// `next_attempt_at` to the lease's end. A row another claim has locked
    /// is skipped, and a row it claimed is no longer due, so two claims at
    /// once never get the same row. Only `tenant_id`'s own queue is read.
    ///
    /// It claims nothing while the tenant has an unexpired index rebuild
    /// fence (`pipeline_index_rebuild_fences`, V113;
    /// `set_index_rebuild_fence`). The fence's `NOT EXISTS` is in the `due`
    /// CTE, so the fences and the invalidations are read in one statement,
    /// with one snapshot, and that is enough. An invalidation this claim can
    /// see was queued by a withdrawal that committed before the claim's
    /// snapshot. A rebuild commits a run's fence before it locks the run
    /// (`PipelineService::rebuild_index_run`). If that fence is not visible
    /// in the snapshot, the withdrawal committed before the fence, so the
    /// rebuild's own lock of that run sees the queued invalidation and skips
    /// the run: `lock_rebuildable_index_run_on_tx` takes the run row
    /// `FOR SHARE` and then, in a second statement whose snapshot is taken
    /// after that lock and after the fence, reads
    /// `rebuildable_index_run_predicate!`, whose `NOT EXISTS` on
    /// `pipeline_index_invalidations` sees the invalidation. If the
    /// withdrawal committed after the fence, the claim sees the fence. A
    /// fence holds nothing back once it has expired or been deleted. A row
    /// expires at the deadline of the last run it covered plus the write
    /// fence margin, and a write starts only before that deadline; a
    /// rebuild deletes its own row only once none of its writes is running
    /// (a call still in flight at the margin leaves the row to expire), and
    /// another rebuild's row only once it has expired. An index call has no time limit of its own, so a write
    /// the fence covered can still be running after the fence is gone, and
    /// this claim can remove the revision before that write lands. That is
    /// not left standing: the late write is awaited until it returns and
    /// then reopens the run's invalidation, so the removal runs again
    /// (`reopen_invalidation_after_index_writes`, issue #1233, which states
    /// the invariant). The fence bounds how long claims are held back; it
    /// does not prove that no covered write is in flight.
    /// `NOW()` is the claim transaction's start, no later than the
    /// statement's snapshot, so a fence is never read as expired early.
    ///
    /// The claim charges no attempt: only `fail_index_invalidation` charges
    /// one, for a failure that waiting cannot heal. An index outage
    /// (`retry_index_invalidation`) is not charged, and neither is an
    /// attempt that never reports back (the worker died): the row is due
    /// again once the lease has passed, the way a run's expired lease is.
    ///
    /// The claims come back in run id order.
    pub async fn claim_due_index_invalidations(
        &self,
        tenant_id: &str,
        limit: usize,
        lease: Duration,
    ) -> Result<Vec<PipelineIndexInvalidationClaim>, DatabaseError> {
        let limit = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let lease_milliseconds = lease.num_milliseconds().max(1);
        let mut client = self.backend.trace_pool().get().await?;
        // READ COMMITTED, stated (G14): the claim's one statement takes its
        // snapshot when it runs, not at the transaction's first statement.
        let tx = Self::read_committed_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                // A materialized CTE, not `IN (SELECT ... LIMIT ...)`: the
                // planner may evaluate an `IN` subquery more than once, and
                // with `SKIP LOCKED` each evaluation can pick other rows, so
                // one claim could take more than `limit`.
                "WITH due AS MATERIALIZED (
                    SELECT run_id FROM pipeline_index_invalidations
                     WHERE tenant_id = $1 AND state = 'pending'
                       AND next_attempt_at <= NOW() AND attempt_count < max_attempts
                       AND NOT EXISTS (
                           SELECT 1 FROM pipeline_index_rebuild_fences f
                            WHERE f.tenant_id = $1 AND f.fenced_until > NOW()
                       )
                     ORDER BY next_attempt_at, run_id
                     LIMIT $2
                     FOR UPDATE SKIP LOCKED
                 )
                 UPDATE pipeline_index_invalidations i
                    SET next_attempt_at = NOW() + ($3::bigint * INTERVAL '1 millisecond')
                   FROM due
                  WHERE i.tenant_id = $1 AND i.run_id = due.run_id
                  RETURNING i.run_id, i.registry_revision_id, i.attempt_count, i.next_attempt_at",
                &[&tenant_id, &limit, &lease_milliseconds],
            )
            .await?;
        tx.commit().await?;
        let mut claims = rows
            .iter()
            .map(|row| {
                Ok(PipelineIndexInvalidationClaim {
                    tenant_id: tenant_id.to_string(),
                    run_id: row.get("run_id"),
                    registry_revision_id: row.get("registry_revision_id"),
                    attempt_count: u32::try_from(row.get::<_, i32>("attempt_count")).map_err(
                        |_| {
                            DatabaseError::Serialization(
                                "invalid index invalidation attempt count".to_string(),
                            )
                        },
                    )?,
                    lease_expires_at: row.get("next_attempt_at"),
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        claims.sort_by_key(|claim| claim.run_id);
        Ok(claims)
    }

    /// Records that `claim`'s attempt removed the revision from the index:
    /// the invalidation becomes `complete`. Fenced as
    /// `fail_index_invalidation` is: the invalidation must still be
    /// `pending` with `next_attempt_at` at `claim`'s lease end. `false`,
    /// with nothing written, otherwise (or when the invalidation is gone
    /// with its run). It touches only the invalidation row, so it takes no
    /// run lock (Zaki review 1, round 2, simplification).
    ///
    /// The fence is issue #1233's: an index write that started before the
    /// withdrawal can land after this claim's removal ran, and the task
    /// that awaited it then reopens the invalidation
    /// (`reopen_index_invalidation_after_late_write`), which moves
    /// `next_attempt_at`. This claim's removal may predate that write, so
    /// it must not end the reopened row; the next claim removes the
    /// revision again. A claim whose lease merely passed, with no other
    /// claim since, still completes the row.
    pub async fn complete_index_invalidation(
        &self,
        claim: &PipelineIndexInvalidationClaim,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &claim.tenant_id).await?;
        let completed = tx
            .execute(
                "UPDATE pipeline_index_invalidations
                    SET state = 'complete', completed_at = NOW(), last_error_label = NULL
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'pending'
                    AND next_attempt_at = $3",
                &[&claim.tenant_id, &claim.run_id, &claim.lease_expires_at],
            )
            .await?;
        tx.commit().await?;
        Ok(completed > 0)
    }

    /// Reopens the index invalidation of `run_id`, if it has one, after an
    /// index write of that run returned late (issue #1233): a Settle
    /// dispatch or a rebuilt run whose deadline and fence margin passed
    /// with an index call still in flight. That call may have written its
    /// entry after a withdrawal's removal ran, so the removal must run
    /// again. Called only once the write has returned, so any invalidation
    /// this statement does not see is queued after the write landed, and
    /// runs after it.
    ///
    /// A `complete` invalidation goes back to `pending`, due at once. A
    /// `pending` one stays `pending`, and its `next_attempt_at` moves one
    /// millisecond past the later of its current value and now: never
    /// earlier than a due time a withdrawal set, but always away from the
    /// token of a claim that may be removing the revision right now, so
    /// that claim cannot record the row `complete`
    /// (`complete_index_invalidation`) with a removal that may have run
    /// before the write landed. A `failed` invalidation is left as it is:
    /// the operator's requeue (`requeue_failed_index_invalidations`) runs
    /// the removal from the start, which removes the late entry too.
    ///
    /// Returns whether a row was reopened. One statement in a READ
    /// COMMITTED tenant transaction of its own, so it waits for a claim or
    /// a completion that holds the row and then updates the row as it now
    /// is.
    pub async fn reopen_index_invalidation_after_late_write(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::read_committed_tenant_transaction(&mut client, tenant_id).await?;
        let reopened = tx
            .execute(
                "UPDATE pipeline_index_invalidations
                    SET next_attempt_at = CASE
                            WHEN state = 'complete' THEN NOW()
                            ELSE GREATEST(next_attempt_at, NOW()) + INTERVAL '1 millisecond'
                        END,
                        state = 'pending',
                        completed_at = NULL
                  WHERE tenant_id = $1 AND run_id = $2
                    AND state IN ('pending', 'complete')",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        Ok(reopened > 0)
    }

    /// Records that the index was unavailable for `claim`'s attempt (it
    /// answered `Failed` or `Uncertain`): an uncharged retry (Ruling T8-2).
    /// The invalidation stays `pending` with its attempt count unchanged,
    /// under `index_invalidation_unavailable`, and is due again after the
    /// FR3 backoff `mark_transient_retry` uses, measured from the row's own
    /// `requested_at` instead of a run's `phase_started_at`: the wait is the
    /// invalidation's age, at least one second and at most one hour, so each
    /// retry lands when the invalidation is about twice as old as at the one
    /// before -- the delay doubles -- until, once it is an hour old, it is
    /// retried once an hour. There is no terminal bound: an outage never
    /// ends a queued invalidation, so the revision is removed once the index
    /// answers again (Review Focus 1).
    ///
    /// Fenced as `fail_index_invalidation` is: `false`, with nothing
    /// written, unless `claim` still holds the row.
    pub async fn retry_index_invalidation(
        &self,
        claim: &PipelineIndexInvalidationClaim,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &claim.tenant_id).await?;
        let retried = tx
            .execute(
                "UPDATE pipeline_index_invalidations
                    SET last_error_label = $3,
                        next_attempt_at = NOW() + LEAST(
                            GREATEST(NOW() - requested_at, INTERVAL '1 second'),
                            INTERVAL '1 hour'
                        )
                  WHERE tenant_id = $1 AND run_id = $2
                    AND state = 'pending' AND next_attempt_at = $4",
                &[
                    &claim.tenant_id,
                    &claim.run_id,
                    &PIPELINE_INDEX_INVALIDATION_UNAVAILABLE_LABEL,
                    &claim.lease_expires_at,
                ],
            )
            .await?;
        tx.commit().await?;
        Ok(retried > 0)
    }

    /// Records that `claim`'s attempt failed in a way waiting cannot heal
    /// (the run's committed Score evidence names no index, or the index
    /// broke its contract and answered `ContentConflict`), charging it one
    /// attempt. With an attempt left, the invalidation stays `pending`
    /// under `index_invalidation_unavailable` and is due again after the
    /// charged-retry backoff `mark_retry` uses (50 ms, doubling with each
    /// charged attempt, capped at 50 ms x 2^9). The attempt that uses the
    /// last one leaves the invalidation `failed` under
    /// `index_invalidation_failed`. An index outage is never recorded here
    /// (`retry_index_invalidation`).
    ///
    /// Only the current claim records a failure: the invalidation must be
    /// `pending` with `next_attempt_at` still at `claim`'s lease end. A
    /// claim whose lease passed and whose row another worker claimed, or a
    /// claim that already recorded its result, gets `false` and writes
    /// nothing.
    pub async fn fail_index_invalidation(
        &self,
        claim: &PipelineIndexInvalidationClaim,
    ) -> Result<bool, DatabaseError> {
        let exponent = claim.attempt_count.min(9);
        let retry_milliseconds = DEFAULT_RETRY_MILLISECONDS.saturating_mul(1_i64 << exponent);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &claim.tenant_id).await?;
        let failed = tx
            .execute(
                "UPDATE pipeline_index_invalidations
                    SET attempt_count = attempt_count + 1,
                        state = CASE
                            WHEN attempt_count + 1 >= max_attempts THEN 'failed'
                            ELSE 'pending'
                        END,
                        last_error_label = CASE
                            WHEN attempt_count + 1 >= max_attempts THEN $4
                            ELSE $3
                        END,
                        next_attempt_at = CASE
                            WHEN attempt_count + 1 >= max_attempts THEN next_attempt_at
                            ELSE NOW() + ($5::bigint * INTERVAL '1 millisecond')
                        END
                  WHERE tenant_id = $1 AND run_id = $2
                    AND state = 'pending' AND next_attempt_at = $6",
                &[
                    &claim.tenant_id,
                    &claim.run_id,
                    &PIPELINE_INDEX_INVALIDATION_UNAVAILABLE_LABEL,
                    &PIPELINE_INDEX_INVALIDATION_FAILED_LABEL,
                    &retry_milliseconds,
                    &claim.lease_expires_at,
                ],
            )
            .await?;
        tx.commit().await?;
        Ok(failed > 0)
    }

    /// Whether the submission behind `run` is operable for Score and Settle:
    /// accepted, not revoked, not purged, not expired, and with no
    /// `trace_withdrawals` row. Runs on the caller's transaction and holds
    /// the submission row `FOR SHARE` until that transaction ends, so a
    /// withdrawal (which locks the row `FOR UPDATE`) waits for whatever the
    /// caller commits under the guard. Callers that also lock the run row
    /// (`ensure_current_lease`) take it first: run row, then submission row,
    /// the order `commit_review` documents.
    ///
    /// Two statements: lock first, read second, as `settle_internal_credit`
    /// does. A single locking read waits for a withdrawal that holds the row
    /// and then still evaluates the `trace_withdrawals` subquery against the
    /// snapshot it took before the wait, so it misses a withdrawal row that
    /// transaction inserted without changing the submission row. Under READ
    /// COMMITTED the second statement takes a new snapshot after the lock
    /// is held, so it sees that withdrawal.
    pub async fn submission_guard_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
    ) -> Result<SubmissionGuard, DatabaseError> {
        tx.query_opt(
            "SELECT 1 FROM trace_submissions
              WHERE tenant_id = $1 AND submission_id = $2
              FOR SHARE",
            &[&run.tenant_id, &run.submission_id],
        )
        .await?;
        let row = tx
            .query_opt(
                concat!(
                    "SELECT ",
                    accepted_submission_sql!(),
                    " FROM trace_submissions s
                      WHERE s.tenant_id = $1 AND s.submission_id = $2
                      FOR SHARE OF s"
                ),
                &[&run.tenant_id, &run.submission_id],
            )
            .await?;
        let operable = row.map(|row| row.get::<_, bool>(0)).unwrap_or(false);
        Ok(SubmissionGuard { operable })
    }

    /// Withdraws a submission for its owner, with every side effect in one
    /// transaction (port `ef97a459` lines 1083 to 1471, written in the shape
    /// `main`'s account withdrawal has now).
    ///
    /// Source session (`main`'s #1021 binding): when `account_id` maps the
    /// submission to one of that account's source sessions, the session is
    /// marked withdrawn, so a new upload of it is refused, and every
    /// submission mapped to the session is withdrawn with this one, each
    /// first checked to belong to the account. This is the session half
    /// `main`'s `withdraw_trace_source_session` also calls
    /// (`withdraw_source_session_on_tx`). Otherwise only the requested
    /// submission is withdrawn.
    ///
    /// Lock order: the session row, then the `pipeline_runs` rows of every
    /// affected submission (`FOR UPDATE`, by `run_id`), then the submission
    /// rows (`FOR UPDATE`, by `submission_id`). Settle's index dispatch holds
    /// a run row and then its submission row `FOR SHARE` for its whole write,
    /// so a withdrawal waits for that write at the run row and never holds a
    /// submission row the dispatch waits for.
    ///
    /// Every affected submission gets `main`'s rows, through the same
    /// per-submission half `main` calls
    /// (`record_source_submission_withdrawal_on_tx`): a `trace_withdrawals`
    /// row (first writer wins, so a retry reports the first tier and time)
    /// and the status `revoked` with `withdrawn_at`, `revoked_at` and
    /// `purged_at` set; a delivered pipeline export snapshot counts as an
    /// export for the tier. One with a pipeline run also gets the pipeline
    /// follow-up (`withdraw_pipeline_content_on_tx`), and each of its runs:
    ///
    /// - an index write that is `pending` may be partly written, so it is
    ///   cancelled, the run is excluded from the index, and an invalidation
    ///   of the revision is queued; a write that is `complete`, `failed` or
    ///   `cancelled` wrote or may have written entries, so an invalidation is
    ///   queued (idempotent); `none` needs nothing;
    /// - a run parked for review goes back to `pending`, due at once, so the
    ///   runner ends it under `submission_inoperable`.
    ///
    /// Nothing here fails a run or changes a settlement row: the runner does
    /// both under the run's lease, and Settle forfeits every leg that is not
    /// complete on its next pass. No audit row is written here either: the
    /// caller appends the event to the mirrored audit log, as `main`'s
    /// withdrawal does.
    ///
    /// Not the owner (the requested submission's `auth_principal_ref` is not
    /// `actor_principal_ref`), or no such submission: `NotFound`, and
    /// nothing is written.
    pub async fn withdraw_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
        account_id: Option<Uuid>,
    ) -> Result<PipelineWithdrawalOutcome, DatabaseError> {
        let not_found = || DatabaseError::NotFound {
            entity: "trace_submission".to_string(),
            id: submission_id.to_string(),
        };
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        // Ownership before any lock is taken; checked again under the lock.
        let owner = tx
            .query_opt(
                "SELECT auth_principal_ref FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await?
            .map(|row| row.get::<_, String>(0));
        if owner.as_deref() != Some(actor_principal_ref) {
            return Err(not_found());
        }

        // The session half of `main`'s source-session withdrawal: it marks
        // the session withdrawn (its row is the first lock taken here) and
        // returns every submission mapped to it, each checked to belong to
        // the account. Without a mapped session, only this submission.
        let now: DateTime<Utc> = tx.query_one("SELECT NOW()", &[]).await?.get(0);
        let session = match account_id {
            Some(account_id) => {
                withdraw_source_session_on_tx(&tx, tenant_id, account_id, submission_id, now)
                    .await?
            }
            None => None,
        };
        let (withdrawn_at, affected) = session.unwrap_or((now, vec![submission_id]));

        let runs = tx
            .query(
                "SELECT * FROM pipeline_runs
                  WHERE tenant_id = $1 AND submission_id = ANY($2)
                  ORDER BY run_id
                  FOR UPDATE",
                &[&tenant_id, &affected],
            )
            .await?
            .iter()
            .map(pipeline_run_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let submissions = tx
            .query(
                "SELECT submission_id, status, auth_principal_ref, trace_id, redaction_hash,
                        canonical_summary_hash
                   FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = ANY($2)
                  ORDER BY submission_id
                  FOR UPDATE",
                &[&tenant_id, &affected],
            )
            .await?
            .into_iter()
            .map(|row| (row.get::<_, Uuid>("submission_id"), row))
            .collect::<BTreeMap<_, _>>();
        let owned = submissions
            .get(&submission_id)
            .is_some_and(|row| row.get::<_, String>("auth_principal_ref") == actor_principal_ref);
        if !owned {
            return Err(not_found());
        }

        // A delivered pipeline export snapshot also put copies out: the
        // affected submissions one carries, read once for all of them (Zaki
        // review 1, round 2, item 5).
        let exported_by_a_pipeline_snapshot = tx
            .query(
                "SELECT DISTINCT item.submission_id
                   FROM pipeline_export_snapshot_items item
                   JOIN pipeline_export_snapshots snapshot
                     ON snapshot.tenant_id = item.tenant_id
                    AND snapshot.snapshot_id = item.snapshot_id
                  WHERE item.tenant_id = $1 AND item.submission_id = ANY($2)
                    AND snapshot.completed_at IS NOT NULL",
                &[&tenant_id, &affected],
            )
            .await?
            .iter()
            .map(|row| row.get::<_, Uuid>(0))
            .collect::<std::collections::BTreeSet<_>>();
        let mut with_runs = Vec::new();
        for affected_id in &affected {
            let submission = submissions.get(affected_id);
            // `main`'s per-submission rows, with its tier rule. They are
            // `main`'s code and stay one call per submission.
            let content_status = submission.map(|row| row.get::<_, String>("status"));
            record_source_submission_withdrawal_on_tx(
                &tx,
                tenant_id,
                *affected_id,
                content_status.as_deref(),
                withdrawn_at,
                exported_by_a_pipeline_snapshot.contains(affected_id),
            )
            .await?;
            if let Some(submission) = submission
                && runs.iter().any(|run| run.submission_id == *affected_id)
            {
                with_runs.push((*affected_id, submission));
            }
        }
        withdraw_pipeline_content_on_tx(
            &tx,
            tenant_id,
            &with_runs,
            actor_principal_ref,
            PIPELINE_WITHDRAWAL_INVALIDATION_REASON,
        )
        .await?;
        let with_runs = with_runs
            .into_iter()
            .map(|(submission_id, _)| submission_id)
            .collect::<Vec<_>>();

        Self::end_runs_of_inoperable_submission_on_tx(
            &tx,
            tenant_id,
            &runs,
            PIPELINE_WITHDRAWAL_INVALIDATION_REASON,
        )
        .await?;

        let withdrawal_row = tx
            .query_one(
                "SELECT tenant_id, submission_id, withdrawn_at, prior_status, distribution_reach
                   FROM trace_withdrawals
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await?;
        let follow_up = tx
            .query_one(
                "SELECT
                    (SELECT COUNT(*) FROM trace_revocation_propagation_items
                      WHERE tenant_id = $1 AND source_submission_id = ANY($2)
                        AND status IN ('pending', 'in_progress', 'failed')),
                    (SELECT COUNT(*) FROM trace_revocation_propagation_items
                      WHERE tenant_id = $1 AND source_submission_id = ANY($2)),
                    (SELECT COALESCE(array_agg(DISTINCT COALESCE(i.state, 'none')), '{}')
                       FROM pipeline_runs r
                       LEFT JOIN pipeline_index_invalidations i
                         ON i.tenant_id = r.tenant_id AND i.run_id = r.run_id
                      WHERE r.tenant_id = $1 AND r.submission_id = ANY($2)),
                    EXISTS (
                        SELECT 1
                          FROM pipeline_run_settlements settlement
                          JOIN pipeline_runs run
                            ON run.tenant_id = settlement.tenant_id
                           AND run.run_id = settlement.run_id
                         WHERE settlement.tenant_id = $1 AND run.submission_id = ANY($2)
                           AND settlement.instrument_id = $3
                           AND (settlement.operation_state NOT IN ('complete', 'forfeited')
                                OR (settlement.operation_state = 'forfeited'
                                    AND settlement.last_error_label = $4))
                    )",
                &[
                    &tenant_id,
                    &with_runs,
                    &InstrumentId::trace_credit().as_str(),
                    &PIPELINE_SUBMISSION_INOPERABLE_LABEL,
                ],
            )
            .await?;
        let unfinished_propagation: i64 = follow_up.get(0);
        let queued_propagation: i64 = follow_up.get(1);
        let invalidation_states: Vec<String> = follow_up.get(2);
        // A leg Settle has not settled yet is forfeited on its next pass; a
        // Trace Credit leg Settle already forfeited for this reason stays in
        // the count, so a retry after that pass reports the same.
        let trace_credit_forfeited: bool = follow_up.get(3);
        // `pending` while any queued payload deletion is unfinished, `complete` once all
        // finished, `not_required` if none was queued; the port said `not_required` once
        // no live object ref remained, which misreports deletions that finished.
        let revocation_propagation = if unfinished_propagation > 0 {
            PipelineWithdrawalFollowUpState::Pending
        } else if queued_propagation > 0 {
            PipelineWithdrawalFollowUpState::Complete
        } else {
            PipelineWithdrawalFollowUpState::NotRequired
        };
        let has_state = |state: &str| invalidation_states.iter().any(|value| value == state);
        let index_invalidation = if has_state("failed") {
            PipelineWithdrawalFollowUpState::Failed
        } else if has_state("pending") {
            PipelineWithdrawalFollowUpState::Pending
        } else if has_state("complete") {
            PipelineWithdrawalFollowUpState::Complete
        } else {
            PipelineWithdrawalFollowUpState::NotRequired
        };
        let withdrawal = crate::trace_corpus_storage::TraceWithdrawalRecord {
            tenant_id: withdrawal_row.get("tenant_id"),
            submission_id: withdrawal_row.get("submission_id"),
            withdrawn_at: withdrawal_row.get("withdrawn_at"),
            prior_status: withdrawal_row.get("prior_status"),
            distribution_reach: withdrawal_row.get("distribution_reach"),
        };
        tx.commit().await?;
        Ok(PipelineWithdrawalOutcome {
            withdrawal,
            affected_submission_ids: affected,
            index_invalidation,
            revocation_propagation,
            trace_credit_forfeited,
        })
    }

    /// Ends the index and review work of `runs`, whose submission is no
    /// longer operable, on the caller's transaction, which holds their rows:
    /// a `pending` index write (which may be partly written) is cancelled
    /// and excluded, and every run whose write may have written entries
    /// queues the removal of its revision under `reason_code`; a run parked
    /// for review is released, so the runner ends it as
    /// `submission_inoperable`.
    pub(super) async fn end_runs_of_inoperable_submission_on_tx(
        tx: &Transaction<'_>,
        tenant_id: &str,
        runs: &[PipelineRunRecord],
        reason_code: &str,
    ) -> Result<(), DatabaseError> {
        for run in runs {
            match run.index_write_state.as_str() {
                "pending" => {
                    tx.execute(
                        "UPDATE pipeline_runs
                            SET index_membership = 'excluded',
                                index_write_state = 'cancelled',
                                updated_at = NOW()
                          WHERE tenant_id = $1 AND run_id = $2",
                        &[&tenant_id, &run.run_id],
                    )
                    .await?;
                    // Multi-lens review L4-3: on a run still `leased`, a
                    // dispatch that lost its row locks (a lost session, a
                    // process exit) may still be writing. It starts no upsert
                    // after the lease's end, and one in flight usually
                    // returns within the fence margin, so the invalidation
                    // is not due before then. A dispatch releases its run
                    // once its writes ended or the margin passed. A call
                    // still running after that is awaited by a task that
                    // reopens this invalidation when it returns (issue
                    // #1233, `reopen_invalidation_after_index_writes`), so
                    // the margin bounds when the removal first runs, not
                    // whether a late entry is removed.
                    let not_before = (run.state == PipelineRunState::Leased)
                        .then_some(run.lease_expires_at)
                        .flatten()
                        .map(|expires_at| {
                            expires_at
                                + Duration::seconds(PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS)
                        });
                    Self::enqueue_index_invalidation_on_tx(tx, run, reason_code, not_before)
                        .await?;
                }
                "complete" | "failed" | "cancelled" => {
                    Self::enqueue_index_invalidation_on_tx(tx, run, reason_code, None).await?;
                }
                _ => {}
            }
            release_awaiting_review(tx, tenant_id, run.run_id).await?;
        }
        Ok(())
    }

    /// The pipeline's follow-up of `main`'s revocation of `submission_id`
    /// (`DELETE /v1/traces/{id}`, `POST /v1/traces/{id}/revoke`, `DELETE
    /// /v1/traces`; Zaki review 1, round 2, finding 5):
    /// `follow_up_inoperable_submission` under reason `revoked`.
    pub async fn follow_up_revocation(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
    ) -> Result<bool, DatabaseError> {
        self.follow_up_inoperable_submission(
            tenant_id,
            submission_id,
            actor_principal_ref,
            PIPELINE_REVOCATION_INVALIDATION_REASON,
        )
        .await
    }

    /// The pipeline's follow-up of a withdrawal `main` completes on its own
    /// path (`main`'s #1155: the merge confirm and the revocation-propagation
    /// worker's source-session reconciler): `follow_up_inoperable_submission`
    /// under reason `withdrawn`, as the pipeline withdrawal queues it.
    pub async fn follow_up_withdrawal(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
    ) -> Result<bool, DatabaseError> {
        self.follow_up_inoperable_submission(
            tenant_id,
            submission_id,
            actor_principal_ref,
            PIPELINE_WITHDRAWAL_INVALIDATION_REASON,
        )
        .await
    }

    /// poldsam P-2: recovers the follow-up of a submission `main` revoked or
    /// withdrew whose follow-up was lost. `main` marks the submission in one
    /// transaction and the follow-up (`follow_up_revocation`,
    /// `follow_up_withdrawal`) runs in another, so a process that stops
    /// between the two leaves runs with index work and no queued
    /// invalidation, and export snapshot items that are not invalidated,
    /// which nothing else reaches. This finds at most `limit` such
    /// submissions of `tenant_id`, in submission id order from just after
    /// the last one the previous pass of this store read for the tenant,
    /// wrapping to the lowest id (PR #1283 review, finding 4: follow-ups
    /// that fail on every pass would otherwise fill the window and keep the
    /// later ones from ever being reached): revoked, or with
    /// a `trace_withdrawals` row, and with either a run whose index write
    /// started (`pending`, `complete`, `failed` or `cancelled`, the states
    /// `end_runs_of_inoperable_submission_on_tx` queues) and no invalidation
    /// row at all, or an export snapshot item that is not invalidated (a
    /// run Settle excludes from the index has no index work, and an export
    /// can still hold it). For each it runs the follow-up, under reason
    /// `withdrawn` when the withdrawal row exists and `revoked` otherwise,
    /// with `actor_principal_ref` as the actor. A follow-up is idempotent,
    /// queues an invalidation for every such run and invalidates every such
    /// item, so a recovered submission is not found again. A follow-up that
    /// fails is logged by label and the pass goes on to the next
    /// submission; the failed one is found again when a later pass wraps
    /// round to it. Returns
    /// how many it recovered, or the first error when every follow-up of
    /// the pass failed. The read probes every revoked or withdrawn
    /// submission of the tenant, recovered or not, so the worker runs it at
    /// a low cadence (`PIPELINE_WORKER_LOST_FOLLOW_UP_INTERVAL`), not on
    /// every pass. A tenant with no pipeline run is answered 0 before that
    /// read.
    pub async fn recover_lost_inoperable_follow_ups(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        limit: usize,
    ) -> Result<usize, DatabaseError> {
        let limit = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let cursor: Option<Uuid> = self
            .lost_follow_up_cursors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(tenant_id)
            .copied();
        let lost = {
            let mut client = self.backend.trace_pool().get().await?;
            let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
            // Merge review M4: a listed tenant with no pipeline run (PR 5
            // asks each replica to list a tenant before its first
            // activation) has nothing to recover, and an export item needs
            // a run too. One read by the runs' key, and none of the
            // tenant's revoked and withdrawn submissions.
            let has_pipeline_run: bool = tx
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pipeline_runs WHERE tenant_id = $1)",
                    &[&tenant_id],
                )
                .await?
                .get(0);
            if !has_pipeline_run {
                tx.commit().await?;
                return Ok(0);
            }
            let rows = tx
                .query(
                    // The owner's runtime lens: read from the two indexed
                    // sources of an inoperable submission (the status index
                    // for `revoked`, the withdrawals key), never from every
                    // submission of the tenant.
                    "SELECT c.submission_id, bool_or(c.withdrawn) AS withdrawn
                       FROM (
                            SELECT s.submission_id, FALSE AS withdrawn
                              FROM trace_submissions s
                             WHERE s.tenant_id = $1 AND s.status = 'revoked'
                            UNION ALL
                            SELECT w.submission_id, TRUE AS withdrawn
                              FROM trace_withdrawals w
                             WHERE w.tenant_id = $1
                       ) c
                      WHERE EXISTS (
                            SELECT 1 FROM pipeline_runs r
                             WHERE r.tenant_id = $1
                               AND r.submission_id = c.submission_id
                               AND r.approved_revision_id IS NOT NULL
                               AND r.index_write_state IN
                                   ('pending', 'complete', 'failed', 'cancelled')
                               AND NOT EXISTS (
                                   SELECT 1 FROM pipeline_index_invalidations i
                                    WHERE i.tenant_id = r.tenant_id AND i.run_id = r.run_id
                               )
                        )
                         OR EXISTS (
                            SELECT 1 FROM pipeline_export_snapshot_items item
                             WHERE item.tenant_id = $1
                               AND item.submission_id = c.submission_id
                               AND item.invalidated_at IS NULL
                        )
                      GROUP BY c.submission_id
                      ORDER BY (c.submission_id <= $3::uuid) IS TRUE, c.submission_id
                      LIMIT $2",
                    &[&tenant_id, &limit, &cursor],
                )
                .await?;
            tx.commit().await?;
            rows.iter()
                .map(|row| (row.get::<_, Uuid>(0), row.get::<_, bool>(1)))
                .collect::<Vec<_>>()
        };
        // The cursor moves as soon as the window is read, whatever its
        // follow-ups do: a window whose follow-ups all fail answers an
        // error, and the next pass must still move past it.
        if let Some((last_read, _)) = lost.last() {
            self.lost_follow_up_cursors
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(tenant_id.to_string(), *last_read);
        }
        let mut recovered = 0;
        let mut first_error = None;
        for (submission_id, withdrawn) in &lost {
            let reason_code = if *withdrawn {
                PIPELINE_WITHDRAWAL_INVALIDATION_REASON
            } else {
                PIPELINE_REVOCATION_INVALIDATION_REASON
            };
            match self
                .follow_up_inoperable_submission(
                    tenant_id,
                    *submission_id,
                    actor_principal_ref,
                    reason_code,
                )
                .await
            {
                Ok(_) => recovered += 1,
                Err(error) => {
                    // Multi-lens review C2: one failed follow-up does not
                    // stop the submissions after it.
                    tracing::warn!(
                        label = "pipeline_lost_follow_up_failed",
                        tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id).as_str(),
                        submission_ref_hash = %sha256_prefixed(submission_id.to_string().as_bytes()),
                        "a lost pipeline follow-up failed again"
                    );
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) if recovered == 0 => Err(error),
            _ => Ok(recovered),
        }
    }

    /// The pipeline's follow-up of a submission `main` made inoperable on its
    /// own path, in one tenant transaction after `main` marked it. For a
    /// submission with a pipeline run it does what the pipeline withdrawal
    /// does for its content (`withdraw_pipeline_content_on_tx`: tombstone,
    /// invalidations, one payload deletion per live object) and ends its
    /// runs' work (`end_runs_of_inoperable_submission_on_tx`, under
    /// `reason_code`). Lock order as the withdrawal's: the run rows, then the
    /// submission row. Returns whether an index invalidation of the
    /// submission's runs is pending afterwards; `false`, with nothing
    /// written, for a submission with no pipeline run.
    pub(super) async fn follow_up_inoperable_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
        reason_code: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let runs = tx
            .query(
                "SELECT * FROM pipeline_runs
                  WHERE tenant_id = $1 AND submission_id = $2
                  ORDER BY run_id
                  FOR UPDATE",
                &[&tenant_id, &submission_id],
            )
            .await?
            .iter()
            .map(pipeline_run_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        if runs.is_empty() {
            tx.commit().await?;
            return Ok(false);
        }
        let submission = tx
            .query_opt(
                "SELECT submission_id, trace_id, redaction_hash, canonical_summary_hash
                   FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2
                  FOR UPDATE",
                &[&tenant_id, &submission_id],
            )
            .await?;
        if let Some(submission) = submission.as_ref() {
            withdraw_pipeline_content_on_tx(
                &tx,
                tenant_id,
                &[(submission_id, submission)],
                actor_principal_ref,
                reason_code,
            )
            .await?;
        }
        Self::end_runs_of_inoperable_submission_on_tx(&tx, tenant_id, &runs, reason_code).await?;
        let run_ids = runs.iter().map(|run| run.run_id).collect::<Vec<_>>();
        let invalidation_pending: bool = tx
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM pipeline_index_invalidations
                     WHERE tenant_id = $1 AND run_id = ANY($2) AND state = 'pending'
                 )",
                &[&tenant_id, &run_ids],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(invalidation_pending)
    }

    /// The pipeline's half of `main`'s retention maintenance for
    /// `submission_id` (ruling RB-30; Zaki review 1, round 2, finding 18),
    /// in one tenant transaction, run before `main` marks the row
    /// (`mirror_expiration_to_db` / `mirror_purge_to_db`), so a pass that
    /// fails after it marks nothing and the next pass repeats it. For a
    /// submission with a pipeline run, it invalidates the pipeline export
    /// snapshots and items that hold it, under the action's reason; for a
    /// purge, it invalidates the submission's object refs and queues one
    /// payload deletion per live one (`delete_object_payload`, done by
    /// `main`'s revocation-propagation worker, as for a pipeline
    /// withdrawal); and it ends its runs' work
    /// (`end_runs_of_inoperable_submission_on_tx`: a written or partly
    /// written index revision is queued for invalidation, a run parked for
    /// review is released). An expiry deletes no payload, as `main`'s
    /// expiry deletes none. `main`'s own rows (derived records, vector
    /// entries, export manifests) are `main`'s mirror functions' to mark.
    /// Every write is idempotent. Lock order as the withdrawal's: the run
    /// rows, then the submission row. Returns whether an index invalidation
    /// of the submission's runs is pending afterwards; `false`, with nothing
    /// written, for a submission with no pipeline run.
    pub async fn follow_up_retention(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        action: PipelineRetentionAction,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let runs = tx
            .query(
                "SELECT * FROM pipeline_runs
                  WHERE tenant_id = $1 AND submission_id = $2
                  ORDER BY run_id
                  FOR UPDATE",
                &[&tenant_id, &submission_id],
            )
            .await?
            .iter()
            .map(pipeline_run_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        if runs.is_empty() {
            tx.commit().await?;
            return Ok(false);
        }
        let submission = tx
            .query_opt(
                "SELECT trace_id FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2
                  FOR UPDATE",
                &[&tenant_id, &submission_id],
            )
            .await?;
        if let Some(submission) = submission.as_ref() {
            invalidate_pipeline_exports_on_tx(
                &tx,
                tenant_id,
                &[submission_id],
                action.export_invalidation_reason(),
            )
            .await?;
            if action == PipelineRetentionAction::Purged {
                queue_pipeline_payload_deletions_on_tx(
                    &tx,
                    tenant_id,
                    &BTreeMap::from([(submission_id, submission.get::<_, Uuid>("trace_id"))]),
                    PipelinePayloadDeletion::RetentionPurge,
                )
                .await?;
            }
        }
        clear_gate_decision_dedup_on_tx(&tx, tenant_id, &[submission_id]).await?;
        Self::end_runs_of_inoperable_submission_on_tx(&tx, tenant_id, &runs, action.reason_code())
            .await?;
        let run_ids = runs.iter().map(|run| run.run_id).collect::<Vec<_>>();
        let invalidation_pending: bool = tx
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM pipeline_index_invalidations
                     WHERE tenant_id = $1 AND run_id = ANY($2) AND state = 'pending'
                 )",
                &[&tenant_id, &run_ids],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(invalidation_pending)
    }

    /// The submissions of `tenant_id` that have a pipeline run: the ones
    /// `main`'s file-driven side paths never see, since the pipeline writes
    /// them to the database only.
    pub async fn pipeline_submission_ids(
        &self,
        tenant_id: &str,
    ) -> Result<std::collections::BTreeSet<Uuid>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT DISTINCT submission_id FROM pipeline_runs WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(|row| row.get::<_, Uuid>(0)).collect())
    }

    /// Whether `submission_id` of `tenant_id` has a pipeline run. `main`'s
    /// gate evaluate route refuses such a submission, so the gate path never
    /// credits a trace the pipeline credits (Zaki review 1, minor item M-f).
    pub async fn submission_has_pipeline_run(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let has_run: bool = tx
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM pipeline_runs
                     WHERE tenant_id = $1 AND submission_id = $2
                 )",
                &[&tenant_id, &submission_id],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(has_run)
    }

    /// Whether a withdrawal of `submission_id` by `account_id` reaches a
    /// pipeline run: the submission itself has one, or a submission mapped
    /// to the same source session of that account does. `main`'s account
    /// withdrawal route uses the pipeline withdrawal exactly when this is
    /// true.
    pub async fn withdrawal_reaches_a_pipeline_run(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        submission_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let reaches: bool = tx
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM pipeline_runs run
                     WHERE run.tenant_id = $1
                       AND (run.submission_id = $3 OR run.submission_id IN (
                            SELECT sibling.submission_id
                              FROM trace_submission_sessions mapping
                              JOIN trace_submission_sessions sibling
                                ON sibling.tenant_id = mapping.tenant_id
                               AND sibling.account_id = mapping.account_id
                               AND sibling.session_digest = mapping.session_digest
                             WHERE mapping.tenant_id = $1 AND mapping.account_id = $2
                               AND mapping.submission_id = $3))
                 )",
                &[&tenant_id, &account_id, &submission_id],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(reaches)
    }

    /// Commits the Settle outcome and completes the run (port 2287 to
    /// 2328), once every settlement operation this task handles (there are
    /// none yet -- Task 13 adds the instrument legs) has reached a terminal
    /// state. `index_membership` was already durable from
    /// `persist_settle_selection`; this call sets it again on the same row
    /// as part of the same transition guard the other `commit_*` methods
    /// use, together with the outcome row and the terminal state change.
    ///
    /// After its lease check the commit locks the Settle policy's status row
    /// `FOR SHARE` (GRD-004) and refuses with
    /// `PIPELINE_POLICY_NOT_RUNNABLE_LABEL`, writing nothing, when the policy
    /// is no longer runnable. The row is held for this short transaction
    /// only, not over the Settle phase's index write or settlement legs, which
    /// run before it: a suspension waits for a commit that is already in
    /// flight and takes effect on the next one.
    pub async fn commit_settle(
        &self,
        run: &PipelineRunRecord,
        outcome: StoredPhaseResult,
        index_membership: &str,
        duplicate_verdict: Option<&str>,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if outcome.phase != Phase::Settle || run.next_phase != Some(Phase::Settle) {
            return Err(DatabaseError::Constraint(
                "phase does not match run transition".to_string(),
            ));
        }
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        // Lock order: the run row, then the policy row.
        ensure_current_lease(&tx, run, lease_token).await?;
        // GRD-004: the Settle policy must still be runnable, and stays so
        // until this commit ends. A refusal writes nothing; the legs the
        // attempt completed stay complete, and the run's next attempt, after
        // the policy resumes, commits from them.
        if !lock_runnable_policy(&tx, &run.tenant_id, &run.bundle_id, Phase::Settle).await? {
            return Err(DatabaseError::Constraint(
                PIPELINE_POLICY_NOT_RUNNABLE_LABEL.to_string(),
            ));
        }
        insert_outcome(
            &tx,
            &run.tenant_id,
            run.run_id,
            run.trace_id,
            &run.bundle_id,
            Uuid::new_v4(),
            outcome,
        )
        .await?;
        // Spec 2026-10-08, Slice C: the run's gate decision row, on this
        // transaction, after the lease and policy checks, so a refused
        // commit writes none.
        Self::write_pipeline_gate_decision_on_tx(&tx, run, duplicate_verdict).await?;
        let row = tx
            .query_one(
                "UPDATE pipeline_runs
                 SET next_phase = 'none', state = 'complete',
                     index_membership = $3,
                     lease_token = NULL, lease_expires_at = NULL,
                     next_attempt_at = NOW(), phase_started_at = NOW(), updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2
                   AND lease_token = $4 AND lease_expires_at > NOW()
                 RETURNING *",
                &[&run.tenant_id, &run.run_id, &index_membership, &lease_token],
            )
            .await?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Writes the run's `trace_gate_decisions` row on the caller's Settle
    /// transaction (spec 2026-10-08, Slice C), so `main`'s consumers --
    /// duplicate clustering, the contributor cap, per-author scoring,
    /// account trust -- see pipeline traffic with no change to their
    /// readers. Every run whose bound bundle's Score runs under the Score
    /// runtime controls writes a row: the compatibility Score policy and
    /// every Score policy a registry supplies
    /// (`score_runs_under_runtime_controls`). A registered policy's evidence
    /// must carry the fields the row needs, as the compatibility evidence
    /// does; one that does not fails the run closed at Settle, before any
    /// leg pays. Only a minimal-reference run writes none: its evidence
    /// carries none of the legacy fields (C-D1). Every column comes from the committed Score
    /// outcome, the bound package's Score configuration hash and the run's
    /// Trace Credit leg (C-D3); the legacy-only columns stay NULL, except
    /// the four dedup columns, which `commit_settle_from_progress` stamps
    /// after the commit (`stamp_settled_gate_decision_dedup`, owner decision
    /// O4).
    ///
    /// The decision id is a UUIDv5 over the tenant and the run (C-D2), and
    /// offering this run's row twice is a no-op. Returns whether a row was
    /// written: `false` for a minimal-reference bundle, for this run's row
    /// already existing, and for the zero outcome the server committed for
    /// a rejected Score hold (`ScoreEvidence::operator_rejected_hold`),
    /// which scored nothing the row could record. A pipeline row for the same submission that
    /// names another run refuses with `PIPELINE_GATE_DECISION_CONFLICT_LABEL`.
    ///
    /// Fails closed, writing nothing, when the run has no Score outcome or
    /// its compatibility evidence lacks a field the row needs
    /// (`PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL`): such a row
    /// would tell every consumer something the Score did not say. The
    /// Settle phase runs the same evidence check before any leg settles
    /// (`PipelineGateDecisionScoreValues::from_evidence`), so this refusal
    /// is reached only if the outcome changed between the two.
    ///
    /// The duplicate verdict ([`PipelineDuplicateControls`]) is decided once
    /// per run, when its Settle selection is persisted
    /// (`PgPipelineStore::duplicate_verdict_for_selection`), and never
    /// re-decided here. That verdict kept a duplicate out of the index and
    /// withheld its Trace Credit leg before the leg could pay; the row reads
    /// the leg, so a paid leg means no duplicate label and a withheld one
    /// its label. A run with no leg (a zero delta) passes the selection's
    /// verdict as `duplicate_verdict`. A duplicate label is recorded as
    /// `credit_withheld_reason` with no credit quality, the shape of
    /// `main`'s `skipped_duplicate` and `cached` rows, which the contributor
    /// status reads as a duplicate with no pending credit. Every other
    /// column still comes from the Score.
    pub async fn write_pipeline_gate_decision_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        duplicate_verdict: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        let incomplete = || {
            DatabaseError::Constraint(PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL.to_string())
        };
        let score_policy = tx
            .query_opt(
                "SELECT package #> '{manifest,score}' AS score_policy
                   FROM pipeline_bundle_packages
                  WHERE tenant_id = $1 AND bundle_id = $2",
                &[&run.tenant_id, &run.bundle_id],
            )
            .await?
            .ok_or_else(incomplete)?;
        let score_policy = serde_json::from_value::<PolicyRef>(score_policy.get("score_policy"))
            .map_err(|_| incomplete())?;
        if !crate::versioned_pipeline_bundle::score_runs_under_runtime_controls(
            &score_policy.implementation_id,
        ) {
            return Ok(false);
        }
        let outcome = tx
            .query_opt(
                "SELECT decision, evidence, evaluation FROM phase_outcomes
                  WHERE tenant_id = $1 AND run_id = $2 AND phase = $3",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &phase_as_db(Some(Phase::Score)),
                ],
            )
            .await?
            .ok_or_else(incomplete)?;
        let decision: serde_json::Value = outcome.get("decision");
        let evidence_value: serde_json::Value = outcome.get("evidence");
        let evaluation: serde_json::Value = outcome.get("evaluation");
        let evidence = serde_json::from_value::<ScoreEvidence>(evidence_value.clone())
            .map_err(|_| incomplete())?;
        // The server's zero outcome for a rejected Score hold: nothing was
        // scored, so no row is written.
        if evidence.operator_rejected_hold.is_some() {
            return Ok(false);
        }
        let values =
            PipelineGateDecisionScoreValues::from_evidence(&evidence).map_err(|_| incomplete())?;
        let attestation_chain_hash =
            pipeline_score_outcome_hash(&decision, &evidence_value, &evaluation)
                .map_err(|_| incomplete())?;
        // The Trace Credit leg's label when it was withheld -- by `main`'s
        // duplicate short-circuits or one of its NoveltyUtility checks:
        // `complete` with no credit event, the shape only the withheld
        // branches write (Ruling F-I1, as `commit_settle_from_progress`
        // reads it). With no leg, the verdict the caller decided.
        let leg = tx
            .query_opt(
                "SELECT operation_state, credit_event_id, last_error_label
                   FROM pipeline_run_settlements
                  WHERE tenant_id = $1 AND run_id = $2 AND instrument_id = $3",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &InstrumentId::trace_credit().as_str(),
                ],
            )
            .await?;
        let credit_withheld_reason: Option<String> = match leg {
            Some(leg) => {
                let withheld = leg.get::<_, String>("operation_state") == "complete"
                    && leg.get::<_, Option<Uuid>>("credit_event_id").is_none();
                if withheld {
                    leg.get("last_error_label")
                } else {
                    None
                }
            }
            None => duplicate_verdict.map(str::to_string),
        };
        let duplicate = credit_withheld_reason
            .as_deref()
            .is_some_and(is_duplicate_withheld_label);
        let (credit_quality_micros, credit_quality_version) = if duplicate {
            (None, None)
        } else {
            (values.credit_quality_micros, values.credit_quality_version)
        };
        let decision_id = pipeline_gate_decision_id(&run.tenant_id, run.run_id);
        let gate_policy_version = format!("pipeline:{}", run.bundle_id);
        let written = tx
            .execute(
                "INSERT INTO trace_gate_decisions (
                     tenant_id, decision_id, submission_id, gate_policy_version,
                     gate_version_hash, perplexity_micros, tail_fraction_micros,
                     perplexity_passed, novelty_score_micros, nearest_neighbor_hash,
                     novelty_passed, embedding_evidence_hash, attestation_chain_hash,
                     decided_at, credit_withheld_reason,
                     peak_perplexity_micros, peak_novelty_micros, chunk_count,
                     chunks_capped, total_chunk_count,
                     credit_quality_micros, credit_quality_calibration_version,
                     index_cardinality_at_scoring, source, pipeline_run_id
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,NOW(),$14,
                           $15,$16,$17,$18,$19,$20,$21,$22,'pipeline_settle',$23)
                 ON CONFLICT DO NOTHING",
                &[
                    &run.tenant_id,
                    &decision_id,
                    &run.submission_id,
                    &gate_policy_version,
                    &score_policy.configuration_hash,
                    &values.perplexity_micros,
                    &values.tail_fraction_micros,
                    &values.perplexity_passed,
                    &values.novelty_score_micros,
                    &values.nearest_neighbor_hash,
                    &values.novelty_passed,
                    &values.embedding_evidence_hash,
                    &attestation_chain_hash,
                    &credit_withheld_reason,
                    &values.peak_perplexity_micros,
                    &values.peak_novelty_micros,
                    &values.chunk_count,
                    &values.chunks_capped,
                    &values.total_chunk_count,
                    &credit_quality_micros,
                    &credit_quality_version,
                    &values.index_cardinality,
                    &run.run_id,
                ],
            )
            .await?;
        if written == 1 {
            return Ok(true);
        }
        // The targetless conflict clause covers both the primary key and
        // V116's `trace_gate_decisions_one_pipeline_row`. Nothing was
        // written, so the submission's pipeline row must already be this
        // run's (a retried commit); a row naming another run means a second
        // run reached Settle for the same submission, which
        // `insert_pipeline_receipt` is meant to prevent. That refuses here,
        // classified, rather than leaving the other run's row standing as
        // this run's verdict.
        let owner: Option<Uuid> = tx
            .query_opt(
                "SELECT pipeline_run_id FROM trace_gate_decisions
                  WHERE tenant_id = $1 AND submission_id = $2
                    AND source = 'pipeline_settle'",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?
            .and_then(|row| row.get("pipeline_run_id"));
        if owner == Some(run.run_id) {
            Ok(false)
        } else {
            Err(DatabaseError::Constraint(
                PIPELINE_GATE_DECISION_CONFLICT_LABEL.to_string(),
            ))
        }
    }

    /// The run's duplicate verdict, decided once, when its Settle
    /// selection is persisted: `duplicate_withheld_reason_on_tx` in a
    /// transaction of its own. `None` when `controls` is `None`.
    pub async fn duplicate_verdict_for_selection(
        &self,
        run: &PipelineRunRecord,
        controls: Option<&PipelineDuplicateControls>,
    ) -> Result<Option<&'static str>, DatabaseError> {
        if controls.is_none() {
            return Ok(None);
        }
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let verdict = Self::duplicate_withheld_reason_on_tx(&tx, run, controls).await?;
        tx.commit().await?;
        Ok(verdict)
    }

    /// `main`'s duplicate short-circuits ([`PipelineDuplicateControls`]) for
    /// the run's submission, on the caller's transaction, in `main`'s
    /// order: `skipped_duplicate`, then `cached`. `None` when `controls` is
    /// `None` or neither applies.
    ///
    /// The inputs are the ones `main`'s driver reads, recorded by the
    /// pipeline's Review commit on its current derived record (the newest):
    /// the submission's canonical summary and its hash
    /// (`canonical_summary_for_embedding` of the approved envelope). The
    /// submission row's own `canonical_summary_hash` stays NULL: withdrawal
    /// tombstones and `main`'s cache lookup read it. An earlier record with
    /// the same hash scores 1.0, a skip at any threshold the knob allows, so
    /// that is checked first and summaries are read only on a miss.
    /// `main` stores the duplicate score at submit, computed against the
    /// derived records that existed then; here it is computed against the
    /// derived records of the tenant's submissions received before this
    /// one (`trace_summary_duplicate_score`, `main`'s precheck score), and
    /// the cache looks only at gate decisions of submissions received
    /// before this one, so two runs settling in either order cannot both
    /// withhold each other. A submission whose Review recorded no summary
    /// (a run that passed Review before this was added) has no duplicate
    /// score, as a `main` submission with no derived record has none.
    pub async fn duplicate_withheld_reason_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        controls: Option<&PipelineDuplicateControls>,
    ) -> Result<Option<&'static str>, DatabaseError> {
        let Some(controls) = controls else {
            return Ok(None);
        };
        let Some(target) = tx
            .query_opt(
                "SELECT s.received_at, d.canonical_summary, d.canonical_summary_hash
                   FROM trace_submissions s
                   LEFT JOIN LATERAL (
                        SELECT canonical_summary, canonical_summary_hash
                          FROM trace_derived_records
                         WHERE tenant_id = s.tenant_id AND submission_id = s.submission_id
                           AND status = 'current'
                           AND canonical_summary IS NOT NULL
                           AND canonical_summary_hash IS NOT NULL
                         ORDER BY created_at DESC, derived_id DESC
                         LIMIT 1
                   ) d ON TRUE
                  WHERE s.tenant_id = $1 AND s.submission_id = $2",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?
        else {
            return Ok(None);
        };
        let received_at: DateTime<Utc> = target.get("received_at");
        if controls.skip_duplicates {
            let summary: Option<String> = target.get("canonical_summary");
            let summary_hash: Option<String> = target.get("canonical_summary_hash");
            if let (Some(summary), Some(summary_hash)) = (summary, summary_hash) {
                // An earlier record with the same hash scores 1.0, at or
                // above every threshold the knob allows (at most 1000000),
                // so it is a skip without reading any summary.
                let hash_match: bool = tx
                    .query_one(
                        "SELECT EXISTS (
                             SELECT 1 FROM trace_derived_records d
                               JOIN trace_submissions s
                                 ON s.tenant_id = d.tenant_id
                                AND s.submission_id = d.submission_id
                              WHERE d.tenant_id = $1 AND d.submission_id <> $2
                                AND (s.received_at, s.submission_id) < ($3, $2)
                                AND d.canonical_summary_hash = $4)",
                        &[
                            &run.tenant_id,
                            &run.submission_id,
                            &received_at,
                            &summary_hash,
                        ],
                    )
                    .await?
                    .get(0);
                if hash_match {
                    return Ok(Some(PIPELINE_SKIPPED_DUPLICATE_LABEL));
                }
                let candidates = tx
                    .query(
                        "SELECT d.canonical_summary, d.canonical_summary_hash
                           FROM trace_derived_records d
                           JOIN trace_submissions s
                             ON s.tenant_id = d.tenant_id AND s.submission_id = d.submission_id
                          WHERE d.tenant_id = $1 AND d.submission_id <> $2
                            AND (s.received_at, s.submission_id) < ($3, $2)
                            AND (d.canonical_summary IS NOT NULL
                                 OR d.canonical_summary_hash IS NOT NULL)",
                        &[&run.tenant_id, &run.submission_id, &received_at],
                    )
                    .await?;
                let candidates = candidates
                    .iter()
                    .map(|row| {
                        (
                            row.get::<_, Option<String>>(0),
                            row.get::<_, Option<String>>(1),
                        )
                    })
                    .collect::<Vec<_>>();
                let duplicate_score =
                    crate::trace_summary_similarity::trace_summary_duplicate_score(
                        &summary,
                        &summary_hash,
                        candidates
                            .iter()
                            .map(|(summary, hash)| (summary.as_deref(), hash.as_deref())),
                    );
                // `main`'s conversion, from the same `f32`.
                let duplicate_micros = (duplicate_score * 1_000_000.0) as i64;
                if duplicate_micros >= controls.skip_duplicate_threshold_micros {
                    return Ok(Some(PIPELINE_SKIPPED_DUPLICATE_LABEL));
                }
            }
        }
        let summary_hash: Option<String> = target.get("canonical_summary_hash");
        if let Some(summary_hash) = summary_hash {
            // A legacy submission carries its hash on the submission row and
            // its precheck derived record; a pipeline submission on its
            // Review derived record only.
            let cached: bool = tx
                .query_one(
                    "SELECT EXISTS (
                         SELECT 1 FROM trace_gate_decisions g
                           JOIN trace_submissions o
                             ON o.tenant_id = g.tenant_id AND o.submission_id = g.submission_id
                          WHERE g.tenant_id = $1 AND g.submission_id <> $3
                            AND (o.received_at, o.submission_id) < ($4, $3)
                            AND (o.canonical_summary_hash = $2
                                 OR EXISTS (
                                     SELECT 1 FROM trace_derived_records od
                                      WHERE od.tenant_id = o.tenant_id
                                        AND od.submission_id = o.submission_id
                                        AND od.canonical_summary_hash = $2)))",
                    &[
                        &run.tenant_id,
                        &summary_hash,
                        &run.submission_id,
                        &received_at,
                    ],
                )
                .await?
                .get(0);
            if cached {
                return Ok(Some(PIPELINE_CACHED_DUPLICATE_LABEL));
            }
        }
        Ok(None)
    }

    /// The unapplied runs a Score of `run_id` (one that runs under the runtime
    /// controls) counted as
    /// neighbours (#1313 case (a), V128 `pipeline_score_bases`), as
    /// `(basis run id, basis revision id)` by basis run id. Only a Score
    /// that passed quality and failed novelty records them.
    pub async fn list_score_bases(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Vec<(Uuid, Uuid)>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT basis_run_id, basis_revision_id FROM pipeline_score_bases
                  WHERE tenant_id = $1 AND run_id = $2
                  ORDER BY basis_run_id",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
    }

    /// An operator's decision on a run held in `awaiting_operator` (#1313).
    /// Returns the updated run, or `None` when the run is not held.
    ///
    /// `Fail` ends the run `failed` and applies N-7 in the same
    /// transaction, as every failure path does
    /// (`invalidate_index_writes_of_failed_runs_on_tx`). `Release` gives it
    /// one more Settle attempt, due now: its paid legs are terminal, so the
    /// attempt only commits. The release counts as the run's extra attempt
    /// (`settle_finish_granted_at`), so no automatic grant follows it; if the
    /// attempt fails in any way -- a refused commit, a charged retry, or a
    /// crash the claim sweep finds -- the run is held again (V128
    /// `operator_held_at`), never failed under N-7.
    /// Lock order: the run row, then the invalidation rows.
    pub async fn resolve_operator_held_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        resolution: PipelineOperatorHeldRunResolution,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let sql = match resolution {
            PipelineOperatorHeldRunResolution::Fail => {
                "UPDATE pipeline_runs
                    SET state = 'failed', last_error_label = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'awaiting_operator'
                  RETURNING *"
            }
            PipelineOperatorHeldRunResolution::Release => {
                "UPDATE pipeline_runs
                    SET state = 'retry',
                        attempt_count = GREATEST(max_attempts - 1, 0),
                        next_attempt_at = NOW(),
                        settle_finish_granted_at =
                            COALESCE(settle_finish_granted_at, clock_timestamp()),
                        last_error_label = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'awaiting_operator'
                  RETURNING *"
            }
        };
        let label = match resolution {
            PipelineOperatorHeldRunResolution::Fail => PIPELINE_OPERATOR_FAILED_HELD_RUN_LABEL,
            PipelineOperatorHeldRunResolution::Release => PIPELINE_OPERATOR_RELEASED_HELD_RUN_LABEL,
        };
        let Some(row) = tx.query_opt(sql, &[&tenant_id, &run_id, &label]).await? else {
            tx.commit().await?;
            return Ok(None);
        };
        let updated = pipeline_run_from_row(&row)?;
        if resolution == PipelineOperatorHeldRunResolution::Fail {
            resolve_open_settlement_legs_on_tx(&tx, tenant_id, &[run_id]).await?;
            invalidate_index_writes_of_failed_runs_on_tx(&tx, tenant_id, &[run_id]).await?;
            reject_received_submissions_of_failed_runs_on_tx(&tx, tenant_id, &[run_id]).await?;
        }
        tx.commit().await?;
        Ok(Some(updated))
    }

    /// Fails the run terminally under its live lease. When the
    /// run is in Settle, every open leg is resolved in this same
    /// transaction (`resolve_open_settlement_legs_on_tx`). A worker calls
    /// this through `PipelineService::fail_run`, which reconciles the
    /// dispatched external legs first, so here they are already resolved.
    /// A run whose submission is still `received` moves it to `rejected` in
    /// the same transaction (`reject_received_submissions_of_failed_runs_on_tx`,
    /// #1346).
    pub async fn mark_failed(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> Result<(), DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
             SET state = 'failed', lease_token = NULL, lease_expires_at = NULL,
                 last_error_label = $3, updated_at = NOW()
             WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
               AND lease_token = $4 AND lease_expires_at > NOW()
             RETURNING next_phase",
                &[&run.tenant_id, &run.run_id, &error_label, &lease_token],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        // #1313, the gap: a Settle run with a paid leg whose commit was
        // refused for good is held for an operator, not failed, so N-7 does
        // not remove the revision that was paid for. The automatic paths
        // keep N-7 for everything else.
        if phase_from_db(row.get("next_phase"))? == Some(Phase::Settle)
            && hold_paid_settle_run_for_operator_on_tx(&tx, run, error_label)
                .await?
                .is_some()
        {
            tx.commit().await?;
            tracing::warn!(
                label = PIPELINE_SETTLE_HELD_FOR_OPERATOR_LABEL,
                reason_label = error_label,
                tenant_storage_ref = pipeline_tenant_storage_ref(&run.tenant_id).as_str(),
                run_ref_hash = %sha256_prefixed(run.run_id.to_string().as_bytes()),
                "a paid pipeline run is held for an operator"
            );
            return Ok(());
        }
        if phase_from_db(row.get("next_phase"))? == Some(Phase::Settle) {
            resolve_open_settlement_legs_on_tx(&tx, &run.tenant_id, &[run.run_id]).await?;
            invalidate_index_writes_of_failed_runs_on_tx(&tx, &run.tenant_id, &[run.run_id])
                .await?;
        }
        reject_received_submissions_of_failed_runs_on_tx(&tx, &run.tenant_id, &[run.run_id])
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// A charged retry: the attempt the claim took stays charged. When the
    /// run has no attempt left, the run is failed as `attempts_exhausted`
    /// instead, and a Settle run's open legs are resolved
    /// in this same transaction (`resolve_open_settlement_legs_on_tx`). A
    /// worker calls this through `PipelineService::charged_retry`, which
    /// first reconciles the dispatched external legs of a run this retry
    /// will exhaust.
    ///
    /// The next attempt is due after a backoff that doubles from
    /// `DEFAULT_RETRY_MILLISECONDS`. Multi-lens review C5 (owner decision):
    /// under `artifact_integrity_failed` it is due one hour later
    /// (`suspension_max_delay_sql!`) instead. A store configuration fault
    /// (a root that is not mounted, a wrong key, a wrong bucket) reads as
    /// an integrity failure, and the fast backoff used the whole attempt
    /// budget in about one second; one hour apart, the budget covers hours
    /// in which an operator can correct the store, and the terminal bound
    /// stays.
    ///
    /// Under `privacy_classification_failed` (the Review-start privacy
    /// pass's classifier failed or timed out, decision D3) the delay is
    /// legacy's PII backstop backoff, `PIPELINE_PRIVACY_RETRY_BASE_SECONDS x
    /// 2^(attempt_count - 1)` (30, 60, 120, 240 s), so the attempt budget
    /// outlasts a classifier outage of minutes rather than a second; and a
    /// run that exhausts its attempts under it is failed with that label,
    /// not `attempts_exhausted`, so its status reads as held content
    /// (`quarantined`, Q1). A worker that crashes mid-pass is not recorded
    /// here: the `claim_next` sweep fails such a run as
    /// `attempts_exhausted`, a crash rather than a classifier verdict.
    ///
    /// #1346: a retry that fails the run moves its still-`received`
    /// submission to `rejected` in the same transaction
    /// (`reject_received_submissions_of_failed_runs_on_tx`); one failed
    /// under `privacy_classification_failed` keeps its submission as it is.
    pub async fn mark_retry(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let exponent = run.attempt_count.saturating_sub(1).min(9);
        let multiplier = 1_i64 << exponent;
        let delay_milliseconds = DEFAULT_RETRY_MILLISECONDS.saturating_mul(multiplier);
        let hourly = error_label == PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL
            || error_label == PIPELINE_INDEX_COMMAND_UNREADABLE_LABEL;
        let privacy_classification = error_label == PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL;
        let privacy_delay_seconds = PIPELINE_PRIVACY_RETRY_BASE_SECONDS.saturating_mul(multiplier);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        // V118: a run that ends `failed` under the privacy label sets the
        // review audit marker, so the worker's review audit pass appends the
        // failure's audit event. A charged retry sets none.
        let row = tx
            .query_opt(
                concat!(
                    "UPDATE pipeline_runs
                 SET state = CASE
                         WHEN attempt_count >= max_attempts THEN 'failed'
                         ELSE 'retry'
                     END,
                     lease_token = NULL,
                     lease_expires_at = NULL,
                     next_attempt_at = CASE
                         WHEN attempt_count >= max_attempts THEN next_attempt_at
                         WHEN $7::boolean THEN NOW() + ",
                    suspension_max_delay_sql!(),
                    "
                         WHEN $8::boolean THEN NOW() + ($9::bigint * INTERVAL '1 second')
                         ELSE NOW() + ($5::bigint * INTERVAL '1 millisecond')
                     END,
                     last_error_label = CASE
                         WHEN attempt_count >= max_attempts AND NOT $8::boolean THEN $4
                         ELSE $3
                     END,
                     review_audit_pending_at = CASE
                         WHEN attempt_count >= max_attempts AND $8::boolean
                         THEN clock_timestamp() ELSE review_audit_pending_at END,
                     updated_at = NOW()
                 WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                   AND lease_token = $6 AND lease_expires_at > NOW()
                 RETURNING *"
                ),
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &error_label,
                    &PIPELINE_ATTEMPTS_EXHAUSTED_LABEL,
                    &delay_milliseconds,
                    &lease_token,
                    &hourly,
                    &privacy_classification,
                    &privacy_delay_seconds,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        // #1313 (b): a retry that exhausts a Settle run that can finish
        // gives it its one extra attempt, due now, instead of failing it.
        if updated.state == PipelineRunState::Failed && updated.next_phase == Some(Phase::Settle) {
            if let Some(granted) = tx
                .query_opt(
                    concat!(
                        "UPDATE pipeline_runs p
                            SET state = 'retry',
                                attempt_count = GREATEST(p.max_attempts - 1, 0),
                                next_attempt_at = NOW(),
                                settle_finish_granted_at = clock_timestamp(),
                                last_error_label = $3, updated_at = NOW()
                          WHERE p.tenant_id = $1 AND p.run_id = $2 AND p.state = 'failed'
                            AND ",
                        finishable_settle_run_sql!(),
                        " RETURNING p.*"
                    ),
                    &[
                        &run.tenant_id,
                        &run.run_id,
                        &PIPELINE_SETTLE_FINISH_GRANTED_LABEL,
                    ],
                )
                .await?
            {
                let granted = pipeline_run_from_row(&granted)?;
                tx.commit().await?;
                return Ok(granted);
            }
            // #1413 review: a released run (held before) whose attempt is
            // exhausted is held again, not failed: the operator chose to
            // keep its paid revision.
            if let Some(held) = hold_paid_settle_run_for_operator_on_tx(
                &tx,
                run,
                updated
                    .last_error_label
                    .as_deref()
                    .unwrap_or(PIPELINE_ATTEMPTS_EXHAUSTED_LABEL),
            )
            .await?
            {
                tx.commit().await?;
                tracing::warn!(
                    label = PIPELINE_SETTLE_HELD_FOR_OPERATOR_LABEL,
                    reason_label = PIPELINE_ATTEMPTS_EXHAUSTED_LABEL,
                    tenant_storage_ref = pipeline_tenant_storage_ref(&run.tenant_id).as_str(),
                    run_ref_hash = %sha256_prefixed(run.run_id.to_string().as_bytes()),
                    "a released pipeline run is held for an operator again"
                );
                return Ok(held);
            }
        }
        if updated.state == PipelineRunState::Failed && updated.next_phase == Some(Phase::Settle) {
            resolve_open_settlement_legs_on_tx(&tx, &run.tenant_id, &[run.run_id]).await?;
            invalidate_index_writes_of_failed_runs_on_tx(&tx, &run.tenant_id, &[run.run_id])
                .await?;
        }
        if updated.state == PipelineRunState::Failed {
            reject_received_submissions_of_failed_runs_on_tx(&tx, &run.tenant_id, &[run.run_id])
                .await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    /// The one suspension path (ruling FR3): a failure that is not the
    /// trace's fault -- a dependency outage, a missing or not-runnable bound
    /// dependency, a credit hold -- releases the claim and schedules a
    /// retry without charging the attempt the claim took. A quarantined run
    /// awaiting human review is parked instead (`mark_awaiting_review`),
    /// not retried here.
    ///
    /// The delay is `clamp(NOW() - phase_started_at, 1 second, 1 hour)`:
    /// the time the run has spent in its current phase. `phase_started_at`
    /// is set at receipt and reset by every phase commit, and nothing else
    /// moves it, so without a retry counter each retry lands when the phase
    /// is twice as old as at the previous one -- the delay doubles -- and it
    /// caps at one retry per hour. There is no terminal bound: a hold must
    /// stay retryable ("suspension leaves work retryable"), so the bound is
    /// on work per hour, not on attempts.
    pub async fn mark_transient_retry(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                concat!(
                    "UPDATE pipeline_runs
                    SET state = 'retry', lease_token = NULL, lease_expires_at = NULL,
                        attempt_count = GREATEST(attempt_count - 1, 0),
                        next_attempt_at = NOW() + LEAST(
                            GREATEST(NOW() - phase_started_at, INTERVAL '1 second'),
                            ",
                    suspension_max_delay_sql!(),
                    "
                        ),
                        last_error_label = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $4 AND lease_expires_at > NOW()
                  RETURNING *"
                ),
                &[&run.tenant_id, &run.run_id, &error_label, &lease_token],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Releases a compatibility Score that found its tenant's Score lock held
    /// by another Score (multi-lens review L3-2): uncharged, like
    /// `mark_transient_retry`, but due again after the fixed
    /// `PIPELINE_SCORE_LOCK_BUSY_RETRY_MILLISECONDS`, not the phase-age
    /// backoff, since the run waited for nothing of its own. Fenced by the
    /// lease like `mark_transient_retry`.
    ///
    /// The Score staged its attempt rows before it tried the lock (rebase
    /// 10, option D; `stage_score_artifacts_before_lock`), with no hash, and
    /// published nothing, so no object is stored at their keys. The same
    /// transaction deletes them, so a busy try, which repeats every
    /// `PIPELINE_SCORE_LOCK_BUSY_RETRY_MILLISECONDS` while another Score of
    /// the tenant runs, leaves no rows for the attempt sweep. A release the
    /// lease fence refuses deletes nothing; the sweep removes those rows
    /// after their `cleanup_after`.
    pub async fn release_score_lock_busy(
        &self,
        run: &PipelineRunRecord,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                    SET state = 'retry', lease_token = NULL, lease_expires_at = NULL,
                        attempt_count = GREATEST(attempt_count - 1, 0),
                        next_attempt_at = clock_timestamp()
                            + ($3::bigint * INTERVAL '1 millisecond'),
                        last_error_label = $4, updated_at = clock_timestamp()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $5 AND lease_expires_at > clock_timestamp()
                  RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &PIPELINE_SCORE_LOCK_BUSY_RETRY_MILLISECONDS,
                    &PIPELINE_SCORE_LOCK_BUSY_LABEL,
                    &lease_token,
                ],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.execute(
            "DELETE FROM pipeline_attempt_artifacts
              WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3
                AND state = 'staged' AND ciphertext_sha256 IS NULL",
            &[&run.tenant_id, &run.run_id, &lease_token],
        )
        .await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Parks a run whose Score policy asked for its trace to be held for
    /// operator review instead of priced (`ScoreOutput::hold_for_review`),
    /// on the caller's transaction: the run moves to the review hold state
    /// (`awaiting_review`) at Score, under the policy's safe label, with its
    /// lease cleared and the claim's attempt given back, as
    /// `mark_awaiting_review` parks a run at Review. Nothing of the Score is
    /// committed, so no award is seeded and no settlement leg exists; the
    /// rows a Score staged before its tenant lock (`staged`, no hash, under
    /// this lease token) are deleted, as `release_score_lock_busy` deletes
    /// them, since the held Score published nothing. Fenced by the lease
    /// against the database clock.
    ///
    /// The same guards as `commit_score_in`, in the same lock order (run
    /// row, submission row, policy row), refuse the hold and write nothing:
    /// an inoperable submission with `PIPELINE_SUBMISSION_INOPERABLE_LABEL`,
    /// so a held run whose submission was withdrawn and that an
    /// invalidation pass released ends as any Score of such a run ends,
    /// rather than parking again; and a Score policy that is no longer
    /// runnable with `PIPELINE_POLICY_NOT_RUNNABLE_LABEL` (GRD-004).
    pub async fn hold_score_for_review_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        label: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        if run.next_phase != Some(Phase::Score) {
            return Err(DatabaseError::Constraint(
                "phase does not match run transition".to_string(),
            ));
        }
        let lease_token = required_lease_token(run)?;
        ensure_current_lease(tx, run, lease_token).await?;
        if !Self::submission_guard_on_tx(tx, run).await?.operable {
            return Err(DatabaseError::Constraint(
                PIPELINE_SUBMISSION_INOPERABLE_LABEL.to_string(),
            ));
        }
        if !lock_runnable_policy(tx, &run.tenant_id, &run.bundle_id, Phase::Score).await? {
            return Err(DatabaseError::Constraint(
                PIPELINE_POLICY_NOT_RUNNABLE_LABEL.to_string(),
            ));
        }
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                    SET state = 'awaiting_review', lease_token = NULL, lease_expires_at = NULL,
                        attempt_count = GREATEST(attempt_count - 1, 0),
                        last_error_label = $3, updated_at = clock_timestamp()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND next_phase = 'score'
                    AND lease_token = $4 AND lease_expires_at > clock_timestamp()
                  RETURNING *",
                &[&run.tenant_id, &run.run_id, &label, &lease_token],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.execute(
            "DELETE FROM pipeline_attempt_artifacts
              WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3
                AND state = 'staged' AND ciphertext_sha256 IS NULL",
            &[&run.tenant_id, &run.run_id, &lease_token],
        )
        .await?;
        Ok(updated)
    }

    /// `hold_score_for_review_on_tx` in a tenant transaction of its own.
    pub async fn hold_score_for_review(
        &self,
        run: &PipelineRunRecord,
        label: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let updated = Self::hold_score_for_review_on_tx(&tx, run, label).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Parks a run Review quarantined with no
    /// human assessment yet (`PipelineRunState::AwaitingReview`, under the
    /// label `review_assessment_required`, or `privacy_pass_review_required`
    /// for a run the privacy pass escalated) instead of retrying it under
    /// `mark_transient_retry`'s hourly backoff forever. No claim query
    /// selects `awaiting_review`, so a parked run does no further work on
    /// its own; `record_review_assessment` moves it back to `pending`, due
    /// at once, once an assessment lands (Ruling T3-3). A parked run whose
    /// submission later stops being operable (withdrawn, revoked, expired,
    /// or purged) is released back to `pending` by whichever comes first:
    /// `claim_review` or `record_review_assessment`, which check the
    /// submission's operability themselves (Ruling T3-6); the pipeline
    /// withdrawal and the follow-up of `main`'s revocation; or the worker,
    /// which releases every such run of a tenant on its invalidation step
    /// (`release_inoperable_parked_runs`; Zaki review 1, round 2, finding 9),
    /// since after expiry or purge nobody else reaches it: the review queue
    /// hides it and no claim query selects it. Review then ends the run under
    /// `submission_inoperable` rather than leaving it parked forever.
    ///
    /// Fenced by the lease exactly like `mark_transient_retry`: the lease
    /// is cleared and the claim's attempt is given back
    /// (`attempt_count - 1`, floor 0), since parking is not a charged
    /// failure. Unlike `mark_transient_retry`, `next_attempt_at` is left
    /// alone -- nothing reads it while the run is parked, and the review
    /// route that unparks it will set its own value.
    pub async fn mark_awaiting_review(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> Result<PipelineRunRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                    SET state = 'awaiting_review', lease_token = NULL, lease_expires_at = NULL,
                        attempt_count = GREATEST(attempt_count - 1, 0),
                        last_error_label = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $4 AND lease_expires_at > NOW()
                  RETURNING *",
                &[&run.tenant_id, &run.run_id, &error_label, &lease_token],
            )
            .await?
            .ok_or_else(stale_lease_error)?;
        let updated = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Records that `run`'s own phase lease went stale before the phase
    /// finished, as its own uncharged reason (`PIPELINE_LEASE_EXPIRED_LABEL`)
    /// with the same FR3 backoff `mark_transient_retry` uses -- never a
    /// charged policy failure, even though the lease has already gone stale
    /// by the time this is called.
    ///
    /// Fenced by the lease TOKEN alone, with no expiry predicate: every
    /// other lease-checked write in this store also requires
    /// `lease_expires_at > NOW()`, which is exactly the condition this call
    /// exists to act past. `state = 'leased' AND lease_token = $token` still
    /// guards it -- once a phase transition, a terminal write, or another
    /// worker's claim has moved the row off this token, this changes
    /// nothing and reports that as `Ok(None)` (a token-only fence) rather
    /// than disturbing whatever holds the run now.
    pub async fn record_lease_expired(
        &self,
        run: &PipelineRunRecord,
    ) -> Result<Option<PipelineRunRecord>, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let row = tx
            .query_opt(
                "UPDATE pipeline_runs
                    SET state = 'retry', lease_token = NULL, lease_expires_at = NULL,
                        attempt_count = GREATEST(attempt_count - 1, 0),
                        next_attempt_at = NOW() + LEAST(
                            GREATEST(NOW() - phase_started_at, INTERVAL '1 second'),
                            INTERVAL '1 hour'
                        ),
                        last_error_label = $3, updated_at = NOW()
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $4
                  RETURNING *",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &PIPELINE_LEASE_EXPIRED_LABEL,
                    &lease_token,
                ],
            )
            .await?;
        let updated = row.as_ref().map(pipeline_run_from_row).transpose()?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn list_settlements(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Vec<PipelineSettlementRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT *, atomic_units::TEXT AS atomic_units_text
                   FROM pipeline_run_settlements
                  WHERE tenant_id = $1 AND run_id = $2
                  ORDER BY instrument_id",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(pipeline_settlement_from_row).collect()
    }

    /// The runs an index rebuild considers, from their own sealed commands
    /// (port `ef97a459` lines 618 to 655, adapted): every run that meets
    /// `rebuildable_index_run_predicate!` -- complete, included in the
    /// index, with a completed index write that no invalidation has touched
    /// since, and a stored command reference and hash to rebuild from.
    /// Ordered by `created_at, run_id` so a rebuild applies commands in the
    /// same order Settle originally committed them.
    ///
    /// A listing only. Whether the run's submission is still operable is not
    /// decided here: the rebuild reads `submission_guard_on_tx` for each run
    /// under that run's lock, immediately before it writes the run's
    /// entries (`lock_rebuildable_index_run_on_tx`), since a withdrawal can
    /// commit at any point after this read (final review I1). Reading the
    /// guard itself, rather than a copy of its predicate, keeps the two from
    /// drifting (M8).
    pub async fn list_rebuildable_index_runs(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<PipelineRunRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                concat!(
                    "SELECT p.* FROM pipeline_runs p WHERE p.tenant_id = $1 AND ",
                    rebuildable_index_run_predicate!(),
                    " ORDER BY p.created_at, p.run_id"
                ),
                &[&tenant_id],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(pipeline_run_from_row).collect()
    }

    /// Locks `run`'s row on the caller's transaction and re-checks, under
    /// that lock, everything an index rebuild needs before it writes the
    /// run's entries: the run still meets `rebuildable_index_run_predicate!`
    /// with the same `index_command_hash` the rebuild listed and loaded, and
    /// its submission is still operable (`submission_guard_on_tx`, which
    /// holds the submission row `FOR SHARE`). `false` when either no longer
    /// holds; the caller writes nothing for the run then.
    ///
    /// Lock order: the run row, then the submission row -- the order a
    /// withdrawal also takes (`withdraw_submission`) and Settle's index
    /// dispatch holds. Both locks last until the caller's transaction ends,
    /// so a withdrawal of the submission waits for the rebuild's writes to
    /// commit, and the invalidation it then queues covers them. The run row
    /// is locked `FOR SHARE`: two rebuilds of one run may overlap (their
    /// writes are idempotent), and a withdrawal's `FOR UPDATE` waits for
    /// both.
    ///
    /// Two statements, lock first, read second, as the submission guard
    /// does: the predicate is read with a snapshot taken after the run lock
    /// is held, so it sees an invalidation row that a holder of the run lock
    /// committed while the lock was awaited. One locking statement would
    /// not: the invalidation lives in its own table (PR 3, 62cc2768), and
    /// PostgreSQL re-evaluates a locked row's predicate only when the
    /// holder updated that row, which queuing an invalidation does not.
    pub async fn lock_rebuildable_index_run_on_tx(
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
    ) -> Result<bool, DatabaseError> {
        let Some(command_hash) = run.index_command_hash.as_deref() else {
            return Ok(false);
        };
        let locked = tx
            .query_opt(
                "SELECT 1 FROM pipeline_runs
                  WHERE tenant_id = $1 AND run_id = $2
                  FOR SHARE",
                &[&run.tenant_id, &run.run_id],
            )
            .await?;
        if locked.is_none() {
            return Ok(false);
        }
        let rebuildable = tx
            .query_opt(
                concat!(
                    "SELECT 1 FROM pipeline_runs p
                      WHERE p.tenant_id = $1 AND p.run_id = $2 AND p.index_command_hash = $3 AND ",
                    rebuildable_index_run_predicate!()
                ),
                &[&run.tenant_id, &run.run_id, &command_hash],
            )
            .await?;
        if rebuildable.is_none() {
            return Ok(false);
        }
        Ok(Self::submission_guard_on_tx(tx, run).await?.operable)
    }

    /// The payout pass's work list (P3-D11): up to `limit` complete runs of
    /// `tenant_id` whose completed `trace_credit` leg is on the `near` rail,
    /// carries its settlement batch, and has a payout still to make
    /// (`pending`) or to confirm (`submitted`), least recently updated
    /// first. A leg without a batch -- a compatibility run's
    /// `NoveltyUtility` event, which the pipeline never batches and `main`
    /// leaves to the pipeline -- is never listed
    /// (Ruling S8), nor is a `disabled`, `confirmed`, or `failed` payout, nor
    /// a `pending` one whose bundle's Settle policy is not runnable
    /// (`list_payout_work_on`).
    pub async fn list_runs_with_pending_payout(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> Result<Vec<Uuid>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        Ok(
            Self::list_payout_work_on(&mut client, tenant_id, limit, std::time::Duration::ZERO)
                .await?
                .into_iter()
                .map(|(run_id, _)| run_id)
                .collect(),
        )
    }

    /// The payout pass's work list, with each leg's payout state:
    /// `list_runs_with_pending_payout`, except that a `submitted` leg is
    /// listed only once `confirmation_interval` has passed since its
    /// `updated_at`. Every payout write sets `updated_at`, confirmation polls
    /// included, so a `submitted` leg is polled at most once per interval
    /// (Ruling T10-10). A `pending` leg is listed at once, unless it carries a
    /// label -- an account hold (`none_enrolled`, `ambiguous_no_designation`)
    /// -- which waits out the same interval before the account is resolved
    /// again (Zaki review 1, round 2, item 2).
    ///
    /// A leg that would only be dispatched -- a `pending` one -- is listed
    /// only while the Settle policy of its run's bundle is runnable (GRD-004;
    /// no status row is not runnable). A dispatch that the policy holds back
    /// changes nothing, so the leg keeps its place in the order, and listed
    /// anyway the held legs would fill the list: with `limit` of them the
    /// pass would list only those and process nothing, and the legs of other
    /// bundles, and every confirmation behind them, would never be reached.
    /// A `submitted` leg is listed whatever the policy says, since a
    /// confirmation is not a dispatch. The dispatch guard
    /// (`settle_policy_allows_dispatch`) stays the authority: a suspension
    /// that lands after the listing is still held back there.
    pub(super) async fn list_payout_work_on(
        client: &mut deadpool_postgres::Client,
        tenant_id: &str,
        limit: usize,
        confirmation_interval: std::time::Duration,
    ) -> Result<Vec<(Uuid, String)>, DatabaseError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let confirmation_interval_seconds = confirmation_interval.as_secs_f64();
        let tx = Self::tenant_transaction(client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT s.run_id, s.payout_state
                   FROM pipeline_run_settlements s
                  WHERE s.tenant_id = $1
                    AND s.instrument_id = $2
                    AND s.operation_state = 'complete'
                    AND s.payout_rail = 'near'
                    AND s.payout_eligible
                    AND s.settlement_batch_id IS NOT NULL
                    AND (
                        (
                            s.payout_state = 'pending'
                            AND (
                                s.last_error_label IS NULL
                                OR s.updated_at <= NOW() - make_interval(secs => $3)
                            )
                            AND EXISTS (
                                SELECT 1
                                  FROM pipeline_runs r
                                  JOIN pipeline_bundle_policy_status ps
                                    ON ps.tenant_id = r.tenant_id
                                   AND ps.bundle_id = r.bundle_id
                                   AND ps.phase = 'settle'
                                 WHERE r.tenant_id = s.tenant_id
                                   AND r.run_id = s.run_id
                                   AND ps.runnable
                            )
                        )
                        OR (
                            s.payout_state = 'submitted'
                            AND s.updated_at <= NOW() - make_interval(secs => $3)
                        )
                    )
                  ORDER BY s.updated_at, s.run_id
                  LIMIT $4",
                &[
                    &tenant_id,
                    &InstrumentId::trace_credit().as_str(),
                    &confirmation_interval_seconds,
                    &limit,
                ],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .iter()
            .map(|row| (row.get("run_id"), row.get("payout_state")))
            .collect())
    }

    /// Advances one instrument leg (port 2160 to 2211). Gated on the run's
    /// own lease, same as every other commit in this phase -- settlement
    /// rows have no lease of their own that this call checks.
    ///
    /// `result_ref_hash` only ever moves from `NULL` to a value on the
    /// transition to `'complete'`; the schema's own
    /// `pipeline_run_settlements_result_shape` check already forbids a
    /// non-`NULL` result on any other state, but `'forfeited'` and
    /// `'retry'` write it as `NULL` outright rather than `COALESCE`ing
    /// through whatever `update.result_ref_hash` carries, so a caller can
    /// never accidentally persist a result alongside a non-`'complete'`
    /// state. `external_receipt_hash` is written only on the transition to
    /// `'complete'` and is `NULL` for every other state
    /// (`pipeline_run_settlements_external_receipt_shape`); a hash another
    /// leg of the tenant already recorded is refused by
    /// `pipeline_run_settlements_external_receipt_unique`.
    /// `credit_event_id`/`settlement_batch_id` only move from `NULL`
    /// to a value (never overwritten or cleared); `payout_state` is written
    /// only when the caller supplies one. `attempt_count` increments on
    /// `'retry'`/`'failed'` (not `'forfeited'` -- forfeiture is not a
    /// dispatch failure) and saturates rather than overflow: it is a
    /// diagnostic count, and an adapter outage is an uncharged run retry
    /// with no terminal bound, so it must never refuse the
    /// update. Every update here moves the leg out of `leased` (none sets
    /// it; `mark_settlement_leased` does), so it clears the lease columns
    /// (`pipeline_run_settlements_lease_shape`); `dispatched_at` is never
    /// touched.
    pub(super) async fn update_settlement(
        &self,
        run: &PipelineRunRecord,
        instrument_id: &str,
        update: SettlementUpdate<'_>,
    ) -> Result<PipelineSettlementRecord, DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        let settlement =
            update_settlement_on_tx(&tx, run, lease_token, instrument_id, update).await?;
        tx.commit().await?;
        Ok(settlement)
    }

    /// Moves one leg to `leased` under the run's own lease token
    /// and expiry immediately before its adapter call, and records the first
    /// dispatch (`dispatched_at`, set once and never cleared). It accepts
    /// exactly the legs Step 6 dispatches (`SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL`),
    /// so a `leased` leg an earlier, crashed attempt left behind is leased
    /// again under this attempt's lease and dispatched again (the adapter is
    /// idempotent by `operation_ref_hash`), and a leg that is never
    /// dispatched again is refused.
    ///
    /// Fenced by the run lease: `ensure_current_lease` locks the run row and
    /// returns the stale-lease error only for a lease that is really stale.
    /// A leg closed to dispatch (or no leg) under a live lease is
    /// `settlement_leg_not_open_error`, never the stale-lease error.
    pub(super) async fn mark_settlement_leased(
        &self,
        run: &PipelineRunRecord,
        instrument_id: &str,
    ) -> Result<(), DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
        let updated = tx
            .execute(
                &format!(
                    "UPDATE pipeline_run_settlements s
                        SET operation_state = 'leased',
                            lease_token = p.lease_token,
                            lease_expires_at = p.lease_expires_at,
                            dispatched_at = COALESCE(s.dispatched_at, NOW()),
                            last_error_label = NULL,
                            updated_at = NOW()
                       FROM pipeline_runs p
                      WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.instrument_id = $3
                        AND p.tenant_id = s.tenant_id AND p.run_id = s.run_id
                        AND p.lease_token = $4
                        AND {SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL}"
                ),
                &[&run.tenant_id, &run.run_id, &instrument_id, &lease_token],
            )
            .await?;
        if updated != 1 {
            return Err(settlement_leg_not_open_error());
        }
        tx.commit().await?;
        Ok(())
    }

    /// Records the reconciling adapter call a worker makes for
    /// an open, dispatched, external leg before it fails the run
    /// (`PipelineService::reconcile_dispatched_settlement_legs`).
    /// `Some(receipt)` -- a receipt that answers the leg's request --
    /// completes the leg with its result reference and its external receipt
    /// hash; `None` records `failed` / `settlement_unreconciled`. Fenced like
    /// `mark_settlement_leased`, with the same distinct refusal for any other
    /// leg, including a leg that is never dispatched again.
    pub(super) async fn record_settlement_reconciliation(
        &self,
        run: &PipelineRunRecord,
        instrument_id: &str,
        receipt: Option<&SettlementReceipt>,
    ) -> Result<(), DatabaseError> {
        let (operation_state, error_label) = match receipt {
            Some(_) => ("complete", None),
            None => ("failed", Some(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL)),
        };
        let result_ref_hash = receipt.map(SettlementReceipt::result_ref_hash);
        let external_receipt_hash = receipt.and_then(SettlementReceipt::external_receipt_hash);
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
        let updated = tx
            .execute(
                &format!(
                    "UPDATE pipeline_run_settlements s
                        SET operation_state = $4,
                            result_ref_hash = $5,
                            external_receipt_hash = $8,
                            last_error_label = $6,
                            lease_token = NULL,
                            lease_expires_at = NULL,
                            updated_at = NOW()
                      WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.instrument_id = $3
                        AND s.instrument_id <> $7
                        AND s.dispatched_at IS NOT NULL
                        AND {UNRESOLVED_SETTLEMENT_LEG_SQL}
                        AND {SETTLEMENT_LEG_OPEN_TO_DISPATCH_SQL}"
                ),
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &instrument_id,
                    &operation_state,
                    &result_ref_hash,
                    &error_label,
                    &InstrumentId::trace_credit().as_str(),
                    &external_receipt_hash,
                ],
            )
            .await?;
        if updated != 1 {
            return Err(settlement_leg_not_open_error());
        }
        tx.commit().await?;
        Ok(())
    }

    /// Inserts a receipt attempt's `staged` row inside the receipt's staging
    /// transaction, which commits before the attempt writes its object. The
    /// row names that object -- its key and ciphertext hash, fixed when the
    /// object was prepared -- so whatever happens after the write (an
    /// Admission error, a failed records insert, a process crash), a
    /// `staged` row names the object, and `PipelineService::sweep_staged_receipts`
    /// deletes both once `cleanup_after` passes. The receipt's final
    /// transaction moves the row to `committed` together with the object ref
    /// (`insert_receipt_records`).
    pub(super) async fn stage_receipt_artifact(
        tx: &Transaction<'_>,
        attempt: &ReceiptAttempt,
    ) -> Result<(), DatabaseError> {
        tx.execute(
            "INSERT INTO pipeline_receipt_artifacts (
                tenant_id, run_id, attempt_id, request_idempotency_key,
                request_content_hash, object_key, ciphertext_sha256
             ) VALUES ($1,$2,$3,$4,$5,$6,$7)",
            &[
                &attempt.tenant_id,
                &attempt.run_id,
                &attempt.attempt_id,
                &attempt.request_idempotency_key,
                &attempt.request_content_hash,
                &attempt.receipt.object_key,
                &attempt.receipt.ciphertext_sha256,
            ],
        )
        .await?;
        Ok(())
    }

    /// Locks a receipt attempt's row inside the receipt's final transaction
    /// and reports whether the attempt may still commit: the row is
    /// `staged` and not yet due. The sweeper deletes a due row (and its
    /// object) while holding the row's lock, and skips a row locked here, so
    /// an attempt whose row it removed cannot commit, and a row this
    /// transaction commits is never swept.
    ///
    /// A sweep that deleted a row's object and then failed
    /// before its commit (a lost connection, a later row's `DELETE`) leaves
    /// the row `staged` and unlocked with its object gone. Every row a sweep
    /// locks was due when that sweep read it, so refusing a due row here
    /// refuses a superset of the rows a failed sweep may have emptied -- it
    /// also refuses a live attempt whose own commit is simply slower than
    /// `cleanup_after`. The due check is its own statement after the lock
    /// is held, against `clock_timestamp()`: a condition in the `FOR
    /// UPDATE` query is evaluated before the lock wait, and `NOW()` is this
    /// transaction's start.
    pub(super) async fn lock_committable_receipt_artifact(
        tx: &Transaction<'_>,
        attempt: &ReceiptAttempt,
    ) -> Result<bool, DatabaseError> {
        let Some(row) = tx
            .query_opt(
                "SELECT cleanup_after FROM pipeline_receipt_artifacts
                  WHERE tenant_id = $1 AND run_id = $2 AND attempt_id = $3
                    AND state = 'staged'
                  FOR UPDATE",
                &[&attempt.tenant_id, &attempt.run_id, &attempt.attempt_id],
            )
            .await?
        else {
            return Ok(false);
        };
        let cleanup_after: DateTime<Utc> = row.get("cleanup_after");
        let due: bool = tx
            .query_one("SELECT $1 <= clock_timestamp()", &[&cleanup_after])
            .await?
            .get(0);
        Ok(!due)
    }

    /// Deletes a refused receipt attempt's `staged` row, once its object is
    /// deleted (`PipelineService::discard_receipt_attempt`).
    pub(super) async fn remove_staged_receipt_artifact(
        &self,
        attempt: &ReceiptAttempt,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &attempt.tenant_id).await?;
        tx.execute(
            "DELETE FROM pipeline_receipt_artifacts
              WHERE tenant_id = $1 AND run_id = $2 AND attempt_id = $3
                AND state = 'staged'",
            &[&attempt.tenant_id, &attempt.run_id, &attempt.attempt_id],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// After a refused receipt attempt's object delete failed: makes its row
    /// `staged` and due now, inserting it again if the sweeper already
    /// removed it, so the sweeper retries the delete on its next pass.
    pub(super) async fn restage_receipt_artifact_due_now(
        &self,
        attempt: &ReceiptAttempt,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &attempt.tenant_id).await?;
        tx.execute(
            "INSERT INTO pipeline_receipt_artifacts (
                tenant_id, run_id, attempt_id, request_idempotency_key,
                request_content_hash, object_key, ciphertext_sha256, cleanup_after
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,NOW())
             ON CONFLICT (tenant_id, run_id, attempt_id) DO UPDATE
                SET cleanup_after = NOW()
              WHERE pipeline_receipt_artifacts.state = 'staged'",
            &[
                &attempt.tenant_id,
                &attempt.run_id,
                &attempt.attempt_id,
                &attempt.request_idempotency_key,
                &attempt.request_content_hash,
                &attempt.receipt.object_key,
                &attempt.receipt.ciphertext_sha256,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records a pipeline phase attempt's object before it is written (PR 4,
    /// the same shape as `stage_receipt_artifact`): its own tenant
    /// transaction, committing before the caller publishes the object the
    /// row names. Whatever happens after the write -- the rest of the phase
    /// failing, a refused commit, a lost lease, a process crash -- the
    /// `staged` row still names the object, and
    /// `PipelineService::sweep_attempt_artifacts` deletes both once
    /// `cleanup_after` passes. `commit_review` and `commit_score` move the
    /// row to `committed` in the same transaction as the phase commit; a
    /// refused commit rolls that back with everything else, leaving the row
    /// `staged` for the sweep.
    ///
    /// `ON CONFLICT DO NOTHING` on the row's own primary key makes a repeat
    /// call for the same attempt a no-op -- safe because
    /// `pipeline_attempt_object_id` is a deterministic function of `(run_id,
    /// lease_token, artifact)`, so a retry for the same triple always
    /// prepares the same object key. A caller that ever computed a different
    /// object key for the same triple would collide instead on `UNIQUE
    /// (tenant_id, object_key)`, which this `INSERT` does not suppress, and
    /// surfaces as an ordinary constraint error.
    pub async fn stage_attempt_artifact(
        &self,
        run: &PipelineRunRecord,
        artifact: PipelineAttemptArtifact,
        object_key: &str,
        ciphertext_sha256: &str,
        cleanup_after: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        tx.execute(
            "INSERT INTO pipeline_attempt_artifacts (
                tenant_id, run_id, lease_token, artifact, object_key,
                ciphertext_sha256, cleanup_after
             ) VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (tenant_id, run_id, lease_token, artifact) DO NOTHING",
            &[
                &run.tenant_id,
                &run.run_id,
                &lease_token,
                &artifact.as_str(),
                &object_key,
                &ciphertext_sha256,
                &cleanup_after,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Stages a compatibility Score attempt's rows before its object
    /// content exists (rebase 10, option D): one row for each `(artifact,
    /// object_key)` in `artifacts`, with the key the write will use and
    /// `ciphertext_sha256` NULL, in one tenant transaction that commits and
    /// returns its connection to the pool before the Score takes its
    /// tenant's Score lock. So the Score holds one pooled connection at a
    /// time while it holds the lock, and every object it later publishes
    /// already has a committed `staged` row. `commit_score_in` sets each
    /// written row's hash as it commits it, and deletes the row of an
    /// artifact the Score did not write; the sweep deletes the object of a
    /// due row with no hash at its key (`sweep_attempt_artifacts`).
    ///
    /// Locks: the foreign key check takes `FOR KEY SHARE` on the run's
    /// `pipeline_runs` row for this short transaction; it takes no other
    /// row lock and holds no lock when it ends, before the Score lock is
    /// requested. `ON CONFLICT DO NOTHING` as `stage_attempt_artifact`.
    pub async fn stage_unhashed_attempt_artifacts(
        &self,
        run: &PipelineRunRecord,
        artifacts: &[(PipelineAttemptArtifact, String)],
        cleanup_after: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        let lease_token = required_lease_token(run)?;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::tenant_transaction(&mut client, &run.tenant_id).await?;
        for (artifact, object_key) in artifacts {
            tx.execute(
                "INSERT INTO pipeline_attempt_artifacts (
                    tenant_id, run_id, lease_token, artifact, object_key,
                    ciphertext_sha256, cleanup_after
                 ) VALUES ($1,$2,$3,$4,$5,NULL,$6)
                 ON CONFLICT (tenant_id, run_id, lease_token, artifact) DO NOTHING",
                &[
                    &run.tenant_id,
                    &run.run_id,
                    &lease_token,
                    &artifact.as_str(),
                    object_key,
                    &cleanup_after,
                ],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
