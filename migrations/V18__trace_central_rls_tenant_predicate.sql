CREATE OR REPLACE FUNCTION trace_current_tenant_id()
RETURNS TEXT
LANGUAGE SQL
STABLE
AS $$
    SELECT NULLIF(current_setting('trace_commons.trace_tenant_id', true), '');
$$;

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_tenants;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tenants
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_tenant_policies;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tenant_policies
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_tenant_access_grants;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tenant_access_grants
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_submissions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_submissions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_object_refs;
CREATE POLICY trace_corpus_tenant_isolation ON trace_object_refs
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_derived_records;
CREATE POLICY trace_corpus_tenant_isolation ON trace_derived_records
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_audit_events;
CREATE POLICY trace_corpus_tenant_isolation ON trace_audit_events
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_credit_ledger;
CREATE POLICY trace_corpus_tenant_isolation ON trace_credit_ledger
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_tombstones;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tombstones
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_vector_entries;
CREATE POLICY trace_corpus_tenant_isolation ON trace_vector_entries
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_export_manifests;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_manifests
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_export_manifest_items;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_manifest_items
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_retention_jobs;
CREATE POLICY trace_corpus_tenant_isolation ON trace_retention_jobs
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_retention_job_items;
CREATE POLICY trace_corpus_tenant_isolation ON trace_retention_job_items
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_export_access_grants;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_access_grants
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_export_jobs;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_jobs
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_revocation_propagation_items;
CREATE POLICY trace_corpus_tenant_isolation ON trace_revocation_propagation_items
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_utility_attestations;
CREATE POLICY trace_corpus_tenant_isolation ON trace_utility_attestations
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_credit_settlement_batches;
CREATE POLICY trace_corpus_tenant_isolation ON trace_credit_settlement_batches
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_credit_holds;
CREATE POLICY trace_corpus_tenant_isolation ON trace_credit_holds
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_near_credit_outbox;
CREATE POLICY trace_corpus_tenant_isolation ON trace_near_credit_outbox
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_benchmark_registry_outbox;
CREATE POLICY trace_corpus_tenant_isolation ON trace_benchmark_registry_outbox
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_ranking_model_versions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_model_versions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_ranking_calibration_datasets;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_calibration_datasets
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_ranking_features;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_features
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_ranking_predictions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_predictions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_ranking_labels;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_labels
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_ranking_preference_labels;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_preference_labels
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_ranking_calibration_runs;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_calibration_runs
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_ranking_worker_runs;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_worker_runs
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
