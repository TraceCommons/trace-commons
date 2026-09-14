-- Phase 8 production switch, tenant activation, and legacy-writer retirement.
--
-- Routing and ownership are explicit records. Timestamps are audit metadata.
-- They do not select the active bundle or the owning implementation.

CREATE TABLE pipeline_tenant_routing (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    routing_state TEXT NOT NULL CHECK (
        routing_state IN ('legacy', 'pipeline', 'contained')
    ),
    selected_bundle_id TEXT CHECK (
        selected_bundle_id IS NULL
        OR selected_bundle_id ~ '^sha256:[0-9a-f]{64}$'
    ),
    activation_record_id UUID NOT NULL,
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^(operator|admin)_sha256:[A-Za-z0-9_-]{1,128}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id),
    CHECK (
        (routing_state = 'pipeline' AND selected_bundle_id IS NOT NULL)
        OR (routing_state IN ('legacy', 'contained'))
    ),
    FOREIGN KEY (tenant_id, selected_bundle_id)
        REFERENCES pipeline_bundle_packages (tenant_id, bundle_id)
        ON DELETE RESTRICT
);

CREATE TABLE pipeline_activation_events (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    event_id UUID NOT NULL,
    action TEXT NOT NULL CHECK (
        action IN (
            'activate',
            'expand',
            'rollback',
            'contain',
            'retire_legacy_writer'
        )
    ),
    previous_state TEXT NOT NULL CHECK (
        previous_state IN ('unselected', 'legacy', 'pipeline', 'contained')
    ),
    resulting_state TEXT NOT NULL CHECK (
        resulting_state IN ('legacy', 'pipeline', 'contained')
    ),
    previous_bundle_id TEXT CHECK (
        previous_bundle_id IS NULL
        OR previous_bundle_id ~ '^sha256:[0-9a-f]{64}$'
    ),
    resulting_bundle_id TEXT CHECK (
        resulting_bundle_id IS NULL
        OR resulting_bundle_id ~ '^sha256:[0-9a-f]{64}$'
    ),
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^(operator|admin)_sha256:[A-Za-z0-9_-]{1,128}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, event_id)
);

CREATE INDEX idx_pipeline_activation_events_recorded
    ON pipeline_activation_events (tenant_id, recorded_at ASC, event_id ASC);

CREATE FUNCTION reject_pipeline_activation_event_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'pipeline activation events are immutable';
END;
$$;

CREATE TRIGGER pipeline_activation_events_reject_update
    BEFORE UPDATE ON pipeline_activation_events
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_event_mutation();

CREATE TRIGGER pipeline_activation_events_reject_delete
    BEFORE DELETE ON pipeline_activation_events
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_event_mutation();

CREATE TABLE pipeline_receipt_ownership (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    request_idempotency_key TEXT NOT NULL CHECK (
        request_idempotency_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    request_content_hash TEXT NOT NULL CHECK (
        request_content_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    owner TEXT NOT NULL CHECK (owner IN ('legacy', 'pipeline')),
    submission_id UUID NOT NULL,
    run_id UUID,
    ledger_source_key TEXT NOT NULL CHECK (
        ledger_source_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, request_idempotency_key),
    UNIQUE (tenant_id, ledger_source_key),
    UNIQUE (tenant_id, submission_id),
    CHECK (
        (owner = 'pipeline' AND run_id IS NOT NULL)
        OR (owner = 'legacy' AND run_id IS NULL)
    ),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE RESTRICT
);

CREATE TABLE pipeline_legacy_owned_work (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    work_id UUID NOT NULL,
    request_idempotency_key TEXT NOT NULL CHECK (
        request_idempotency_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    submission_id UUID NOT NULL,
    work_state TEXT NOT NULL CHECK (work_state IN ('pending', 'complete')),
    executor TEXT NOT NULL CHECK (executor = 'legacy'),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, work_id),
    UNIQUE (tenant_id, request_idempotency_key),
    CHECK (
        (work_state = 'pending' AND completed_at IS NULL)
        OR (work_state = 'complete' AND completed_at IS NOT NULL)
    ),
    FOREIGN KEY (tenant_id, request_idempotency_key)
        REFERENCES pipeline_receipt_ownership (tenant_id, request_idempotency_key)
        ON DELETE RESTRICT
);

CREATE INDEX idx_pipeline_legacy_owned_work_pending
    ON pipeline_legacy_owned_work (tenant_id, work_state)
    WHERE work_state = 'pending';

CREATE TABLE pipeline_legacy_writer_status (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    writer_state TEXT NOT NULL CHECK (
        writer_state IN ('enabled', 'draining', 'disabled')
    ),
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^(operator|admin)_sha256:[A-Za-z0-9_-]{1,128}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id)
);

ALTER TABLE trace_credit_ledger
    ADD COLUMN ledger_source_key TEXT CHECK (
        ledger_source_key IS NULL
        OR ledger_source_key ~ '^sha256:[0-9a-f]{64}$'
    );

CREATE UNIQUE INDEX idx_trace_credit_ledger_source_key
    ON trace_credit_ledger (tenant_id, ledger_source_key)
    WHERE ledger_source_key IS NOT NULL
      AND event_type = 'accepted';

ALTER TABLE pipeline_tenant_routing ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_tenant_routing FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_tenant_routing;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_tenant_routing
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_activation_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_activation_events FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_activation_events;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_activation_events
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_receipt_ownership ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_receipt_ownership FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_receipt_ownership;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_receipt_ownership
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_legacy_owned_work ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_legacy_owned_work FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_legacy_owned_work;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_legacy_owned_work
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_legacy_writer_status ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_legacy_writer_status FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_legacy_writer_status;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_legacy_writer_status
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
