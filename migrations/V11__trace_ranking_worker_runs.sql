-- Trace ranking worker run ledger for credit-grade automation accounting.

CREATE TABLE IF NOT EXISTS trace_ranking_worker_runs (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    ranking_worker_run_id UUID NOT NULL,
    run_kind TEXT NOT NULL,
    dry_run BOOLEAN NOT NULL,
    reason_hash TEXT NOT NULL,
    model_version TEXT,
    target_use TEXT,
    policy_version TEXT,
    limit_count INTEGER NOT NULL,
    checked_count INTEGER NOT NULL,
    succeeded_count INTEGER NOT NULL,
    skipped_existing_count INTEGER NOT NULL,
    skipped_model_risk_count INTEGER NOT NULL,
    skipped_ineligible_count INTEGER NOT NULL,
    pending_after_count INTEGER NOT NULL,
    result_refs JSONB NOT NULL DEFAULT '[]'::JSONB,
    reason_counts JSONB NOT NULL DEFAULT '{}'::JSONB,
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, ranking_worker_run_id)
);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_worker_runs_kind
    ON trace_ranking_worker_runs (tenant_id, run_kind, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_worker_runs_model
    ON trace_ranking_worker_runs (tenant_id, model_version, target_use, policy_version, created_at DESC);

ALTER TABLE trace_ranking_worker_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_ranking_worker_runs FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_worker_runs
    USING (tenant_id = current_setting('trace-commons.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('trace-commons.trace_tenant_id', true));
