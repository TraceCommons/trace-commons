-- Hash-only registry for calibration/holdout datasets used by ranking promotion gates.

CREATE TABLE IF NOT EXISTS trace_ranking_calibration_datasets (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    calibration_dataset_hash TEXT NOT NULL CHECK (calibration_dataset_hash LIKE 'sha256:%'),
    target_use TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    source_manifest_hash TEXT NOT NULL CHECK (source_manifest_hash LIKE 'sha256:%'),
    source_count INTEGER NOT NULL CHECK (source_count > 0),
    label_source_count INTEGER NOT NULL CHECK (label_source_count > 0),
    label_actor_count INTEGER NOT NULL CHECK (label_actor_count > 0),
    status TEXT NOT NULL CHECK (status IN ('candidate', 'active', 'deprecated', 'archived')),
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, calibration_dataset_hash, target_use, policy_version)
);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_calibration_datasets_status
    ON trace_ranking_calibration_datasets (tenant_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_calibration_datasets_policy
    ON trace_ranking_calibration_datasets (tenant_id, target_use, policy_version, created_at DESC);

ALTER TABLE trace_ranking_calibration_datasets ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_ranking_calibration_datasets FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_calibration_datasets
    USING (tenant_id = current_setting('trace-commons.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('trace-commons.trace_tenant_id', true));
