-- Durable external benchmark registry outbox for published benchmark artifacts.

CREATE TABLE IF NOT EXISTS trace_benchmark_registry_outbox (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    benchmark_outbox_id UUID NOT NULL,
    conversion_id UUID NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('publish', 'revoke')),
    registry_ref TEXT NOT NULL,
    artifact_payload_hash TEXT NOT NULL,
    source_submission_ids_hash TEXT NOT NULL,
    evaluator_ref TEXT,
    evaluation_score REAL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'submitted', 'confirmed', 'failed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    submitted_at TIMESTAMPTZ,
    external_receipt_ref TEXT,
    last_error_hash TEXT,
    confirmed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, benchmark_outbox_id),
    UNIQUE (tenant_id, conversion_id, operation)
);
CREATE INDEX IF NOT EXISTS idx_trace_benchmark_registry_outbox_status
    ON trace_benchmark_registry_outbox (tenant_id, status, created_at);

ALTER TABLE trace_benchmark_registry_outbox ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_benchmark_registry_outbox
    USING (tenant_id = current_setting('trace-commons.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('trace-commons.trace_tenant_id', true));
ALTER TABLE trace_benchmark_registry_outbox FORCE ROW LEVEL SECURITY;
