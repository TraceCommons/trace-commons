-- Trace ranking model versions, features, predictions, and labels.

CREATE TABLE IF NOT EXISTS trace_ranking_model_versions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    model_version TEXT NOT NULL,
    feature_schema_version TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('candidate', 'active', 'deprecated', 'archived')),
    training_dataset_hash TEXT NOT NULL,
    calibration_dataset_hash TEXT NOT NULL,
    model_artifact_hash TEXT NOT NULL,
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, model_version)
);

CREATE TABLE IF NOT EXISTS trace_ranking_features (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    ranking_feature_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    target_use TEXT NOT NULL,
    feature_schema_version TEXT NOT NULL,
    feature_vector_hash TEXT NOT NULL,
    feature_names_hash TEXT NOT NULL,
    source_feature_hash TEXT NOT NULL,
    duplicate_score REAL,
    novelty_score REAL,
    privacy_risk_score REAL,
    quality_score REAL,
    coverage_tags JSONB NOT NULL DEFAULT '[]'::JSONB,
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, ranking_feature_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_features_submission
    ON trace_ranking_features (tenant_id, submission_id, created_at DESC);

CREATE TABLE IF NOT EXISTS trace_ranking_predictions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    ranking_prediction_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    target_use TEXT NOT NULL,
    model_version TEXT NOT NULL,
    feature_schema_version TEXT NOT NULL,
    prediction_policy_version TEXT NOT NULL,
    feature_vector_hash TEXT NOT NULL,
    predicted_utility_micros BIGINT NOT NULL,
    uncertainty_micros BIGINT NOT NULL,
    confidence REAL NOT NULL,
    risk_penalty_micros BIGINT NOT NULL,
    novelty_bonus_micros BIGINT NOT NULL,
    settlement_score_micros BIGINT NOT NULL,
    explanation_codes JSONB NOT NULL DEFAULT '[]'::JSONB,
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, ranking_prediction_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_predictions_submission
    ON trace_ranking_predictions (tenant_id, submission_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_predictions_model
    ON trace_ranking_predictions (tenant_id, model_version, created_at DESC);

CREATE TABLE IF NOT EXISTS trace_ranking_labels (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    ranking_label_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    target_use TEXT NOT NULL,
    label_source TEXT NOT NULL CHECK (label_source IN ('frontier_lab', 'reviewer', 'benchmark', 'system')),
    utility_category TEXT NOT NULL CHECK (utility_category IN ('model_training', 'ranking_training', 'evaluation', 'regression')),
    label_outcome TEXT NOT NULL CHECK (label_outcome IN ('useful', 'neutral', 'rejected', 'regression_caught', 'disputed')),
    utility_delta_micros BIGINT NOT NULL,
    evidence_hash TEXT NOT NULL,
    external_ref_hash TEXT NOT NULL,
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, ranking_label_id),
    UNIQUE (tenant_id, submission_id, target_use, label_source, external_ref_hash),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_labels_submission
    ON trace_ranking_labels (tenant_id, submission_id, created_at DESC);

ALTER TABLE trace_ranking_model_versions ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_model_versions
    USING (tenant_id = current_setting('trace_commons.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('trace_commons.trace_tenant_id', true));

ALTER TABLE trace_ranking_features ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_features
    USING (tenant_id = current_setting('trace_commons.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('trace_commons.trace_tenant_id', true));

ALTER TABLE trace_ranking_predictions ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_predictions
    USING (tenant_id = current_setting('trace_commons.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('trace_commons.trace_tenant_id', true));

ALTER TABLE trace_ranking_labels ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_labels
    USING (tenant_id = current_setting('trace_commons.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('trace_commons.trace_tenant_id', true));
