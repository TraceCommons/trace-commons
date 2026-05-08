CREATE TABLE IF NOT EXISTS trace_near_credit_account_outbox (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    near_outbox_id UUID NOT NULL,
    credit_hold_id UUID NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('freeze_credit_account', 'unfreeze_credit_account')),
    credit_account_hash TEXT NOT NULL,
    near_call_json JSONB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('disabled', 'pending', 'submitted', 'confirmed', 'failed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    submitted_at TIMESTAMPTZ,
    near_transaction_hash TEXT,
    last_error_hash TEXT,
    confirmed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, near_outbox_id),
    UNIQUE (tenant_id, credit_hold_id, operation),
    FOREIGN KEY (tenant_id, credit_hold_id)
        REFERENCES trace_credit_holds (tenant_id, hold_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_near_credit_account_outbox_status
    ON trace_near_credit_account_outbox (tenant_id, status, created_at);

ALTER TABLE trace_near_credit_account_outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_near_credit_account_outbox FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_near_credit_account_outbox;
CREATE POLICY trace_corpus_tenant_isolation ON trace_near_credit_account_outbox
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
