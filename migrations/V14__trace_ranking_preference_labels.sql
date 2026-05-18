-- Persist hash-only pairwise preference evidence for ranking training.

CREATE TABLE IF NOT EXISTS trace_ranking_preference_labels (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    preference_label_id UUID NOT NULL,
    preferred_submission_id UUID NOT NULL,
    preferred_trace_id UUID NOT NULL,
    rejected_submission_id UUID NOT NULL,
    rejected_trace_id UUID NOT NULL,
    target_use TEXT NOT NULL,
    label_source TEXT NOT NULL CHECK (label_source IN ('frontier_lab', 'reviewer', 'benchmark', 'system')),
    utility_category TEXT NOT NULL CHECK (utility_category IN ('model_training', 'ranking_training', 'evaluation', 'regression')),
    preference_strength_micros BIGINT NOT NULL CHECK (preference_strength_micros > 0),
    evidence_hash TEXT NOT NULL,
    external_ref_hash TEXT NOT NULL,
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, preference_label_id),
    UNIQUE (
        tenant_id,
        preferred_submission_id,
        rejected_submission_id,
        target_use,
        label_source,
        external_ref_hash
    ),
    CHECK (preferred_submission_id <> rejected_submission_id),
    FOREIGN KEY (tenant_id, preferred_submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, rejected_submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_ranking_preference_labels_preferred_submission
    ON trace_ranking_preference_labels (tenant_id, preferred_submission_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_ranking_preference_labels_rejected_submission
    ON trace_ranking_preference_labels (tenant_id, rejected_submission_id, created_at DESC);

ALTER TABLE trace_ranking_preference_labels ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_ranking_preference_labels
    USING (tenant_id = current_setting('trace_commons.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('trace_commons.trace_tenant_id', true));
ALTER TABLE trace_ranking_preference_labels FORCE ROW LEVEL SECURITY;
