-- Trace ranking calibration run summaries for model promotion evidence.

CREATE TABLE IF NOT EXISTS trace_ranking_calibration_runs (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    calibration_run_id UUID NOT NULL,
    model_version TEXT NOT NULL,
    target_use TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    evaluation_dataset_hash TEXT NOT NULL,
    prediction_count INTEGER NOT NULL,
    label_count INTEGER NOT NULL,
    joined_label_prediction_count INTEGER NOT NULL,
    average_predicted_utility_micros BIGINT,
    average_label_utility_delta_micros BIGINT,
    average_absolute_error_micros BIGINT,
    mean_signed_error_micros BIGINT,
    low_confidence_prediction_count INTEGER NOT NULL,
    confidence_threshold REAL NOT NULL,
    min_label_count INTEGER NOT NULL,
    max_average_absolute_error_micros BIGINT NOT NULL,
    promotable BOOLEAN NOT NULL,
    reason_codes JSONB NOT NULL DEFAULT '[]'::JSONB,
    report_hash TEXT NOT NULL,
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, calibration_run_id)
);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_calibration_runs_model
    ON trace_ranking_calibration_runs (tenant_id, model_version, target_use, created_at DESC);

ALTER TABLE trace_ranking_calibration_runs ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_calibration_runs
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));
