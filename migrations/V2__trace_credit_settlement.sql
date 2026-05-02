-- Trace credit settlement, holds, utility attestations, and NEAR receipt outbox.

CREATE TABLE IF NOT EXISTS trace_utility_attestations (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    attestation_id UUID NOT NULL,
    event_type TEXT NOT NULL,
    use_category TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    evidence_hash TEXT NOT NULL,
    external_ref_hash TEXT NOT NULL,
    source_submission_ids UUID[] NOT NULL DEFAULT '{}',
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, attestation_id),
    UNIQUE (tenant_id, event_type, external_ref_hash)
);
CREATE INDEX IF NOT EXISTS idx_trace_utility_attestations_created
    ON trace_utility_attestations (tenant_id, created_at DESC);

CREATE TABLE IF NOT EXISTS trace_credit_settlement_batches (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    settlement_batch_id UUID NOT NULL,
    policy_version TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('dry_run', 'finalized', 'failed')),
    reason_hash TEXT NOT NULL,
    source_credit_event_ids UUID[] NOT NULL DEFAULT '{}',
    source_submission_ids UUID[] NOT NULL DEFAULT '{}',
    source_list_hash TEXT NOT NULL,
    settled_credit_points TEXT NOT NULL,
    settled_credit_micros BIGINT NOT NULL,
    line_items_json JSONB NOT NULL DEFAULT '[]'::JSONB,
    near_contract_id TEXT,
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, settlement_batch_id),
    UNIQUE (tenant_id, source_list_hash, policy_version)
);
CREATE INDEX IF NOT EXISTS idx_trace_credit_settlement_batches_created
    ON trace_credit_settlement_batches (tenant_id, created_at DESC);

CREATE TABLE IF NOT EXISTS trace_credit_holds (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    hold_id UUID NOT NULL,
    credit_account_ref TEXT NOT NULL,
    credit_account_hash TEXT NOT NULL,
    reason TEXT NOT NULL,
    reason_hash TEXT NOT NULL,
    actor_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    released_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, hold_id)
);
CREATE INDEX IF NOT EXISTS idx_trace_credit_holds_account
    ON trace_credit_holds (tenant_id, credit_account_ref, released_at);

CREATE TABLE IF NOT EXISTS trace_near_credit_outbox (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    near_outbox_id UUID NOT NULL,
    settlement_batch_id UUID NOT NULL,
    credit_account_hash TEXT NOT NULL,
    near_call_json JSONB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('disabled', 'pending', 'submitted', 'confirmed', 'failed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    submitted_at TIMESTAMPTZ,
    near_transaction_hash TEXT,
    last_error_hash TEXT,
    confirmed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, near_outbox_id),
    UNIQUE (tenant_id, settlement_batch_id, credit_account_hash),
    FOREIGN KEY (tenant_id, settlement_batch_id)
        REFERENCES trace_credit_settlement_batches (tenant_id, settlement_batch_id)
        ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_trace_near_credit_outbox_status
    ON trace_near_credit_outbox (tenant_id, status, created_at);

ALTER TABLE trace_utility_attestations ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_utility_attestations
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_credit_settlement_batches ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_credit_settlement_batches
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_credit_holds ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_credit_holds
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_near_credit_outbox ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_near_credit_outbox
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));
