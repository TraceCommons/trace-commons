// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `PipelineService`: the pipeline's phase runtime over the store.

use super::*;

impl PipelineService {
    /// Wakes `notifier`'s payout rails after every commit that finalizes a
    /// credit settlement batch. Set once; false if one was already set.
    /// The pipeline holds nothing else of the payout and never waits on it.
    pub fn attach_payout_notifier(&self, notifier: crate::payout_worker::PayoutNotifier) -> bool {
        self.payout_notifier.set(notifier).is_ok()
    }

    pub fn bundle_id(&self) -> &str {
        &self.default_package.bundle_id
    }

    /// The package this service registers as every rollout tenant's active
    /// bundle (`register_default_bundle`) and the one
    /// `pipeline_runtime_is_production_qualified` checks at startup: startup
    /// qualification is scoped to the bundle a runtime actually starts
    /// (decision P4-D7), not every dependency the service happens to hold.
    pub fn default_package(&self) -> &BundlePackage {
        &self.default_package
    }

    /// The Score policies an embedding binary registered
    /// (`PipelineServiceBuilder::with_score_policies`); empty in the open
    /// build. The qualification store and the production observation read
    /// it, so they qualify exactly the Score policies this service can bind.
    pub fn score_policies(&self) -> &ScorePolicyRegistry {
        &self.score_policies
    }

    /// The `trace_object_refs.object_store` label this service records.
    pub fn object_store_name(&self) -> &str {
        &self.object_store_name
    }

    /// The per-phase claim lease this service's `process_one`/`process_run`
    /// use.
    pub fn lease_config(&self) -> PipelineLeaseConfig {
        self.lease_config
    }

    /// Whether a tenant with no committed routing row may start new runs
    /// (`PipelineServiceBuilder::with_unqualified_routing`).
    pub fn unqualified_routing(&self) -> bool {
        self.unqualified_routing
    }

    /// How many times the Settle policy actually ran for this service. A
    /// retry that reuses a persisted selection (`load_settle_selection`)
    /// does not increment this -- the policy runs at most once per run.
    pub fn settle_evaluations(&self) -> usize {
        self.settle_evaluations.load(Ordering::SeqCst)
    }

    /// The tenant's authority as the provider resolves it now: `None`
    /// with no provider or no authority for the tenant, an error (its
    /// label) when the provider's read fails.
    pub(super) async fn resolve_authority(
        &self,
        tenant_id: &str,
    ) -> anyhow::Result<Option<crate::trace_authority::SubmissionAuthority>> {
        match self.authority.as_ref() {
            Some(provider) => provider.resolve_authority(tenant_id).await,
            None => Ok(None),
        }
    }

    pub fn dependency_qualification(&self) -> PipelineDependencyQualification {
        PipelineDependencyQualification {
            scorer: self
                .scorers
                .values()
                .all(|scorer| scorer.production_qualified()),
            embedder: self
                .embedders
                .values()
                .all(|embedder| embedder.production_qualified()),
            index_reader: self.index_reader.production_qualified(),
            index_writer: self.index_writer.production_qualified(),
            settlement_adapters: self.settlement_adapters.production_qualifications(),
            authority: self
                .authority
                .as_ref()
                .is_some_and(|authority| authority.production_qualified()),
            privacy: self
                .privacy
                .as_ref()
                .is_some_and(|privacy| privacy.production_qualified()),
            payout: self
                .payout
                .as_ref()
                .is_some_and(|(adapter, _)| adapter.production_qualified()),
        }
    }

    /// Whether this service pays settled Trace Credit out through NEAR: it
    /// holds a payout adapter and its configuration (`with_payout`).
    pub fn payout_enabled(&self) -> bool {
        self.payout.is_some()
    }

    /// How often an enabled payout polls a `submitted` payout for its
    /// confirmation (`PipelinePayoutConfig::confirmation_interval`); `None`
    /// while payout is disabled.
    pub fn payout_confirmation_interval(&self) -> Option<std::time::Duration> {
        self.payout
            .as_ref()
            .map(|(_, config)| config.confirmation_interval)
    }

    /// Takes the follow-up steps this service queued work for on
    /// `tenant_id` since the last call, and clears them: the index
    /// invalidation step once a withdrawal, a cancelled index write, or a
    /// requeue queued an invalidation, and the payout step once Settle
    /// completed a Trace Credit leg. The worker runs a woken step at once,
    /// instead of waiting out its interval (Zaki review 1, round 2, item 4).
    /// Only this process's own work wakes a step; another replica's waits
    /// for the interval.
    pub fn take_follow_ups(&self, tenant_id: &str) -> PipelineFollowUps {
        self.follow_ups
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(tenant_id)
            .unwrap_or_default()
    }

    /// Marks `wake`'s steps as having work for `tenant_id`
    /// (`take_follow_ups`).
    pub(super) fn wake_follow_ups(&self, tenant_id: &str, wake: PipelineFollowUps) {
        let mut follow_ups = self
            .follow_ups
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let woken = follow_ups.entry(tenant_id.to_string()).or_default();
        woken.index_invalidations |= wake.index_invalidations;
        woken.payouts |= wake.payouts;
        woken.credit_audits |= wake.credit_audits;
    }

    /// The configuration of `main`'s `NoveltyUtility` credit checks this
    /// service applies (`PipelineServiceBuilder::with_novelty_utility_checks`).
    pub fn novelty_utility_checks(&self) -> &PipelineNoveltyUtilityChecks {
        &self.novelty_utility_checks
    }

    /// `main`'s NEAR payout controls an enabled payout applies
    /// (`PipelinePayoutConfig::controls`); `None` while payout is disabled.
    pub fn payout_controls(&self) -> Option<PipelineNearPayoutControls> {
        self.payout.as_ref().map(|(_, config)| config.controls)
    }

    /// Whether this service's privacy boundary runs a prose-PII classifier
    /// (`PipelinePrivacyBoundary::classifies_prose_pii`); `false` with no
    /// boundary.
    pub fn privacy_classifies_prose_pii(&self) -> bool {
        self.privacy
            .as_ref()
            .is_some_and(|privacy| privacy.classifies_prose_pii())
    }

    /// Whether the default bundle is the compatibility bundle.
    pub fn binds_compatibility_bundle(&self) -> bool {
        crate::versioned_pipeline_bundle::package_compatibility_config(&self.default_package)
            .is_some()
    }

    /// The default bundle's compatibility configuration, which ingest
    /// compares with `main`'s gate configuration at assembly; `None` when the
    /// default bundle is not the compatibility bundle.
    pub fn compatibility_config(
        &self,
    ) -> Option<crate::versioned_pipeline_compat::CompatibilityBundleConfig> {
        crate::versioned_pipeline_bundle::package_compatibility_config(&self.default_package)
    }

    /// The NEAR credit contract an enabled payout names
    /// (`PipelinePayoutConfig::near_contract_id`); `None` while payout is
    /// disabled.
    pub fn payout_near_contract_id(&self) -> Option<&str> {
        self.payout
            .as_ref()
            .and_then(|(_, config)| config.near_contract_id.as_deref())
    }

    /// Connectivity probe: a bare `SELECT 1` through the trace pool. Not
    /// tenant-scoped -- it proves the pool can reach PostgreSQL, nothing
    /// about tenant data.
    pub async fn readiness(&self) -> anyhow::Result<()> {
        self.backend
            .trace_pool()
            .get()
            .await?
            .simple_query("SELECT 1")
            .await?;
        Ok(())
    }

    pub async fn register_bundle(
        &self,
        tenant_id: &str,
        package: &BundlePackage,
    ) -> anyhow::Result<()> {
        self.store.register_bundle(tenant_id, package).await?;
        Ok(())
    }

    pub async fn inspect(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineInspection>> {
        let Some(run) = self.store.get_run(tenant_id, run_id).await? else {
            return Ok(None);
        };
        let outcomes = self.store.list_outcomes(tenant_id, run_id).await?;
        let settlements = self.store.list_settlements(tenant_id, run_id).await?;
        Ok(Some(PipelineInspection {
            run,
            outcomes,
            settlements,
        }))
    }

    #[doc(hidden)]
    pub fn store(&self) -> &PgPipelineStore {
        &self.store
    }

    /// The index writer Settle's own index dispatch writes through
    /// (`PipelineServiceBuilder::new`). Exposed so a caller -- the index
    /// rebuild worker route -- can rebuild through the exact same writer a
    /// live run would dispatch to, rather than a second one constructed
    /// independently.
    pub fn index_writer(&self) -> Arc<dyn IdentifiedIndexWriter> {
        self.index_writer.clone()
    }

    pub async fn withdraw_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
        account_id: Option<Uuid>,
    ) -> anyhow::Result<PipelineWithdrawalOutcome> {
        let outcome = self
            .store
            .withdraw_submission(tenant_id, submission_id, actor_principal_ref, account_id)
            .await?;
        if outcome.index_invalidation == PipelineWithdrawalFollowUpState::Pending {
            self.wake_follow_ups(
                tenant_id,
                PipelineFollowUps {
                    index_invalidations: true,
                    payouts: false,
                    credit_audits: false,
                },
            );
        }
        Ok(outcome)
    }

    /// The pipeline's follow-up of `main`'s revocation of `submission_id`
    /// (`PgPipelineStore::follow_up_revocation`); a queued invalidation
    /// wakes the worker's invalidation step.
    pub async fn follow_up_revocation(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
    ) -> anyhow::Result<()> {
        if self
            .store
            .follow_up_revocation(tenant_id, submission_id, actor_principal_ref)
            .await?
        {
            self.wake_follow_ups(
                tenant_id,
                PipelineFollowUps {
                    index_invalidations: true,
                    payouts: false,
                    credit_audits: false,
                },
            );
        }
        Ok(())
    }

    /// The pipeline's follow-up of a withdrawal `main` completes on its own
    /// path (`PgPipelineStore::follow_up_withdrawal`); a queued invalidation
    /// wakes the worker's invalidation step.
    pub async fn follow_up_withdrawal(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
    ) -> anyhow::Result<()> {
        if self
            .store
            .follow_up_withdrawal(tenant_id, submission_id, actor_principal_ref)
            .await?
        {
            self.wake_follow_ups(
                tenant_id,
                PipelineFollowUps {
                    index_invalidations: true,
                    payouts: false,
                    credit_audits: false,
                },
            );
        }
        Ok(())
    }

    /// Re-enqueues every `failed` index invalidation of `tenant_id`
    /// (`PgPipelineStore::requeue_failed_index_invalidations`) and returns
    /// how many; when there were any, the worker's next pass takes them up.
    pub async fn requeue_failed_index_invalidations(&self, tenant_id: &str) -> anyhow::Result<u64> {
        let requeued = self
            .store
            .requeue_failed_index_invalidations(tenant_id)
            .await?;
        if requeued > 0 {
            self.wake_follow_ups(
                tenant_id,
                PipelineFollowUps {
                    index_invalidations: true,
                    payouts: false,
                    credit_audits: false,
                },
            );
        }
        Ok(requeued)
    }

    /// Recovers `tenant_id`'s revocation and withdrawal follow-ups that were
    /// lost (`PgPipelineStore::recover_lost_inoperable_follow_ups`; poldsam
    /// P-2); the worker runs it once a minute for each tenant, before it
    /// takes the tenant's woken steps. A recovered follow-up wakes the
    /// invalidation step, so its invalidations are processed in the same
    /// pass.
    pub async fn recover_lost_inoperable_follow_ups(
        &self,
        tenant_id: &str,
        actor_principal_ref: &str,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let recovered = self
            .store
            .recover_lost_inoperable_follow_ups(tenant_id, actor_principal_ref, limit)
            .await?;
        if recovered > 0 {
            self.wake_follow_ups(
                tenant_id,
                PipelineFollowUps {
                    index_invalidations: true,
                    payouts: false,
                    credit_audits: false,
                },
            );
        }
        Ok(recovered)
    }

    /// Releases `tenant_id`'s parked runs whose submission is no longer
    /// operable (`PgPipelineStore::release_inoperable_parked_runs`); the
    /// worker runs it on its invalidation step.
    pub async fn release_inoperable_parked_runs(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> anyhow::Result<u64> {
        Ok(self
            .store
            .release_inoperable_parked_runs(tenant_id, limit)
            .await?)
    }

    /// Processes up to `limit` of `tenant_id`'s due index invalidations and
    /// returns how many it recorded a result for. Only `tenant_id`'s own
    /// queue is read. One statement claims them all
    /// (`PgPipelineStore::claim_due_index_invalidations`, leased for the
    /// Settle phase's lease, since Settle is the phase that writes the
    /// index), and each claim is then processed in turn
    /// (`process_claimed_index_invalidation`). A database error ends the
    /// pass; each claim it had not recorded a result for waits out its
    /// lease, uncharged.
    pub async fn process_index_invalidations(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let mut processed = 0;
        for claim in self
            .store
            .claim_due_index_invalidations(tenant_id, limit, self.lease_config.settle())
            .await?
        {
            if self.process_claimed_index_invalidation(&claim).await? {
                processed += 1;
            }
        }
        Ok(processed)
    }

    /// One attempt at a claimed index invalidation: removes the queued
    /// revision from the index under the tenant's storage reference and the
    /// index id of the run's committed Score evidence, and records the
    /// result. `Ok(true)` or `Ok(false)` from the index completes the
    /// invalidation. An index outage (`Failed` or `Uncertain`) is an
    /// uncharged retry with backoff (`retry_index_invalidation`, Ruling
    /// T8-2), so an outage never ends the invalidation. Only a failure
    /// waiting cannot heal is charged (`fail_index_invalidation`), and ends
    /// `failed` once the attempts run out: Score evidence that names no
    /// index, or a `ContentConflict`, which the `invalidate_revision`
    /// contract rules out.
    ///
    /// Returns whether this call recorded a result: `false` when another
    /// claim recorded one first, took the row over once this claim's lease
    /// had passed, or a late index write reopened the row while this claim
    /// held it (issue #1233; the next claim removes the revision again).
    /// Each step checks out its own pooled connection and
    /// returns it before the next, and none is held across the index call.
    pub(super) async fn process_claimed_index_invalidation(
        &self,
        claim: &PipelineIndexInvalidationClaim,
    ) -> anyhow::Result<bool> {
        let tenant_id = claim.tenant_id.as_str();
        let Some(index_id) = self.committed_index_id(tenant_id, claim.run_id).await? else {
            return Ok(self.store.fail_index_invalidation(claim).await?);
        };
        // A synchronous index call, on the blocking pool (N-6). A task that
        // did not return is an outage: the invalidation is retried.
        let writer = self.index_writer.clone();
        let tenant = pipeline_tenant_storage_ref(tenant_id);
        let revision_id = claim.registry_revision_id;
        let result = tokio::task::spawn_blocking(move || {
            writer.invalidate_revision(&tenant, &index_id, revision_id)
        })
        .await
        .unwrap_or(Err(IndexWriteError::Uncertain));
        Ok(match result {
            Ok(_) => self.store.complete_index_invalidation(claim).await?,
            Err(IndexWriteError::Failed | IndexWriteError::Uncertain) => {
                self.store.retry_index_invalidation(claim).await?
            }
            Err(IndexWriteError::ContentConflict) => {
                self.store.fail_index_invalidation(claim).await?
            }
        })
    }

    /// The index id the run's committed Score evidence names: the index its
    /// Settle wrote to. `None` when there is no Score outcome, or its
    /// evidence does not decode or names no index.
    pub(super) async fn committed_index_id(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        let Some(outcome) = self
            .store
            .outcome_for_phase(tenant_id, run_id, Phase::Score)
            .await?
        else {
            return Ok(None);
        };
        Ok(serde_json::from_value::<ScoreEvidence>(outcome.evidence)
            .ok()
            .and_then(|evidence| evidence.index_id))
    }

    /// Registers this service's default bundle for `tenant_id` and, if the
    /// tenant has no active bundle yet, activates it. Idempotent: calling it
    /// again for a tenant that already has an active bundle changes
    /// nothing.
    ///
    /// Ingest startup calls this once for every rollout tenant before it
    /// starts taking receipts, so a receipt never has to. A test that calls
    /// `submit` directly, on a tenant startup never touched, must call this
    /// first for the same reason.
    pub async fn register_default_bundle(&self, tenant_id: &str) -> anyhow::Result<()> {
        self.store
            .register_bundle(tenant_id, &self.default_package)
            .await?;
        self.store
            .activate_bundle_if_none(tenant_id, self.bundle_id())
            .await?;
        Ok(())
    }

    /// `PgPipelineStore::intervene_policy`: suspend or resume one policy of a
    /// bound bundle.
    pub async fn intervene_policy(
        &self,
        tenant_id: &str,
        bundle_id: &str,
        phase: Phase,
        action: &str,
        actor_principal_ref: &str,
        reason_code: &str,
    ) -> anyhow::Result<PipelinePolicyInterventionRecord> {
        Ok(self
            .store
            .intervene_policy(
                tenant_id,
                bundle_id,
                phase,
                action,
                actor_principal_ref,
                reason_code,
            )
            .await?)
    }

    /// `PgPipelineStore::list_policy_interventions`: the interventions on a
    /// bundle's policies, oldest first.
    pub async fn list_policy_interventions(
        &self,
        tenant_id: &str,
        bundle_id: &str,
    ) -> anyhow::Result<Vec<PipelinePolicyInterventionRecord>> {
        Ok(self
            .store
            .list_policy_interventions(tenant_id, bundle_id)
            .await?)
    }

    /// The startup checks of every bundle a worker may run for `tenant_id`
    /// (multi-lens review L5-2, Zaki review 3, Z3-2): its active bundle and
    /// the bundle of each run in flight (`runnable_bundle_ids`), which a
    /// tenant keeps when the default package changes
    /// (`activate_bundle_if_none`). Each gets the checks ingest gives the
    /// default package at assembly: its dependencies resolve (`construct`),
    /// a compatibility package holds `main`'s gate configuration
    /// (`main_gate`) and has the pipeline's credit issuer, and, with
    /// `require_qualifiable`, the package passes the default package's
    /// qualification gate: its own bundle qualification
    /// (`bundle_qualification`, decision P4-D7), every dependency it names
    /// and its configuration term. P4-D7 lets the service hold a dependency
    /// that is not production-qualified while the default package does not
    /// name it, so a tenant bundle that names one is refused here. The error
    /// is the first failure's label, the default package check's where they
    /// share one; ingest refuses to start on it.
    pub async fn check_tenant_bundles(
        &self,
        tenant_id: &str,
        main_gate: &crate::versioned_pipeline_compat::MainGateConfig,
        require_qualifiable: bool,
    ) -> anyhow::Result<()> {
        let bundle_ids = self
            .store
            .runnable_bundle_ids(tenant_id)
            .await
            .map_err(|_| anyhow::anyhow!(PIPELINE_TENANT_BUNDLE_UNREADABLE_LABEL))?;
        for bundle_id in bundle_ids {
            let package = self
                .store
                .load_bundle(tenant_id, &bundle_id)
                .await
                .map_err(|_| anyhow::anyhow!(PIPELINE_TENANT_BUNDLE_UNREADABLE_LABEL))?
                .ok_or_else(|| anyhow::anyhow!(PIPELINE_TENANT_BUNDLE_UNREADABLE_LABEL))?;
            self.check_tenant_package(&package, main_gate, require_qualifiable)
                .map_err(|label| anyhow::anyhow!(label))?;
        }
        Ok(())
    }

    /// The checks of a bundle a tenant already selected, or is being moved
    /// to by an activation or a rollback: [`Self::check_runnable_package`],
    /// except that a compatibility package may also hold `main`'s scoring
    /// configuration with no award at all (delta 0, rule `flat`), where
    /// `main` now awards more (owner decision D6, 2026-11-03). Such a bundle
    /// scores exactly as `main` does and pays less, never more, so a tenant
    /// that selected it keeps running it after the delta or the credit rule
    /// is raised, until the default-routing loop or an operator moves it.
    /// The default package and an armed default-routing set never take this
    /// path: they must hold `main`'s configuration exactly.
    pub fn check_tenant_package(
        &self,
        package: &BundlePackage,
        main_gate: &crate::versioned_pipeline_compat::MainGateConfig,
        require_qualifiable: bool,
    ) -> Result<(), &'static str> {
        self.check_package(package, main_gate, require_qualifiable, true)
    }

    /// Whether `tenant_id`'s bundle `bundle_id` is a compatibility package
    /// that awards nothing under `main`'s scoring configuration (owner
    /// decision D6): the bundles the default-routing loop moves to the armed
    /// bundle once the delta or the credit rule is raised. `false` for a
    /// bundle that matches `main` exactly, of another family, or unreadable.
    pub async fn bundle_awards_nothing_under(
        &self,
        tenant_id: &str,
        bundle_id: &str,
        main_gate: &crate::versioned_pipeline_compat::MainGateConfig,
    ) -> bool {
        let Ok(Some(package)) = self.store.load_bundle(tenant_id, bundle_id).await else {
            return false;
        };
        crate::versioned_pipeline_bundle::package_compatibility_config(&package).is_some_and(
            |config| {
                !config.matches_main_gate(main_gate)
                    && config.matches_main_gate_without_award(main_gate)
            },
        )
    }

    /// `check_tenant_bundles`'s checks of one package. The activation route
    /// runs them for the bundle it activates, before the store's gate
    /// (`PipelineActivationStore::activate_tenant`, `rollback_bundle`), so
    /// the startup checks of a tenant bundle also hold for a bundle that is
    /// activated after start: the gate itself holds no service.
    pub fn check_runnable_package(
        &self,
        package: &BundlePackage,
        main_gate: &crate::versioned_pipeline_compat::MainGateConfig,
        require_qualifiable: bool,
    ) -> Result<(), &'static str> {
        self.check_package(package, main_gate, require_qualifiable, false)
    }

    pub(super) fn check_package(
        &self,
        package: &BundlePackage,
        main_gate: &crate::versioned_pipeline_compat::MainGateConfig,
        require_qualifiable: bool,
        allow_without_award: bool,
    ) -> Result<(), &'static str> {
        let bundle = self.construct(package.clone()).map_err(|label| {
            if label == PIPELINE_DEPENDENCY_MISSING_LABEL {
                PIPELINE_TENANT_BUNDLE_DEPENDENCY_MISSING_LABEL
            } else {
                PIPELINE_TENANT_BUNDLE_NOT_RUNNABLE_LABEL
            }
        })?;
        if let Some(config) =
            crate::versioned_pipeline_bundle::package_compatibility_config(package)
        {
            if !config.matches_main_gate(main_gate)
                && !(allow_without_award && config.matches_main_gate_without_award(main_gate))
            {
                return Err("pipeline_runtime_main_gate_config_mismatch");
            }
        }
        // Keyed on the Trace Credit event the bundle settles under, not on
        // its family: a compatibility bundle settles under `NoveltyUtility`,
        // and so does a bundle whose registered Score policy declares it.
        if bundle.trace_credit_event.is_novelty_utility()
            && self.novelty_utility_checks.issuer_principal_ref.is_none()
        {
            return Err("pipeline_credit_issuer_principal_missing");
        }
        // The default package's gate (`pipeline_runtime_is_production_qualified`
        // in ingest): the bundle qualification of this package, which also
        // carries the configuration term (`bundle_configuration_not_qualifiable`).
        if require_qualifiable
            && !self
                .bundle_qualification(package)
                .is_ok_and(|qualification| qualification.is_production_qualified())
        {
            return Err("pipeline_runtime_dependencies_not_production_qualified");
        }
        Ok(())
    }

    pub(super) fn inject_crash(&self, point: PipelineCrashPoint) -> anyhow::Result<()> {
        inject_crash_at(self.crash_point, &self.crash_pending, point)
    }

    /// Whether the submission behind a run is still operable right now
    /// (`PgPipelineStore::submission_guard_on_tx`), read in a transaction of
    /// its own that commits at once. Settle reads this fresh before it
    /// decides index membership and again before its settlement legs,
    /// since those checks can be far apart in wall-clock time across a
    /// crash and retry. The index dispatch does not use it: it reads the
    /// guard on its own transaction and holds it until the write commits.
    pub(super) async fn submission_guard(
        &self,
        run: &PipelineRunRecord,
    ) -> anyhow::Result<SubmissionGuard> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
        let guard = PgPipelineStore::submission_guard_on_tx(&tx, run).await?;
        tx.commit().await?;
        Ok(guard)
    }

    /// Confirms the lease this call still holds is the current one,
    /// immediately before an effect on the external index that a
    /// transaction rollback cannot undo (port line 5367). Unlike
    /// `ensure_current_lease`, this has no open transaction to read
    /// through -- it goes back through the store, which is the "service
    /// level" check the port's version also is.
    pub(super) async fn ensure_live_lease(&self, run: &PipelineRunRecord) -> anyhow::Result<()> {
        let current = self
            .store
            .get_run(&run.tenant_id, run.run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("pipeline lease is stale"))?;
        if current.state != PipelineRunState::Leased
            || current.lease_token != run.lease_token
            || current
                .lease_expires_at
                .is_none_or(|expires_at| expires_at <= Utc::now())
        {
            anyhow::bail!("pipeline lease is stale");
        }
        Ok(())
    }

    /// Resolves the scorer and embedder `package` names among the held
    /// dependencies, by content hash -- the same hash `with_scorer`/
    /// `with_embedder` index by. A hash the service does not hold, whether
    /// because it holds none at all or because a same-labeled dependency's
    /// content changed, fails closed with `bundle_dependency_missing`.
    /// Shared by `construct_with_index_reader` (which builds the runnable
    /// bundle from the result, for `construct` and for a compatibility
    /// Score's serialized read) and `bundle_qualification` (which only
    /// needs the resolved objects' identity and qualification, never a
    /// runnable bundle).
    pub(super) fn resolve_score_dependencies(
        &self,
        package: &BundlePackage,
    ) -> Result<
        (
            Arc<dyn IdentifiedPerplexityScorer>,
            Arc<dyn IdentifiedEmbedder>,
        ),
        &'static str,
    > {
        let named = &package.manifest.score.data_artifact_hashes;
        let scorer = named
            .iter()
            .find_map(|hash| self.scorers.get(hash))
            .cloned();
        let embedder = named
            .iter()
            .find_map(|hash| self.embedders.get(hash))
            .cloned();
        let (Some(scorer), Some(embedder)) = (scorer, embedder) else {
            return Err(PIPELINE_DEPENDENCY_MISSING_LABEL);
        };
        Ok((scorer, embedder))
    }

    /// Resolve the dependencies the package names and construct its bundle.
    /// A hash the service does not hold fails closed.
    pub(super) fn construct(
        &self,
        package: BundlePackage,
    ) -> Result<MinimalPolicyBundle, &'static str> {
        self.construct_with_index_reader(package, self.index_reader.clone())
    }

    /// `construct`, with `index_reader` as the index the Score policy reads.
    pub(super) fn construct_with_index_reader(
        &self,
        package: BundlePackage,
        index_reader: Arc<dyn IdentifiedIndexReader>,
    ) -> Result<MinimalPolicyBundle, &'static str> {
        let (scorer, embedder) = self.resolve_score_dependencies(&package)?;
        MinimalPolicyBundle::from_package_with_runtime_and_score_policies(
            package,
            scorer,
            embedder,
            index_reader,
            &self.score_policies,
        )
        .map_err(|error| match error.to_string().as_str() {
            PIPELINE_DEPENDENCY_MISSING_LABEL => PIPELINE_DEPENDENCY_MISSING_LABEL,
            "bundle_policy_not_runnable" => PIPELINE_POLICY_NOT_RUNNABLE_LABEL,
            _ => PIPELINE_BUNDLE_INVALID_LABEL,
        })
    }

    /// Whether every dependency the package `package` actually uses is
    /// production-qualified (decision P4-D7): the scorer and embedder it
    /// names, the held index reader and writer, one settlement-adapter check
    /// per instrument it pins, `authority` and `privacy`, and `payout`
    /// whenever payout is enabled (service-wide, final review M5).
    /// `pipeline_runtime_is_production_qualified` calls this with
    /// `default_package()` at startup, so an unqualified scorer, embedder,
    /// or settlement adapter the default bundle never touches no longer
    /// blocks boot.
    ///
    /// `Err("bundle_package_invalid")` when `package` itself does not
    /// validate (`package_digests`); `Err("bundle_dependency_missing")` when
    /// it names a scorer or embedder hash the service does not hold -- the
    /// same two failure labels `construct` can return for the same reasons.
    pub fn bundle_qualification(
        &self,
        package: &BundlePackage,
    ) -> Result<PipelineBundleQualification, &'static str> {
        let digests = package_digests(package).map_err(|_| PIPELINE_BUNDLE_INVALID_LABEL)?;
        let (scorer, embedder) = self.resolve_score_dependencies(package)?;
        let settlement_adapters = package
            .manifest
            .instruments
            .keys()
            .map(|instrument_id| {
                let check = match self.settlement_adapters.get(instrument_id) {
                    Some(adapter) => PipelineDependencyCheck {
                        identity: adapter.adapter_identity().to_string(),
                        production_qualified: adapter.production_qualified(),
                    },
                    None => PipelineDependencyCheck {
                        identity: PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL.to_string(),
                        production_qualified: false,
                    },
                };
                (instrument_id.as_str().to_string(), check)
            })
            .collect();
        // Payout is service-wide, like the index writer: `process_payouts`
        // pays the complete runs of every bundle, including runs bound to
        // an earlier default package. So an enabled payout's adapter counts
        // for every package, not only one that pins `trace_credit` (final
        // review M5, reversing the payout part of P4-D7).
        let payout = self.payout_enabled().then(|| {
            self.payout
                .as_ref()
                .is_some_and(|(adapter, _)| adapter.production_qualified())
        });
        // A Score policy a registry supplies is one of this bundle's
        // dependencies, qualified only when its factory says so; so is the
        // configuration it decodes. A built-in Score policy reports no
        // check, which keeps every built-in bundle's qualification as it
        // was. A Score id that is neither built in nor registered is not
        // runnable, and fails closed here as well.
        let score_implementation = package.manifest.score.implementation_id.as_str();
        let (score_policy, score_configuration_qualifiable) =
            if is_built_in_score_implementation(score_implementation) {
                (None, true)
            } else {
                match registered_score_factory(package, &self.score_policies) {
                    Some(factory) => {
                        let configuration = package
                            .artifacts
                            .get(&package.manifest.score.configuration_hash)
                            .map(Vec::as_slice)
                            .unwrap_or_default();
                        (
                            Some(PipelineDependencyCheck {
                                identity: factory.dependency_identity().to_string(),
                                production_qualified: factory.production_qualified(),
                            }),
                            factory.configuration_is_qualifiable(configuration),
                        )
                    }
                    None => (
                        Some(PipelineDependencyCheck {
                            identity: SCORE_POLICY_MISSING_LABEL.to_string(),
                            production_qualified: false,
                        }),
                        false,
                    ),
                }
            };
        Ok(PipelineBundleQualification {
            bundle_id: package.bundle_id.clone(),
            scorer: PipelineDependencyCheck {
                identity: scorer.dependency_identity().to_string(),
                production_qualified: scorer.production_qualified(),
            },
            embedder: PipelineDependencyCheck {
                identity: embedder.dependency_identity().to_string(),
                production_qualified: embedder.production_qualified(),
            },
            index_reader: PipelineDependencyCheck {
                identity: self.index_reader.dependency_identity().to_string(),
                production_qualified: self.index_reader.production_qualified(),
            },
            index_writer: PipelineDependencyCheck {
                identity: self.index_writer.dependency_identity().to_string(),
                production_qualified: self.index_writer.production_qualified(),
            },
            settlement_adapters,
            authority: self
                .authority
                .as_ref()
                .is_some_and(|authority| authority.production_qualified()),
            privacy: self
                .privacy
                .as_ref()
                .is_some_and(|privacy| privacy.production_qualified()),
            payout,
            configuration_qualifiable:
                crate::versioned_pipeline_bundle::package_configuration_is_qualifiable(package)
                    && score_configuration_qualifiable,
            score_policy,
            dependency_digest: digests.dependency_digest,
        })
    }

    /// Loads and constructs the bundle a run is bound to, checking the
    /// operator-controlled runnable flag for the run's current phase.
    pub(super) async fn load_bound_bundle(
        &self,
        run: &PipelineRunRecord,
    ) -> Result<MinimalPolicyBundle, &'static str> {
        let phase = run.next_phase.ok_or(PIPELINE_BUNDLE_INVALID_LABEL)?;
        let package = self
            .store
            .load_bundle(&run.tenant_id, &run.bundle_id)
            .await
            .map_err(|error| bound_bundle_load_error_label(&error))?
            .ok_or(PIPELINE_BUNDLE_MISSING_LABEL)?;
        if !self
            .store
            .policy_is_runnable(&run.tenant_id, &run.bundle_id, phase)
            .await
            .map_err(|error| bound_bundle_load_error_label(&error))?
        {
            return Err(PIPELINE_POLICY_NOT_RUNNABLE_LABEL);
        }
        self.construct(package)
    }

    /// Accepts a receipt as one attempt. The attempt's object
    /// is recorded before it is written, and no lock is held while it is
    /// written or while Admission runs:
    ///
    /// 1. The tenant authority lookup and the privacy-boundary presence
    ///    check run before any database work (Ruling T2-1): a tenant with no
    ///    configured authority fails closed with
    ///    `authority_control_missing`, and a service built without a privacy
    ///    boundary fails closed with `privacy_control_missing`. Neither
    ///    creates a run or a staging row.
    /// 2. A read-only check with no advisory lock (`precheck_receipt`)
    ///    refuses the common cases -- replay, content conflict, a legacy-owned
    ///    submission id, a tenant that is not routed to the pipeline,
    ///    tombstone, quota -- before any encryption. The staging transaction
    ///    repeats every check, so this one only saves work. A replayed key
    ///    returns here, so a replay never calls the privacy boundary again.
    /// 3. The deterministic rescrub: the privacy boundary's bounded, local
    ///    redactor (`rescrub_deterministic`) transforms a clone of the
    ///    server envelope and returns any residual-risk conditions it found,
    ///    merged into the caller's own basis. A failure refuses the receipt
    ///    with `privacy_rescrub_failed`. The receipt never calls the
    ///    prose-PII classifier: that runs in the Review-start privacy pass,
    ///    over the stored source. From here on the transformed envelope
    ///    replaces `request.server_envelope` everywhere -- the staged and
    ///    stored source bytes (the post-deterministic envelope), the
    ///    retention derivation, and the Admission input -- while
    ///    `request_content_hash` stays the hash of the raw
    ///    `request.request_bytes` (replay identity). This runs with no
    ///    pooled connection held.
    /// 4. The attempt's object -- the (now transformed) server envelope,
    ///    wrapped per decision P1 -- is encrypted under the attempt's own
    ///    object id (`pipeline_receipt_object_id`, a fresh random attempt
    ///    id), which fixes its object key and ciphertext hash. Nothing is
    ///    stored.
    /// 5. The staging transaction (`stage_receipt_attempt`) checks replay, an
    ///    attempt for the key staged with other content, the legacy owner and
    ///    the tenant's routing (under the routing lock, shared), the bound
    ///    bundle, tombstones, and the quota -- in that order -- counts the
    ///    quota, and inserts the attempt's `staged` row naming the object. It
    ///    commits before anything is stored, so every refusal there stores
    ///    nothing.
    /// 6. The object is written and Admission runs, with no transaction
    ///    open. A failure from here on (an error or a crash) leaves the
    ///    `staged` row naming the object; `sweep_staged_receipts` deletes
    ///    both once the row's `cleanup_after` passes.
    /// 7. The final transaction (`commit_receipt_attempt`) re-checks an
    ///    existing run for the key, the legacy owner and the tenant's routing
    ///    (under the routing lock, shared, so a containment that committed
    ///    during the write is seen and one that is waiting waits for this
    ///    transaction), and the tombstones, requires the attempt's row to be
    ///    still `staged` and not yet due, takes the Admission policy's status
    ///    row of the bound bundle `FOR SHARE` as its last lock and refuses with
    ///    `bundle_policy_not_runnable` when the policy was suspended while the
    ///    object was written (an operator's suspension waits for this
    ///    transaction), and commits the records, the pipeline's ownership row
    ///    among them, and the row's move to `committed` together. On a refusal
    ///    there, the attempt deletes its own object and row
    ///    (`discard_receipt_attempt`).
    ///
    /// Each attempt writes its own object, so two concurrent receipts for
    /// one key never overwrite each other's object: the one that commits
    /// second finds the first one's run and returns `Replayed`. A tenant
    /// with no active bundle is not registered here -- that happens once,
    /// at startup (`register_default_bundle`), and a tenant this call finds
    /// without one fails closed with `PIPELINE_BUNDLE_MISSING_LABEL`. Every
    /// synchronous object-store call (the encrypt-and-prepare in step 2, the
    /// write in step 4, and a discarded attempt's delete) runs on a blocking
    /// thread rather than this task, and each transaction returns its
    /// connection to the pool before the next step checks one out, which is
    /// what keeps a pool of size one safe.
    pub async fn submit(
        &self,
        request: PipelineReceiptRequest<'_>,
    ) -> anyhow::Result<PipelineReceiptResult> {
        anyhow::ensure!(
            !request.request_idempotency_key.trim().is_empty()
                && request.request_idempotency_key.len() <= 200,
            "invalid idempotency key"
        );
        let tenant_id = request.tenant_id;
        let request_content_hash = sha256_prefixed(request.request_bytes);
        let request_idempotency_key_hash =
            sha256_prefixed(request.request_idempotency_key.as_bytes());

        let run_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!(
                "tracecommons:pipeline-run:{tenant_id}:{}",
                request.request_idempotency_key
            )
            .as_bytes(),
        );
        let object_ref_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:pipeline-source-object:{run_id}").as_bytes(),
        );

        // 1. The authority lookup and the privacy-boundary presence check
        // run before any database work, so a tenant with a missing control
        // fails closed with no run and no staging row (Ruling T2-1). The
        // authority is resolved now, as `main`'s admission resolves it: a
        // policy read from the database is read here, on its own pooled
        // connection, returned before anything else checks one out. A
        // failed read refuses with its label.
        let authority = self
            .resolve_authority(tenant_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_AUTHORITY_CONTROL_MISSING_LABEL))?;
        let privacy = self
            .privacy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_PRIVACY_CONTROL_MISSING_LABEL))?;

        // 2. The early, lock-free refusal check.
        if let Some(refused) = self
            .precheck_receipt(
                &request,
                &request_idempotency_key_hash,
                &request_content_hash,
            )
            .await?
        {
            return Ok(refused);
        }

        // 3. The deterministic rescrub: after the lock-free precheck (a
        // replay or a refusal returns before this, so a replay never calls
        // the privacy boundary again) and before the staging row, with no
        // pooled connection held. Only the bounded, local redactor runs
        // here; the receipt never calls the prose-PII classifier, which
        // runs in the Review-start privacy pass over the stored source.
        // From here on, the transformed envelope replaces
        // `request.server_envelope` everywhere PR 2 used it: the staged and
        // stored source bytes (the post-deterministic envelope), the
        // retention derivation, and the Admission input.
        // `request_content_hash` (above) stays the hash of the raw
        // `request.request_bytes` -- the replay identity never moves.
        let mut envelope = request.server_envelope.clone();
        let mut consent_scopes = envelope.consent.scopes.clone();
        if !consent_scopes.contains(&envelope.trace_card.consent_scope) {
            consent_scopes.push(envelope.trace_card.consent_scope);
        }
        let grant_valid = authority.permits(&consent_scopes, &envelope.trace_card.allowed_uses);
        let mut residual_risk_basis = request.residual_risk_basis.to_vec();
        let deterministic_basis = privacy
            .rescrub_deterministic(&mut envelope)
            .await
            .map_err(|_| anyhow::anyhow!(PIPELINE_PRIVACY_RESCRUB_FAILED_LABEL))?;
        for condition in deterministic_basis {
            if !residual_risk_basis.contains(&condition) {
                residual_risk_basis.push(condition);
            }
        }
        let request = PipelineReceiptRequest {
            source_session: request.source_session,
            tenant_id: request.tenant_id,
            actor_principal_ref: request.actor_principal_ref,
            counts_toward_quota: request.counts_toward_quota,
            request_idempotency_key: request.request_idempotency_key,
            request_bytes: request.request_bytes,
            server_envelope: &envelope,
            residual_risk_basis: &residual_risk_basis,
            limits: request.limits,
            accept_medium_risk: request.accept_medium_risk,
        };
        let envelope = request.server_envelope;

        // 4. Prepare this attempt's object. Nothing is stored yet. The
        // store's encrypt call is synchronous (it may reach a remote
        // key-wrap service), so it runs on a blocking thread rather than
        // this task.
        let tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id);
        let server_envelope_bytes = serde_json::to_vec(envelope)?;
        let wrapper = encode_pipeline_artifact_bytes(&server_envelope_bytes)?;
        let attempt_id = Uuid::new_v4();
        let object_id = pipeline_receipt_object_id(run_id, attempt_id);
        let prepared = {
            let store = self.artifact_store.clone();
            let tenant_storage_ref = tenant_storage_ref.clone();
            tokio::task::spawn_blocking(move || {
                store.prepare_serialized_json(
                    tenant_storage_ref.as_str(),
                    TraceArtifactKind::ContributionEnvelope,
                    &object_id,
                    &wrapper,
                )
            })
            .await
            .map_err(|_| anyhow::anyhow!(PIPELINE_RECEIPT_OBJECT_TASK_FAILED_LABEL))??
        };
        let attempt = ReceiptAttempt {
            tenant_id: tenant_id.to_string(),
            run_id,
            attempt_id,
            request_idempotency_key: request_idempotency_key_hash.clone(),
            request_content_hash: request_content_hash.clone(),
            receipt: prepared.receipt().clone(),
        };

        // 5. The staging transaction: every refusal, then the attempt's row.
        let (bundle_id, bundle) = match self.stage_receipt_attempt(&request, &attempt).await? {
            ReceiptStage::Refused(result) => return Ok(result),
            ReceiptStage::Staged { bundle_id, bundle } => (bundle_id, bundle),
        };

        // 6. Write the object the row names, then run Admission. The write
        // is synchronous file or network I/O, so it also runs on a blocking
        // thread -- with no transaction open and no advisory lock held (the
        // staging transaction above already committed and released both
        // locks it took).
        let written = {
            let store = self.artifact_store.clone();
            tokio::task::spawn_blocking(move || store.publish_serialized_json(&prepared))
                .await
                .map_err(|_| anyhow::anyhow!(PIPELINE_RECEIPT_OBJECT_TASK_FAILED_LABEL))??
        };
        if !written.matches_identity(&attempt.receipt) {
            // A store that wrote some other object than the one it
            // prepared: that object is not the one the row names, so delete
            // it here (best effort) and fail closed. The row's own object,
            // if the store wrote it too, is the sweeper's.
            let store = self.artifact_store.clone();
            let delete_tenant_storage_ref = tenant_storage_ref.clone();
            let delete_failed = match tokio::task::spawn_blocking(move || {
                store.delete_artifact(delete_tenant_storage_ref.as_str(), &written)
            })
            .await
            {
                Ok(result) => result.is_err(),
                Err(_) => true,
            };
            if delete_failed {
                tracing::warn!(
                    label = "pipeline_receipt_object_delete_failed",
                    "best-effort delete of a receipt object the store wrote \
                     under an unexpected key failed"
                );
            }
            anyhow::bail!(PIPELINE_RECEIPT_OBJECT_MISMATCH_LABEL);
        }
        self.inject_crash(PipelineCrashPoint::AfterArtifactStorage)?;

        // `authenticated` stays fixed `true` (decision D15): the legacy
        // handler already authenticated the request before ever calling
        // `submit`. Ruling T2-3: a missing authority source stops the
        // receipt outright (`PIPELINE_AUTHORITY_CONTROL_MISSING_LABEL`,
        // above, before any database work); reaching here means a source
        // was resolved and validated the request, so `authority_valid` is
        // `true`. Whether that source permits this submission's consent
        // scopes and allowed uses (`grant_valid`, computed above) is an
        // ordinary business decision, not a structural failure -- a denial
        // is an Admission Reject with reason `grant_invalid`, the same as
        // any other Reject, never `MinimalAdmissionPolicy`'s permanent
        // `authority_missing` error (SYS-003, reserved for the missing-
        // source case this receipt already handled).
        let (privacy_risk, accepted_medium) = admission_privacy_risk(
            &envelope.privacy.residual_pii_risk,
            request.residual_risk_basis,
            request.accept_medium_risk,
        );
        let admission_input = AdmissionInput {
            run_id,
            tenant_storage_ref: tenant_storage_ref.clone(),
            trace_id: envelope.trace_id,
            request_content_hash: request_content_hash.clone(),
            schema_version: envelope.schema_version.clone(),
            authenticated: true,
            authority_valid: true,
            contribution_path_valid: !envelope.ironclaw.version.trim().is_empty()
                && !envelope
                    .privacy
                    .redaction_pipeline_version
                    .trim()
                    .is_empty(),
            grant_valid,
            consent_valid: envelope.consent.revocable,
            allowed_uses_valid: true,
            tombstoned: false,
            quota_available: true,
            privacy_risk,
        };
        let mut admission = bundle
            .admission
            .execute(&admission_input)
            .await
            .map_err(|error| anyhow::anyhow!(error.label().to_string()))?;
        if accepted_medium && matches!(admission.decision, AdmissionDecision::Admit) {
            // The policy saw Low; the evidence records the Medium the
            // receipt mapped, and the rule names the setting that admitted
            // it, so the stored outcome stays truthful.
            admission.evidence.privacy_risk = Some(PrivacyRisk::Medium);
            admission.evaluation.rule_id =
                PIPELINE_ADMISSION_ACCEPT_FOUND_AND_REMOVED_RULE_ID.to_string();
        }
        let stored = StoredPhaseResult::from_result(Phase::Admission, &admission)?;

        // 7. The final transaction.
        let run = NewPipelineRun {
            tenant_id: tenant_id.to_string(),
            run_id,
            submission_id: envelope.submission_id,
            trace_id: envelope.trace_id,
            bundle_id,
            request_idempotency_key: request_idempotency_key_hash,
            request_content_hash,
            source_object_ref_id: object_ref_id,
        };
        match self
            .commit_receipt_attempt(
                &request,
                &run,
                &attempt,
                server_envelope_bytes.len(),
                stored,
                &admission.decision,
            )
            .await?
        {
            ReceiptCommit::Created(created) => Ok(PipelineReceiptResult::Created(created)),
            ReceiptCommit::Refused(result) => {
                self.discard_receipt_attempt(&attempt).await;
                Ok(result)
            }
            ReceiptCommit::StagingMissing => {
                self.discard_receipt_attempt(&attempt).await;
                Err(anyhow::anyhow!(PIPELINE_RECEIPT_STAGING_MISSING_LABEL))
            }
            ReceiptCommit::PolicyNotRunnable => {
                self.discard_receipt_attempt(&attempt).await;
                Err(anyhow::anyhow!(PIPELINE_POLICY_NOT_RUNNABLE_LABEL))
            }
        }
    }

    /// A read-only replay lookup for a caller that already knows a run may
    /// exist for this key and only needs to know what to hand back. Two
    /// callers use it: `trace-commons-ingest`'s `submit_trace_handler`, for
    /// a retried upload whose legacy admission ledger already marked it
    /// `completed` (that path never wrote a legacy file record for a
    /// pipeline-routed tenant, so it cannot replay from there); and
    /// `route_pipeline_receipt`, when the pipeline's own `submit` call
    /// itself returned `Replayed` or `ContentConflict`. Neither caller may
    /// hand back what this returns without checking ownership first -- see
    /// below.
    ///
    /// One short tenant transaction, read-only (`existing_receipt_run`, the
    /// same lookup `receipt_key_refusal` uses): `Replayed` for a run whose
    /// content hash matches `request_bytes`, `ContentConflict` for a run
    /// whose content differs, `None` when no run exists for the key yet.
    /// Unlike `precheck_receipt`, this never checks a staged attempt, a
    /// tombstone, or the quota, and it creates, stages, stores, or counts
    /// nothing -- `submit` keeps its own replay check
    /// (`receipt_key_refusal`) for the write path; this is a narrower read
    /// for a caller that isn't submitting anything new. It also does not
    /// need the re-scrubbed envelope, only the raw request bytes whose hash
    /// the key was recorded against.
    ///
    /// Also reads the run's `trace_submissions.auth_principal_ref` (the
    /// principal `submit` recorded when it first created the run --
    /// `PipelineRunRecord` itself carries only `submission_id`, not who
    /// submitted it) in the same transaction. The caller must check
    /// ownership against it before handing back either outcome: nothing
    /// upstream of this call proves the retrying principal is the one that
    /// made the original submission -- an admission anchor, for one, can
    /// cover more than one principal (one per device on the same account),
    /// so even a completed reservation does not prove it on its own.
    pub async fn replay_receipt(
        &self,
        tenant_id: &str,
        request_idempotency_key: &str,
        request_bytes: &[u8],
    ) -> anyhow::Result<Option<PipelineReplayReceipt>> {
        let request_content_hash = sha256_prefixed(request_bytes);
        let request_idempotency_key_hash = sha256_prefixed(request_idempotency_key.as_bytes());
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;
        let Some(existing) =
            existing_receipt_run(&tx, tenant_id, &request_idempotency_key_hash).await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let auth_principal_ref: String = tx
            .query_one(
                "SELECT auth_principal_ref FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &existing.submission_id],
            )
            .await?
            .get(0);
        let result = replay_result(existing, &request_content_hash);
        tx.commit().await?;
        Ok(Some(PipelineReplayReceipt {
            result,
            auth_principal_ref,
        }))
    }

    /// The receipt's early refusal check (`submit` step 2): one short
    /// tenant transaction that only reads and takes no
    /// advisory lock. It refuses the common cases -- an existing run for
    /// the key, an attempt staged with other content, a legacy-owned
    /// submission id or a tenant that is not routed to the pipeline (after
    /// the check for an existing run, so a retry is answered in every
    /// routing state), a tombstone, and the quota (for a key not yet
    /// counted) -- before the attempt's object is encrypted, so a refused
    /// receipt costs no encryption and, on a remote store, no key-wrap call.
    /// It decides nothing on its own: the staging transaction repeats every
    /// check under its locks, since the state can change in between. Its
    /// connection returns to the pool before the next step checks one out.
    pub(super) async fn precheck_receipt(
        &self,
        request: &PipelineReceiptRequest<'_>,
        request_idempotency_key: &str,
        request_content_hash: &str,
    ) -> anyhow::Result<Option<PipelineReceiptResult>> {
        let tenant_id = request.tenant_id;
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;
        let refused = if let Some(refused) = receipt_key_refusal(
            &tx,
            tenant_id,
            request_idempotency_key,
            request_content_hash,
        )
        .await?
        {
            Some(refused)
        } else if let Some(refused) = new_receipt_refusal_in(
            &tx,
            tenant_id,
            request.server_envelope.submission_id,
            self.unqualified_routing,
            false,
        )
        .await?
        {
            // A legacy-owned id or a tenant that is not routed here is
            // refused before the rescrub, so it costs no redactor work and
            // never creates a run for the privacy pass to classify.
            Some(refused)
        } else if receipt_is_tombstoned(
            &tx,
            tenant_id,
            request.server_envelope,
            request_content_hash,
        )
        .await?
        {
            Some(PipelineReceiptResult::Tombstoned)
        } else if request.counts_toward_quota {
            let principal_ref_hash = sha256_prefixed(request.actor_principal_ref.as_bytes());
            receipt_quota_refusal(&tx, request, request_idempotency_key, &principal_ref_hash)
                .await?
                .map(PipelineReceiptResult::QuotaExceeded)
        } else {
            None
        };
        tx.commit().await?;
        Ok(refused)
    }

    /// The receipt's staging transaction (`submit` step 5). It takes the
    /// receipt lock and then the tenant quota lock -- transaction-scoped
    /// advisory locks that serialize concurrent receipts for one key (or
    /// one tenant's quota) without a row lock that would block other
    /// tenants -- and releases both when it commits, before the object is
    /// written. In order: an existing run for the key (`Replayed` or
    /// `ContentConflict`); an attempt for the key staged with other content
    /// (`ContentConflict`); the tenant's routing lock, taken shared, and the
    /// refusal of a new receipt (`NotRouted(Contained)` for a contained
    /// tenant, then `LegacyOwned` when the legacy path owns the submission
    /// id, then `NotRouted` when the tenant's routing does not send new
    /// receipts to the pipeline: `new_receipt_refusal_in`); the bound bundle;
    /// tombstones (`Tombstoned`); the quota (`QuotaExceeded`). The routing
    /// lock is the last lock this transaction takes, after the receipt lock
    /// and the quota lock, and it is held to the commit: a routing change
    /// (`PipelineActivationStore`, exclusive) waits for this transaction, and
    /// a receipt staged after the change reads it. The quota is counted here
    /// (`pipeline_admission_usage`, once per key), so a receipt that fails
    /// after its write stays counted, and a retry of that key is neither
    /// counted again nor refused for its own earlier count; a receipt that
    /// this transaction's routing check refuses is not counted. A receipt
    /// that the commit transaction refuses afterwards (`NotRouted`,
    /// `LegacyOwned`, `PolicyNotRunnable`: a routing change, a legacy claim,
    /// or a suspension that committed while the object was written) keeps
    /// the usage row this transaction inserted, so the hourly quota counts
    /// it, as it counts every receipt that fails after its write: the
    /// runtime role has no `DELETE` on the table (V95). Last, the attempt's
    /// `staged` row.
    pub(super) async fn stage_receipt_attempt(
        &self,
        request: &PipelineReceiptRequest<'_>,
        attempt: &ReceiptAttempt,
    ) -> anyhow::Result<ReceiptStage> {
        let tenant_id = request.tenant_id;
        let mut client = self.backend.trace_pool().get().await?;
        // READ COMMITTED, stated (G14): the routing read below must see a
        // routing change that committed while this waited for the lock.
        let tx = PgPipelineStore::read_committed_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&pipeline_receipt_lock(
                tenant_id,
                &attempt.request_idempotency_key,
            )],
        )
        .await?;
        let quota_lock = format!("pipeline-quota:{tenant_id}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 1))",
            &[&quota_lock],
        )
        .await?;

        if let Some(refused) = receipt_key_refusal(
            &tx,
            tenant_id,
            &attempt.request_idempotency_key,
            &attempt.request_content_hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(ReceiptStage::Refused(refused));
        }

        // The routing lock, shared, and the refusal of a new receipt. After
        // the key's own refusal, so a retry of an existing receipt is
        // answered in every routing state, and before anything is read or
        // written for this key, so a refusal stores nothing.
        if let Some(refused) = new_receipt_refusal_in(
            &tx,
            tenant_id,
            request.server_envelope.submission_id,
            self.unqualified_routing,
            true,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(ReceiptStage::Refused(refused));
        }

        // Bound bundle: the active bundle id (from pipeline_active_bundles
        // only -- the routing row decides whether a new receipt may start,
        // above, and selects no bundle), then construct it.
        let bundle_id: String = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await?
            .map(|row| row.get::<_, String>("bundle_id"))
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_BUNDLE_MISSING_LABEL))?;
        let package = load_bundle_from_transaction(&tx, tenant_id, &bundle_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_BUNDLE_MISSING_LABEL))?;
        let runnable = tx
            .query_opt(
                "SELECT runnable FROM pipeline_bundle_policy_status
                 WHERE tenant_id = $1 AND bundle_id = $2 AND phase = 'admission'",
                &[&tenant_id, &bundle_id],
            )
            .await?
            .map(|row| row.get::<_, bool>("runnable"))
            .unwrap_or(false);
        anyhow::ensure!(runnable, PIPELINE_POLICY_NOT_RUNNABLE_LABEL);
        let bundle = self
            .construct(package)
            .map_err(|error| anyhow::anyhow!(error))?;

        // A tombstone hit is a normal outcome, not an error: nothing has
        // been staged or stored.
        if receipt_is_tombstoned(
            &tx,
            tenant_id,
            request.server_envelope,
            &attempt.request_content_hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(ReceiptStage::Refused(PipelineReceiptResult::Tombstoned));
        }

        // Quota, only when the caller wants this receipt counted.
        if request.counts_toward_quota {
            let principal_ref_hash = sha256_prefixed(request.actor_principal_ref.as_bytes());
            if let Some(scope) = receipt_quota_refusal(
                &tx,
                request,
                &attempt.request_idempotency_key,
                &principal_ref_hash,
            )
            .await?
            {
                tx.commit().await?;
                return Ok(ReceiptStage::Refused(PipelineReceiptResult::QuotaExceeded(
                    scope,
                )));
            }
            tx.execute(
                "INSERT INTO pipeline_admission_usage (
                    tenant_id, request_idempotency_key, principal_ref_hash
                 ) VALUES ($1, $2, $3)
                 ON CONFLICT (tenant_id, request_idempotency_key) DO NOTHING",
                &[
                    &tenant_id,
                    &attempt.request_idempotency_key,
                    &principal_ref_hash,
                ],
            )
            .await?;
        }

        PgPipelineStore::stage_receipt_artifact(&tx, attempt).await?;
        tx.commit().await?;
        Ok(ReceiptStage::Staged { bundle_id, bundle })
    }

    /// The receipt's final transaction (`submit` step 7). It takes the
    /// receipt lock again, so it commits one attempt for the key at a time:
    /// the attempt that comes second finds the first one's run and returns
    /// `Replayed` rather than failing on the run's unique key or the
    /// submission id. With a source session
    /// (`PipelineReceiptRequest::source_session`), it then locks the
    /// session row and refuses a withdrawn session
    /// (`SourceSessionWithdrawn`). After the check for an existing run it
    /// takes the tenant's routing lock, shared, and repeats the refusal of a
    /// new receipt (`LegacyOwned`, `NotRouted`: `new_receipt_refusal_in`):
    /// the routing can change, and the legacy path can claim the id, while
    /// the object is written, and a routing change (`PipelineActivationStore`,
    /// exclusive) waits for this transaction, so no receipt commits after it.
    /// The routing lock comes after the receipt lock and the session row
    /// lock, and it is held to the commit. A refusal commits nothing, and
    /// `submit` discards the attempt's object and row; the
    /// `pipeline_admission_usage` row that the staging transaction inserted
    /// stays, so a receipt refused here is counted by the hourly quota. Then
    /// it re-checks the
    /// tombstones (one can arrive while the object is written), locks the
    /// attempt's row, which must still be `staged` and not due
    /// (`lock_committable_receipt_artifact`), takes the Admission policy row
    /// of the bound bundle `FOR SHARE` and refuses when the policy is no
    /// longer runnable (`ReceiptCommit::PolicyNotRunnable`: the policy can be
    /// suspended while the object is written, and an intervention waits for
    /// this transaction), and inserts the records and the row's move to
    /// `committed` together. The locks, in order: the receipt lock, the
    /// session row, the routing lock, the attempt's row, the policy row.
    ///
    /// Retention is derived on the server, as the legacy receipt derives it
    /// (`retention_policy_for_trace` over the envelope's allowed uses and
    /// consent; expiry `received_at + max_age_days`), never taken from the
    /// envelope's own `trace_card.retention_policy`. `received_at` is the
    /// column default, this transaction's `NOW()`, read here so the expiry
    /// is anchored to the exact stored receipt time.
    ///
    /// The records include the pipeline's `pipeline_receipt_ownership` row
    /// (`insert_receipt_records` inserts it first). When the legacy path
    /// claimed or recorded the id after the checks above (a conflict on the
    /// ownership row or the submission row with a legacy owner), the
    /// transaction rolls back -- its deferred foreign key to the run is never
    /// checked -- and the result is `LegacyOwned`. A conflict with a pipeline
    /// owner (a second key for an id a run owns) is not the legacy path's: it
    /// is `main`'s "already bound to another receipt" error, as before PR 5,
    /// and like every error from this transaction it leaves the attempt's
    /// object and `staged` row to the sweeper.
    pub(super) async fn commit_receipt_attempt(
        &self,
        request: &PipelineReceiptRequest<'_>,
        run: &NewPipelineRun,
        attempt: &ReceiptAttempt,
        size_bytes: usize,
        stored: StoredPhaseResult,
        admission: &AdmissionDecision,
    ) -> anyhow::Result<ReceiptCommit> {
        let tenant_id = request.tenant_id;
        let envelope = request.server_envelope;
        let mut client = self.backend.trace_pool().get().await?;
        // READ COMMITTED, stated (G14), as the staging transaction.
        let tx = PgPipelineStore::read_committed_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&pipeline_receipt_lock(
                tenant_id,
                &attempt.request_idempotency_key,
            )],
        )
        .await?;
        // Zaki review 1, item 5: the upload's source session, locked before
        // any row is read or written (the lock order stays session, then
        // run, then submission, as a withdrawal takes them). A session
        // withdrawal either committed before this lock -- the receipt sees
        // it and records nothing, as `main`'s receipt refuses it -- or waits
        // here until this receipt commits, and then finds the submission
        // mapped to the session and withdraws it.
        if let Some((account_id, session_digest)) = request.source_session {
            let withdrawn = tx
                .query_opt(
                    "SELECT withdrawn_at IS NOT NULL FROM trace_source_sessions
                      WHERE tenant_id = $1 AND account_id = $2 AND session_digest = $3
                      FOR UPDATE",
                    &[&tenant_id, &account_id, &session_digest.as_slice()],
                )
                .await?
                .is_some_and(|row| row.get::<_, bool>(0));
            if withdrawn {
                tx.commit().await?;
                return Ok(ReceiptCommit::Refused(
                    PipelineReceiptResult::SourceSessionWithdrawn,
                ));
            }
        }
        if let Some(existing) =
            existing_receipt_run(&tx, tenant_id, &attempt.request_idempotency_key).await?
        {
            tx.commit().await?;
            return Ok(ReceiptCommit::Refused(replay_result(
                existing,
                &attempt.request_content_hash,
            )));
        }
        // The routing lock, shared, and the refusal of a new receipt, again:
        // containment, a deactivation, or a legacy claim may have committed
        // since the staging transaction. After the existing-run check, so a
        // replay is answered in every routing state.
        if let Some(refused) = new_receipt_refusal_in(
            &tx,
            tenant_id,
            envelope.submission_id,
            self.unqualified_routing,
            true,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(ReceiptCommit::Refused(refused));
        }
        if receipt_is_tombstoned(&tx, tenant_id, envelope, &attempt.request_content_hash).await? {
            tx.commit().await?;
            return Ok(ReceiptCommit::Refused(PipelineReceiptResult::Tombstoned));
        }
        if !PgPipelineStore::lock_committable_receipt_artifact(&tx, attempt).await? {
            tx.commit().await?;
            return Ok(ReceiptCommit::StagingMissing);
        }
        // GRD-004: the Admission policy of the bundle the run is bound to must
        // still be runnable, and stays so until this transaction ends. The
        // staging transaction read the flag without a lock, and a suspension
        // can land while the object is written. This is the last lock before
        // the first write, after the receipt lock, the session row, the
        // routing lock, and the attempt's row; an intervention takes only the
        // policy row. A refusal commits nothing.
        if !lock_runnable_policy(&tx, tenant_id, &run.bundle_id, Phase::Admission).await? {
            tx.commit().await?;
            return Ok(ReceiptCommit::PolicyNotRunnable);
        }

        let retention_policy = retention_policy_for_trace(envelope);
        let received_at: DateTime<Utc> = tx.query_one("SELECT NOW()", &[]).await?.get(0);
        let expires_at = retention_policy
            .max_age_days
            .map(|days| received_at + Duration::days(i64::from(days)));
        let submission = TraceSubmissionWrite {
            tenant_id: tenant_id.to_string(),
            submission_id: envelope.submission_id,
            trace_id: envelope.trace_id,
            auth_principal_ref: request.actor_principal_ref.to_string(),
            contributor_pseudonym: envelope.contributor.pseudonymous_contributor_id.clone(),
            submitted_tenant_scope_ref: None,
            schema_version: envelope.schema_version.clone(),
            consent_policy_version: envelope.consent.policy_version.clone(),
            consent_scopes: enum_strings(&envelope.consent.scopes)?,
            allowed_uses: enum_strings(&envelope.trace_card.allowed_uses)?,
            retention_policy_id: retention_policy.name,
            status: TraceCorpusStatus::Received,
            privacy_risk: enum_string(&envelope.privacy.residual_pii_risk)?,
            residual_risk_basis: Some(safe_residual_risk_basis_labels(request.residual_risk_basis)),
            redaction_pipeline_version: envelope.privacy.redaction_pipeline_version.clone(),
            redaction_counts: envelope.privacy.redaction_counts.clone(),
            redaction_hash: envelope.privacy.redaction_hash.clone(),
            canonical_summary_hash: None,
            submission_score: None,
            credit_points_pending: None,
            credit_points_final: None,
            expires_at,
        };
        let tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id);
        let object_ref = TraceObjectRefWrite {
            object_ref_id: run.source_object_ref_id,
            tenant_id: tenant_id.to_string(),
            submission_id: envelope.submission_id,
            artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
            object_store: self.object_store_name.clone(),
            object_key: attempt.receipt.object_key.clone(),
            content_sha256: format!("sha256:{}", attempt.receipt.ciphertext_sha256),
            encryption_key_ref: format!("tenant:{}", tenant_storage_ref.as_str()),
            size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
            compression: None,
            created_by_job_id: None,
        };
        match insert_receipt_records(
            &tx,
            run,
            attempt,
            &submission,
            &object_ref,
            stored,
            admission,
        )
        .await
        {
            Ok(()) => {}
            Err(DatabaseError::Constraint(label))
                if label == PIPELINE_LEGACY_RECEIPT_OWNED_LABEL =>
            {
                // The legacy path took the id since the check above. Dropping
                // the transaction rolls it back, ownership row included, and
                // the deferred run foreign key is never checked.
                drop(tx);
                return Ok(ReceiptCommit::Refused(PipelineReceiptResult::LegacyOwned));
            }
            Err(error) => return Err(error.into()),
        }
        let row = tx
            .query_one(
                "SELECT * FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant_id, &run.run_id],
            )
            .await?;
        let created = pipeline_run_from_row(&row)?;
        tx.commit().await?;
        Ok(ReceiptCommit::Created(created))
    }

    /// Removes an attempt the final transaction refused: its object, then
    /// its `staged` row. Best effort, with the sweeper as the backstop: when
    /// the delete fails, the row stays (or is inserted again, if the
    /// sweeper already removed it) and is made due now, so the next sweep
    /// retries it. Logs labels only. The delete is synchronous I/O, so it
    /// runs on a blocking thread rather than this task.
    pub(super) async fn discard_receipt_attempt(&self, attempt: &ReceiptAttempt) {
        let tenant_storage_ref = pipeline_tenant_storage_ref(&attempt.tenant_id);
        let store = self.artifact_store.clone();
        let receipt = attempt.receipt.clone();
        let deleted = tokio::task::spawn_blocking(move || {
            store.delete_artifact(tenant_storage_ref.as_str(), &receipt)
        })
        .await;
        let cleanup = match deleted {
            Ok(Ok(_)) => self.store.remove_staged_receipt_artifact(attempt).await,
            Ok(Err(_)) | Err(_) => {
                tracing::warn!(
                    label = "pipeline_receipt_object_delete_failed",
                    "best-effort delete of a refused receipt attempt's object failed; \
                     the sweeper retries it"
                );
                self.store.restage_receipt_artifact_due_now(attempt).await
            }
        };
        if cleanup.is_err() {
            tracing::warn!(
                label = "pipeline_receipt_staging_cleanup_failed",
                "could not update a refused receipt attempt's staging row"
            );
        }
    }

    /// Deletes the objects of receipt attempts that never committed: in one
    /// tenant transaction, up to `limit` `staged` rows whose
    /// `cleanup_after` has passed, oldest first, each locked
    /// (`FOR UPDATE SKIP LOCKED`). For each, it deletes the object the row
    /// names and then the row. A delete that fails keeps its row for the
    /// next pass and logs a label only. Returns how many rows it removed.
    ///
    /// It never deletes an object a run references: an attempt's row moves
    /// to `committed` in the same transaction that inserts its object ref,
    /// under the row lock the sweeper skips, and a row this sweep holds is
    /// deleted before that transaction can lock it -- which then refuses
    /// (`ReceiptCommit::StagingMissing`). If this sweep fails after it
    /// deleted an object but before it commits, the row stays `staged` with
    /// its object gone; it is due, and the final transaction refuses a due
    /// row too (`lock_committable_receipt_artifact`). The object keys are
    /// unique per attempt (`UNIQUE (tenant_id, object_key)`).
    pub async fn sweep_staged_receipts(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT run_id, attempt_id, object_key, ciphertext_sha256, staged_at
                   FROM pipeline_receipt_artifacts
                  WHERE tenant_id = $1 AND state = 'staged' AND cleanup_after <= NOW()
                  ORDER BY cleanup_after, run_id, attempt_id
                  LIMIT $2
                  FOR UPDATE SKIP LOCKED",
                &[&tenant_id, &limit],
            )
            .await?;
        let mut removed = 0;
        for row in &rows {
            let run_id: Uuid = row.get("run_id");
            let attempt_id: Uuid = row.get("attempt_id");
            let receipt = EncryptedTraceArtifactReceipt {
                tenant_storage_ref: tenant_storage_ref.as_str().to_string(),
                artifact_kind: TraceArtifactKind::ContributionEnvelope,
                object_key: row.get("object_key"),
                ciphertext_sha256: row.get("ciphertext_sha256"),
                encrypted_at: row.get("staged_at"),
            };
            let store = self.artifact_store.clone();
            let delete_tenant_storage_ref = tenant_storage_ref.clone();
            if on_blocking_pool(move || {
                store.delete_artifact(delete_tenant_storage_ref.as_str(), &receipt)
            })
            .await
            .is_err()
            {
                tracing::warn!(
                    label = "pipeline_receipt_sweep_delete_failed",
                    "a staged receipt object could not be deleted; its row is kept \
                     for the next pass"
                );
                continue;
            }
            tx.execute(
                "DELETE FROM pipeline_receipt_artifacts
                  WHERE tenant_id = $1 AND run_id = $2 AND attempt_id = $3
                    AND state = 'staged'",
                &[&tenant_id, &run_id, &attempt_id],
            )
            .await?;
            removed += 1;
        }
        tx.commit().await?;
        Ok(removed)
    }

    /// How long a phase attempt's staged object outlives its own lease
    /// before the sweep may remove it: `PIPELINE_LEASE_RENEWAL_CAP_FACTOR`
    /// times `phase`'s configured lease -- the same bound
    /// `PipelineLeaseRenewal::start` caps a live renewal at, so an attempt
    /// still legitimately renewing its lease never has its own object swept
    /// out from under it -- plus one hour of margin for the commit itself to
    /// land once the phase work is done.
    pub(super) fn attempt_artifact_cleanup_after(&self, phase: Phase) -> DateTime<Utc> {
        self.attempt_artifact_lease_bound(phase) + PIPELINE_ATTEMPT_ARTIFACT_COMMIT_MARGIN
    }

    /// Now plus `PIPELINE_LEASE_RENEWAL_CAP_FACTOR` times `phase`'s lease:
    /// no lease claimed before now is still live after it, however often it
    /// is renewed. A compatibility Score that staged its rows before its
    /// tenant lock publishes nothing after this bound (rebase 10, option D):
    /// by then its lease is certainly gone and its commit would be refused,
    /// and the sweep may remove its rows `PIPELINE_ATTEMPT_ARTIFACT_COMMIT_MARGIN`
    /// later, so a later publish could leave an object no row names.
    pub(super) fn attempt_artifact_lease_bound(&self, phase: Phase) -> DateTime<Utc> {
        truncate_to_microseconds(
            Utc::now() + self.lease_config.for_phase(phase) * PIPELINE_LEASE_RENEWAL_CAP_FACTOR,
        )
    }

    /// Deletes the objects of pipeline phase attempts that never committed
    /// (PR 4): each `staged` row whose `cleanup_after` has passed -- an
    /// attempt that crashed, lost its lease, failed between two of its
    /// writes, or had its commit refused -- loses its object and then the
    /// row itself.
    ///
    /// Who owns which deletion (controller ruling R2-1): a `staged` row's
    /// object is named by no object ref, so no withdrawal ever deletes it;
    /// this sweep owns it. A `committed` row's object is an object ref of
    /// the submission, recorded by the same phase commit that committed the
    /// row (Review's approved revision, Score's index command and neighbour
    /// set), so deleting it belongs to the withdrawal: the withdrawal
    /// invalidates the ref and queues its payload deletion in the same
    /// transaction as the tombstone, and `main`'s revocation-propagation
    /// worker deletes it. This sweep never touches a `committed` row. A
    /// phase attempt whose commit is refused, for any reason, deletes the
    /// objects it wrote itself, best effort (Review its approved object,
    /// Score its index command and neighbour set; PR 3), and so does a Score
    /// attempt whose second write fails after its first; their `staged`
    /// rows stay, and this sweep later finds each object absent and drops
    /// the row, or deletes an object that path failed to delete. The
    /// objects only this sweep deletes are those of an attempt that stopped
    /// after writing and before any commit or refusal (a crashed process),
    /// and those a lost connection kept because the commit may have landed
    /// when it did not.
    ///
    /// One tenant transaction, `FOR UPDATE SKIP LOCKED`, removing up to
    /// `limit` rows. It reads the due rows oldest first, a page at a time,
    /// with a keyset cursor on `(cleanup_after, run_id, lease_token,
    /// artifact)`, so a row it keeps does not stop it: it goes on past kept
    /// rows to later ones, examining at most
    /// `PIPELINE_ATTEMPT_SWEEP_EXAMINED_PER_REMOVAL` times `limit` rows in
    /// one pass (wave 2; follow-up review, m2). The pass holds its pooled
    /// connection and the locks of the rows it read across every store call
    /// it makes, up to about two per row examined (fix round 1, review M6).
    /// For each row with a hash, a store that confirms the object
    /// absent (`Some(false)`) needs no delete; a store that reports it
    /// present, or cannot tell (`None`), gets one. A row with no hash (a
    /// compatibility Score staged it before its tenant lock and never
    /// committed it; rebase 10, option D) gets
    /// `delete_artifact_at_object_key` at its key, but only when the store
    /// derives that same key from the row's artifact, run and lease token
    /// (rebase 10 review, M1); a row whose key differs is kept, with its
    /// object, and logs `pipeline_attempt_sweep_key_mismatch`. A delete
    /// failure (or a presence check or key derivation that itself errors)
    /// logs `pipeline_attempt_sweep_delete_failed`, with the store's own
    /// refusal when that is a label (`store_refusal_label`), and keeps the
    /// row for the next pass; otherwise the row is deleted. A kept row keeps
    /// its `cleanup_after` (V108's guard moves nothing), so it is examined
    /// again on each pass. Every store call runs on the blocking pool.
    /// The run's own `pipeline_runs` row, and its committed objects, are
    /// never touched here. Returns how many rows it removed.
    ///
    /// This pass starts at the oldest due row; see
    /// [`Self::sweep_attempt_artifacts_from`] for a pass that resumes where
    /// an earlier one stopped.
    pub async fn sweep_attempt_artifacts(
        &self,
        tenant_id: &str,
        limit: usize,
    ) -> anyhow::Result<usize> {
        Ok(self
            .sweep_attempt_artifacts_from(tenant_id, limit, None)
            .await?
            .removed)
    }

    /// `sweep_attempt_artifacts`, starting after `resume_after` (the due
    /// rows before it wait for a later pass) instead of at the oldest due
    /// row. The pass's `resume_after` is where the next pass resumes: the
    /// last row it examined when it stopped at its limit or its bound, and
    /// `None` when it reached the end of the due rows, so the next pass
    /// starts over at the oldest. The worker carries it from one pass to the
    /// next for each tenant (wave 2; follow-up review, m2): however many
    /// rows a tenant's sweep keeps, every due row is examined within a few
    /// passes, so kept rows never stall the tenant.
    pub async fn sweep_attempt_artifacts_from(
        &self,
        tenant_id: &str,
        limit: usize,
        resume_after: Option<AttemptSweepCursor>,
    ) -> anyhow::Result<AttemptSweepPass> {
        let examine_bound = limit.saturating_mul(PIPELINE_ATTEMPT_SWEEP_EXAMINED_PER_REMOVAL);
        let tenant_storage_ref = pipeline_tenant_storage_ref(tenant_id);
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, tenant_id).await?;

        let mut cursor = resume_after;
        let mut removed = 0usize;
        let mut examined = 0usize;
        let mut reached_the_end = false;
        'pages: while removed < limit && examined < examine_bound {
            let page_size = limit.min(examine_bound - examined);
            let rows = tx
                .query(
                    "SELECT run_id, lease_token, artifact, object_key, ciphertext_sha256,
                            cleanup_after
                       FROM pipeline_attempt_artifacts
                      WHERE tenant_id = $1 AND state = 'staged' AND cleanup_after <= NOW()
                        AND ($3::TIMESTAMPTZ IS NULL
                             OR (cleanup_after, run_id, lease_token, artifact)
                                > ($3::TIMESTAMPTZ, $4::UUID, $5::UUID, $6::TEXT))
                      ORDER BY cleanup_after, run_id, lease_token, artifact
                      LIMIT $2
                      FOR UPDATE SKIP LOCKED",
                    &[
                        &tenant_id,
                        &i64::try_from(page_size).unwrap_or(i64::MAX),
                        &cursor.as_ref().map(|cursor| cursor.cleanup_after),
                        &cursor.as_ref().map(|cursor| cursor.run_id),
                        &cursor.as_ref().map(|cursor| cursor.lease_token),
                        &cursor.as_ref().map(|cursor| cursor.artifact.as_str()),
                    ],
                )
                .await?;
            let page_was_full = rows.len() == page_size;
            for row in &rows {
                if removed >= limit {
                    break 'pages;
                }
                examined += 1;
                let run_id: Uuid = row.get("run_id");
                let lease_token: Uuid = row.get("lease_token");
                let artifact: String = row.get("artifact");
                let object_key: String = row.get("object_key");
                let ciphertext_sha256: Option<String> = row.get("ciphertext_sha256");
                cursor = Some(AttemptSweepCursor {
                    cleanup_after: row.get("cleanup_after"),
                    run_id,
                    lease_token,
                    artifact: artifact.clone(),
                });
                // The column is CHECK-constrained to the four known artifacts,
                // so this should never fire; if it ever does (a future migration
                // loosens the constraint, or direct DB tampering), fail this
                // sweep pass closed -- an error the caller logs and retries next
                // pass -- rather than panic the worker's supervised task.
                let attempt_artifact =
                    PipelineAttemptArtifact::from_db_str(&artifact).ok_or_else(|| {
                        anyhow::anyhow!("pipeline_attempt_artifact_kind_unrecognized")
                    })?;
                let artifact_kind = attempt_artifact.store_kind();

                // The store calls are synchronous, so they run on the blocking
                // pool while this transaction keeps the rows locked, as every
                // other object-store call of the pipeline does (PR 3, e2873401,
                // N-6). A blocking task that did not return counts as a failed
                // delete: the row stays for the next pass.
                let store = self.artifact_store.clone();
                let tenant = tenant_storage_ref.clone();
                let key = object_key.clone();
                let outcome = on_blocking_pool(move || {
                    Ok(match ciphertext_sha256 {
                        // Rebase 10, option D: a row a compatibility Score staged
                        // before its tenant lock and never committed has no
                        // hash. Its object, if the attempt published one, is at
                        // the key the store derives from the row's own artifact,
                        // run and lease token (`pipeline_attempt_object_id`), and
                        // the lease token is the attempt's own, so no other
                        // object is ever stored there. The sweep derives that key
                        // again and deletes only when the row names it (rebase
                        // 10 review, M1): a row whose key names another object is
                        // kept, and that object is never deleted by key alone.
                        // `Ok(false)` from the delete is an attempt that
                        // published nothing.
                        None => match store.serialized_json_object_key(
                            tenant.as_str(),
                            artifact_kind.clone(),
                            &pipeline_attempt_object_id(
                                attempt_artifact.as_str(),
                                run_id,
                                lease_token,
                            ),
                        ) {
                            Ok(derived) if derived == key => {
                                match store.delete_artifact_at_object_key(
                                    tenant.as_str(),
                                    artifact_kind,
                                    &key,
                                ) {
                                    Ok(_) => AttemptSweepRow::Remove,
                                    Err(error) => AttemptSweepRow::Keep(
                                        PIPELINE_ATTEMPT_SWEEP_DELETE_FAILED,
                                        store_refusal_label(&error),
                                    ),
                                }
                            }
                            Ok(_) => {
                                AttemptSweepRow::Keep(PIPELINE_ATTEMPT_SWEEP_KEY_MISMATCH, None)
                            }
                            Err(error) => AttemptSweepRow::Keep(
                                PIPELINE_ATTEMPT_SWEEP_DELETE_FAILED,
                                store_refusal_label(&error),
                            ),
                        },
                        Some(ciphertext_sha256) => {
                            let present = store.artifact_present_by_object_key(
                                tenant.as_str(),
                                artifact_kind.clone(),
                                &key,
                                &ciphertext_sha256,
                            );
                            // `Some(false)` -- confirmed absent -- is the only
                            // answer that skips the delete outright.
                            // `Some(true)` and `None` ("the store cannot tell",
                            // the trait's documented default, and what
                            // `GcsRemoteTraceArtifactProvider` answers today
                            // since it does not override this method) both
                            // attempt the delete: a store that cannot report
                            // presence may still hold the object, and a store
                            // whose delete errors on an absent object keeps the
                            // row for the next pass rather than mistaking
                            // "unknown" for "gone".
                            let deleted = match present {
                                Ok(Some(false)) => Ok(()),
                                Ok(Some(true)) | Ok(None) => {
                                    let receipt = EncryptedTraceArtifactReceipt {
                                        tenant_storage_ref: tenant.as_str().to_string(),
                                        artifact_kind,
                                        object_key: key,
                                        ciphertext_sha256,
                                        encrypted_at: Utc::now(),
                                    };
                                    store.delete_artifact(tenant.as_str(), &receipt).map(|_| ())
                                }
                                Err(error) => Err(error),
                            };
                            match deleted {
                                Ok(()) => AttemptSweepRow::Remove,
                                Err(error) => AttemptSweepRow::Keep(
                                    PIPELINE_ATTEMPT_SWEEP_DELETE_FAILED,
                                    store_refusal_label(&error),
                                ),
                            }
                        }
                    })
                })
                .await
                .unwrap_or(AttemptSweepRow::Keep(
                    PIPELINE_ATTEMPT_SWEEP_DELETE_FAILED,
                    None,
                ));
                if let AttemptSweepRow::Keep(label, store_label) = outcome {
                    if label == PIPELINE_ATTEMPT_SWEEP_KEY_MISMATCH {
                        tracing::warn!(
                            label,
                            "a pipeline attempt row names an object key other than its own; \
                         the row and that object are kept"
                        );
                    } else {
                        tracing::warn!(
                            label,
                            store_label = store_label.as_deref(),
                            "a pipeline attempt object could not be deleted; its row is kept \
                         for the next pass"
                        );
                    }
                    continue;
                }
                tx.execute(
                    "DELETE FROM pipeline_attempt_artifacts
                  WHERE tenant_id = $1 AND run_id = $2 AND lease_token = $3 AND artifact = $4
                    AND state = 'staged'",
                    &[&tenant_id, &run_id, &lease_token, &artifact],
                )
                .await?;
                removed += 1;
            }
            if !page_was_full {
                reached_the_end = true;
                break;
            }
        }
        tx.commit().await?;
        Ok(AttemptSweepPass {
            removed,
            resume_after: if reached_the_end { None } else { cursor },
        })
    }

    /// Loads a run's committed outcome for `phase` and decodes its decision.
    pub(super) async fn committed_decision<T: serde::de::DeserializeOwned>(
        &self,
        run: &PipelineRunRecord,
        phase: Phase,
    ) -> anyhow::Result<T> {
        let outcome = self
            .store
            .outcome_for_phase(&run.tenant_id, run.run_id, phase)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{phase:?} outcome is missing"))?;
        serde_json::from_value(outcome.decision)
            .map_err(|_| anyhow::anyhow!("{phase:?} outcome is malformed"))
    }

    /// Loads a run's committed outcome for `phase` and decodes its evidence.
    /// Alongside `committed_decision`, which decodes the same outcome's
    /// decision instead.
    pub(super) async fn committed_evidence<T: serde::de::DeserializeOwned>(
        &self,
        run: &PipelineRunRecord,
        phase: Phase,
    ) -> anyhow::Result<T> {
        let outcome = self
            .store
            .outcome_for_phase(&run.tenant_id, run.run_id, phase)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{phase:?} outcome is missing"))?;
        serde_json::from_value(outcome.evidence)
            .map_err(|_| anyhow::anyhow!("{phase:?} outcome is malformed"))
    }

    /// Reads and decrypts the bytes stored at `object_ref_id` for `run`,
    /// under the port's operability predicate (the submission must not be
    /// revoked/expired/purged or have a `trace_withdrawals` row, the object
    /// ref must not be invalidated or deleted), inside one tenant-scoped
    /// `FOR SHARE` transaction, then
    /// decodes the P1 wrapper back to the exact bytes. Shared by
    /// `load_source_bytes` and `load_approved_bytes`, which differ only in
    /// which object ref id they read and what they do with the bytes
    /// afterward.
    pub(super) async fn load_object_bytes(
        &self,
        run: &PipelineRunRecord,
        object_ref_id: Uuid,
    ) -> anyhow::Result<Vec<u8>> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
        let object_ref = tx
            .query_opt(
                concat!(
                    "SELECT object_ref.object_key, object_ref.content_sha256
                       FROM trace_submissions s
                       JOIN trace_object_refs object_ref
                         ON object_ref.tenant_id = s.tenant_id
                        AND object_ref.submission_id = s.submission_id
                      WHERE s.tenant_id = $1
                        AND s.submission_id = $2
                        AND object_ref.object_ref_id = $3
                        AND s.status NOT IN ('revoked', 'expired', 'purged')
                        AND ",
                    live_submission_sql!(),
                    " AND object_ref.invalidated_at IS NULL
                        AND object_ref.deleted_at IS NULL
                      FOR SHARE OF s, object_ref"
                ),
                &[&run.tenant_id, &run.submission_id, &object_ref_id],
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_SUBMISSION_INOPERABLE_LABEL))?;
        let object_key: String = object_ref.get("object_key");
        let content_sha256: String = object_ref.get("content_sha256");
        let ciphertext_sha256 = content_sha256
            .strip_prefix("sha256:")
            .ok_or_else(|| anyhow::anyhow!("object artifact hash is malformed"))?;
        let tenant_storage_ref = pipeline_tenant_storage_ref(&run.tenant_id);
        // The read keeps the rows locked `FOR SHARE`, so the object cannot be
        // deleted under it, and runs on the blocking pool (N-6).
        let store = self.artifact_store.clone();
        let ciphertext_sha256 = ciphertext_sha256.to_string();
        let wrapper = artifact_store_call(move || {
            store.read_json_by_object_key(
                tenant_storage_ref.as_str(),
                TraceArtifactKind::ContributionEnvelope,
                &object_key,
                &ciphertext_sha256,
            )
        })
        .await?;
        tx.commit().await?;
        decode_pipeline_artifact_bytes(&wrapper)
    }

    /// Reads the source envelope bytes the receipt staged at Admission,
    /// decoding the P1 wrapper back to the exact bytes Task 9 stored.
    pub async fn load_source_bytes(&self, run: &PipelineRunRecord) -> anyhow::Result<Vec<u8>> {
        self.load_object_bytes(run, run.source_object_ref_id).await
    }

    /// Reads the privacy pass object recorded on `run`, through the same
    /// operability-guarded read as the source, and requires the decoded
    /// bytes to hash to the `privacy_pass_content_hash` the pass recorded
    /// (`artifact_integrity_failed` otherwise, as for any stored object that
    /// no longer matches its record).
    pub(super) async fn load_privacy_pass_bytes(
        &self,
        run: &PipelineRunRecord,
    ) -> anyhow::Result<Vec<u8>> {
        let (Some(object_ref_id), Some(content_hash)) = (
            run.privacy_pass_object_ref_id,
            run.privacy_pass_content_hash.as_deref(),
        ) else {
            anyhow::bail!(PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL);
        };
        let bytes = self.load_object_bytes(run, object_ref_id).await?;
        anyhow::ensure!(
            sha256_prefixed(&bytes) == content_hash,
            PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL
        );
        Ok(bytes)
    }

    /// The bound on one privacy pass classifier call, fixed when the
    /// service was built (`privacy_pass_timeout_for`, decision P8).
    pub(super) fn privacy_pass_timeout(&self) -> std::time::Duration {
        self.privacy_pass_timeout
    }

    /// The Review-start privacy pass (spec 2026-10-09): returns the run as
    /// recorded and the bytes Review reads.
    ///
    /// A run whose pass is recorded reads the recorded object back and makes
    /// no classifier call: a pass is recorded once per run and never re-run
    /// (decision P7). Otherwise the pass loads the source (the receipt's
    /// post-deterministic envelope), runs the boundary's classifier half on
    /// it under `privacy_pass_timeout`, and merges the receipt's basis with
    /// the classifier's conditions. It escalates only when the classified
    /// risk is High and strictly above the receipt-time risk, both on
    /// Admission's scale (`pipeline_privacy_risk`, `privacy_pass_outcome`,
    /// decision P3 as amended 2026-10-10): a Medium from PII the classifier
    /// found and removed clears, and Review reads the redacted output. It
    /// stores the output under
    /// this attempt's own key (staged as `privacy-pass`, so the sweep deletes
    /// it if nothing commits it), then records the pass on the run
    /// (`record_privacy_pass`). A refused record deletes this attempt's
    /// object. The classifier call can run more than once for one run (a
    /// crash before the record, or a lease lost mid-call); exactly one result
    /// is recorded.
    ///
    /// Fail-closed: no boundary is `privacy_control_missing` (an uncharged
    /// wait, decision P6), and Review is never run on the unclassified
    /// source; a classifier error or timeout is the permanent `PolicyError`
    /// `privacy_classification_failed` (a charged retry on the legacy
    /// backoff that fails the run under that label once its attempts are
    /// spent, `PgPipelineStore::mark_retry`), never a fall back to the
    /// deterministic result.
    pub(super) async fn ensure_privacy_pass(
        &self,
        run: &PipelineRunRecord,
    ) -> anyhow::Result<(PipelineRunRecord, Vec<u8>)> {
        if run.privacy_pass_object_ref_id.is_some() {
            let bytes = self.load_privacy_pass_bytes(run).await?;
            return Ok((run.clone(), bytes));
        }
        let classification_failed = || -> anyhow::Error {
            match PolicyError::permanent(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL) {
                Ok(error) => error.into(),
                Err(_) => anyhow::anyhow!(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL),
            }
        };
        let privacy = self
            .privacy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_PRIVACY_CONTROL_MISSING_LABEL))?;
        let source = self.load_source_bytes(run).await?;
        let source_hash = sha256_prefixed(&source);
        let mut envelope: TraceContributionEnvelope = serde_json::from_slice(&source)
            .map_err(|_| anyhow::anyhow!(PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL))?;
        let receipt = self
            .store
            .load_submission_receipt_privacy(&run.tenant_id, run.submission_id)
            .await?;
        let receipt_basis = receipt
            .residual_risk_basis
            .iter()
            .map(|label| ResidualRiskCondition::from_label(label))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(classification_failed)?;
        let classifier_basis = match tokio::time::timeout(
            self.privacy_pass_timeout(),
            privacy.rescrub_classifier(&mut envelope),
        )
        .await
        {
            Ok(Ok(basis)) => basis,
            Ok(Err(_)) | Err(_) => return Err(classification_failed()),
        };
        let mut merged = receipt_basis.clone();
        for condition in classifier_basis {
            if !merged.contains(&condition) {
                merged.push(condition);
            }
        }
        let receipt_risk = pipeline_privacy_risk(&receipt.residual_pii_risk, &receipt_basis);
        let risk = pipeline_privacy_risk(&envelope.privacy.residual_pii_risk, &merged);
        let outcome = privacy_pass_outcome(receipt_risk, risk);
        let basis_labels = safe_residual_risk_basis_labels(&merged);

        let bytes = serde_json::to_vec(&envelope)?;
        let content_hash = sha256_prefixed(&bytes);
        let wrapper = encode_pipeline_artifact_bytes(&bytes)?;
        let object_id = pipeline_attempt_object_id(
            PipelineAttemptArtifact::PrivacyPass.as_str(),
            run.run_id,
            required_lease_token(run)?,
        );
        // Prepared, staged, then published, both object-store calls on the
        // blocking pool (N-6), as the Review arm stores the approved object.
        let store = self.artifact_store.clone();
        let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
        let prepared = artifact_store_call(move || {
            store.prepare_serialized_json(
                tenant.as_str(),
                TraceArtifactKind::ContributionEnvelope,
                &object_id,
                &wrapper,
            )
        })
        .await?;
        self.store
            .stage_attempt_artifact(
                run,
                PipelineAttemptArtifact::PrivacyPass,
                &prepared.receipt().object_key,
                &prepared.receipt().ciphertext_sha256,
                self.attempt_artifact_cleanup_after(Phase::Review),
            )
            .await?;
        let store = self.artifact_store.clone();
        let written = artifact_store_call(move || store.publish_serialized_json(&prepared)).await?;
        self.inject_crash(PipelineCrashPoint::AfterPrivacyPassArtifactStorage)?;

        let object_ref =
            privacy_pass_object_ref(run, &written, bytes.len(), &self.object_store_name);
        let recorded = self
            .store
            .record_privacy_pass(
                run,
                PrivacyPassRecord {
                    object_ref: &object_ref,
                    ciphertext_sha256: &written.ciphertext_sha256,
                    content_hash: &content_hash,
                    source_hash: &source_hash,
                    basis_labels: &basis_labels,
                    outcome,
                    residual_pii_risk: envelope.privacy.residual_pii_risk,
                    redaction_counts: &envelope.privacy.redaction_counts,
                    redaction_pipeline_version: &envelope.privacy.redaction_pipeline_version,
                },
            )
            .await;
        let recorded = match recorded {
            Ok(recorded) => recorded,
            Err(error) => {
                // Nothing names this attempt's object: delete it, best
                // effort, as a refused Review commit deletes its approved
                // object. A failed delete leaves it to the sweep.
                if phase_commit_refused(&error) {
                    self.delete_attempt_objects(
                        &run.tenant_id,
                        std::iter::once(&written),
                        "privacy_pass_object_delete_failed",
                    )
                    .await;
                }
                return Err(match error {
                    DatabaseError::Constraint(label)
                        if label == PIPELINE_SUBMISSION_INOPERABLE_LABEL
                            || label == PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL =>
                    {
                        anyhow::anyhow!(label)
                    }
                    error => error.into(),
                });
            }
        };
        self.inject_crash(PipelineCrashPoint::AfterPrivacyPassCommit)?;
        Ok((recorded, bytes))
    }

    /// Reads the approved-content object the run's Review commit wrote,
    /// decoding the P1 wrapper and requiring the decoded bytes hash to the
    /// `approved_content_hash` the run committed -- never trusting a
    /// re-serialization of whatever the store handed back.
    pub async fn load_approved_bytes(&self, run: &PipelineRunRecord) -> anyhow::Result<Vec<u8>> {
        let approved_object_ref_id = run
            .approved_object_ref_id
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_SUBMISSION_INOPERABLE_LABEL))?;
        let approved_content_hash = run
            .approved_content_hash
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_SUBMISSION_INOPERABLE_LABEL))?;
        let bytes = self.load_object_bytes(run, approved_object_ref_id).await?;
        anyhow::ensure!(
            sha256_prefixed(&bytes) == approved_content_hash,
            "approved_content_mismatch"
        );
        Ok(bytes)
    }

    /// Reads and validates the sealed index command a committed Score
    /// outcome stored, per decision P1 (the byte wrapper) and the runtime
    /// plan's ruling A7. `evidence` is the same Score outcome's own
    /// evidence; `None` when Score proposed no command
    /// (`embedding_artifact_hash` absent). Any failure -- a failure the
    /// store reports, an integrity failure included
    /// (`index_command_unreadable`), or a missing or malformed reference, a
    /// decode failure, or a mismatch against the evidence or the run's own
    /// recorded hash/revision (`index_command_invalid`) -- is a safe label.
    pub async fn load_index_command(
        &self,
        run: &PipelineRunRecord,
        evidence: &ScoreEvidence,
    ) -> anyhow::Result<Option<SealedIndexCommand>> {
        let Some(expected_hash) = evidence.embedding_artifact_hash.as_deref() else {
            return Ok(None);
        };
        let (Some(stored), Some(run_hash), Some(revision_id)) = (
            run.index_command_ref.as_deref(),
            run.index_command_hash.as_deref(),
            run.approved_revision_id,
        ) else {
            anyhow::bail!("index_command_invalid");
        };
        let command = self
            .read_index_command(&run.tenant_id, stored, run_hash, revision_id)
            .await?;
        anyhow::ensure!(
            run_hash == expected_hash
                && Some(command.index_id()) == evidence.index_id.as_deref()
                && Some(command.model_id()) == evidence.embedder_model_id.as_deref(),
            "index_command_invalid"
        );
        Ok(Some(command))
    }

    /// Reads the index command a run committed at Score from its stored ref
    /// (`object_key#ciphertext_sha256`) and checks that it hashes to
    /// `command_hash` and names `revision_id`. Each failure the store
    /// reports is the safe label `index_command_unreadable`, an integrity
    /// failure included (the object missing, corrupt, bound to another
    /// tenant, kind or hash, or not decrypted under the loaded key): a store
    /// fault (a root that is not mounted, a wrong key, a wrong bucket) reads
    /// the same, and gets the hourly wait (plan RB-D6 as the owner decided it
    /// on 2026-10-10, review of #1331). Content that the store read, and
    /// that fails a later check (the byte wrapper, the decode, the hash, the
    /// revision), is `index_command_invalid`.
    pub(super) async fn read_index_command(
        &self,
        tenant_id: &str,
        stored_ref: &str,
        command_hash: &str,
        revision_id: Uuid,
    ) -> anyhow::Result<SealedIndexCommand> {
        let (object_key, ciphertext_sha256) = stored_ref
            .rsplit_once('#')
            .ok_or_else(|| anyhow::anyhow!("index_command_invalid"))?;
        let tenant = pipeline_tenant_storage_ref(tenant_id);
        let store = self.artifact_store.clone();
        let (object_key, ciphertext_sha256) =
            (object_key.to_string(), ciphertext_sha256.to_string());
        // Not `artifact_store_call`: a stored command that cannot be read is
        // a charged failure, not an uncharged suspension. A suspended Settle
        // run would keep the command in every compatibility Score's
        // unapplied set, which fails each of them closed
        // (`index_unavailable`), while a charged failure ends the run after
        // its Settle budget and the tenant recovers. The wait between
        // charged attempts is one hour (`mark_retry`).
        let wrapper = on_blocking_pool(move || {
            store.read_json_by_object_key(
                tenant.as_str(),
                TraceArtifactKind::VectorPayload,
                &object_key,
                &ciphertext_sha256,
            )
        })
        .await
        .map_err(|_| anyhow::anyhow!(PIPELINE_INDEX_COMMAND_UNREADABLE_LABEL))?;
        let bytes = decode_pipeline_artifact_bytes(&wrapper)
            .map_err(|_| anyhow::anyhow!("index_command_invalid"))?;
        let command = serde_json::from_slice::<SealedIndexCommand>(&bytes)
            .map_err(|_| anyhow::anyhow!("index_command_invalid"))?;
        let hash = command
            .content_hash()
            .map_err(|_| anyhow::anyhow!("index_command_invalid"))?;
        anyhow::ensure!(
            hash == command_hash && command.revision_id() == revision_id,
            "index_command_invalid"
        );
        Ok(command)
    }

    /// Rebuilds `writer`'s index for `tenant_id` from the sealed index
    /// commands Settle already wrote and committed -- no new outcome, no
    /// evaluation of any policy, and no credit. Meant for after a restore,
    /// once the pipeline's own rows are back but the vector index is a
    /// fresh, empty store: replays exactly what Settle already settled,
    /// through the same command validation (`load_index_command`, keyed
    /// against each run's own committed Score evidence) and the same key
    /// derivation Settle's own index dispatch uses
    /// (`SealedIndexCommand::keyed_entries`), so a rebuilt index matches
    /// what the pipeline actually settled, bit for bit.
    ///
    /// Reads `PgPipelineStore::list_rebuildable_index_runs` for the run
    /// list -- complete runs, included in the index, with a completed index
    /// write no invalidation has touched -- in `created_at, run_id` order,
    /// the same order Settle committed them in, then rebuilds each run with
    /// `rebuild_index_run`, which re-checks the run and its submission under
    /// the run's lock and holds both through that run's writes. A run
    /// withdrawn or invalidated after the listing is skipped
    /// (`skipped_run_count`), never written (final review I1).
    ///
    /// The rebuild holds a committed fence of its own (V113): one
    /// `fence_id`, which `rebuild_index_run` sets again before each run's
    /// writes. While it is unexpired, no worker in any process claims the
    /// tenant's invalidations (`PgPipelineStore::claim_due_index_invalidations`).
    /// When the rebuild returns `Ok`, or an error once every write it started
    /// has returned, it clears its fence. When a run fails as
    /// `index_unavailable` with an index call still in flight, the fence is
    /// left to expire, at that run's deadline plus the write fence margin. A
    /// clear that fails is logged and changes nothing else: the row expires
    /// on its own, and the next clear of the tenant deletes it.
    ///
    /// A run whose stored command fails validation (a tampered
    /// `index_command_hash`, or a mismatched revision, index, or model)
    /// fails the whole rebuild closed with `index_command_invalid` before
    /// writing anything from that run; entries an earlier run in the same
    /// pass already wrote are not rolled back, since a fail-closed rebuild
    /// is meant to be re-run after the operator resolves the tampered row,
    /// and every entry write is idempotent (`writer.upsert`'s `Unchanged`
    /// result) so a re-run never double-counts what an earlier attempt
    /// already applied. A stored command that the store cannot read (an
    /// outage, a missing object, a wrong key) fails the rebuild the same way
    /// with `index_command_unreadable` (`read_index_command`).
    pub async fn rebuild_index_from_authoritative_commands(
        &self,
        tenant_id: &str,
        writer: Arc<dyn IdentifiedIndexWriter>,
    ) -> anyhow::Result<PipelineIndexRebuildReport> {
        let fence_id = Uuid::new_v4();
        let rebuilt = self.rebuild_index_runs(tenant_id, &writer, fence_id).await;
        let write_in_flight = rebuilt
            .as_ref()
            .is_err_and(|error| error.is::<IndexRebuildWriteInFlight>());
        if !write_in_flight
            && self
                .store
                .clear_index_rebuild_fence(tenant_id, fence_id)
                .await
                .is_err()
        {
            tracing::warn!(
                label = "pipeline_index_rebuild_fence_clear_failed",
                "could not clear an index rebuild's fence; it expires on its own"
            );
        }
        rebuilt
    }

    /// The runs of `rebuild_index_from_authoritative_commands`, each under
    /// the fence `fence_id`.
    pub(super) async fn rebuild_index_runs(
        &self,
        tenant_id: &str,
        writer: &Arc<dyn IdentifiedIndexWriter>,
        fence_id: Uuid,
    ) -> anyhow::Result<PipelineIndexRebuildReport> {
        let runs = self.store.list_rebuildable_index_runs(tenant_id).await?;
        let mut command_hashes = Vec::with_capacity(runs.len());
        let mut entry_count = 0usize;
        let mut unchanged_entry_count = 0usize;
        let mut skipped_run_count = 0usize;
        for run in runs {
            match self.rebuild_index_run(&run, writer, fence_id).await? {
                PipelineIndexRebuildRun::Rebuilt {
                    command_hash,
                    entry_count: run_entries,
                    unchanged_entry_count: run_unchanged,
                } => {
                    entry_count += run_entries;
                    unchanged_entry_count += run_unchanged;
                    command_hashes.push(command_hash);
                }
                PipelineIndexRebuildRun::Skipped => skipped_run_count += 1,
            }
        }
        let command_set_hash = sha256_prefixed(
            trace_commons_protocol::canonical_json::to_canonical_vec(&serde_json::json!(
                command_hashes
            ))?
            .as_slice(),
        );
        Ok(PipelineIndexRebuildReport {
            command_count: command_hashes.len(),
            entry_count,
            unchanged_entry_count,
            skipped_run_count,
            command_set_hash,
        })
    }

    /// Rebuilds one listed run's entries into `writer`, under the guard
    /// (final review I1, the shape of Settle's index dispatch under P3-D9):
    /// one tenant transaction locks the run row, re-checks that the run is
    /// still rebuildable with the command hash it was listed with and that
    /// its submission is still operable
    /// (`PgPipelineStore::lock_rebuildable_index_run_on_tx`), and holds both
    /// locks through every upsert, then commits. A withdrawal locks the run
    /// row `FOR UPDATE` first, so it either committed before the re-check
    /// (the run is `Skipped`) or waits until these writes are done, and the
    /// invalidation it queues then removes them.
    ///
    /// Before that transaction opens, the rebuild sets its committed fence
    /// `fence_id` (`PgPipelineStore::set_index_rebuild_fence`, V113) to the
    /// run's deadline (below) plus the write fence margin
    /// (`PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS`), on the connection that
    /// then opens the transaction. While the fence is unexpired, no worker in
    /// any process claims the tenant's invalidations
    /// (`PgPipelineStore::claim_due_index_invalidations`, which says why one
    /// statement is enough). A fence write that fails stops the rebuild
    /// before the run's first write, as `index_rebuild_fence_unavailable`.
    ///
    /// The committed Score evidence and the sealed command are read before
    /// the fence and the transaction, so it never holds two pooled
    /// connections at once. The command's error is raised only once the
    /// re-check lets the write go ahead: a skipped run does not need its
    /// command, which the withdrawal's own object deletion may already have
    /// removed.
    ///
    /// The writes are synchronous index calls, so they run on the blocking
    /// pool while the transaction keeps both rows locked, as Settle's index
    /// writes do (PR 3, e2873401, N-6): the rebuild runs inside ingest's
    /// worker route and never parks a runtime worker the routes share.
    ///
    /// A blocking task is not cancelled when the future awaiting it is
    /// dropped. If this future is dropped while the writes run, the
    /// transaction rolls back and releases both locks, and the writes go on.
    /// The rebuild route therefore runs the rebuild in a task of its own, so
    /// a client disconnect does not drop it (review of the follow-up wave,
    /// m1), as Settle's index dispatch runs in a task that owns its
    /// transaction (PR 3, b14d25e9). The route also runs one rebuild per
    /// tenant at a time and its shutdown waits for running rebuilds with the
    /// worker's grace period (`PipelineIndexRebuilds` in ingest; Zaki's
    /// re-review of #1166, Low). Four cases still release the locks while a
    /// write goes on: a lost database session, a rebuild aborted past the
    /// shutdown grace period, the process exit, and a deadline that passes
    /// with a call in flight. In each, a withdrawal can commit and queue the
    /// run's invalidation at once, since the run is `complete` and holds no
    /// lease for Settle's fence to wait out. The committed fence closes
    /// these cases: no worker, in this process or in another replica,
    /// claims that invalidation until the fence has expired, and it expires
    /// only after the last write the deadline let start has returned, as
    /// long as each index call returns within the margin. A call that does
    /// not is awaited by a task of its own, which reopens the run's
    /// invalidation once it returns (issue #1233,
    /// `reopen_invalidation_after_index_writes`), so its entry is removed
    /// again even though the fence expired first. Withdrawals come
    /// from clients, from `main`'s retention maintenance and from the
    /// revocation-propagation reconciler, so the guarantee is not that none
    /// happens: it is that none is removed from the index before the
    /// rebuild's last write lands. A lost rebuild's fence delays the
    /// tenant's invalidations by at most the run deadline plus the margin
    /// (90 seconds at the defaults).
    ///
    /// The rows are held no longer than a deadline, as Settle's dispatch
    /// holds its rows (merge review I1; PR 3, 82d276c1): the smaller of
    /// Settle's configured lease and `PIPELINE_INDEX_DISPATCH_BUDGET_SECONDS`.
    /// Since PR 5 the deadline starts just before the fence write, not once
    /// both locks are held: the fence's end, which the database reads from
    /// its own clock after that, then reaches past the deadline plus the
    /// margin, and the lock wait counts against the deadline. A run whose
    /// lock wait outlasts the deadline fails as `index_unavailable` and
    /// writes nothing; a run never writes later than before. The deadline
    /// is monotonic (`std::time::Instant`), so a step of the application's
    /// wall clock cannot let an upsert start after the fence's end. No upsert
    /// starts past the deadline. At the deadline the transaction rolls back
    /// at once, so the rows are free, the rebuild waits at most the margin
    /// for the call in flight, and it fails closed as `index_unavailable`
    /// without writing the later runs. A rerun is safe: every upsert is
    /// idempotent. So a slow index holds a withdrawal of the run (and the
    /// session's uploads queued behind it) no longer than the deadline.
    ///
    /// What stays: ingest's rebuild route refuses a tenant its own process
    /// routes or drains (`pipeline_index_rebuild_tenant_active`, merge
    /// review M1), because a Score of that tenant must not read a partly
    /// rebuilt index. That refusal reaches only its own process, so the
    /// restore runbook still starts every process with both tenant lists
    /// unset for the rebuild (`backup-restore.md`, step 3).
    pub(super) async fn rebuild_index_run(
        &self,
        run: &PipelineRunRecord,
        writer: &Arc<dyn IdentifiedIndexWriter>,
        fence_id: Uuid,
    ) -> anyhow::Result<PipelineIndexRebuildRun> {
        let command = match self
            .committed_evidence::<ScoreEvidence>(run, Phase::Score)
            .await
        {
            Ok(evidence) => self.load_index_command(run, &evidence).await,
            Err(error) => Err(error),
        };
        let mut client = self.backend.trace_pool().get().await?;
        // Merge review I1, as Settle's dispatch (PR 3, 82d276c1): the rows
        // are held no longer than the smaller of Settle's lease and the
        // dispatch budget, and no upsert starts past that deadline. PR 5:
        // the deadline starts before the fence write, so the fence's end,
        // read from the database's clock after this, reaches past the
        // deadline plus the margin. The deadline is on the monotonic clock,
        // so a step of the wall clock cannot let an upsert start after the
        // fence's end.
        let run_deadline = self
            .lease_config
            .settle()
            .min(Duration::seconds(PIPELINE_INDEX_DISPATCH_BUDGET_SECONDS))
            .to_std()
            .unwrap_or_default();
        let deadline = std::time::Instant::now() + run_deadline;
        let margin = self.index_rebuild_fence_margin;
        PgPipelineStore::set_index_rebuild_fence_on(
            &mut client,
            &run.tenant_id,
            fence_id,
            run_deadline + margin,
        )
        .await
        .map_err(|_| anyhow::anyhow!(PIPELINE_INDEX_REBUILD_FENCE_UNAVAILABLE_LABEL))?;
        // READ COMMITTED, stated (G14): the re-check of the run, a statement
        // after the lock of the run row, must see an invalidation that
        // committed before that lock was granted.
        let tx =
            PgPipelineStore::read_committed_tenant_transaction(&mut client, &run.tenant_id).await?;
        if !PgPipelineStore::lock_rebuildable_index_run_on_tx(&tx, run).await? {
            tx.commit().await?;
            return Ok(PipelineIndexRebuildRun::Skipped);
        }
        let command = command?.ok_or_else(|| anyhow::anyhow!("index_command_invalid"))?;
        let command_hash = run
            .index_command_hash
            .clone()
            .ok_or_else(|| anyhow::anyhow!("index_command_invalid"))?;
        let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
        let entries = command
            .keyed_entries(&tenant)
            .map(|(key, entry)| (key, entry.embedding.clone(), entry.content_hash.clone()))
            .collect::<Vec<_>>();
        let writer = writer.clone();
        let mut writes = tokio::task::spawn_blocking(move || {
            let mut entry_count = 0usize;
            let mut unchanged_entry_count = 0usize;
            for (key, embedding, content_hash) in &entries {
                if std::time::Instant::now() >= deadline {
                    return Err(anyhow::anyhow!(PIPELINE_INDEX_UNAVAILABLE_LABEL));
                }
                match writer.upsert(key, embedding, content_hash) {
                    Ok(IndexUpsertResult::Inserted) => {}
                    Ok(IndexUpsertResult::Unchanged) => unchanged_entry_count += 1,
                    Err(error) => return Err(anyhow::anyhow!("index rebuild failed: {error}")),
                }
                entry_count += 1;
            }
            Ok((entry_count, unchanged_entry_count))
        });
        let budget = deadline.saturating_duration_since(std::time::Instant::now());
        let (entry_count, unchanged_entry_count) =
            match tokio::time::timeout(budget, &mut writes).await {
                Ok(joined) => {
                    joined.map_err(|_| anyhow::anyhow!(PIPELINE_BLOCKING_CALL_FAILED_LABEL))??
                }
                Err(_) => {
                    // Past the deadline: roll back at once, so the rows are
                    // free. A task of its own awaits the call in flight,
                    // however long it runs, and then reopens the run's
                    // invalidation (issue #1233,
                    // `reopen_invalidation_after_index_writes`). The rebuild
                    // waits for that task, bounded by the fence margin. A
                    // call that has not returned by then is reported as such,
                    // so the rebuild leaves its fence to expire rather than
                    // clearing it.
                    drop(tx);
                    drop(client);
                    let reopen = tokio::spawn(reopen_invalidation_after_index_writes(
                        self.store.clone(),
                        run.tenant_id.clone(),
                        run.run_id,
                        writes,
                    ));
                    if tokio::time::timeout(margin, reopen).await.is_err() {
                        return Err(anyhow::Error::new(IndexRebuildWriteInFlight));
                    }
                    return Err(anyhow::anyhow!(PIPELINE_INDEX_UNAVAILABLE_LABEL));
                }
            };
        tx.commit().await?;
        Ok(PipelineIndexRebuildRun::Rebuilt {
            command_hash,
            entry_count,
            unchanged_entry_count,
        })
    }

    /// Claims the next due run for `tenant_id` and advances it one phase.
    /// The claim itself picks the lease by the claimed row's own
    /// `next_phase`; see `PgPipelineStore::claim_next`. A background task
    /// renews that lease for as long as the phase actually runs (bounded by
    /// `PIPELINE_LEASE_RENEWAL_CAP_FACTOR`), so a phase slower than its own
    /// claim's lease is not mistaken for a crashed worker mid-run; the
    /// renewal task never runs inside this call's own transactions, and is
    /// always stopped before this call returns.
    pub async fn process_one(&self, tenant_id: &str) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(run) = self.store.claim_next(tenant_id, self.lease_config).await? else {
            return Ok(None);
        };
        let renewal = PipelineLeaseRenewal::start(self.store.clone(), &run, self.lease_config);
        let result = self.process_claimed_run(run).await;
        if let Some(renewal) = renewal {
            renewal.stop().await;
        }
        result
    }

    /// Claims a specific run and advances it one phase. The claim itself
    /// picks the lease by the row's own `next_phase` in SQL -- there is no
    /// separate read of the run before the claim, so a phase commit that
    /// lands concurrently can
    /// never hand out a lease sized for a phase the row is no longer in. As
    /// with `process_one`, a background task renews that lease for as long
    /// as the phase runs, stopped before this call returns.
    pub async fn process_run(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let Some(run) = self
            .store
            .claim_run_with_lease_config(tenant_id, run_id, self.lease_config)
            .await?
        else {
            return Ok(None);
        };
        let renewal = PipelineLeaseRenewal::start(self.store.clone(), &run, self.lease_config);
        let result = self.process_claimed_run(run).await;
        if let Some(renewal) = renewal {
            renewal.stop().await;
        }
        result
    }

    /// Loads the run's bound bundle and dispatches its current phase,
    /// mapping every outcome to a durable state change per decision P2. An
    /// injected crash propagates unchanged: the caller is expected to have
    /// simulated a real process crash, so nothing here gets to clean up
    /// after it.
    pub(super) async fn process_claimed_run(
        &self,
        run: PipelineRunRecord,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        let bundle = match self.load_bound_bundle(&run).await {
            Ok(bundle) => bundle,
            // D9 / FR3: a dependency the service does not hold, a bundle
            // whose operator-controlled runnable flag is off, or a store that
            // could not load the bound bundle is not this run's fault --
            // it waits in retry without the claim's attempt being charged,
            // with `mark_transient_retry`'s backoff.
            Err(label)
                if label == PIPELINE_DEPENDENCY_MISSING_LABEL
                    || label == PIPELINE_POLICY_NOT_RUNNABLE_LABEL
                    || label == PIPELINE_BUNDLE_STORE_UNAVAILABLE_LABEL =>
            {
                return self
                    .mark_transient_retry_or_record_lease_expired(&run, label)
                    .await;
            }
            Err(label) => {
                return self.mark_failed_or_record_lease_expired(&run, label).await;
            }
        };
        match self.process_claimed(&run, &bundle).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if error.to_string() == INJECTED_PIPELINE_CRASH => Err(error),
            // The phase itself found its
            // own lease already gone -- from a commit's `ensure_current_lease`
            // re-check, from a commit's plain `WHERE ... lease_token = $t AND
            // lease_expires_at > NOW()` finding no row, or from
            // `ensure_live_lease`'s service-level check before an
            // uncommittable external effect (the index write, a settlement
            // adapter dispatch). Recorded as its own uncharged reason ahead
            // of every other P2 branch below -- a Score or Settle phase slow
            // enough to outrun its lease (a chunked NEAR AI scorer, a
            // CPU-bound embedder) must never be mistaken for
            // `index_key_conflict`, an inoperable submission, a
            // typed `PolicyError`, a missing settlement adapter, or the
            // generic `minimal_policy_failed` retry.
            Err(error) if is_stale_lease_error(&error) => {
                Ok(self.store.record_lease_expired(&run).await?)
            }
            // A transient database failure (the pool
            // could not give a connection, the connection failed, or a
            // serialization failure or deadlock) is not the trace's fault in
            // any phase -- the uncharged suspension, ahead of every charged
            // P2 branch below. In Settle this covers a failure after
            // `adapter.settle` returned: the leg stays `leased`, and the next
            // attempt repeats the idempotent adapter call. If the database is
            // still down, recording the suspension fails too and its error
            // is returned: nothing can be recorded, the lease expires, and
            // the next claim (or its sweep) handles the run.
            Err(error) if is_transient_database_error(&error) => {
                self.mark_transient_retry_or_record_lease_expired(
                    &run,
                    PIPELINE_DATABASE_UNAVAILABLE_LABEL,
                )
                .await
            }
            // The tenant authority could not be read (a tenant whose policy
            // `main` reads from the database, and the read failed). It is
            // not the trace's fault either: the same uncharged suspension,
            // so an outage cannot spend the run's attempts and fail it, and
            // the retry reads the policy again. No leg settles without it:
            // the Settle reads run before the dispatch and before the
            // ledger transaction.
            Err(error) if error.to_string() == PIPELINE_AUTHORITY_READ_FAILED_LABEL => {
                self.mark_transient_retry_or_record_lease_expired(
                    &run,
                    PIPELINE_AUTHORITY_READ_FAILED_LABEL,
                )
                .await
            }
            Err(error) => {
                let label = error.to_string();
                if label == PIPELINE_INDEX_CONFLICT_LABEL {
                    return self
                        .mark_failed_or_record_lease_expired(&run, PIPELINE_INDEX_CONFLICT_LABEL)
                        .await;
                }
                // Spec 2026-10-08, Slice C: Settle's gate decision row
                // cannot be written -- the compatibility Score evidence lacks
                // a field the row needs, or the submission's pipeline row
                // names another run. Both are deterministic, so the run fails
                // terminally instead of spending its attempts. Settle checks
                // the evidence before any leg settles, and `mark_failed`
                // resolves only legs still open, so a completed leg stays
                // complete.
                if label == PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL
                    || label == PIPELINE_GATE_DECISION_CONFLICT_LABEL
                {
                    return self.mark_failed_or_record_lease_expired(&run, &label).await;
                }
                // A Score policy held a trace again after the operator
                // approved its hold. Deterministic for the same policy and
                // input, and nothing of the Score was committed: the run
                // fails terminally rather than going back to the queue.
                if label == PIPELINE_SCORE_HOLD_REPEATED_LABEL
                    && run.next_phase == Some(Phase::Score)
                {
                    return self
                        .mark_failed_or_record_lease_expired(
                            &run,
                            PIPELINE_SCORE_HOLD_REPEATED_LABEL,
                        )
                        .await;
                }
                // In Review and Score, an inoperable submission (revoked,
                // withdrawn, expired, or purged) is permanent -- the
                // condition that caused it can never reverse -- so the run
                // ends terminally here, on the attempt the claim charged,
                // rather than spending its budget in P2's ordinary charged
                // retry for this label and ending `attempts_exhausted`
                // (#1345). In Review this covers `commit_review`'s and
                // `record_privacy_pass`'s own refusals and the
                // `load_object_bytes` refusal `load_source_bytes` hits
                // before Review runs its policy; in Score, the same refusal
                // on the approved-bytes read (`load_approved_bytes`) and
                // `commit_score`'s re-check under its own transaction. Both
                // commits wrote nothing, so there is nothing to undo.
                //
                // Settle is not listed: it never raises this label. Its
                // guard forfeits the open legs under the withdrawal rules
                // (`withdrawal_forfeit_label`) and commits the run
                // `complete`, excluded from the index as
                // `submission_inoperable`. Failing a Settle run here would
                // instead forfeit its legs as `run_failed`
                // (`resolve_open_settlement_legs_on_tx`), so a Settle run
                // that ever raised it keeps the charged retry below.
                if label == PIPELINE_SUBMISSION_INOPERABLE_LABEL
                    && matches!(run.next_phase, Some(Phase::Review | Phase::Score))
                {
                    return self
                        .mark_failed_or_record_lease_expired(
                            &run,
                            PIPELINE_SUBMISSION_INOPERABLE_LABEL,
                        )
                        .await;
                }
                // GRD-004 / FR3: the phase's own commit found its bound policy
                // no longer runnable -- an operator suspended it after the
                // load-time check above passed. The commit wrote nothing, so
                // this is the same uncharged suspension with backoff as a
                // policy that was not runnable at load: the run keeps its
                // bundle and continues under it after the resume.
                if label == PIPELINE_POLICY_NOT_RUNNABLE_LABEL {
                    return self
                        .mark_transient_retry_or_record_lease_expired(
                            &run,
                            PIPELINE_POLICY_NOT_RUNNABLE_LABEL,
                        )
                        .await;
                }
                // D9 / FR3: a typed `PolicyError` raised while a
                // phase runs is budgeted by kind, ahead of the P2 string
                // allowlist below -- a transient failure (an outage or a
                // timeout) is the uncharged suspension with backoff, exactly
                // like a missing bound dependency; a Review quarantine
                // awaiting human assessment instead parks the run rather
                // than retrying it (below); a permanent policy failure is
                // charged like any other labeled retry.
                if let Some(policy) = error.downcast_ref::<PolicyError>() {
                    // A Score whose embedder refused the trace's own input
                    // would fail the same way on every attempt: the run
                    // fails terminally on the attempt the claim charged,
                    // like a repeated hold above, rather than spending its
                    // budget and ending `attempts_exhausted` under a label
                    // that no longer says why. Nothing of the Score was
                    // committed.
                    if !policy.is_transient()
                        && policy.label() == PIPELINE_SCORE_INPUT_UNEMBEDDABLE_LABEL
                        && run.next_phase == Some(Phase::Score)
                    {
                        return self
                            .mark_failed_or_record_lease_expired(
                                &run,
                                PIPELINE_SCORE_INPUT_UNEMBEDDABLE_LABEL,
                            )
                            .await;
                    }
                    return if policy.is_transient() {
                        // A Review quarantine
                        // waiting on a human assessment parks the run
                        // instead of joining `mark_transient_retry`'s
                        // hourly backoff -- the condition never resolves on
                        // its own, so retrying it costs a worker pass every
                        // hour for nothing. Every other transient
                        // `PolicyError`, in Review or any other phase,
                        // keeps the ordinary uncharged retry.
                        if (policy.label() == PIPELINE_REVIEW_ASSESSMENT_REQUIRED_LABEL
                            || policy.label() == PIPELINE_PRIVACY_PASS_REVIEW_REQUIRED_LABEL)
                            && run.next_phase == Some(Phase::Review)
                        {
                            self.mark_awaiting_review_or_record_lease_expired(&run, policy.label())
                                .await
                        } else {
                            self.mark_transient_retry_or_record_lease_expired(&run, policy.label())
                                .await
                        }
                    } else {
                        self.mark_retry_or_record_lease_expired(&run, policy.label())
                            .await
                    };
                }
                // Ruling FR3: a settlement adapter the service does not hold
                // is a deployment gap, not the trace's fault -- the same
                // uncharged suspension as a missing bound dependency. So is
                // a missing per-instrument cap, and so is a worker built
                // without the privacy boundary the Review-start privacy pass
                // needs (decision P6): Review never runs on the unclassified
                // source, and the run waits for a deployment that has one.
                // (An artifact store that
                // cannot derive a compatibility Score's object key, or
                // prepares the object under another key, is a failed store
                // call: the typed transient `PolicyError` above, from
                // `artifact_store_call`.)
                if let Some(gap) = [
                    PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL,
                    PIPELINE_SETTLEMENT_CAP_MISSING_LABEL,
                    PIPELINE_PRIVACY_CONTROL_MISSING_LABEL,
                ]
                .into_iter()
                .find(|gap| *gap == label)
                {
                    return self
                        .mark_transient_retry_or_record_lease_expired(&run, gap)
                        .await;
                }
                // The fixed allowlist from decision P2: a charged retry
                // (the attempt already taken by the claim stays charged)
                // labeled with the safe message the dispatch code raised.
                // Anything else -- including a raw, unlabeled error -- is a
                // charged retry under the generic operational label, never
                // the raw message itself.
                let retry_label = match label.as_str() {
                    "index_command_invalid"
                    | PIPELINE_INDEX_COMMAND_UNREADABLE_LABEL
                    | "artifact_integrity_failed"
                    | "approved_content_mismatch"
                    | "score_outcome_invalid"
                    | "settlement_operation_mismatch"
                    | "review_output_invalid"
                    | "submission_inoperable"
                    | PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL => label.as_str(),
                    _ => PIPELINE_OPERATIONAL_ERROR_LABEL,
                };
                self.mark_retry_or_record_lease_expired(&run, retry_label)
                    .await
            }
        }
    }

    /// The phase failed for `error_label`, a reason
    /// `process_claimed_run` charges as a terminal `mark_failed` -- unless
    /// the lease itself has gone stale between the phase's own failure and
    /// this call (the phase ran right up to its lease's edge), in which case
    /// the expiry is recorded instead and `error_label` is never charged.
    /// A Settle run resolves its open legs first (`fail_run`);
    /// a stale lease anywhere in that resolution is recorded the same way.
    pub(super) async fn mark_failed_or_record_lease_expired(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        match self.fail_run(run, error_label).await {
            Ok(()) => self
                .store
                .get_run(&run.tenant_id, run.run_id)
                .await
                .map_err(Into::into),
            Err(error) if is_stale_lease_error(&error) => {
                Ok(self.store.record_lease_expired(run).await?)
            }
            Err(error) => Err(error),
        }
    }

    /// The `mark_retry` shape of the same stale-lease-vs-charged-failure
    /// case -- see `mark_failed_or_record_lease_expired` and
    /// `charged_retry`.
    pub(super) async fn mark_retry_or_record_lease_expired(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        match self.charged_retry(run, error_label).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if is_stale_lease_error(&error) => {
                Ok(self.store.record_lease_expired(run).await?)
            }
            Err(error) => Err(error),
        }
    }

    /// The terminal `mark_failed` failure taken with a worker present. A
    /// Settle run first reconciles its dispatched external legs
    /// (`reconcile_dispatched_settlement_legs`); `mark_failed` then forfeits
    /// every other open leg in the transaction that fails the run.
    pub(super) async fn fail_run(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<()> {
        self.reconcile_dispatched_settlement_legs(run).await?;
        self.store.mark_failed(run, error_label).await?;
        Ok(())
    }

    /// P2's charged retry with a worker present. When this retry exhausts
    /// the run's attempts -- `mark_retry` then fails the run as
    /// `attempts_exhausted`, or as `privacy_classification_failed` under
    /// that label -- a Settle run first reconciles its dispatched
    /// external legs, and `mark_retry` forfeits every other
    /// open leg in the transaction that fails the run. The claim holds the
    /// lease, so no other writer moves `attempt_count` under it: the claimed
    /// record predicts the outcome `mark_retry`'s SQL decides.
    pub(super) async fn charged_retry(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<PipelineRunRecord> {
        if run.attempt_count >= run.max_attempts {
            self.reconcile_dispatched_settlement_legs(run).await?;
        }
        Ok(self.store.mark_retry(run, error_label).await?)
    }

    /// Before a worker fails a
    /// Settle run, each open, dispatched, external leg gets one more call to
    /// its idempotent adapter with the request Step 6 made. A receipt that
    /// answers the request completes the leg with its result reference and
    /// external receipt hash. Anything else records `failed` /
    /// `settlement_unreconciled`: a receipt that does not answer the request
    /// or that another leg already recorded, an adapter error, or no call at
    /// all -- for a missing adapter, a missing cap or an amount over it (the
    /// reconciling call never pays what Step 6 would refuse), a selection
    /// without the leg's result, a request that cannot be formed, or a
    /// submission that is no longer operable.
    ///
    /// A leg that is never dispatched again (`settlement_leg_is_open_to_dispatch`)
    /// gets no call: an external leg `failed` as `settlement_result_mismatch`
    /// is already resolved and stays `failed`, and a
    /// `settlement_request_conflict` or `settlement_request_rejected` leg is
    /// forfeited by the transaction that fails the run. (A Trace Credit leg
    /// gets no call either; that transaction forfeits it, a
    /// `settlement_result_mismatch` one included.)
    ///
    /// Each call is fenced by `ensure_live_lease` and each record by the run
    /// lease, so a stale lease here ends as `lease_expired`, never as a
    /// charge. A Trace Credit leg and an undispatched leg are left for the
    /// transaction that fails the run (`resolve_open_settlement_legs_on_tx`),
    /// which forfeits them. Not a Settle run: nothing to do.
    pub(super) async fn reconcile_dispatched_settlement_legs(
        &self,
        run: &PipelineRunRecord,
    ) -> anyhow::Result<()> {
        if run.next_phase != Some(Phase::Settle) {
            return Ok(());
        }
        let dispatched = self
            .store
            .list_settlements(&run.tenant_id, run.run_id)
            .await?
            .into_iter()
            .filter(|settlement| {
                settlement.dispatched_at.is_some()
                    && settlement.instrument_id != InstrumentId::trace_credit().as_str()
                    && settlement_leg_is_unresolved(
                        &settlement.instrument_id,
                        &settlement.operation_state,
                        settlement.last_error_label.as_deref(),
                    )
                    && settlement_leg_is_open_to_dispatch(
                        &settlement.operation_state,
                        settlement.last_error_label.as_deref(),
                    )
            })
            .collect::<Vec<_>>();
        if dispatched.is_empty() {
            return Ok(());
        }
        let operable = self.submission_guard(run).await?.operable;
        let expected_results = match self.store.load_settle_selection(run).await {
            Ok(selection) => selection
                .and_then(|selection| {
                    serde_json::from_value::<SettleDecision>(selection.decision).ok()
                })
                .map(|decision| {
                    decision
                        .settlement_operations()
                        .iter()
                        .filter_map(|operation| {
                            operation.result_ref_hash().map(|result_ref_hash| {
                                (
                                    operation.instrument_id().as_str().to_string(),
                                    result_ref_hash.to_string(),
                                )
                            })
                        })
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default(),
            // A selection that no longer decodes cannot form the request:
            // no reconciling call, so the leg is `settlement_unreconciled`.
            Err(DatabaseError::Serialization(_)) => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        for settlement in dispatched {
            let reconciled = if operable {
                self.reconciling_settle_call(run, &settlement, &expected_results)
                    .await?
            } else {
                None
            };
            match self
                .store
                .record_settlement_reconciliation(
                    run,
                    &settlement.instrument_id,
                    reconciled.as_ref(),
                )
                .await
            {
                Ok(()) => {}
                // One external receipt answers one leg: a receipt another
                // leg already recorded does not reconcile this one.
                Err(DatabaseError::Postgres(error))
                    if reconciled.is_some() && is_external_receipt_reuse(&error) =>
                {
                    self.store
                        .record_settlement_reconciliation(run, &settlement.instrument_id, None)
                        .await?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// The one reconciling adapter call for `settlement`, or `None` when it
    /// is not made or its receipt does not answer the request (see
    /// `reconcile_dispatched_settlement_legs`).
    pub(super) async fn reconciling_settle_call(
        &self,
        run: &PipelineRunRecord,
        settlement: &PipelineSettlementRecord,
        expected_results: &BTreeMap<String, String>,
    ) -> anyhow::Result<Option<SettlementReceipt>> {
        let Ok(instrument_id) = InstrumentId::new(settlement.instrument_id.clone()) else {
            return Ok(None);
        };
        let Some(adapter) = self.settlement_adapters.get(&instrument_id) else {
            return Ok(None);
        };
        let Some(expected_result_ref_hash) = expected_results.get(instrument_id.as_str()) else {
            return Ok(None);
        };
        let within_cap = self
            .caps
            .per_instrument_atomic_units
            .get(instrument_id.as_str())
            .is_some_and(|cap| settlement.atomic_units <= *cap);
        if !within_cap {
            return Ok(None);
        }
        let Ok(request) = SettlementRequest::new(
            pipeline_tenant_storage_ref(&run.tenant_id),
            run.run_id,
            instrument_id,
            settlement.atomic_units,
            settlement.operation_ref_hash.clone(),
            expected_result_ref_hash.clone(),
        ) else {
            return Ok(None);
        };
        self.ensure_live_lease(run).await?;
        Ok(match adapter.settle(&request).await {
            Ok(receipt) if receipt.answers(&request) => Some(receipt),
            Ok(_) | Err(_) => None,
        })
    }

    /// The `mark_transient_retry` shape of the same
    /// stale-lease-vs-charged-failure case -- see
    /// `mark_failed_or_record_lease_expired`.
    pub(super) async fn mark_transient_retry_or_record_lease_expired(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        match self.store.mark_transient_retry(run, error_label).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if is_stale_lease_db_error(&error) => {
                Ok(self.store.record_lease_expired(run).await?)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// The `mark_awaiting_review` shape of the same
    /// stale-lease-vs-charged-failure case -- see
    /// `mark_failed_or_record_lease_expired`.
    pub(super) async fn mark_awaiting_review_or_record_lease_expired(
        &self,
        run: &PipelineRunRecord,
        error_label: &str,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        match self.store.mark_awaiting_review(run, error_label).await {
            Ok(updated) => Ok(Some(updated)),
            Err(error) if is_stale_lease_db_error(&error) => {
                Ok(self.store.record_lease_expired(run).await?)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Runs the claimed run's current phase against its bound bundle.
    pub(super) async fn process_claimed(
        &self,
        run: &PipelineRunRecord,
        bundle: &MinimalPolicyBundle,
    ) -> anyhow::Result<PipelineRunRecord> {
        let phase = run
            .next_phase
            .ok_or_else(|| anyhow::anyhow!("claimed run has no phase"))?;
        match phase {
            Phase::Admission => anyhow::bail!("Admission cannot run asynchronously"),
            Phase::Review => {
                let admission = self
                    .committed_decision::<AdmissionDecision>(run, Phase::Admission)
                    .await?;
                // The server's privacy pass runs before the Review policy,
                // which reads its output, never the source. The run as
                // recorded carries the pass record `commit_review` checks
                // and links; its lease is the claim's.
                let (run, source_artifact) = self.ensure_privacy_pass(run).await?;
                let run = &run;
                let source_content_hash = sha256_prefixed(&source_artifact);
                // Ruling T3-5: loads whatever assessment a reviewer has
                // already recorded for this run, so `MinimalReviewPolicy`
                // (and any other Review policy) can resolve a quarantine
                // once `record_review_assessment` has moved it back to
                // `pending`. `None` for a run that has never been
                // quarantined, or one still waiting on a human assessment.
                let stored_assessment = self
                    .store
                    .load_review_assessment(&run.tenant_id, run.run_id)
                    .await?;
                // The pass escalated the run: the server holds it for a human
                // whatever Admission decided, and the policy is not called
                // until an assessment recorded after the pass exists. One
                // recorded before the pass (possible only for a run received
                // before V117) did not see what the pass found and is
                // ignored (critique 1 (2), decision P9). The transient
                // label parks the run (`mark_awaiting_review`).
                if run.privacy_pass_outcome == Some(PrivacyPassOutcome::Escalated) {
                    let current = stored_assessment.as_ref().is_some_and(|stored| {
                        run.privacy_pass_recorded_at
                            .is_some_and(|recorded_at| stored.recorded_at >= recorded_at)
                    });
                    if !current {
                        return Err(PolicyError::transient(
                            PIPELINE_PRIVACY_PASS_REVIEW_REQUIRED_LABEL,
                        )?
                        .into());
                    }
                }
                let human_assessment = stored_assessment.map(|stored| stored.assessment);
                let escalated = run.privacy_pass_outcome == Some(PrivacyPassOutcome::Escalated);
                // Q2, spec correction 3: `MinimalReviewPolicy` ignores the
                // assessment when Admission admitted, so it would approve an
                // escalated run a reviewer rejected. The server commits that
                // rejection itself, on the pass output, and never calls the
                // policy. An Admission-quarantined escalated run goes through
                // the policy, which rejects it as it does today.
                if escalated && admission == AdmissionDecision::Admit {
                    if let Some(assessment) = human_assessment.as_ref().filter(|assessment| {
                        assessment.recommendation == ReviewRecommendation::Reject
                    }) {
                        return self
                            .commit_privacy_pass_rejection(run, source_content_hash, assessment)
                            .await;
                    }
                }
                // Q2: the assessment that approves an escalated run is
                // linked on the pass record by `commit_review`.
                let pass_approval = human_assessment.clone().filter(|_| escalated);
                let output = bundle
                    .review
                    .execute(&ReviewInput {
                        run_id: run.run_id,
                        tenant_storage_ref: pipeline_tenant_storage_ref(&run.tenant_id),
                        trace_id: run.trace_id,
                        source_content_hash: source_content_hash.clone(),
                        source_artifact,
                        admission,
                        human_assessment,
                    })
                    .await?;
                let (result, content) = output.into_parts();
                let mut written_receipt = None;
                let approved = match (&result.decision, content) {
                    (
                        ReviewDecision::Approved {
                            registry_revision_id,
                        },
                        Some(content),
                    ) => {
                        anyhow::ensure!(
                            sha256_prefixed(content.bytes()) == content.content_hash(),
                            "review_output_invalid"
                        );
                        let object_id = pipeline_attempt_object_id(
                            PipelineAttemptArtifact::Approved.as_str(),
                            run.run_id,
                            required_lease_token(run)?,
                        );
                        let wrapper = encode_pipeline_artifact_bytes(content.bytes())?;
                        // Prepared, staged (PR 4), then published; both
                        // object-store calls run on the blocking pool (N-6).
                        let store = self.artifact_store.clone();
                        let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
                        let prepared = artifact_store_call(move || {
                            store.prepare_serialized_json(
                                tenant.as_str(),
                                TraceArtifactKind::ContributionEnvelope,
                                &object_id,
                                &wrapper,
                            )
                        })
                        .await?;
                        self.store
                            .stage_attempt_artifact(
                                run,
                                PipelineAttemptArtifact::Approved,
                                &prepared.receipt().object_key,
                                &prepared.receipt().ciphertext_sha256,
                                self.attempt_artifact_cleanup_after(Phase::Review),
                            )
                            .await?;
                        let store = self.artifact_store.clone();
                        let receipt =
                            artifact_store_call(move || store.publish_serialized_json(&prepared))
                                .await?;
                        self.inject_crash(PipelineCrashPoint::AfterReviewArtifactStorage)?;
                        written_receipt = Some(receipt.clone());
                        let (canonical_summary, canonical_summary_hash) =
                            approved_canonical_summary(content.bytes());
                        Some(ApprovedRevision {
                            revision_id: *registry_revision_id,
                            object_ref: approved_object_ref(
                                run,
                                &receipt,
                                content.bytes().len(),
                                &self.object_store_name,
                            ),
                            content_hash: content.content_hash().to_string(),
                            source_content_hash,
                            worker_identity: content.worker_identity().to_string(),
                            canonical_summary,
                            canonical_summary_hash,
                        })
                    }
                    (ReviewDecision::Rejected { .. }, None) => None,
                    _ => anyhow::bail!("review_output_invalid"),
                };
                let commit_result = self
                    .store
                    .commit_review(
                        run,
                        StoredPhaseResult::from_result(Phase::Review, &result)?,
                        approved,
                        pass_approval.as_ref(),
                    )
                    .await;
                let updated = match commit_result {
                    Ok(updated) => updated,
                    Err(error) => {
                        // Nothing committed, so no object ref names the
                        // approved object this attempt wrote under its own
                        // lease key: delete it, whatever refused the commit
                        // -- an inoperable submission, a stale lease, or
                        // anything else (Zaki review 1, round 2, finding 4).
                        // Best effort: a failed delete never masks the
                        // refusal.
                        if phase_commit_refused(&error) {
                            self.delete_attempt_objects(
                                &run.tenant_id,
                                written_receipt.iter(),
                                "review_approved_object_delete_failed",
                            )
                            .await;
                        }
                        return Err(review_commit_refusal(error));
                    }
                };
                self.inject_crash(PipelineCrashPoint::AfterReviewCommit)?;
                Ok(updated)
            }
            Phase::Score => self.commit_score_phase(run, bundle).await,
            Phase::Settle => self.complete_settle_phase(run, bundle).await,
        }
    }

    /// Commits a reviewer's rejection of an Admission-admitted run the
    /// privacy pass escalated (Q2), without calling the Review policy. The
    /// result mirrors `MinimalReviewPolicy`'s human rejection: it names the
    /// pass output it was made on (`source_content_hash`, the pass content
    /// hash), the assessment's `evidence_hash` and reason, and the server's
    /// rule id, `PIPELINE_PRIVACY_PASS_REJECTED_RULE_ID`. `commit_review`
    /// then rejects the submission and completes the run; no object was
    /// written, so a refused commit has nothing to delete.
    pub(super) async fn commit_privacy_pass_rejection(
        &self,
        run: &PipelineRunRecord,
        source_content_hash: String,
        assessment: &HumanReviewAssessment,
    ) -> anyhow::Result<PipelineRunRecord> {
        let result = PhaseResult {
            decision: ReviewDecision::Rejected {
                reason: assessment.reason.clone(),
            },
            evidence: ReviewEvidence {
                result_content_hash: source_content_hash.clone(),
                source_content_hash,
                content_changed: false,
                worker_identity: None,
                transformation_metadata_hash: None,
                human_assessment_hash: Some(assessment.evidence_hash.clone()),
                resolved_quarantine_reasons: Vec::new(),
            },
            evaluation: ReviewEvaluation {
                rule_id: PIPELINE_PRIVACY_PASS_REJECTED_RULE_ID.to_string(),
            },
        };
        // The contract's own check of a rejection's shape.
        let (result, _) = ReviewOutput::rejected(result)
            .map_err(|_| anyhow::anyhow!("review_output_invalid"))?
            .into_parts();
        let updated = self
            .store
            .commit_review(
                run,
                StoredPhaseResult::from_result(Phase::Review, &result)?,
                None,
                None,
            )
            .await
            .map_err(review_commit_refusal)?;
        self.inject_crash(PipelineCrashPoint::AfterReviewCommit)?;
        Ok(updated)
    }

    /// Runs Score over the approved bytes, stores the exact index command
    /// and neighbor artifact it proposed (each wrapped per decision P1),
    /// and commits the Score outcome together with one pending settlement
    /// operation per award (decision D5).
    ///
    /// A compatibility Score, and the Score of every policy a registry
    /// supplies (`score_runs_under_runtime_controls`), is serialized per
    /// tenant (Zaki review 1, round
    /// 2, finding 12, and the owner ruling of 2026-09-30). It opens a tenant
    /// transaction, takes the tenant's Score lock on it (a transaction-scoped
    /// advisory lock, `pipeline_compatibility_score_lock`) before it reads
    /// any neighbour, reads on it the index commands of the tenant's other
    /// runs that committed at Score and that Settle has not applied
    /// (`unapplied_index_commands_on_tx`), and commits on it
    /// (`commit_score_in`), so the lock is held from the neighbour read
    /// through the commit. The Score policy counts the unapplied entries as
    /// neighbours with the live index (`UnappliedIndexCommandsReader`). So
    /// of several near-duplicate traces received together the first is
    /// novel and each later one is a duplicate at Score: no award and no
    /// index command, as on `main`, whose gate inserts a trace's entries
    /// before it scores the next one. An unapplied command that cannot be
    /// read fails the Score closed (`index_unavailable`, uncharged), since
    /// leaving it out could credit a duplicate.
    ///
    /// Lock order: the tenant's Score lock first, then the run row
    /// (`ensure_current_lease`) and the submission row
    /// (`submission_guard_on_tx`) that the commit locks. No other path takes
    /// the Score lock, so none holds a run or submission row while it waits
    /// for it. The approved bytes are read before the transaction opens, so
    /// the Score holds one pooled connection at a time. The lock is tried,
    /// never waited for (multi-lens review L3-2): a Score that finds it held
    /// releases its run uncharged (`release_score_lock_busy`), and a lease
    /// that has passed stops the Score before it scores (uncharged, as a
    /// stale lease). A registered Score policy takes the same lock under the
    /// same name, since it reads the same tenant index. Only the minimal
    /// reference's Score takes no lock.
    ///
    /// Rebase 10, option D: a compatibility Score also stages its attempt
    /// rows (PR 4) before it opens that transaction
    /// (`stage_score_artifacts_before_lock`): one row for each artifact it
    /// can write, under the object key the store derives for the attempt's
    /// lease token before any content exists, with no hash yet, on a short
    /// transaction of its own that commits and returns its connection
    /// first. So it still holds one pooled connection at a time, and every
    /// object it publishes already has a committed `staged` row. That
    /// transaction takes only the foreign key's `FOR KEY SHARE` on the run
    /// row and ends before the Score lock is requested, so it adds no
    /// lock-order pair. A Score that then finds the lock held has published
    /// nothing, so the transaction that releases its run also deletes those
    /// rows (`release_score_lock_busy`).
    pub(super) async fn commit_score_phase(
        &self,
        run: &PipelineRunRecord,
        bundle: &MinimalPolicyBundle,
    ) -> anyhow::Result<PipelineRunRecord> {
        let revision_id = run
            .approved_revision_id
            .ok_or_else(|| anyhow::anyhow!("approved revision is missing"))?;
        self.score_evaluations.fetch_add(1, Ordering::SeqCst);
        let reviewed_artifact = self.load_approved_bytes(run).await?;
        // An operator's resolution of an earlier Score hold of this run, read
        // before any lock is taken, so the Score still holds one pooled
        // connection at a time.
        let hold_review = self
            .store
            .load_score_hold_assessment(&run.tenant_id, run.run_id)
            .await?;
        if !bundle.score_runs_under_runtime_controls() {
            return self
                .score_and_commit(
                    run,
                    bundle,
                    revision_id,
                    reviewed_artifact,
                    hold_review,
                    None,
                    None,
                    &[],
                )
                .await;
        }
        let prestaged = self.stage_score_artifacts_before_lock(run).await?;
        let mut client = self.backend.trace_pool().get().await?;
        let lock = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
        // Multi-lens review L3-2: the lock is tried, never waited for. A
        // replica that waited would hold this pooled connection and stall
        // its other tenants behind one tenant's Score; a busy lock releases
        // the run instead, uncharged, after a fixed delay, and the worker
        // ends this tenant's batch for the pass.
        let locked: bool = lock
            .query_one(
                "SELECT pg_try_advisory_xact_lock(hashtextextended($1, 1))",
                &[&pipeline_compatibility_score_lock(&run.tenant_id)],
            )
            .await?
            .get(0);
        if !locked {
            drop(lock);
            drop(client);
            return Ok(self.store.release_score_lock_busy(run).await?);
        }
        let lease_live = lock
            .query_opt(
                // Multi-lens review L2-5: the database clock now, not the
                // transaction's start.
                "SELECT 1 FROM pipeline_runs
                  WHERE tenant_id = $1 AND run_id = $2 AND state = 'leased'
                    AND lease_token = $3 AND lease_expires_at > clock_timestamp()",
                &[&run.tenant_id, &run.run_id, &required_lease_token(run)?],
            )
            .await?;
        if lease_live.is_none() {
            return Err(stale_lease_error().into());
        }
        let mut commands = Vec::new();
        let mut bases = Vec::new();
        for (unapplied_run_id, stored_ref, command_hash, revision) in
            PgPipelineStore::unapplied_index_commands_on_tx(&lock, run).await?
        {
            bases.push((unapplied_run_id, revision));
            commands.push(
                self.read_index_command(&run.tenant_id, &stored_ref, &command_hash, revision)
                    .await
                    .map_err(|_| {
                        // Zaki review 3, Z3-L1: fail closed, and name the run
                        // whose command cannot be read by its hash, so an
                        // operator can find it.
                        tracing::warn!(
                            label = "pipeline_unapplied_index_command_unreadable",
                            tenant_storage_ref = pipeline_tenant_storage_ref(&run.tenant_id).as_str(),
                            run_ref_hash = %sha256_prefixed(unapplied_run_id.to_string().as_bytes()),
                            "a compatibility Score cannot read another run's index command"
                        );
                        PolicyError::transient(PIPELINE_INDEX_UNAVAILABLE_LABEL)
                            .expect("static label")
                    })?,
            );
        }
        let reader = Arc::new(UnappliedIndexCommandsReader::new(
            self.index_reader.clone(),
            pipeline_tenant_storage_ref(&run.tenant_id),
            &commands,
        ));
        let bundle = self
            .construct_with_index_reader(bundle.package.clone(), reader)
            .map_err(|label| anyhow::anyhow!(label))?;
        self.score_and_commit(
            run,
            &bundle,
            revision_id,
            reviewed_artifact,
            hold_review,
            Some(lock),
            Some(&prestaged),
            &bases,
        )
        .await
    }

    /// Stages a compatibility Score attempt's rows before it takes its
    /// tenant's Score lock (rebase 10, option D): derives the object key of
    /// each artifact it can write (`index-command`, `score-neighbors`) from
    /// the run and its lease token (`pipeline_attempt_object_id`) through
    /// the store, before any content exists, and stages a row with that key
    /// and no hash (`PgPipelineStore::stage_unhashed_attempt_artifacts`). A
    /// store that cannot derive a key refuses
    /// (`serialized_json_object_key_unavailable`), and one whose derivation
    /// fails otherwise reports `artifact_store_unavailable`: the Score stops
    /// before it scores, and the run waits in retry without being charged,
    /// as any failed store call does (`artifact_store_call`, ruling FR3;
    /// rebase 10 review, M2).
    pub(super) async fn stage_score_artifacts_before_lock(
        &self,
        run: &PipelineRunRecord,
    ) -> anyhow::Result<PrestagedScoreArtifacts> {
        let lease_token = required_lease_token(run)?;
        let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
        let run_id = run.run_id;
        // The derivation is a synchronous store call, so it runs on the
        // blocking pool, as every other object-store call of a phase does
        // (PR 3, e2873401, N-6), and its failure is the uncharged
        // suspension of every failed store call (PR 3, c8d65fcb).
        let store = self.artifact_store.clone();
        let object_keys = artifact_store_call(move || {
            [
                PipelineAttemptArtifact::IndexCommand,
                PipelineAttemptArtifact::ScoreNeighbors,
            ]
            .into_iter()
            .map(|artifact| {
                store
                    .serialized_json_object_key(
                        tenant.as_str(),
                        artifact.store_kind(),
                        &pipeline_attempt_object_id(artifact.as_str(), run_id, lease_token),
                    )
                    .map(|object_key| (artifact, object_key))
            })
            .collect::<anyhow::Result<Vec<_>>>()
        })
        .await?;
        let publish_deadline = self.attempt_artifact_lease_bound(Phase::Score);
        let cleanup_after = publish_deadline + PIPELINE_ATTEMPT_ARTIFACT_COMMIT_MARGIN;
        self.store
            .stage_unhashed_attempt_artifacts(run, &object_keys, cleanup_after)
            .await?;
        Ok(PrestagedScoreArtifacts {
            object_keys,
            publish_deadline,
        })
    }

    /// `commit_score_phase`'s Score and commit over `reviewed_artifact`.
    /// The commit runs on `lock` when the caller holds the tenant's Score
    /// lock on it, and on a transaction of its own otherwise. `prestaged`
    /// is the rows a compatibility Score staged before that lock (rebase 10,
    /// option D); with none, each object's row is staged here, with its
    /// hash, just before the object is published.
    ///
    /// `hold_review` is the operator's resolution of an earlier Score hold
    /// of this run (`load_score_hold_assessment`). A rejection is never
    /// shown to the policy: the server commits a zero Score outcome itself
    /// (`ScoreOutput::rejected_after_hold`: no award, no index command,
    /// evidence naming the hold label), so the run settles with no credit.
    /// An approval reaches the policy on its input
    /// (`ScoreInput::operator_review`); a policy that asks for a hold again
    /// on it fails the run closed (`PIPELINE_SCORE_HOLD_REPEATED_LABEL`)
    /// rather than parking it again, so a hold cannot loop.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn score_and_commit(
        &self,
        run: &PipelineRunRecord,
        bundle: &MinimalPolicyBundle,
        revision_id: Uuid,
        reviewed_artifact: Vec<u8>,
        hold_review: Option<ScoreHoldReview>,
        lock: Option<Transaction<'_>>,
        prestaged: Option<&PrestagedScoreArtifacts>,
        unapplied_bases: &[(Uuid, Uuid)],
    ) -> anyhow::Result<PipelineRunRecord> {
        let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
        let output = match hold_review {
            Some(ScoreHoldReview {
                decision: ReviewRecommendation::Reject,
                hold_label,
            }) => ScoreOutput::rejected_after_hold(&bundle.package.manifest, hold_label)
                .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?,
            approved => {
                let approved_hold = approved.is_some();
                let output = bundle
                    .score
                    .execute(&ScoreInput {
                        run_id: run.run_id,
                        tenant_storage_ref: tenant.clone(),
                        trace_id: run.trace_id,
                        registry_revision_id: revision_id,
                        source_content_hash: run.approved_content_hash.clone().unwrap_or_default(),
                        reviewed_artifact,
                        operator_review: approved,
                    })
                    .await?;
                if approved_hold && output.hold_for_review().is_some() {
                    anyhow::bail!(PIPELINE_SCORE_HOLD_REPEATED_LABEL);
                }
                output
            }
        };
        // The policy asked for the trace to be held for operator review
        // instead of priced: the run parks in the review hold state at
        // Score under the policy's label, and nothing of the Score is
        // committed, so no award is seeded. On the tenant lock's own
        // transaction when the Score holds one, so the hold commits under
        // the lock and the Score still holds one pooled connection.
        if let Some(label) = output.hold_for_review() {
            output
                .result()
                .decision
                .require_bound(&bundle.package.manifest)
                .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
            let label = label.as_str().to_string();
            let held = match lock {
                Some(lock) => {
                    match PgPipelineStore::hold_score_for_review_on_tx(&lock, run, &label).await {
                        Ok(updated) => lock.commit().await.map(|()| updated).map_err(Into::into),
                        Err(error) => Err(error),
                    }
                }
                None => self.store.hold_score_for_review(run, &label).await,
            };
            // As for a refused Score commit below: the labels the
            // classification allowlist expects are re-raised bare.
            return held.map_err(|error| match error {
                DatabaseError::Constraint(label)
                    if label == PIPELINE_SUBMISSION_INOPERABLE_LABEL
                        || label == PIPELINE_POLICY_NOT_RUNNABLE_LABEL =>
                {
                    anyhow::anyhow!(label)
                }
                error => error.into(),
            });
        }
        let (result, command, neighbor) = output.into_parts();
        // Before any artifact is stored, the decision must be bound to the
        // run's bundle: it records the bound manifest's bundle identifier,
        // and every award names an instrument that manifest pins. A policy
        // can build a decision under a manifest of its own making, so the
        // pin check alone does not bind the decision to the run.
        result
            .decision
            .require_bound(&bundle.package.manifest)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        // M4: a Trace Credit award must fit the ledger's signed 64-bit
        // microcredit column; refuse one that does not here, before any
        // artifact is stored, rather than at the database CHECK.
        ensure_trace_credit_awards_fit_the_ledger(result.decision.awards())?;
        let lease_token = required_lease_token(run)?;
        let cleanup_after = self.attempt_artifact_cleanup_after(Phase::Score);
        let command_bytes = match &command {
            None => None,
            Some(command) => {
                anyhow::ensure!(
                    command.revision_id() == revision_id,
                    "index_command_invalid"
                );
                Some((serde_json::to_vec(command)?, command.content_hash()?))
            }
        };
        // Each stored object is `(artifact, stored ref, hash, object ref,
        // receipt)`. Each has a committed `staged` row (PR 4) before it is
        // published: staged here, with its hash, for a Score that takes no
        // tenant lock, or before the lock, with no hash, for a compatibility
        // Score (`prestaged`). A write that fails deletes the objects stored
        // before it (best effort), as a refused commit does below; an
        // attempt that crashes before its commit leaves every object it
        // stored named by a `staged` row that `sweep_attempt_artifacts`
        // removes once `cleanup_after` passes.
        let mut written: Vec<(
            &str,
            String,
            String,
            TraceObjectRefWrite,
            EncryptedTraceArtifactReceipt,
        )> = Vec::new();
        for (artifact, bytes, hash) in [
            command_bytes
                .as_ref()
                .map(|(bytes, hash)| (PipelineAttemptArtifact::IndexCommand, bytes, hash.clone())),
            neighbor.as_ref().map(|bytes| {
                (
                    PipelineAttemptArtifact::ScoreNeighbors,
                    bytes,
                    sha256_prefixed(bytes),
                )
            }),
        ]
        .into_iter()
        .flatten()
        {
            // Stage the object's row (PR 4), then publish it. A failure here
            // deletes the objects this attempt already published, best
            // effort, as a refused commit does below; their `staged` rows
            // stay for `sweep_attempt_artifacts`. Both object-store calls
            // run on the blocking pool (N-6).
            let put: anyhow::Result<EncryptedTraceArtifactReceipt> = async {
                let wrapper = encode_pipeline_artifact_bytes(bytes)?;
                let store = self.artifact_store.clone();
                let prepare_tenant = tenant.clone();
                let object_id =
                    pipeline_attempt_object_id(artifact.as_str(), run.run_id, lease_token);
                let prepared = artifact_store_call(move || {
                    store.prepare_serialized_json(
                        prepare_tenant.as_str(),
                        TraceArtifactKind::VectorPayload,
                        &object_id,
                        &wrapper,
                    )
                })
                .await?;
                match prestaged {
                    // Rebase 10, option D: the row was staged before the
                    // tenant lock, under the key the store derived then. A
                    // prepared key that differs would publish an object no
                    // row names, so it fails closed before the publish, as
                    // the uncharged suspension of a failed store call
                    // (`artifact_store_call`). Past `publish_deadline` the
                    // attempt's lease is certainly gone and the sweep may
                    // soon remove its rows, so it publishes nothing and stops
                    // as a stale lease.
                    Some(prestaged) => {
                        if prestaged.object_key(artifact)
                            != Some(prepared.receipt().object_key.as_str())
                        {
                            return Err(PolicyError::transient(
                                PIPELINE_ATTEMPT_OBJECT_KEY_MISMATCH_LABEL,
                            )
                            .expect("static label")
                            .into());
                        }
                        if Utc::now() >= prestaged.publish_deadline {
                            return Err(stale_lease_error().into());
                        }
                    }
                    None => {
                        self.store
                            .stage_attempt_artifact(
                                run,
                                artifact,
                                &prepared.receipt().object_key,
                                &prepared.receipt().ciphertext_sha256,
                                cleanup_after,
                            )
                            .await?;
                    }
                }
                let store = self.artifact_store.clone();
                artifact_store_call(move || store.publish_serialized_json(&prepared)).await
            }
            .await;
            let receipt = match put {
                Ok(receipt) => receipt,
                Err(error) => {
                    self.delete_attempt_objects(
                        &run.tenant_id,
                        written.iter().map(|(.., receipt)| receipt),
                        "score_object_delete_failed",
                    )
                    .await;
                    return Err(error);
                }
            };
            let object_ref = score_object_ref(
                run,
                artifact.as_str(),
                &receipt,
                bytes.len(),
                &self.object_store_name,
            );
            written.push((
                artifact.as_str(),
                format!("{}#{}", receipt.object_key, receipt.ciphertext_sha256),
                hash,
                object_ref,
                receipt,
            ));
        }
        let stored = |name: &str| {
            written.iter().find(|(artifact, ..)| *artifact == name).map(
                |(_, stored_ref, hash, object_ref, _)| {
                    (stored_ref.as_str(), hash.as_str(), object_ref)
                },
            )
        };
        self.inject_crash(PipelineCrashPoint::AfterScoreArtifactStorage)?;
        let outcome = StoredPhaseResult::from_result(Phase::Score, &result)?;
        // #1313 case (a): a Score that passed quality and failed novelty may
        // have been judged against the unapplied commands it counted; those
        // runs are recorded as its bases. Any other Score records none.
        let score_bases = if result.evidence.quality_passed == Some(true)
            && result.evidence.novelty_passed == Some(false)
        {
            unapplied_bases
        } else {
            &[]
        };
        let payout_rails = self.settlement_adapters.payout_rails();
        let commit_result = match lock {
            Some(lock) => {
                PgPipelineStore::commit_score_in(
                    lock,
                    run,
                    outcome,
                    result.decision.awards(),
                    stored("index-command"),
                    stored("score-neighbors"),
                    &payout_rails,
                    self.payout_enabled(),
                    bundle.trace_credit_event,
                    score_bases,
                )
                .await
            }
            None => {
                self.store
                    .commit_score(
                        run,
                        outcome,
                        result.decision.awards(),
                        stored("index-command"),
                        stored("score-neighbors"),
                        &payout_rails,
                        self.payout_enabled(),
                        bundle.trace_credit_event,
                    )
                    .await
            }
        };
        let updated = match commit_result {
            Ok(updated) => updated,
            Err(error) => {
                // Nothing committed, so no object ref names the objects this
                // attempt wrote under its own lease key: delete every one,
                // whatever refused the commit -- an inoperable submission, a
                // stale lease, `settlement_adapter_missing`, or anything else
                // (Zaki review 1, round 2, finding 4). Best effort: a failed
                // delete never masks the refusal. Their `staged` rows (PR 4)
                // stay, because the commit that would have committed them
                // rolled back; once `cleanup_after` passes,
                // `sweep_attempt_artifacts` finds each object absent and drops
                // its row, or deletes an object whose delete failed here.
                if phase_commit_refused(&error) {
                    self.delete_attempt_objects(
                        &run.tenant_id,
                        written.iter().map(|(.., receipt)| receipt),
                        "score_object_delete_failed",
                    )
                    .await;
                }
                // The store's Display prefixes every Constraint error
                // ("Constraint violation: ..."), which would not match
                // decision P2's fixed allowlist verbatim; re-raise the labels
                // the allowlist expects as bare anyhow errors.
                return Err(match error {
                    DatabaseError::Constraint(label)
                        if label == PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL
                            || label == PIPELINE_SUBMISSION_INOPERABLE_LABEL
                            || label == PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL
                            || label == PIPELINE_POLICY_NOT_RUNNABLE_LABEL =>
                    {
                        anyhow::anyhow!(label)
                    }
                    error => error.into(),
                });
            }
        };
        self.inject_crash(PipelineCrashPoint::AfterScoreCommit)?;
        Ok(updated)
    }

    /// Deletes objects a phase attempt wrote that no committed row names,
    /// best effort: a failed delete is logged under `label` alone and never
    /// masks the error that led here.
    pub(super) async fn delete_attempt_objects<'a>(
        &self,
        tenant_id: &str,
        receipts: impl Iterator<Item = &'a EncryptedTraceArtifactReceipt>,
        label: &'static str,
    ) {
        let tenant = pipeline_tenant_storage_ref(tenant_id);
        for receipt in receipts {
            let store = self.artifact_store.clone();
            let tenant = tenant.clone();
            let receipt = receipt.clone();
            if on_blocking_pool(move || store.delete_artifact(tenant.as_str(), &receipt))
                .await
                .is_err()
            {
                tracing::warn!(
                    label,
                    "best-effort delete of a phase attempt's object failed"
                );
            }
        }
    }

    /// Settle's first half (brief 3B/3C, port 4547 to 4723 under the #971
    /// settlement shape): validates the committed Score outcome and its
    /// seeded settlement operations, persists the Settle selection before
    /// any external effect, and applies the stored index
    /// command to the index without re-querying it (ruling P1). For a run
    /// with no settlement operations at all, this also completes the run --
    /// Task 13 extends the final commit for the case with real per-
    /// instrument legs (amendments-971 A9: each leg is independent).
    pub(super) async fn complete_settle_phase(
        &self,
        run: &PipelineRunRecord,
        bundle: &MinimalPolicyBundle,
    ) -> anyhow::Result<PipelineRunRecord> {
        let mut run = run.clone();
        let revision_id = run
            .approved_revision_id
            .ok_or_else(|| anyhow::anyhow!("approved revision is missing"))?;

        // Step 1 (brief 3C): the three parts of the committed Score outcome
        // must still agree -- each is stored as an independent JSONB
        // column, so this is a defensive re-check, not a repeat of the
        // `ScoreOutput::new` invariant that already held at commit time.
        // One query for the Score phase's own row, not a list of every
        // outcome the run has: the other phases' outcomes are never read
        // here.
        let score_outcome = self
            .store
            .outcome_for_phase(&run.tenant_id, run.run_id, Phase::Score)
            .await?
            .ok_or_else(|| anyhow::anyhow!("score_outcome_invalid"))?;
        // The stored decision loads unverified and becomes a
        // `ScoreDecision` only against the run's bound manifest: a decision
        // under another bundle, or an award for an instrument that manifest
        // does not pin, fails closed before anything settles.
        let score_decision =
            serde_json::from_value::<UnverifiedScoreDecision>(score_outcome.decision)
                .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?
                .verify(&bundle.package.manifest)
                .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        let score_evidence = serde_json::from_value::<ScoreEvidence>(score_outcome.evidence)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        let score_evaluation = serde_json::from_value::<ScoreEvaluation>(score_outcome.evaluation)
            .map_err(|_| anyhow::anyhow!("score_outcome_invalid"))?;
        anyhow::ensure!(
            *score_decision.awards() == score_evidence.fixed_awards
                && *score_decision.awards() == score_evaluation.awards,
            "score_outcome_invalid"
        );
        // Spec 2026-10-08, Slice C: the commit of a run under the Score
        // runtime controls (compatibility or registered) writes a gate
        // decision row from this evidence. Checked here, before the
        // index write or any leg settles, so evidence the row cannot be
        // built from fails the run with nothing paid.
        // A rejected Score hold's outcome (`operator_rejected_hold`) was
        // never scored, so it writes no gate decision row and needs none of
        // its fields.
        if bundle.score_runs_under_runtime_controls()
            && score_evidence.operator_rejected_hold.is_none()
        {
            PipelineGateDecisionScoreValues::from_evidence(&score_evidence)
                .map_err(|label| anyhow::anyhow!(label))?;
        }

        // Step 2 (brief 3C, `ensure_operations_match_committed_awards`):
        // the settlement rows Score seeded (decision D5) must still be
        // exactly one per award. Listed once per pass: only the worker
        // holding the run's lease changes these rows, and nothing between
        // here and Step 6 below writes to them (Step 4 persists the
        // selection on `pipeline_runs`, not the settlement rows; Step 5
        // touches only the index and `index_write_state`), so the list read
        // here is still current when Step 6 uses it.
        let settlements = self
            .store
            .list_settlements(&run.tenant_id, run.run_id)
            .await?;
        ensure_operations_match_committed_awards(
            run.run_id,
            score_decision.awards(),
            &settlements,
        )?;

        // Step 4: reuse a persisted selection on retry; otherwise run the
        // Settle policy once and persist its selection before any external
        // effect.
        let selection = match self.store.load_settle_selection(&run).await? {
            Some(selection) => selection,
            None => {
                // Step 3: the submission-operability guard, read fresh,
                // directly before the index-membership decision below that
                // this branch persists. A retry that already
                // has a persisted selection never reaches this branch, so
                // it never pays for this read only to see it replaced
                // before use at Step 5 or Step 6 below.
                //
                // The stored command is read first. A withdrawal's queued
                // follow-up deletes the objects Score stored (they are the
                // submission's object refs), and it can do so before this
                // runs; the deletion is queued only once the submission is
                // no longer operable, so a guard read after a failed load
                // sees that. An inoperable submission is excluded from the
                // index whatever the policy decides, so it settles without
                // the command.
                let index_command = self.load_index_command(&run, &score_evidence).await;
                let guard = self.submission_guard(&run).await?;
                let index_command = match index_command {
                    Ok(command) => command,
                    Err(_) if !guard.operable => None,
                    Err(error) => return Err(error),
                };
                // `main`'s duplicate short-circuits, decided here, once per
                // run, before the index write this selection arms: `main`'s
                // driver records a duplicate without scoring it, so a
                // duplicate never enters its index. The verdict is a server
                // runtime input (`PipelineNoveltyUtilityChecks::
                // duplicate_controls`), not bundle policy, so it overrides
                // the policy's index membership below rather than changing
                // the policy: the bundle and its package hash stay as they
                // were. The persisted selection then carries it, and the
                // Trace Credit leg (Step 6) and the gate decision row
                // (`commit_settle`) read it from there
                // (`selection_duplicate_verdict`), never deciding again.
                // Its own pooled connection, returned before the policy runs.
                let duplicate_verdict = if bundle.score_runs_under_runtime_controls() {
                    self.store
                        .duplicate_verdict_for_selection(
                            &run,
                            self.novelty_utility_checks.duplicate_controls.as_ref(),
                        )
                        .await?
                } else {
                    None
                };
                let tenant = pipeline_tenant_storage_ref(&run.tenant_id);
                self.settle_evaluations.fetch_add(1, Ordering::SeqCst);
                let mut result = bundle
                    .settle
                    .execute(&SettleInput {
                        run_id: run.run_id,
                        tenant_storage_ref: tenant,
                        trace_id: run.trace_id,
                        registry_revision_id: revision_id,
                        source_content_hash: run.approved_content_hash.clone().unwrap_or_default(),
                        score: score_decision.clone(),
                        score_evidence: score_evidence.clone(),
                        index_command,
                    })
                    .await?;
                result
                    .decision
                    .matches_score(&score_decision)
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                // A duplicate is excluded under its label whatever the
                // policy chose; its settlement operations stay the policy's.
                if let Some(label) = duplicate_verdict {
                    result.decision = SettleDecision::new(
                        IndexMembershipDecision::Exclude {
                            reason: ReasonCode::new(label)?,
                        },
                        &score_decision,
                        result.decision.settlement_operations().to_vec(),
                    )
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                    result.evidence.index_operation_required = false;
                }
                for operation in result.decision.settlement_operations() {
                    let award = score_decision
                        .awards()
                        .iter()
                        .find(|award| award.instrument_id() == operation.instrument_id())
                        .ok_or_else(|| anyhow::anyhow!("settlement_operation_mismatch"))?;
                    anyhow::ensure!(
                        operation.operation_ref_hash()
                            == pipeline_operation_ref(run.run_id, award).as_str(),
                        "settlement_operation_mismatch"
                    );
                    if let Some(result_ref_hash) = operation.result_ref_hash() {
                        anyhow::ensure!(
                            result_ref_hash == pipeline_result_ref(run.run_id, award).as_str(),
                            "settlement_operation_mismatch"
                        );
                    }
                }
                let membership = match &result.decision.index_membership {
                    IndexMembershipDecision::Include { command_hash, .. } if guard.operable => {
                        anyhow::ensure!(
                            Some(command_hash.as_str()) == run.index_command_hash.as_deref(),
                            "index_command_invalid"
                        );
                        "included"
                    }
                    _ => "excluded",
                };
                let stored = StoredPhaseResult::from_result(Phase::Settle, &result)?;
                let selection_hash = sha256_prefixed(&serde_json::to_vec(&stored)?);
                // GRD-004: a run that reaches this point under a suspended
                // Settle policy stores no new selection. A selection that is
                // already stored is not read here, and is reused after the
                // resume. The read takes its own pooled connection, and this
                // attempt holds none.
                if !self
                    .store
                    .policy_is_runnable(&run.tenant_id, &run.bundle_id, Phase::Settle)
                    .await?
                {
                    return Err(anyhow::anyhow!(PIPELINE_POLICY_NOT_RUNNABLE_LABEL));
                }
                run = self
                    .store
                    .persist_settle_selection(&run, &stored, &selection_hash, membership)
                    .await?;
                self.inject_crash(PipelineCrashPoint::AfterSettleSelection)?;
                stored
            }
        };

        // The run's duplicate verdict, as Step 3 decided it and the
        // selection persisted it: the one value the leg and the gate
        // decision row read.
        let duplicate_verdict = selection_duplicate_verdict(&selection)?;

        // Step 5: dispatch to the index only when a prior attempt left it
        // pending (an include whose entries are not yet all applied). The
        // guard is read here too, rather than trusting whatever Step 3 saw,
        // since a withdrawal can land in the gap between them. It is also
        // held (brief section 2): one transaction reads the guard, keeps the
        // submission row locked `FOR SHARE` through every upsert, and
        // commits the `index_write_state` it records. A withdrawal locks
        // that row `FOR UPDATE`, so it either commits before the guard read
        // (the dispatch then sees it, cancels, and queues an invalidation
        // for whatever an earlier attempt wrote) or waits until the write
        // and its `complete` have committed.
        if run.index_write_state == "pending" {
            // Both reads below come before the dispatch transaction opens:
            // `ensure_live_lease` checks out a pooled connection of its own,
            // and the dispatch never holds two at once (with a pool of one it
            // would wait for itself). The stored command's error is raised
            // only once the guard lets the write go ahead: a cancelled
            // dispatch does not need the command.
            self.ensure_live_lease(&run).await?;
            let command = self.load_index_command(&run, &score_evidence).await;
            // Multi-lens review L4-3: the dispatch runs in a task of its own,
            // which owns the transaction holding the run and submission rows
            // and awaits the blocking writes, so a dropped Settle future
            // (`join_or_abort` at shutdown) does not roll the transaction back
            // while the writes go on: the task ends the writes and commits or
            // rolls back itself.
            let dispatch = tokio::spawn(dispatch_index_write(IndexDispatch {
                backend: self.backend.clone(),
                writer: self.index_writer.clone(),
                run: run.clone(),
                command,
                crash_point: self.crash_point,
                crash_pending: self.crash_pending.clone(),
                margin: self.index_write_fence_margin,
            }))
            .await
            .map_err(|_| anyhow::anyhow!(PIPELINE_BLOCKING_CALL_FAILED_LABEL))??;
            match dispatch {
                IndexDispatchOutcome::Cancelled(updated) => {
                    run = updated;
                    self.wake_follow_ups(
                        &run.tenant_id,
                        PipelineFollowUps {
                            index_invalidations: true,
                            payouts: false,
                            credit_audits: false,
                        },
                    );
                }
                IndexDispatchOutcome::Complete(updated) => run = updated,
                // GRD-004: the Settle policy was suspended before the dispatch
                // transaction took its row. No upsert started and nothing was
                // written (`pending` stays), so the run is released uncharged,
                // bare, as `process_claimed_run` classifies it, and the retry
                // after the resume dispatches from the stored selection.
                IndexDispatchOutcome::PolicyNotRunnable => {
                    return Err(anyhow::anyhow!(PIPELINE_POLICY_NOT_RUNNABLE_LABEL));
                }
                IndexDispatchOutcome::WriteFailed(
                    IndexWriteError::Uncertain | IndexWriteError::Failed,
                ) => {
                    // An index outage is a dependency failure, like Score's
                    // own `index_unavailable` -- an uncharged suspension,
                    // which the Settle code records itself rather than
                    // returning an `Err`. The dispatch's transaction rolled
                    // back (`pending` stays) before this store call.
                    return Ok(self
                        .store
                        .mark_transient_retry(&run, PIPELINE_INDEX_UNAVAILABLE_LABEL)
                        .await?);
                }
                IndexDispatchOutcome::WriteFailed(IndexWriteError::ContentConflict) => {
                    self.store.mark_index_write_state(&run, "failed").await?;
                    return Err(anyhow::anyhow!(PIPELINE_INDEX_CONFLICT_LABEL));
                }
            }
        }

        // Step 6: each settlement row Score seeded settles as an independent
        // leg, with no atomicity across instruments: no leg waits for or
        // reverses another leg, and a retry never repeats a leg that already
        // reached `complete`. The guard is read again immediately before any
        // adapter call, for the same reason Step 5 reads it before its own
        // external effect: a withdrawal can land after Step 3's read (when
        // this pass took it -- it does not run on the persisted-selection
        // branch) or Step 5's, when that ran. The settlement rows themselves
        // are not listed again: Step 2's list is still current, per the note
        // there.
        let mut guard = self.submission_guard(&run).await?;
        if !guard.operable {
            // The submission stopped being operable: every leg that has not
            // already completed is forfeited without calling its adapter.
            // Credit that already settled stays settled (skipped below);
            // credit that did not is forfeited, the same as every other
            // instrument -- there is no special case for Trace Credit here.
            //
            // Any leg of an external instrument whose `dispatched_at` is set
            // may have taken effect when it is forfeited here, whatever its
            // state or label: its adapter was called at least once. That
            // includes a `settlement_unreconciled` leg of a live run and a leg
            // dispatched earlier and later refused by a lowered cap. Such a
            // leg is flagged `settlement_unreconciled` here instead of
            // `submission_inoperable`, for an operator to reconcile by hand
            // against the adapter's own records -- this forfeit does not make
            // that reconciling call itself, and the leg keeps its
            // `dispatched_at`. The exception is a leg whose own label already
            // says no effect happened (`Conflict`/`Rejected`): it keeps
            // `submission_inoperable`. An undispatched leg had no effect, and
            // a Trace Credit leg that is not `complete` paid nothing: it pays
            // only through the ledger row that commits with its completion.
            // `withdrawal_forfeit_label` is the one rule both this branch and
            // the in-pass branch below share.
            for settlement in &settlements {
                if settlement.operation_state != "complete" {
                    self.store
                        .update_settlement(
                            &run,
                            &settlement.instrument_id,
                            withdrawal_forfeited_settlement_update(settlement),
                        )
                        .await?;
                }
            }
        } else {
            // The expected result comes from the persisted selection,
            // not from the row -- the row's own `result_ref_hash` column
            // holds `NULL` until the leg actually completes.
            let selection_decision =
                serde_json::from_value::<SettleDecision>(selection.decision.clone())
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
            let tenant_storage_ref = pipeline_tenant_storage_ref(&run.tenant_id);
            // Why legs this attempt leaves open are waiting: a hold or an
            // `Unavailable` answer (both uncharged suspensions), or a charged
            // blocker -- a leg that fails closed in this pass (a receipt that
            // does not answer its request, a `Conflict` or `Rejected` answer,
            // an amount over the cap, a request that cannot be formed) or a
            // leg an earlier pass failed closed for good.
            let mut held = false;
            let mut adapter_unavailable = false;
            let mut settlement_blocked = false;
            for settlement in settlements.clone() {
                if matches!(
                    settlement.operation_state.as_str(),
                    "complete" | "forfeited"
                ) {
                    continue;
                }
                let instrument_id = InstrumentId::new(settlement.instrument_id.clone())
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                // The Trace Credit leg's own ledger transaction
                // can discover, partway through this pass, that the
                // submission stopped being operable (a withdrawal landed
                // after this pass's guard read but before that transaction).
                // From that point on this pass treats every remaining
                // non-terminal leg the way Step 6's top-level inoperable
                // branch treats a submission already known inoperable:
                // forfeited without calling its adapter, including a leg that
                // was dispatched before and whose effect may have happened
                // (see that branch, and `withdrawal_forfeit_label`, the same
                // rule this branch shares with it). A leg that already
                // completed earlier in this pass is untouched: no leg
                // reverses another.
                if !guard.operable {
                    self.store
                        .update_settlement(
                            &run,
                            instrument_id.as_str(),
                            withdrawal_forfeited_settlement_update(&settlement),
                        )
                        .await?;
                    continue;
                }
                // A leg an earlier pass failed closed for good is never
                // dispatched again. While one exists, the run's retry stays
                // charged, so the other legs still progress, the attempts run
                // out, and the failure path resolves it.
                if !settlement_leg_is_open_to_dispatch(
                    &settlement.operation_state,
                    settlement.last_error_label.as_deref(),
                ) {
                    settlement_blocked = true;
                    continue;
                }
                let adapter = self
                    .settlement_adapters
                    .get(&instrument_id)
                    .ok_or_else(|| anyhow::anyhow!(PIPELINE_SETTLEMENT_ADAPTER_MISSING_LABEL))?;
                // No configured cap for the
                // instrument is a configuration gap, an uncharged suspension
                // (`process_claimed_run`), raised before the leg's adapter
                // is called. An amount over a configured cap is a spend
                // limit that refuses the payment: the leg fails below and
                // the retry stays charged.
                let cap = self
                    .caps
                    .per_instrument_atomic_units
                    .get(instrument_id.as_str())
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!(PIPELINE_SETTLEMENT_CAP_MISSING_LABEL))?;
                if settlement.atomic_units > cap {
                    self.store
                        .update_settlement(
                            &run,
                            instrument_id.as_str(),
                            failed_settlement_update(PIPELINE_CREDIT_CAP_LABEL),
                        )
                        .await?;
                    settlement_blocked = true;
                    continue;
                }
                let expected_result_ref_hash = selection_decision
                    .settlement_operations()
                    .iter()
                    .find(|operation| operation.instrument_id() == &instrument_id)
                    .and_then(|operation| operation.result_ref_hash())
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("settlement result reference is missing"))?;
                // A request that `SettlementRequest::new` refuses cannot be
                // sent: the leg fails closed before any adapter call, and the
                // retry is charged.
                let request = match SettlementRequest::new(
                    tenant_storage_ref.clone(),
                    run.run_id,
                    instrument_id.clone(),
                    settlement.atomic_units,
                    settlement.operation_ref_hash.clone(),
                    expected_result_ref_hash,
                ) {
                    Ok(request) => request,
                    Err(_) => {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                failed_settlement_update(PIPELINE_SETTLEMENT_REQUEST_INVALID_LABEL),
                            )
                            .await?;
                        settlement_blocked = true;
                        continue;
                    }
                };
                // `main`'s duplicate short-circuits gate the Trace Credit
                // leg of every Score policy that runs under the Score
                // runtime controls (`score_runs_under_runtime_controls`),
                // whichever event the bundle settles under: a duplicate
                // earns nothing. The leg completes withheld, with no ledger
                // row, under the verdict's label, the shape the
                // `NoveltyUtility` checks below write. A `NoveltyUtility`
                // leg reaches the same result through
                // `novelty_utility_withheld_reason`, which answers the
                // verdict first; checking it here, before the hold check,
                // keeps a `PipelineScore` leg from paying a duplicate. As
                // there (poldsam P-1), only a leg not yet dispatched is
                // withheld as a duplicate.
                if instrument_id == InstrumentId::trace_credit()
                    && settlement.dispatched_at.is_none()
                    && let Some(label) = duplicate_verdict
                {
                    self.store
                        .update_settlement(
                            &run,
                            instrument_id.as_str(),
                            SettlementUpdate {
                                operation_state: "complete",
                                result_ref_hash: Some(request.expected_result_ref_hash()),
                                external_receipt_hash: None,
                                credit_event_id: None,
                                settlement_batch_id: None,
                                payout_state: None,
                                error_label: Some(label),
                            },
                        )
                        .await?;
                    continue;
                }
                // A held Trace Credit account is checked before any external
                // effect, so a held account never reaches its adapter. Only a
                // leg that settles into a batch is held: a flat-rule
                // `NoveltyUtility` leg never settles, and `main` writes that
                // credit regardless of holds (owner ruling, Zaki review 1,
                // round 2, finding 13); a graded-rule leg settles, so it is
                // held (owner decision D7, 2026-11-03).
                let credit_account_ref = if instrument_id == InstrumentId::trace_credit() {
                    let account_ref = self.credit_account_ref(&run).await?;
                    if bundle.trace_credit_event.settles_into_batch()
                        && self
                            .credit_account_is_held(&run.tenant_id, &account_ref)
                            .await?
                    {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                held_settlement_update(),
                            )
                            .await?;
                        held = true;
                        continue;
                    }
                    Some(account_ref)
                } else {
                    None
                };
                // Zaki review 1, round 2, finding 19: `main` applies its
                // `NoveltyUtility` credit checks before it credits anything, so
                // they run here, before the adapter call: a leg they withhold
                // never reaches its adapter. It completes with no ledger row,
                // under `main`'s label, at the result reference the persisted
                // selection recorded. The ledger transaction checks again
                // (`settle_internal_credit`), since the tenant policy can change
                // between the two.
                if instrument_id == InstrumentId::trace_credit()
                    && bundle.trace_credit_event.is_novelty_utility()
                {
                    let amount =
                        InstrumentAward::new(InstrumentId::trace_credit(), settlement.atomic_units)
                            .and_then(|award| award.trace_credit_microcredits())
                            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                    let withheld = {
                        // Resolved before the transaction takes a pooled
                        // connection: a policy read from the database uses
                        // its own, as `main`'s credit check reads it.
                        let authority = self.resolve_authority(&run.tenant_id).await?;
                        let mut client = self.backend.trace_pool().get().await?;
                        let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id)
                            .await?;
                        let withheld = self
                            .novelty_utility_withheld_reason(
                                &tx,
                                &run,
                                amount.get(),
                                authority.as_ref(),
                                // Only a leg not yet dispatched is withheld
                                // as a duplicate (poldsam P-1 below).
                                duplicate_verdict.filter(|_| settlement.dispatched_at.is_none()),
                            )
                            .await?;
                        tx.commit().await?;
                        withheld
                    };
                    // poldsam P-1: a leg an earlier attempt already
                    // dispatched (its adapter answered `Unavailable`, the
                    // process stopped after the call, or the Settle policy
                    // was suspended while the call was in flight, GRD-004,
                    // so the effect may have happened) is not completed as
                    // withheld with no receipt: it fails as
                    // `settlement_unreconciled`, as a dispatched leg
                    // forfeited by a withdrawal is, for an operator to
                    // reconcile against the adapter's records by
                    // `operation_ref_hash`. The retry stays charged, so the
                    // attempts run out and the run fails with the leg kept.
                    if withheld.is_some() && settlement.dispatched_at.is_some() {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                failed_settlement_update(PIPELINE_SETTLEMENT_UNRECONCILED_LABEL),
                            )
                            .await?;
                        settlement_blocked = true;
                        continue;
                    }
                    if let Some(label) = withheld {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                SettlementUpdate {
                                    operation_state: "complete",
                                    result_ref_hash: Some(request.expected_result_ref_hash()),
                                    external_receipt_hash: None,
                                    credit_event_id: None,
                                    settlement_batch_id: None,
                                    payout_state: None,
                                    error_label: Some(label),
                                },
                            )
                            .await?;
                        continue;
                    }
                }
                // Zaki review 1, minor item M-e: `guard` was read once,
                // before this loop, and only the Trace Credit leg reads it
                // again under the submission's row lock (its ledger
                // transaction, `settle_internal_credit`). Every other leg's
                // adapter call below is not re-checked under that lock, so a
                // withdrawal that lands after the one read does not stop it.
                // Before a second non-Trace-Credit adapter is registered,
                // this call must re-check the guard under the submission row
                // lock first, as the Trace Credit leg does.
                //
                // Every adapter call is fenced by the lease this attempt
                // still holds, the same check Step 5 runs before the index
                // write.
                self.ensure_live_lease(&run).await?;
                // GRD-004: the policy guard of the external call, beside the
                // lease fence and before the leg is marked leased: no call
                // is dispatched under a Settle policy that is not runnable.
                // The read takes its own pooled connection, as the lease
                // fence does, and this attempt holds none here. A leg whose
                // call is already in flight is not retracted; this leg stays
                // as it is and the run is released uncharged
                // (`process_claimed_run`), to continue after the resume.
                if !self
                    .store
                    .policy_is_runnable(&run.tenant_id, &run.bundle_id, Phase::Settle)
                    .await?
                {
                    return Err(anyhow::anyhow!(PIPELINE_POLICY_NOT_RUNNABLE_LABEL));
                }
                // The leg is `leased` under this attempt's lease
                // while its adapter call is in flight, and `dispatched_at`
                // records that it was dispatched -- a failed run reconciles
                // such a leg against its adapter instead of forfeiting it.
                self.store
                    .mark_settlement_leased(&run, instrument_id.as_str())
                    .await?;
                let receipt = match adapter.settle(&request).await {
                    // The receipt must answer the request: its result
                    // reference is the one the persisted selection recorded.
                    Ok(receipt) if receipt.answers(&request) => receipt,
                    // Any other receipt fails the leg closed, and the leg is
                    // never dispatched again.
                    Ok(_) => {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                failed_settlement_update(PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL),
                            )
                            .await?;
                        settlement_blocked = true;
                        continue;
                    }
                    // The effect may or may not have happened. The leg waits
                    // in `retry`, uncharged, and a later attempt sends the
                    // same request; the adapter's idempotency contract makes
                    // that safe.
                    Err(SettlementError::Unavailable) => {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                SettlementUpdate {
                                    operation_state: "retry",
                                    result_ref_hash: None,
                                    external_receipt_hash: None,
                                    credit_event_id: None,
                                    settlement_batch_id: None,
                                    payout_state: None,
                                    error_label: Some(SettlementError::Unavailable.label()),
                                },
                            )
                            .await?;
                        adapter_unavailable = true;
                        continue;
                    }
                    // No effect happened, and the operation must not be sent
                    // again: the leg fails under the adapter's own label and
                    // is never dispatched again.
                    Err(error @ (SettlementError::Conflict | SettlementError::Rejected)) => {
                        self.store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                failed_settlement_update(error.label()),
                            )
                            .await?;
                        settlement_blocked = true;
                        continue;
                    }
                };
                match credit_account_ref {
                    // The ledger row, the finalized batch, and the completed
                    // settlement row commit in one transaction.
                    Some(account_ref) => match self
                        .settle_internal_credit(
                            &run,
                            &settlement,
                            score_outcome.outcome_id,
                            &account_ref,
                            &receipt,
                            bundle.trace_credit_event,
                        )
                        .await
                    {
                        // The completed leg's batch is the payout step's
                        // work: wake it (Zaki review 1, round 2, item 4).
                        Ok(InternalCreditResult::Complete) => {
                            self.wake_follow_ups(
                                &run.tenant_id,
                                PipelineFollowUps {
                                    index_invalidations: false,
                                    payouts: true,
                                    credit_audits: false,
                                },
                            );
                        }
                        Ok(InternalCreditResult::Held) => {
                            self.store
                                .update_settlement(
                                    &run,
                                    instrument_id.as_str(),
                                    held_settlement_update(),
                                )
                                .await?;
                            held = true;
                            continue;
                        }
                        // GRD-004: the Settle policy was suspended while this
                        // leg's adapter call was in flight. The call is not
                        // retracted and the leg stays as it is (`leased`, as
                        // after a crash between the call and this
                        // transaction), the ledger transaction wrote nothing,
                        // and the run is released uncharged, bare, as
                        // `process_claimed_run` classifies it. After the
                        // resume the retry repeats the idempotent call and
                        // this transaction.
                        Ok(InternalCreditResult::PolicyNotRunnable) => {
                            return Err(anyhow::anyhow!(PIPELINE_POLICY_NOT_RUNNABLE_LABEL));
                        }
                        // The ledger transaction's own
                        // submission re-check found the submission
                        // inoperable (a withdrawal landed during this leg's
                        // adapter call, between Step 6's guard read and this
                        // transaction). Forfeit this leg the same way the
                        // top-level inoperable branch does -- no result, no
                        // event, no batch -- and treat the submission as
                        // inoperable for the rest of this pass, so every
                        // remaining non-terminal leg is forfeited too. When
                        // no earlier leg in this same pass blocked, the
                        // committed Settle outcome records
                        // `submission_inoperable`; when one did, this
                        // attempt retries instead, and the next pass's guard
                        // read forfeits every leg before any adapter call.
                        Ok(InternalCreditResult::Inoperable) => {
                            self.store
                                .update_settlement(
                                    &run,
                                    instrument_id.as_str(),
                                    forfeited_settlement_update(
                                        PIPELINE_SUBMISSION_INOPERABLE_LABEL,
                                    ),
                                )
                                .await?;
                            guard.operable = false;
                            continue;
                        }
                        // One external receipt answers one leg: a receipt
                        // another leg already recorded does not answer this
                        // one. The credit transaction rolled back, so no
                        // ledger row was written.
                        Err(error) if is_external_receipt_reuse_error(&error) => {
                            self.store
                                .update_settlement(
                                    &run,
                                    instrument_id.as_str(),
                                    failed_settlement_update(
                                        PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL,
                                    ),
                                )
                                .await?;
                            settlement_blocked = true;
                            continue;
                        }
                        Err(error) => return Err(error),
                    },
                    None => {
                        let completed = self
                            .store
                            .update_settlement(
                                &run,
                                instrument_id.as_str(),
                                SettlementUpdate {
                                    operation_state: "complete",
                                    result_ref_hash: Some(receipt.result_ref_hash()),
                                    external_receipt_hash: receipt.external_receipt_hash(),
                                    credit_event_id: None,
                                    settlement_batch_id: None,
                                    payout_state: None,
                                    error_label: None,
                                },
                            )
                            .await;
                        match completed {
                            Ok(_) => {}
                            // One external receipt answers one leg: a receipt
                            // another leg already recorded does not answer
                            // this one.
                            Err(DatabaseError::Postgres(error))
                                if is_external_receipt_reuse(&error) =>
                            {
                                self.store
                                    .update_settlement(
                                        &run,
                                        instrument_id.as_str(),
                                        failed_settlement_update(
                                            PIPELINE_SETTLEMENT_RESULT_MISMATCH_LABEL,
                                        ),
                                    )
                                    .await?;
                                settlement_blocked = true;
                                continue;
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                }
                self.inject_crash(PipelineCrashPoint::AfterInstrumentOperation)?;
            }
            // Every non-terminal row this attempt leaves behind is retried
            // together; the Settle code records the retry itself and never
            // returns `Err` for this case. A charged blocker fails closed and
            // is charged, whatever else happened; otherwise a hold or an
            // `Unavailable` answer is an uncharged suspension, labeled by the
            // hold when there is one. The charged retry that exhausts the
            // attempts resolves every open leg before the run fails
            // (`charged_retry`).
            if settlement_blocked {
                return self
                    .charged_retry(&run, PIPELINE_SETTLEMENT_RETRY_LABEL)
                    .await;
            }
            if held || adapter_unavailable {
                let label = if held {
                    PIPELINE_CREDIT_HELD_LABEL
                } else {
                    PIPELINE_SETTLEMENT_RETRY_LABEL
                };
                return Ok(self.store.mark_transient_retry(&run, label).await?);
            }
        }

        self.commit_settle_from_progress(&run, &selection, guard, &score_decision)
            .await
    }

    /// Commits the Settle outcome once every settlement row is terminal
    /// (`complete` or `forfeited`) -- Step 6 above guarantees that by the
    /// time this runs, either through completion or through the guard's
    /// forfeiture pass. Builds the final decision from what actually
    /// happened to each leg (amendments-971 A9), never from the selection's
    /// own proposed operations. `score_decision` is the same value Step 1 of
    /// `complete_settle_phase` already decoded; this does not re-read it.
    pub(super) async fn commit_settle_from_progress(
        &self,
        run: &PipelineRunRecord,
        selection: &StoredPhaseResult,
        guard: SubmissionGuard,
        score_decision: &ScoreDecision,
    ) -> anyhow::Result<PipelineRunRecord> {
        let stored_decision = serde_json::from_value::<SettleDecision>(selection.decision.clone())
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        // The committed decision records what actually happened, not what
        // the policy raw-selected: the selection persists `Include` even
        // when the guard was already inoperable at persist time (only the
        // DB `index_membership` column was corrected then), and the guard
        // can newly fail between persist and dispatch, leaving
        // `index_write_state = 'cancelled'` with `index_membership` still
        // `'included'`. Task 13's rule (applies here per the controller
        // ruling): the selection's `Include` only if `run.index_membership
        // == "included"` and `run.index_write_state == "complete"`; else
        // `Exclude { reason: submission_inoperable }` when the guard is
        // inoperable now; else the selection's own `Exclude` (a plain
        // policy exclusion, unrelated to the guard).
        let observed_include =
            run.index_membership == "included" && run.index_write_state == "complete";
        let final_index_membership = if observed_include {
            stored_decision.index_membership.clone()
        } else if !guard.operable {
            IndexMembershipDecision::Exclude {
                reason: ReasonCode::new(PIPELINE_SUBMISSION_INOPERABLE_LABEL)?,
            }
        } else {
            match &stored_decision.index_membership {
                IndexMembershipDecision::Exclude { reason } => IndexMembershipDecision::Exclude {
                    reason: reason.clone(),
                },
                // Unreachable given the state machine above (an operable
                // guard with `observed_include` false and a selection that
                // still says `Include` would mean dispatch neither
                // completed, retried, nor cancelled) -- fail closed rather
                // than commit an unproven `Include`.
                IndexMembershipDecision::Include { .. } => IndexMembershipDecision::Exclude {
                    reason: ReasonCode::new(PIPELINE_SUBMISSION_INOPERABLE_LABEL)?,
                },
            }
        };
        // Every row is terminal here (Step 6's contract): `complete` becomes
        // a `Completed` operation carrying the adapter's actual result;
        // `forfeited` becomes a `Forfeited` operation under the same safe
        // label the row itself was written with. `list_settlements` orders
        // by `instrument_id`, so this is already instrument order;
        // `SettleDecision::from_parts` re-sorts regardless.
        //
        // Ruling F-I1: a `trace_credit` row `complete` with no credit event
        // is a leg one of `main`'s NoveltyUtility checks withheld (the
        // withheld branch of `settle_internal_credit`, the only writer of
        // such a row). No credit was issued, so the outcome records it as
        // `Forfeited` under the row's own withheld label, not as a completed
        // settlement of the award. A row of that shape with no label cannot
        // be told apart from a lost event, and fails closed.
        let settlements = self
            .store
            .list_settlements(&run.tenant_id, run.run_id)
            .await?;
        let operations = settlements
            .iter()
            .map(|settlement| -> anyhow::Result<InstrumentSettlement> {
                let instrument_id = InstrumentId::new(settlement.instrument_id.clone())
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                match settlement.operation_state.as_str() {
                    "complete"
                        if settlement.instrument_id == InstrumentId::trace_credit().as_str()
                            && settlement.credit_event_id.is_none() =>
                    {
                        let withheld_label = settlement
                            .last_error_label
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("settlement_operation_mismatch"))?;
                        InstrumentSettlement::forfeited(
                            instrument_id,
                            settlement.atomic_units,
                            settlement.operation_ref_hash.clone(),
                            ReasonCode::new(withheld_label)
                                .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?,
                        )
                        .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))
                    }
                    "complete" => {
                        let result_ref_hash = settlement
                            .result_ref_hash
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("settlement_operation_mismatch"))?;
                        // The committed operation carries the receipt the leg
                        // recorded, its external receipt hash included.
                        let receipt = match settlement.external_receipt_hash.clone() {
                            Some(external_receipt_hash) => {
                                SettlementReceipt::external(result_ref_hash, external_receipt_hash)
                            }
                            None => SettlementReceipt::internal(result_ref_hash),
                        }
                        .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
                        InstrumentSettlement::completed(
                            instrument_id,
                            settlement.atomic_units,
                            settlement.operation_ref_hash.clone(),
                            &receipt,
                        )
                        .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))
                    }
                    "forfeited" => InstrumentSettlement::forfeited(
                        instrument_id,
                        settlement.atomic_units,
                        settlement.operation_ref_hash.clone(),
                        ReasonCode::new(PIPELINE_SUBMISSION_INOPERABLE_LABEL)
                            .expect("static safe label"),
                    )
                    .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch")),
                    // Step 6 never returns here with a row left `pending`,
                    // `retry`, `held`, or `failed` -- it retries the run
                    // itself first. Fail closed rather than commit a
                    // decision that misrepresents a leg still in flight.
                    _ => Err(anyhow::anyhow!("settlement_operation_mismatch")),
                }
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let final_decision =
            SettleDecision::new(final_index_membership, score_decision, operations)?;
        let mut evidence = serde_json::from_value::<SettleEvidence>(selection.evidence.clone())
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        evidence.index_command_hash = run.index_command_hash.clone();
        evidence.settlement_progress = settlements
            .iter()
            .map(|settlement| {
                Ok(InstrumentSettlementProgress {
                    instrument_id: InstrumentId::new(settlement.instrument_id.clone())?,
                    operation_ref_hash: settlement.operation_ref_hash.clone(),
                    result_ref_hash: settlement.result_ref_hash.clone(),
                    external_receipt_hash: settlement.external_receipt_hash.clone(),
                })
            })
            .collect::<Result<Vec<_>, trace_commons_gate_api::pipeline::ContractError>>()
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        evidence.index_progress = Some(run.index_write_state.clone());
        evidence.submission_operable = Some(guard.operable);
        evidence.guard_reason = if guard.operable {
            None
        } else {
            Some(ReasonCode::new(PIPELINE_SUBMISSION_INOPERABLE_LABEL)?)
        };
        let evaluation = serde_json::from_value::<SettleEvaluation>(selection.evaluation.clone())
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        let final_membership = match &final_decision.index_membership {
            IndexMembershipDecision::Include { .. } => "included",
            IndexMembershipDecision::Exclude { .. } => "excluded",
        };
        let result = PhaseResult {
            decision: final_decision,
            evidence,
            evaluation,
        };
        let outcome = StoredPhaseResult::from_result(Phase::Settle, &result)?;
        // The duplicate verdict the selection persisted (Step 3), the same
        // one that kept the run out of the index and withheld its leg; the
        // commit's locked transaction reads no summaries.
        let duplicate_verdict = selection_duplicate_verdict(selection)?;
        let updated = match self
            .store
            .commit_settle(run, outcome, final_membership, duplicate_verdict)
            .await
        {
            Ok(updated) => updated,
            // GRD-004: the Settle policy was suspended while the phase ran.
            // The store's `Display` prefixes every `Constraint` error, which
            // would not match the label `process_claimed_run` classifies
            // verbatim (as for Review and Score above), so it is re-raised
            // bare. The commit wrote nothing; the legs this attempt completed
            // stay complete, and the retry after the resume commits from them.
            //
            // The gate decision row's refusals are re-raised bare for the
            // same reason, and `process_claimed_run` fails the run
            // terminally on them: both are deterministic, so a retry would
            // fail the same way.
            Err(DatabaseError::Constraint(label))
                if label == PIPELINE_POLICY_NOT_RUNNABLE_LABEL
                    || label == PIPELINE_GATE_DECISION_EVIDENCE_INCOMPLETE_LABEL
                    || label == PIPELINE_GATE_DECISION_CONFLICT_LABEL =>
            {
                return Err(anyhow::anyhow!(label));
            }
            Err(error) => return Err(error.into()),
        };
        self.inject_crash(PipelineCrashPoint::AfterSettleCommit)?;
        // Owner decision O4: the pipeline stamps dedup on its own gate
        // decision row, after the commit, best-effort.
        self.stamp_settled_gate_decision_dedup(&updated).await;
        Ok(updated)
    }

    /// Stamps the cross-trace dedup columns (simhash, cluster id, cluster
    /// size, signal version) on the run's `pipeline_settle` gate decision
    /// row, after Settle committed it (owner decision O4). The assignment is
    /// `main`'s inline one (`dedup_inline::record_inline_dedup`), so pipeline
    /// and legacy rows cluster together under the same rule.
    ///
    /// The simhash is derived under `ACTIVE_DEDUP_ALGORITHM` over the
    /// metadata-free render (`dedup_canonical_text`) of the submission's
    /// source envelope: the `submitted_envelope` object the receipt staged,
    /// which is the object `main`'s re-derivation pass loads for the same
    /// submission, so the inline stamp and a later re-derivation agree. The
    /// plaintext never leaves this method; only the 64-bit value is written.
    ///
    /// Best-effort and fail-closed: the Settle commit already happened and
    /// is never undone. A failed read or write logs a label and the tenant
    /// storage ref, never the error text, and leaves the row unclustered,
    /// which earned trust reads as not first in any cluster until the
    /// re-derivation pass reaches it. The write is guarded by
    /// `dedup_cluster_id IS NULL`, so a row already stamped keeps its
    /// cluster, and a run that wrote no row (a minimal-reference bundle, a
    /// rejected Score hold) matches nothing.
    pub(super) async fn stamp_settled_gate_decision_dedup(&self, run: &PipelineRunRecord) {
        let source = match self.load_source_bytes(run).await {
            Ok(bytes) => bytes,
            Err(_) => {
                tracing::warn!(
                    error_class = "pipeline_settle_dedup_source_unreadable",
                    tenant_storage_ref = pipeline_tenant_storage_ref(&run.tenant_id).as_str(),
                    "pipeline settle dedup stamp skipped; the row stays unclustered"
                );
                return;
            }
        };
        let algorithm = crate::dedup_simhash::ACTIVE_DEDUP_ALGORITHM;
        let simhash =
            algorithm.simhash(&crate::trace_gate_service::dedup_canonical_text(&source)) as i64;
        let signal_version = crate::trace_gate_service::dedup_signal_version_for(algorithm);
        let outcome = crate::dedup_inline::record_inline_dedup(
            self.backend.as_ref(),
            &run.tenant_id,
            pipeline_gate_decision_id(&run.tenant_id, run.run_id),
            simhash,
            &signal_version,
            crate::dedup_inline::InlineDedupWrite::OnlyIfUnset,
        )
        .await;
        if outcome == crate::dedup_inline::InlineDedupOutcome::WriteFailed {
            tracing::warn!(
                error_class = "pipeline_settle_dedup_write_failed",
                tenant_storage_ref = pipeline_tenant_storage_ref(&run.tenant_id).as_str(),
                "pipeline settle dedup stamp failed; the row stays unclustered"
            );
        }
    }

    /// The Trace Credit account a run's credit settles to: the submission's
    /// authenticated principal reference.
    pub(super) async fn credit_account_ref(
        &self,
        run: &PipelineRunRecord,
    ) -> anyhow::Result<String> {
        Ok(self
            .backend
            .get_trace_submission(&run.tenant_id, run.submission_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("submission is missing"))?
            .auth_principal_ref)
    }

    /// The witness provenance label `main` records on a credit event
    /// (`credit_witness_provenance_class`; Zaki review 1, round 2, N-5): the
    /// submission's current verified witness evidence, `unattested` when it
    /// has none. A credit is never refused or delayed for want of a label,
    /// so a failed read records nothing (`None`) and logs a label only. Its
    /// own pooled connection, which it returns before the credit
    /// transaction takes one.
    pub(super) async fn credit_witness_provenance_class(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Option<&'static str> {
        match TraceCorpusStore::list_current_verified_witness_evidence(
            self.backend.as_ref(),
            tenant_id,
            &[submission_id],
        )
        .await
        {
            Ok(claims) => Some(
                claims
                    .get(&submission_id)
                    .map(TraceWitnessProvenanceClass::from_claim)
                    .unwrap_or(TraceWitnessProvenanceClass::Unattested)
                    .as_str(),
            ),
            Err(_) => {
                tracing::warn!(
                    label = "pipeline_credit_witness_provenance_unavailable",
                    "pipeline credit event witness provenance label unavailable"
                );
                None
            }
        }
    }

    /// `main`'s settlement key for `principal_ref`'s credit
    /// (`settlement_group_key`: its account when it is linked to one) and,
    /// for an account key, the hold `main` records when the account has no
    /// unambiguous NEAR payout target (`resolve_payout_near_account_id`), as
    /// `main`'s credit settlement does when it finalizes a batch.
    pub(super) async fn settlement_key_and_hold(
        &self,
        tenant_id: &str,
        principal_ref: &str,
    ) -> anyhow::Result<(String, Option<&'static str>)> {
        let principal_to_account = crate::db::Database::resolve_principals_to_accounts(
            self.backend.as_ref(),
            tenant_id,
            &[principal_ref.to_string()],
        )
        .await?;
        let settlement_key = crate::db::settlement_group_key(principal_ref, &principal_to_account);
        let hold = match crate::db::settlement_key_account_id(&settlement_key) {
            Some(account_id) => match crate::db::Database::resolve_payout_near_account_id(
                self.backend.as_ref(),
                tenant_id,
                account_id,
            )
            .await?
            {
                crate::db::PayoutResolution::Hold(reason) => Some(reason.label()),
                crate::db::PayoutResolution::Designated(_)
                | crate::db::PayoutResolution::SoleActive(_) => None,
            },
            None => None,
        };
        Ok((settlement_key, hold))
    }

    /// Whether an unreleased hold names `account_ref` (ruling FR2, step 1:
    /// read before any external effect).
    pub(super) async fn credit_account_is_held(
        &self,
        tenant_id: &str,
        account_ref: &str,
    ) -> anyhow::Result<bool> {
        let refs = self
            .backend
            .list_trace_credit_holds(tenant_id)
            .await?
            .into_iter()
            .filter(|hold| hold.released_at.is_none())
            .map(|hold| hold.credit_account_ref)
            .collect();
        Ok(crate::credit_hold_ownership::expand_credit_hold_ownership(
            self.backend.as_ref(),
            tenant_id,
            refs,
        )
        .await?
        .contains(account_ref))
    }

    /// Settles the Trace Credit leg into the internal credit ledger (port
    /// 4937 to 5112) in ONE tenant transaction, after the caller has checked
    /// the hold, fenced the lease, and received the adapter's receipt
    /// (`receipt`, which already answers the leg's request). The
    /// transaction:
    ///
    /// 1. locks the run row and confirms this attempt's lease is still the
    ///    current one (`ensure_current_lease`, as the commit transactions do);
    /// 2. takes the per-account advisory lock, so two runs for one account
    ///    never select the same pending event into two batches;
    /// 3. re-checks the hold under that lock -- a hold returns `Held` and
    ///    rolls the transaction back with nothing written;
    /// 4. locks the submission row (`FOR SHARE`) and, in a later statement,
    ///    re-checks it is still operable -- an inoperable submission returns
    ///    `Inoperable` and rolls the transaction back with nothing written
    ///    (the guard Step 6 read before this leg's adapter call
    ///    is only a snapshot; a withdrawal can land in the gap between that
    ///    read and this transaction, and must forfeit the pending award
    ///    rather than let it be paid);
    /// 5. locks the Settle policy's status row `FOR SHARE` as the last lock
    ///    (`lock_runnable_policy`, GRD-004) and, when the policy is no longer
    ///    runnable, returns `PolicyNotRunnable` and rolls the transaction back
    ///    with nothing written: a suspension that returned to the operator
    ///    stops the credit event and its batch, and one that is waiting for
    ///    this transaction waits for it to end;
    /// 6. inserts the ledger row idempotently (the event id is derived from
    ///    the run and its Score outcome);
    /// 7. if that event is already final, reuses the one finalized batch that
    ///    carries it; otherwise composes the batch from the account's pending
    ///    events, writes it finalized, sets its `instrument_id`, and marks
    ///    those events final;
    /// 8. completes the settlement row with the receipt's result reference
    ///    and external receipt hash, the event, and the batch. A receipt hash
    ///    another leg already recorded is refused here, and the whole
    ///    transaction rolls back.
    ///
    /// Everything commits together or nothing does, so a crash or a stale
    /// lease anywhere in it leaves the leg exactly as it was before the
    /// attempt, and the retry repeats the idempotent adapter call and this
    /// transaction.
    ///
    /// Adapted for the #971 settlement shape and PR 2's scope:
    ///
    /// - Ruling A8: the microcredit amount comes from
    ///   `InstrumentAward::trace_credit_microcredits`, not the removed
    ///   `Microcredits::from_atomic_units` (private as of #971).
    /// - `external_ref` carries `pipeline_ledger_source_key` (a hash of the
    ///   tenant and request idempotency key); PR 2's schema has no dedicated
    ///   `ledger_source_key` column on `trace_credit_ledger`.
    /// - The batch never dispatches a NEAR payout in PR 2: `near_status` is
    ///   always `Disabled` and `near_contract_id` always `None`.
    pub(super) async fn settle_internal_credit(
        &self,
        run: &PipelineRunRecord,
        settlement: &PipelineSettlementRecord,
        score_outcome_id: Uuid,
        account_ref: &str,
        receipt: &SettlementReceipt,
        trace_credit_event: PipelineTraceCreditEvent,
    ) -> anyhow::Result<InternalCreditResult> {
        let lease_token = required_lease_token(run)?;
        let account_hash = credit_account_hash(account_ref);
        let event_id = pipeline_credit_event_id(&run.tenant_id, run.run_id, score_outcome_id);
        let amount = InstrumentAward::new(InstrumentId::trace_credit(), settlement.atomic_units)
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?
            .trace_credit_microcredits()
            .map_err(|_| anyhow::anyhow!("settlement_operation_mismatch"))?;
        let external_ref = pipeline_ledger_source_key(&run.tenant_id, &run.request_idempotency_key);
        // Ruling S10: only the stored event type varies by bundle family.
        // `PipelineScore` keeps PR 2's ledger event (`Accepted`); a
        // compatibility run's `NoveltyUtility` leg is main's gate-emitted
        // event type, which the pipeline never settles (below), and which
        // `main`'s settlement leaves to the pipeline:
        // `trace_credit_event_counts_toward_settlement` in
        // `trace-commons-ingest.rs` takes only the `external_ref` its own gate
        // writes, never this run's ledger source key. Rulings T15-1 and T15-2:
        // the rest of the row goes with the event type. A `NoveltyUtility`
        // row is written as `main`'s gate path writes that event: the gate
        // caller's actor role (which `main`'s readers can parse), `final`
        // (it never settles, and the batch composition's event-type filter,
        // Ruling T5-1, keeps it out of every batch), and `main`'s reason
        // shape.
        let (ledger_event_type, actor_role, settlement_state, reason) = match trace_credit_event {
            PipelineTraceCreditEvent::PipelineScore => (
                TraceCreditEventType::Accepted,
                PIPELINE_CREDIT_ACTOR_ROLE,
                "pending",
                PIPELINE_CREDIT_REASON.to_string(),
            ),
            PipelineTraceCreditEvent::NoveltyUtility => (
                TraceCreditEventType::NoveltyUtility,
                PIPELINE_NOVELTY_UTILITY_ACTOR_ROLE,
                "final",
                pipeline_novelty_utility_reason(trace_credit_event),
            ),
            // Owner decision D2 (2026-11-03): the graded rule's leg is the
            // same event, written `pending` so its batch composes it below.
            PipelineTraceCreditEvent::SettledNoveltyUtility => (
                TraceCreditEventType::NoveltyUtility,
                PIPELINE_NOVELTY_UTILITY_ACTOR_ROLE,
                "pending",
                pipeline_novelty_utility_reason(trace_credit_event),
            ),
        };
        let ledger_event_type_label = enum_string(&ledger_event_type)?;
        // Zaki review 1, round 2, finding 15: `main` records the principal
        // that issued a `NoveltyUtility` event -- its calling gate worker --
        // as the event's actor. The pipeline issues it as its configured
        // issuer (`TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`), the
        // principal Settle checks against `main`'s central-issuer list. With
        // no issuer the leg is withheld before its ledger row
        // (`novelty_utility_withheld_reason`; Zaki review 3, Z3-2): nobody
        // stands in for the issuer. A `PipelineScore` event keeps the
        // contributor, as PR 2 wrote it.
        let actor_principal_ref = match trace_credit_event {
            PipelineTraceCreditEvent::NoveltyUtility
            | PipelineTraceCreditEvent::SettledNoveltyUtility => {
                self.novelty_utility_checks.issuer_principal_ref.clone()
            }
            PipelineTraceCreditEvent::PipelineScore => Some(account_ref.to_string()),
        };
        // Zaki review 1, item 4 (owner: as `main`): a batched leg's line
        // settles under `main`'s settlement key -- the principal's account,
        // `account:{account_id}`, when it is linked to one -- and records the
        // hold `main` would record when that account has no unambiguous
        // NEAR payout target. Both reads use their own pooled connection and
        // finish before the credit transaction takes one.
        let (settlement_key, payout_hold_reason) = match trace_credit_event {
            PipelineTraceCreditEvent::NoveltyUtility => (account_ref.to_string(), None),
            PipelineTraceCreditEvent::PipelineScore
            | PipelineTraceCreditEvent::SettledNoveltyUtility => {
                self.settlement_key_and_hold(&run.tenant_id, account_ref)
                    .await?
            }
        };
        let witness_provenance_class = self
            .credit_witness_provenance_class(&run.tenant_id, run.submission_id)
            .await;
        // The tenant authority the `NoveltyUtility` check below reads,
        // resolved now, before the credit transaction takes a pooled
        // connection: `main` reads the tenant policy, then credits in a
        // separate transaction, and so does this.
        let novelty_utility_authority = if trace_credit_event.is_novelty_utility() {
            self.resolve_authority(&run.tenant_id).await?
        } else {
            None
        };
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::tenant_transaction(&mut client, &run.tenant_id).await?;
        ensure_current_lease(&tx, run, lease_token).await?;
        let account_lock = format!("pipeline-credit-account:{}:{account_hash}", run.tenant_id);
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 1))",
            &[&account_lock],
        )
        .await?;
        // Holds gate settlement: `main` leaves a held account out of its
        // settlement batches and payouts, and writes `NoveltyUtility` credit
        // regardless of holds (its hold filter covers only settlement-eligible
        // events). So a hold stops only a leg that settles into a batch
        // (owner ruling on Zaki review 1, round 2, finding 13): the minimal
        // family's, and the graded rule's (owner decision D7, 2026-11-03).
        if trace_credit_event.settles_into_batch()
            && crate::credit_hold_ownership::active_credit_holds_on_tx(&tx, &run.tenant_id)
                .await?
                .contains(account_ref)
        {
            // Dropping the transaction rolls it back: nothing was written.
            return Ok(InternalCreditResult::Held);
        }
        // The submission-operability
        // guard Step 6 read before this leg's adapter call is only a
        // snapshot -- `submission_guard` commits and releases its lock
        // immediately. A withdrawal can land in the gap between that read
        // and this transaction, so re-check under this transaction's own
        // lock, before the ledger row is inserted and the batch finalized,
        // and roll back rather than pay a forfeited award.
        //
        // Two statements: lock first, read second. Under READ COMMITTED a
        // later statement in the same transaction is guaranteed to see a
        // withdrawal that committed while the first statement waited for
        // the row lock; relying on a single combined lock-and-read statement
        // for that is not the same guarantee. The `FOR SHARE` lock also
        // makes a withdrawal that has not yet reached its own row update
        // wait until this transaction commits -- the payment then happens
        // before the withdrawal, and withdrawal is not a clawback.
        tx.query_opt(
            "SELECT 1 FROM trace_submissions
              WHERE tenant_id = $1 AND submission_id = $2
              FOR SHARE",
            &[&run.tenant_id, &run.submission_id],
        )
        .await?;
        let submission_operable = tx
            .query_opt(
                concat!(
                    "SELECT ",
                    accepted_submission_sql!(),
                    " FROM trace_submissions s
                      WHERE s.tenant_id = $1 AND s.submission_id = $2"
                ),
                &[&run.tenant_id, &run.submission_id],
            )
            .await?
            .map(|row| row.get::<_, bool>(0))
            .unwrap_or(false);
        if !submission_operable {
            // Dropping the transaction rolls it back: nothing was written.
            return Ok(InternalCreditResult::Inoperable);
        }
        // GRD-004: the Settle policy of the run's bundle must still be
        // runnable, and stays so until this transaction ends. The load-time
        // check and the leg's own dispatch check are snapshots, and a
        // suspension can land after both; this one is under the policy row's
        // lock, the last lock taken (run row, account lock, submission row,
        // policy row) and before the first write, so a suspension that
        // returned to the operator before this point writes no credit event
        // and no batch, and one that has not yet returned waits for this
        // transaction. Dropping the transaction rolls it back.
        if !lock_runnable_policy(&tx, &run.tenant_id, &run.bundle_id, Phase::Settle).await? {
            return Ok(InternalCreditResult::PolicyNotRunnable);
        }
        // Rulings T15-6 and T15-12: `main` appends a `NoveltyUtility` event
        // only when its credit checks pass. A compatibility leg one of them
        // refuses completes here with no ledger row, under `main`'s label;
        // the Score decision stays as it was.
        if trace_credit_event.is_novelty_utility() {
            if let Some(label) = self
                .novelty_utility_withheld_reason(
                    &tx,
                    run,
                    amount.get(),
                    novelty_utility_authority.as_ref(),
                    // Applied by the pre-dispatch check, never here.
                    None,
                )
                .await?
            {
                update_settlement_on_tx(
                    &tx,
                    run,
                    lease_token,
                    &settlement.instrument_id,
                    SettlementUpdate {
                        operation_state: "complete",
                        result_ref_hash: Some(receipt.result_ref_hash()),
                        external_receipt_hash: receipt.external_receipt_hash(),
                        credit_event_id: None,
                        settlement_batch_id: None,
                        payout_state: None,
                        error_label: Some(label),
                    },
                )
                .await?;
                tx.commit().await?;
                return Ok(InternalCreditResult::Complete);
            }
        }
        tx.execute(
            "INSERT INTO trace_credit_ledger (
                tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                event_type, points_delta, reason, external_ref, actor_principal_ref,
                actor_role, settlement_state, pipeline_run_id, score_outcome_id, instrument_id,
                witness_provenance_class
             ) VALUES (
                $1,$2,$3,$4,$5,$12,$6,$7,$8,$15,$13,$14,$9,$10,$11,$16
             )
             ON CONFLICT (tenant_id, credit_event_id) DO NOTHING",
            &[
                &run.tenant_id,
                &event_id,
                &run.submission_id,
                &run.trace_id,
                &account_ref,
                &amount.to_credit_decimal(),
                &reason,
                &external_ref,
                &run.run_id,
                &score_outcome_id,
                &settlement.instrument_id,
                &ledger_event_type_label,
                &actor_role,
                &settlement_state,
                &actor_principal_ref.as_deref().ok_or_else(|| {
                    anyhow::anyhow!(PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL)
                })?,
                &witness_provenance_class,
            ],
        )
        .await?;
        self.inject_crash(PipelineCrashPoint::AfterCreditLedgerInsert)?;
        let event_state: String = tx
            .query_opt(
                "SELECT settlement_state FROM trace_credit_ledger
                  WHERE tenant_id = $1 AND credit_event_id = $2
                    AND pipeline_run_id = $3 AND instrument_id = $4",
                &[
                    &run.tenant_id,
                    &event_id,
                    &run.run_id,
                    &settlement.instrument_id,
                ],
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("settlement_operation_mismatch"))?
            .get("settlement_state");
        // Ruling S10: a flat-rule `NoveltyUtility` leg is never composed or
        // finalized into a batch -- it completes with no
        // `settlement_batch_id`, same as `main` never pays that event.
        // `PipelineScore` keeps PR 2's batch-or-reuse behavior, and the
        // graded rule's leg takes it too (owner decision D2, 2026-11-03), so
        // a payout rail pays it from the batch.
        let batch_id = match trace_credit_event {
            PipelineTraceCreditEvent::NoveltyUtility => None,
            PipelineTraceCreditEvent::PipelineScore
            | PipelineTraceCreditEvent::SettledNoveltyUtility => Some(if event_state == "final" {
                // The event already settled in an earlier batch: reuse that
                // batch rather than composing a second one that carries it.
                let carrying = tx
                    .query(
                        "SELECT settlement_batch_id FROM trace_credit_settlement_batches
                          WHERE tenant_id = $1 AND instrument_id = $2
                            AND status = 'finalized'
                            AND $3 = ANY(source_credit_event_ids)",
                        &[&run.tenant_id, &settlement.instrument_id, &event_id],
                    )
                    .await?;
                anyhow::ensure!(carrying.len() == 1, "settlement_operation_mismatch");
                carrying[0].get::<_, Uuid>("settlement_batch_id")
            } else {
                self.finalize_pending_credit_batch(
                    &tx,
                    run,
                    &settlement.instrument_id,
                    account_ref,
                    &settlement_key,
                    payout_hold_reason,
                    event_id,
                    &ledger_event_type_label,
                )
                .await?
            }),
        };
        update_settlement_on_tx(
            &tx,
            run,
            lease_token,
            &settlement.instrument_id,
            SettlementUpdate {
                operation_state: "complete",
                result_ref_hash: Some(receipt.result_ref_hash()),
                external_receipt_hash: receipt.external_receipt_hash(),
                credit_event_id: Some(event_id),
                settlement_batch_id: batch_id,
                payout_state: None,
                error_label: None,
            },
        )
        .await?;
        tx.commit().await?;
        // A finalized batch is new credit for the payout rails to accrue
        // and pay (payout spec section 7). After the commit, never inside
        // it; a reused batch was announced when it was finalized.
        if batch_id.is_some() && event_state != "final" {
            if let Some(notifier) = self.payout_notifier.get() {
                notifier.notify_all();
            }
        }
        // The leg's credit event needs `main`'s `CreditMutate` audit event,
        // which ingest's worker appends (Zaki review 1, round 2, N-5).
        self.wake_follow_ups(
            &run.tenant_id,
            PipelineFollowUps {
                credit_audits: true,
                ..PipelineFollowUps::default()
            },
        );
        Ok(InternalCreditResult::Complete)
    }

    /// Rulings T15-6 and T15-8 to T15-11: `main`'s `NoveltyUtility` credit
    /// checks (`attempt_emit_novelty_utility_credit` in
    /// `trace-commons-ingest.rs`, and what it calls), in `main`'s order, for
    /// a compatibility leg about to write its ledger row, on the leg's own
    /// credit transaction. `Some` is the label the leg is withheld under,
    /// the label `main` records on its gate decision for the same refusal.
    ///
    /// - The production gate: with `require_production_gate`, the service's
    ///   scorer and embedder must be production-qualified, where `main`
    ///   requires a production gate service (`non_production_gate`).
    /// - The central issuer: with a non-empty issuer allowlist, a positive
    ///   award needs the pipeline's configured issuer on it, where `main`
    ///   needs its calling gate worker there (`central_issuer_denied`).
    /// - The issuer: with no configured pipeline issuer (and an empty list),
    ///   the event has no actor, where `main` always has its calling gate
    ///   worker (`credit_check_error`, as `main` fails a check it cannot
    ///   make; Zaki review 3, Z3-2).
    /// - `main` also applies its calling token's scoped allowlists. Settle
    ///   has no calling token, so that check does not apply here (Ruling
    ///   T15-9); the tenant authority's own allowlists were applied to this
    ///   submission at the receipt (`SubmissionAuthority::permits`).
    /// - The tenant policy comes from the authority provider, the source the
    ///   receipt uses, resolved by the caller just before its transaction
    ///   (`resolve_authority`; a failed read is an error, retried, never a
    ///   withheld leg). No authority for the tenant, no policy while one is
    ///   required, or a submission row whose scopes or uses do not decode is
    ///   `credit_check_error`, where `main` fails the call; a withheld leg is
    ///   never a charged Settle error.
    /// - The submission's allowed uses must include model training, and its
    ///   consent scopes and model training must be inside the policy's
    ///   non-empty allowlists (`policy_mismatch`). `main` also requires the
    ///   submission to be `accepted`; the leg's operability re-check, which
    ///   runs first, already does (Ruling T15-8).
    ///
    /// This is a copy of `main`'s checks in `attempt_emit_novelty_utility_credit`
    /// (`bin/trace-commons-ingest.rs`), which the library cannot call; a
    /// change to either must be made to both.
    pub(super) async fn novelty_utility_withheld_reason(
        &self,
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        amount_microcredits: u64,
        authority: Option<&crate::trace_authority::SubmissionAuthority>,
        duplicate_verdict: Option<&'static str>,
    ) -> anyhow::Result<Option<&'static str>> {
        let checks = &self.novelty_utility_checks;
        // `main` records a duplicate without scoring it, so it never reaches
        // the credit checks below or emits the event. The verdict was
        // decided once, when the run's Settle selection was persisted
        // (`selection_duplicate_verdict`), and the pre-dispatch check of a
        // leg not yet dispatched passes it as `duplicate_verdict`; nothing
        // here decides it again, so a neighbour's later progress cannot
        // withhold a leg already on its way to paying.
        if duplicate_verdict.is_some() {
            return Ok(duplicate_verdict);
        }
        if checks.require_production_gate {
            let qualification = self.dependency_qualification();
            if !(qualification.scorer && qualification.embedder) {
                return Ok(Some(PIPELINE_NOVELTY_UTILITY_NON_PRODUCTION_GATE_LABEL));
            }
        }
        if amount_microcredits > 0
            && !checks.central_issuer_principal_refs.is_empty()
            && !checks
                .issuer_principal_ref
                .as_ref()
                .is_some_and(|issuer| checks.central_issuer_principal_refs.contains(issuer))
        {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_CENTRAL_ISSUER_DENIED_LABEL));
        }
        // Zaki review 3, Z3-2: the event's actor is the configured issuer,
        // and nobody stands in for a missing one, so the leg is withheld, as
        // `main` fails a credit check it cannot make.
        if checks.issuer_principal_ref.is_none() {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL));
        }
        let Some(authority) = authority else {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL));
        };
        if authority.policy.is_none() && authority.require_policy {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL));
        }
        let row = tx
            .query_one(
                "SELECT consent_scopes, allowed_uses FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&run.tenant_id, &run.submission_id],
            )
            .await?;
        let (Ok(consent_scopes), Ok(allowed_uses)) = (
            serde_json::from_value::<Vec<ConsentScope>>(row.get("consent_scopes")),
            serde_json::from_value::<Vec<TraceAllowedUse>>(row.get("allowed_uses")),
        ) else {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_CREDIT_CHECK_ERROR_LABEL));
        };
        let required = TraceAllowedUse::ModelTraining;
        if !allowed_uses.contains(&required) {
            return Ok(Some(PIPELINE_NOVELTY_UTILITY_POLICY_MISMATCH_LABEL));
        }
        if let Some(policy) = authority.policy.as_ref() {
            let scope_outside = !policy.allowed_consent_scopes.is_empty()
                && !consent_scopes
                    .iter()
                    .any(|scope| policy.allowed_consent_scopes.contains(scope));
            let use_outside =
                !policy.allowed_uses.is_empty() && !policy.allowed_uses.contains(&required);
            if scope_outside || use_outside {
                return Ok(Some(PIPELINE_NOVELTY_UTILITY_POLICY_MISMATCH_LABEL));
            }
        }
        Ok(None)
    }

    /// Composes the account's pending pipeline credit events for
    /// `instrument_id` into one batch, writes it finalized, sets its
    /// `instrument_id`, and marks those events final -- all inside the
    /// caller's credit transaction, under the per-account advisory lock the
    /// caller holds. `event_id` (the run's own event) must be among them.
    ///
    /// Ruling T5-1: only pending events whose `event_type` is `event_type`
    /// (the caller's own leg's ledger event type -- `PipelineScore`'s
    /// `Accepted`, computed once by the caller with the same enum-to-string
    /// path the ledger insert used) are eligible, so a `NoveltyUtility` row
    /// for the same account and instrument is never swept into this batch,
    /// since `main` never pays that event (Review Focus 3). Ruling T15-2
    /// writes that row `final`, so the state predicate leaves it out as
    /// well; the event-type predicate does not depend on that.
    ///
    /// The events are the principal's (`account_ref`), as `main`'s events
    /// are; the batch line is `settlement_key`'s, `main`'s settlement key
    /// (`settlement_key_and_hold`), and records `payout_hold_reason`.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn finalize_pending_credit_batch(
        &self,
        tx: &Transaction<'_>,
        run: &PipelineRunRecord,
        instrument_id: &str,
        account_ref: &str,
        settlement_key: &str,
        payout_hold_reason: Option<&'static str>,
        event_id: Uuid,
        event_type: &str,
    ) -> anyhow::Result<Uuid> {
        let pending_events = tx
            .query(
                "SELECT credit_event_id, submission_id, points_delta
                   FROM trace_credit_ledger
                  WHERE tenant_id = $1
                    AND pipeline_run_id IS NOT NULL
                    AND instrument_id = $2
                    AND credit_account_ref = $3
                    AND settlement_state = 'pending'
                    AND event_type = $4
                  ORDER BY credit_event_id",
                &[&run.tenant_id, &instrument_id, &account_ref, &event_type],
            )
            .await?;
        anyhow::ensure!(
            pending_events
                .iter()
                .any(|event| event.get::<_, Uuid>("credit_event_id") == event_id),
            "eligible credit event is missing"
        );
        let event_ids = pending_events
            .iter()
            .map(|event| event.get::<_, Uuid>("credit_event_id"))
            .collect::<Vec<_>>();
        let submission_ids = pending_events
            .iter()
            .map(|event| event.get::<_, Uuid>("submission_id"))
            .collect::<Vec<_>>();
        let list_hash = source_list_hash(&event_ids);
        let settled_micros = pending_events.iter().try_fold(0_i64, |total, event| {
            let points_delta: String = event.get("points_delta");
            let amount = Microcredits::from_credit_decimal(&points_delta)
                .map_err(|_| anyhow::anyhow!("credit_amount_overflow"))?;
            let amount = microcredits_to_settled_i64(amount)?;
            total
                .checked_add(amount)
                .ok_or_else(|| anyhow::anyhow!("credit_amount_overflow"))
        })?;
        let batch_id = pipeline_settlement_batch_id(&run.tenant_id, &list_hash);
        let line_item = TraceCreditAccountSettlementLineItem {
            credit_account_ref: settlement_key.to_string(),
            credit_account_hash: credit_account_hash(settlement_key),
            settled_credit_delta_micros: settled_micros,
            source_credit_event_ids: event_ids.clone(),
            source_submission_ids: submission_ids.clone(),
            source_list_hash: list_hash.clone(),
            near_status: TraceCreditSettlementNearStatus::Disabled,
            near_outbox_id: None,
            near_payout_hold_reason: payout_hold_reason.map(str::to_string),
        };
        let batch = TraceCreditSettlementBatchWrite {
            tenant_id: run.tenant_id.clone(),
            settlement_batch_id: batch_id,
            policy_version: PIPELINE_SETTLEMENT_POLICY_VERSION.to_string(),
            status: TraceCreditSettlementBatchStatus::Finalized,
            reason_hash: list_hash.clone(),
            // `main` records the issuer approval evidence its settlement
            // request named, and none when it named none. Settle names none,
            // and a payout is refused while `main` requires one
            // (`PipelineServiceBuilder::build`).
            issuer_approval_evidence_hash: None,
            source_credit_event_ids: event_ids.clone(),
            source_submission_ids: submission_ids,
            source_list_hash: list_hash,
            settled_credit_points: Microcredits::from_raw(
                u64::try_from(settled_micros).unwrap_or(0),
            )
            .to_credit_decimal(),
            settled_credit_micros: settled_micros,
            line_items: vec![line_item],
            near_contract_id: None,
            ranking_model_version: None,
            ranking_target_use: None,
            ranking_calibration_run_id: None,
            ranking_calibration_report_hash: None,
            ranking_calibration_joined_evidence_hash: None,
            ranking_credit_events_excluded_count: 0,
            ranking_credit_events_excluded_reason_counts: BTreeMap::new(),
            actor_principal_ref: account_ref.to_string(),
        };
        insert_credit_settlement_batch_on_tx(tx, &batch).await?;
        let tagged = tx
            .execute(
                "UPDATE trace_credit_settlement_batches
                    SET instrument_id = $3
                  WHERE tenant_id = $1 AND settlement_batch_id = $2
                    AND (instrument_id IS NULL OR instrument_id = $3)",
                &[&run.tenant_id, &batch_id, &instrument_id],
            )
            .await?;
        anyhow::ensure!(tagged == 1, "settlement_operation_mismatch");
        let finalized = tx
            .execute(
                "UPDATE trace_credit_ledger
                    SET settlement_state = 'final'
                  WHERE tenant_id = $1 AND credit_event_id = ANY($2)
                    AND pipeline_run_id IS NOT NULL
                    AND instrument_id = $3
                    AND settlement_state = 'pending'",
                &[&run.tenant_id, &event_ids, &instrument_id],
            )
            .await?;
        anyhow::ensure!(
            usize::try_from(finalized).ok() == Some(event_ids.len()),
            "settlement_operation_mismatch"
        );
        self.inject_crash(PipelineCrashPoint::AfterCreditBatchFinalize)?;
        Ok(batch_id)
    }

    /// The payout pass (P3-D11): pays out the settled Trace Credit of up
    /// to `limit` of `tenant_id`'s complete runs and returns how many runs
    /// it processed. Nothing at all unless payout is enabled. Payout runs
    /// after Settle, outside any run lease, and never writes a phase
    /// outcome.
    ///
    /// The work list (`PgPipelineStore::list_payout_work_on`, which leaves out
    /// a `pending` leg whose bundle's Settle policy is not runnable) is split in
    /// two:
    ///
    /// - A `submitted` leg only needs its confirmation looked up, which can
    ///   submit nothing, so it is polled without the tenant's NEAR submit
    ///   lock (Ruling T10-10), on one pooled connection, and at most once per
    ///   `PipelinePayoutConfig::confirmation_interval` -- `main`'s NEAR
    ///   outbox scheduler cadence.
    /// - A `pending` leg has a line to submit. Those legs are paid under the
    ///   tenant's NEAR submit lock -- the session advisory lock `main`'s NEAR
    ///   outbox submitter takes (`Database::try_acquire_near_credit_submit_lock`,
    ///   same key) -- held across every external submit, so no two payout
    ///   passes (two ingest replicas, or a pass and a direct
    ///   `process_payout`), and no payout pass and `main`'s submitter, ever
    ///   submit for one tenant at once (Finding I1). When the lock is held,
    ///   those legs wait for the next pass. The lock is taken only when there
    ///   is something to submit, and the locked work runs on the lock's own
    ///   connection, never on a second one (the pool-size-one rule). Every
    ///   path to `NearPayoutAdapter::submit` runs under this lock.
    /// - Before each submit to the adapter -- a first submit, never a
    ///   confirmation lookup -- the pass reads the Settle policy of the run's
    ///   own bundle (GRD-004; `settle_policy_allows_dispatch`, called from
    ///   `dispatch_near_settlements`). A submit that the policy holds back
    ///   writes nothing, charges nothing, and leaves the payout as it was, so
    ///   the run is due again in a later pass; the run is not counted, and the
    ///   pass goes on to its other runs. A `submitted` payout is still
    ///   confirmed while the policy is suspended: the policy guards what is
    ///   dispatched, and an operation the adapter already accepted is
    ///   neither retracted nor left unconfirmed. The read is made on the
    ///   connection the pass holds, so the guard takes no second connection
    ///   (the pool-size-one rule).
    ///
    /// Ruling T10-5: an error in one run's payout -- a missing batch, a
    /// call that cannot be built, confirmation evidence that is not
    /// hash-only -- is recorded on that run's leg (payout `failed`, under the
    /// safe label `payout_error_label` picks) and the pass goes on with the
    /// next run, so one bad run cannot hold up the tenant's other payouts.
    /// Only a database error ends the pass, as does an injected crash (a
    /// test's stand-in for the process dying).
    pub async fn process_payouts(&self, tenant_id: &str, limit: usize) -> anyhow::Result<usize> {
        self.process_payouts_tallied(tenant_id, limit, &mut PipelinePayoutTally::default())
            .await
    }

    /// `process_payouts`, adding to `tally` each outbox line the pass
    /// changed. The counts stay in `tally` when the pass ends with an error,
    /// so the caller can audit the lines that changed before it.
    pub async fn process_payouts_tallied(
        &self,
        tenant_id: &str,
        limit: usize,
        tally: &mut PipelinePayoutTally,
    ) -> anyhow::Result<usize> {
        let Some((_, config)) = self.payout.as_ref() else {
            return Ok(0);
        };
        // `main`'s disabled NEAR settlement mode advances nothing (Zaki
        // review 1, round 2, finding 2).
        if config.controls.settlement_mode == PipelineNearSettlementMode::Disabled {
            return Ok(0);
        }
        let mut client = self.backend.trace_pool().get().await?;
        let work = PgPipelineStore::list_payout_work_on(
            &mut client,
            tenant_id,
            limit,
            config.confirmation_interval,
        )
        .await?;
        let (to_confirm, to_submit): (Vec<_>, Vec<_>) = work
            .into_iter()
            .partition(|(_, payout_state)| payout_state == "submitted");
        let to_confirm = to_confirm
            .into_iter()
            .map(|(run_id, _)| run_id)
            .collect::<Vec<_>>();
        let to_submit = to_submit
            .into_iter()
            .map(|(run_id, _)| run_id)
            .collect::<Vec<_>>();
        let mut processed = self
            .pay_out_runs_on(&mut client, tenant_id, &to_confirm, false, tally)
            .await?;
        drop(client);
        if to_submit.is_empty() {
            return Ok(processed);
        }
        let Some(mut lock) = self.try_lock_payouts(tenant_id).await? else {
            return Ok(processed);
        };
        let result = match lock.client_mut() {
            Some(client) => {
                self.pay_out_runs_on(client, tenant_id, &to_submit, true, tally)
                    .await
            }
            None => Err(anyhow::anyhow!(PIPELINE_PAYOUT_LOCK_HELD_LABEL)),
        };
        let released = lock.release().await;
        processed += payout_result_after_release(result, released)?;
        Ok(processed)
    }

    /// Pays out one run's completed Trace Credit legs
    /// (`dispatch_near_settlements`), whatever the state of the run, under
    /// the same tenant lock as the pass's submits (Finding I1): while a pass
    /// or `main`'s submitter holds it, this is refused with
    /// `payout_lock_held`. `None` when the run does not exist; a run with no
    /// completed, payout-eligible leg is returned untouched. Unlike the
    /// pass, this also takes up a `failed` payout again (each `failed` line
    /// but one that failed on chain), does not wait for
    /// the confirmation interval, and returns a per-run error to its caller
    /// instead of recording it.
    ///
    /// It shares the pass's dispatch guard (GRD-004): a submit, a retry of a
    /// `failed` payout included, is held back while the Settle policy of the
    /// run's bundle is not runnable. Then nothing is written, charged, or
    /// called, and the run is returned untouched, as for a run that has
    /// nothing to pay now. A confirmation lookup of a `submitted` payout is
    /// not a dispatch and proceeds.
    pub async fn process_payout(
        &self,
        tenant_id: &str,
        run_id: Uuid,
    ) -> anyhow::Result<Option<PipelineRunRecord>> {
        if !self.payout_enabled()
            || self
                .payout_controls()
                .map(|controls| controls.settlement_mode)
                == Some(PipelineNearSettlementMode::Disabled)
        {
            return Ok(self.store.get_run(tenant_id, run_id).await?);
        }
        let Some(mut lock) = self.try_lock_payouts(tenant_id).await? else {
            anyhow::bail!(PIPELINE_PAYOUT_LOCK_HELD_LABEL);
        };
        let result = match lock.client_mut() {
            Some(client) => {
                self.process_payout_on(
                    client,
                    tenant_id,
                    run_id,
                    true,
                    true,
                    &mut PipelinePayoutTally::default(),
                )
                .await
                .map(|attempt| match attempt {
                    PayoutAttempt::NoRun => None,
                    // A held run is returned as a run with nothing to pay
                    // now is: untouched.
                    PayoutAttempt::Done(run) | PayoutAttempt::Held(run) => Some(run),
                })
            }
            None => Err(anyhow::anyhow!(PIPELINE_PAYOUT_LOCK_HELD_LABEL)),
        };
        let released = lock.release().await;
        payout_result_after_release(result, released)
    }

    /// The tenant's NEAR submit lock (`main`'s key), or `None` when another
    /// pass or `main`'s submitter holds it.
    pub(super) async fn try_lock_payouts(
        &self,
        tenant_id: &str,
    ) -> anyhow::Result<Option<crate::db::NearCreditSubmitAdvisoryLock>> {
        Ok(crate::db::Database::try_acquire_near_credit_submit_lock(
            self.backend.as_ref(),
            tenant_id,
        )
        .await?)
    }

    /// Pays out `run_ids` on `client`, recording a per-run error on its leg
    /// and going on (Ruling T10-5). `may_submit` is true only on the
    /// connection that holds the tenant's NEAR submit lock. The pass never
    /// takes up a `failed` payout again (Ruling F-I3), including one that
    /// failed after the pass listed it: only `process_payout` retries one.
    /// A run that does not exist, or whose dispatch the Settle policy held
    /// back, is not counted, and does not stop the runs after it.
    pub(super) async fn pay_out_runs_on(
        &self,
        client: &mut deadpool_postgres::Client,
        tenant_id: &str,
        run_ids: &[Uuid],
        may_submit: bool,
        tally: &mut PipelinePayoutTally,
    ) -> anyhow::Result<usize> {
        let mut processed = 0;
        for &run_id in run_ids {
            match self
                .process_payout_on(client, tenant_id, run_id, may_submit, false, tally)
                .await
            {
                Ok(PayoutAttempt::Done(_)) => processed += 1,
                Ok(PayoutAttempt::NoRun | PayoutAttempt::Held(_)) => {}
                Err(error)
                    if error.to_string() == INJECTED_PIPELINE_CRASH
                        || is_database_error(&error) =>
                {
                    return Err(error);
                }
                Err(error) => {
                    set_payout_state_on(
                        client,
                        tenant_id,
                        run_id,
                        TraceCreditSettlementNearStatus::Failed,
                        Some(payout_error_label(&error)),
                    )
                    .await?;
                    processed += 1;
                }
            }
        }
        Ok(processed)
    }

    /// `process_payout`'s body, on `client`. `may_submit` as in
    /// `pay_out_runs_on`; `retry_failed` as in `dispatch_near_settlements`.
    /// `Held` when `dispatch_near_settlements` held a submit back because the
    /// Settle policy of the run's bundle is not runnable (GRD-004).
    pub(super) async fn process_payout_on(
        &self,
        client: &mut deadpool_postgres::Client,
        tenant_id: &str,
        run_id: Uuid,
        may_submit: bool,
        retry_failed: bool,
        tally: &mut PipelinePayoutTally,
    ) -> anyhow::Result<PayoutAttempt> {
        let tx = PgPipelineStore::tenant_transaction(client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT * FROM pipeline_runs WHERE tenant_id = $1 AND run_id = $2",
                &[&tenant_id, &run_id],
            )
            .await?;
        tx.commit().await?;
        let Some(run) = row.as_ref().map(pipeline_run_from_row).transpose()? else {
            return Ok(PayoutAttempt::NoRun);
        };
        // Zaki review 1, round 2, finding 16: a leg Settle completed and
        // ledgered is final, whatever the run did afterwards -- a run that
        // then failed for good (attempts exhausted, or a crash before Settle's
        // own commit on its last attempt) still pays its completed leg, as a
        // withdrawal does not stop one. `dispatch_near_settlements` pays only
        // complete, payout-eligible legs.
        let held = self
            .dispatch_near_settlements(client, &run, may_submit, retry_failed, tally)
            .await?;
        Ok(if held {
            PayoutAttempt::Held(run)
        } else {
            PayoutAttempt::Done(run)
        })
    }

    /// Port lines 5114 to 5309, for PR 3. For each completed `trace_credit`
    /// leg of `run` on the `near` rail whose payout is `pending` or
    /// `submitted` -- or `failed`, only when `retry_failed` -- pays each line
    /// of the leg's finalized batch
    /// through one `trace_near_credit_outbox` row, keyed by
    /// `pipeline_near_outbox_line_id` and tagged `instrument_id =
    /// 'trace_credit'`, then records the leg's payout state. It submits only
    /// when `may_submit` -- only on the connection that holds the tenant's
    /// NEAR submit lock (Finding I1); without it, a line that still needs a
    /// submit is left for a locked pass, and only confirmations are looked
    /// up (Ruling T10-10). Every outbox read and write is tenant-scoped,
    /// and every write is conditional on the line's current status, so a
    /// repeat is a no-op.
    ///
    /// - A leg without a batch (a compatibility `NoveltyUtility` event) is
    ///   never paid (Ruling S8).
    /// - A line with no outbox row gets a new call on the configured NEAR
    ///   contract (Ruling T10-4). A line with a row replays the call stored
    ///   in it, never one rebuilt from the current configuration (Finding
    ///   I2): its idempotency key names the contract it was first built for.
    ///   A stored call whose contract is no longer the configured one is
    ///   never submitted again -- that could pay twice, on two contracts --
    ///   and the payout ends `failed` under `near_contract_changed`; a
    ///   `submitted` line is still confirmed through its stored key.
    /// - The submission guard is not read: a leg reaches `complete` only
    ///   through Settle, while the submission was operable, and a later
    ///   withdrawal forfeits only the legs Settle has not completed. Its
    ///   settled credit is paid and confirmed after a withdrawal, as
    ///   `main`'s submitter pays finalized credit (owner decision, Zaki
    ///   review 1, item S).
    /// - A line already `submitted` is never submitted again: only its
    ///   confirmation is looked up. The submit is recorded after the
    ///   adapter returns, so a crash between the two repeats the call on
    ///   the next pass, under the same idempotency key
    ///   (`NearPayoutAdapter::submit`).
    /// - A failed submit marks the line and the payout `failed` under
    ///   `near_submit_failed`; the pass does not list it again.
    /// - `retry_failed` is true only from a direct `process_payout`
    ///   (Ruling F-I3). It takes up each `failed` line again, apart from a
    ///   line that failed on chain (`near_transaction_failed`), which no
    ///   pass submits again. Without it, a `failed` payout, or a `failed` line of
    ///   a payout still `pending`, is never submitted again: another replica
    ///   can fail a payout after this pass listed it as `pending`, and the
    ///   re-read here sees that.
    /// - Confirmation evidence is hash-only (else `near_confirmation_invalid`);
    ///   the evidence and the `confirmed` status commit together.
    /// - A `submitted` line whose transaction the adapter reports as failed
    ///   on chain becomes `failed` under `near_transaction_failed`, in the
    ///   mode that submitted it, and keeps its transaction hash. The pass
    ///   does not submit it again.
    /// - The one policy guard (GRD-004) sits at the dispatch, where a line
    ///   would be sent to the adapter -- a first submit, or a new submit of a
    ///   `failed` line under `retry_failed` -- and before anything that
    ///   belongs to it is written (the outbox line, the payout state): the
    ///   Settle policy of the run's bundle must be runnable
    ///   (`settle_policy_allows_dispatch`). A line held back is left as it
    ///   was, and the settlement's payout state is not recomputed, because a
    ///   line without a row would otherwise read as paid. A confirmation
    ///   lookup of a `submitted` line is not a dispatch and is never guarded;
    ///   it records what the adapter reports. Both callers reach the submit
    ///   here, so no path dispatches around the guard.
    ///
    /// Returns whether any submit was held back by the guard.
    pub(super) async fn dispatch_near_settlements(
        &self,
        client: &mut deadpool_postgres::Client,
        run: &PipelineRunRecord,
        may_submit: bool,
        retry_failed: bool,
        tally: &mut PipelinePayoutTally,
    ) -> anyhow::Result<bool> {
        let mut any_held = false;
        let Some((injected, config)) = self.payout.as_ref() else {
            return Ok(false);
        };
        // `main`'s NEAR settlement mode picks who pays, as it picks the
        // submitter and confirmer `main`'s outbox worker drives (Zaki review
        // 1, round 2, finding 2): nobody while it is disabled, the in-process
        // dry-run adapter under `dry_run`, and the injected one under `http`.
        let dry_run = DryRunNearPayoutAdapter;
        let adapter: &dyn NearPayoutAdapter = match config.controls.settlement_mode {
            PipelineNearSettlementMode::Disabled => return Ok(false),
            PipelineNearSettlementMode::DryRun => &dry_run,
            PipelineNearSettlementMode::Http => injected.as_ref(),
        };
        // `PipelineServiceBuilder::build` refuses an enabled payout without one.
        let near_contract_id = config
            .near_contract_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_PAYOUT_NEAR_CONTRACT_MISSING_LABEL))?;
        let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
        let settlements = tx
            .query(
                "SELECT *, atomic_units::TEXT AS atomic_units_text
                   FROM pipeline_run_settlements
                  WHERE tenant_id = $1 AND run_id = $2
                  ORDER BY instrument_id",
                &[&run.tenant_id, &run.run_id],
            )
            .await?;
        tx.commit().await?;
        let settlements = settlements
            .iter()
            .map(pipeline_settlement_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        for settlement in settlements.into_iter().filter(|settlement| {
            settlement.instrument_id == InstrumentId::trace_credit().as_str()
                && settlement.operation_state == "complete"
                && settlement.payout_rail == "near"
                && settlement.payout_eligible
                && match settlement.payout_state.as_str() {
                    "pending" | "submitted" => true,
                    "failed" => retry_failed,
                    _ => false,
                }
        }) {
            let Some(batch_id) = settlement.settlement_batch_id else {
                continue;
            };
            let (batch_source_list_hash, lines) =
                load_payout_batch(client, run, &settlement, batch_id).await?;
            let mut work = Vec::new();
            for line in lines
                .iter()
                .filter(|line| line.settled_credit_delta_micros > 0)
            {
                let outbox_id = pipeline_near_outbox_line_id(
                    &run.tenant_id,
                    batch_id,
                    &line.credit_account_hash,
                );
                let (status, chain_failed, submission_mode, call) =
                    match near_outbox_line(client, run, outbox_id).await? {
                        Some((status, last_error_hash, stored_call)) => {
                            let chain_failed = status == "failed"
                                && last_error_hash.as_deref()
                                    == Some(near_transaction_failed_hash().as_str());
                            // Multi-lens review C3 (owner decision): a line with
                            // no key was submitted by code from before the key.
                            // It reads as `http`, so `http` confirms it and the
                            // dry-run adapter never puts a synthetic hash on it.
                            let submission_mode = Some(
                                stored_call
                                    .get(PIPELINE_NEAR_SUBMISSION_MODE_KEY)
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or(PIPELINE_NEAR_SUBMISSION_MODE_DEFAULT.as_label())
                                    .to_string(),
                            );
                            let call: crate::near_credit::NearCreditReceiptCall =
                                serde_json::from_value(stored_call).map_err(|_| {
                                    anyhow::anyhow!(PIPELINE_NEAR_CALL_INVALID_LABEL)
                                })?;
                            call.validate()
                                .map_err(|_| anyhow::anyhow!(PIPELINE_NEAR_CALL_INVALID_LABEL))?;
                            (Some(status), chain_failed, submission_mode, call)
                        }
                        None => (
                            None,
                            false,
                            None,
                            disabled_near_call(
                                near_contract_id,
                                batch_id,
                                &line.credit_account_hash,
                                &batch_source_list_hash,
                                line.settled_credit_delta_micros,
                            )
                            .map_err(|_| anyhow::anyhow!(PIPELINE_NEAR_CALL_INVALID_LABEL))?,
                        ),
                    };
                work.push((line, call, outbox_id, status, chain_failed, submission_mode));
            }

            let mut contract_changed = false;
            let mut held_payout = None;
            let mut dispatch_held = false;
            let mode = config.controls.settlement_mode.as_label();
            for (line, call, outbox_id, status, chain_failed, submission_mode) in &work {
                let outbox_id = *outbox_id;
                let mut submission_mode = submission_mode.clone();
                match status.as_deref() {
                    Some("confirmed") | Some("disabled") => continue,
                    // A line that failed on chain is final: not even a direct
                    // `process_payout` submits it again.
                    Some("failed") if *chain_failed => continue,
                    Some("failed") if !retry_failed => continue,
                    Some("submitted") => {}
                    _ => {
                        if !may_submit {
                            continue;
                        }
                        // GRD-004: the dispatch guard, before the contract
                        // check, the outbox line, and the submit. A held line
                        // changes nothing; it goes in a later pass.
                        if !settle_policy_allows_dispatch(client, run).await? {
                            dispatch_held = true;
                            continue;
                        }
                        if call.contract_id != near_contract_id {
                            contract_changed = true;
                            continue;
                        }
                        if status.is_none() {
                            // Zaki review 1, item 4: an account's line is
                            // paid to the account's NEAR payout target,
                            // resolved again now, as `main`'s held-line
                            // repair does; a line `main` would hold writes
                            // no outbox row and waits for a later pass.
                            let payout_near_account_id =
                                match payout_target_for_line(client, run, line).await? {
                                    Ok(target) => target,
                                    Err(hold_label) => {
                                        held_payout = Some(hold_label);
                                        continue;
                                    }
                                };
                            insert_near_outbox_line(
                                client,
                                run,
                                &settlement,
                                batch_id,
                                outbox_id,
                                &line.credit_account_hash,
                                payout_near_account_id.as_deref(),
                                call,
                            )
                            .await?;
                        }
                        match adapter.submit(call).await {
                            Ok(transaction_ref) => {
                                mark_near_outbox_line_submitted(
                                    client,
                                    run,
                                    outbox_id,
                                    &sha256_prefixed(transaction_ref.as_bytes()),
                                    mode,
                                )
                                .await?;
                                tally.submitted += 1;
                                submission_mode = Some(mode.to_string());
                                self.inject_crash(PipelineCrashPoint::AfterNearSubmit)?;
                            }
                            Err(_) => {
                                mark_near_outbox_line_failed(
                                    client,
                                    run,
                                    outbox_id,
                                    &sha256_prefixed(PIPELINE_NEAR_SUBMIT_FAILED_LABEL.as_bytes()),
                                )
                                .await?;
                                tally.submit_failed += 1;
                                continue;
                            }
                        }
                    }
                }
                // Zaki review 3, Z3-L2: a line is confirmed only in the mode
                // that submitted it. The dry-run adapter cannot know what
                // became of a line the injected adapter sent (its synthetic
                // hash would replace the real one), and the injected one
                // never saw a dry-run line; such a line waits for its mode.
                if submission_mode.as_deref() != Some(mode) {
                    continue;
                }
                let evidence = match adapter.confirmation(&call.idempotency_key).await {
                    NearPayoutConfirmation::Pending => continue,
                    NearPayoutConfirmation::Confirmed(evidence) => evidence,
                    NearPayoutConfirmation::Failed => {
                        // The transaction failed on chain. The line becomes
                        // `failed` only in the mode that submitted it, and
                        // keeps its transaction hash; no pass submits it
                        // again.
                        let tx =
                            PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
                        let failed_rows = tx
                            .execute(
                                "UPDATE trace_near_credit_outbox
                                    SET status = 'failed', confirmed_at = NULL,
                                        last_error_hash = $3
                                  WHERE tenant_id = $1 AND near_outbox_id = $2
                                    AND status = 'submitted'
                                    AND COALESCE(
                                            near_call_json ->> 'pipeline_submission_mode',
                                            $5::TEXT
                                        ) = $4",
                                &[
                                    &run.tenant_id,
                                    &outbox_id,
                                    &near_transaction_failed_hash(),
                                    &mode,
                                    &PIPELINE_NEAR_SUBMISSION_MODE_DEFAULT.as_label(),
                                ],
                            )
                            .await?;
                        tx.commit().await?;
                        tally.chain_failed += failed_rows;
                        continue;
                    }
                };
                if !(evidence.transaction_hash_hash.starts_with("sha256:")
                    && evidence.receipt_hash.starts_with("sha256:"))
                {
                    anyhow::bail!(PIPELINE_NEAR_CONFIRMATION_INVALID_LABEL);
                }
                let tx = PgPipelineStore::tenant_transaction(client, &run.tenant_id).await?;
                let confirmed_rows = tx
                    .execute(
                        "UPDATE trace_near_credit_outbox
                        SET near_call_json = jsonb_set(
                                near_call_json,
                                '{confirmation_evidence}',
                                jsonb_build_object(
                                    'transaction_hash_hash', $3::TEXT,
                                    'receipt_hash', $4::TEXT
                                ),
                                TRUE
                            ),
                            status = 'confirmed',
                            near_transaction_hash = $3,
                            confirmed_at = NOW(),
                            last_error_hash = NULL
                      WHERE tenant_id = $1 AND near_outbox_id = $2
                        AND status = 'submitted'
                        AND COALESCE(
                                near_call_json ->> 'pipeline_submission_mode', $6::TEXT
                            ) = $5",
                        &[
                            &run.tenant_id,
                            &outbox_id,
                            &evidence.transaction_hash_hash,
                            &evidence.receipt_hash,
                            &mode,
                            &PIPELINE_NEAR_SUBMISSION_MODE_DEFAULT.as_label(),
                        ],
                    )
                    .await?;
                tx.commit().await?;
                tally.confirmed += confirmed_rows;
                self.inject_crash(PipelineCrashPoint::AfterNearConfirm)?;
            }
            if dispatch_held {
                // A line was held back, so the lines that have a row do not
                // say how the payout stands; its state stays as it was until
                // the pass that dispatches the line.
                any_held = true;
                continue;
            }
            if contract_changed {
                set_payout_state_on(
                    client,
                    &run.tenant_id,
                    run.run_id,
                    TraceCreditSettlementNearStatus::Failed,
                    Some(PIPELINE_NEAR_CONTRACT_CHANGED_LABEL),
                )
                .await?;
            } else if let Some(label) = held_payout {
                // Held, as `main` holds it: still `pending`, under the hold
                // label, so a later pass resolves the account again (once the
                // confirmation interval has passed, `list_payout_work_on`).
                set_payout_state_on(
                    client,
                    &run.tenant_id,
                    run.run_id,
                    TraceCreditSettlementNearStatus::Pending,
                    Some(label),
                )
                .await?;
            } else {
                record_payout_state_on(client, run, batch_id).await?;
            }
        }
        Ok(any_held)
    }
}
