-- Qualified routing, activation history, and receipt ownership for the
-- versioned pipeline (delivery PR 5). Routing and ownership are explicit
-- records. Timestamps are audit metadata: they select no bundle and no owner.
-- The selected bundle stays in the active bundle table (V93); the routing row
-- holds only the state.

CREATE TABLE pipeline_tenant_routing (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    routing_state TEXT NOT NULL CHECK (
        routing_state IN ('legacy', 'pipeline', 'contained')
    ),
    activation_record_id UUID NOT NULL,
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^[A-Za-z0-9_:.-]{1,160}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id)
);

CREATE TABLE pipeline_activation_events (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    event_id UUID NOT NULL,
    action TEXT NOT NULL CHECK (
        action IN ('activate', 'rollback', 'contain', 'deactivate')
    ),
    previous_state TEXT NOT NULL CHECK (
        previous_state IN ('unselected', 'legacy', 'pipeline', 'contained')
    ),
    resulting_state TEXT NOT NULL CHECK (
        resulting_state IN ('legacy', 'pipeline', 'contained')
    ),
    previous_bundle_id TEXT CHECK (
        previous_bundle_id IS NULL OR previous_bundle_id ~ '^sha256:[0-9a-f]{64}$'
    ),
    resulting_bundle_id TEXT CHECK (
        resulting_bundle_id IS NULL OR resulting_bundle_id ~ '^sha256:[0-9a-f]{64}$'
    ),
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^[A-Za-z0-9_:.-]{1,160}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, event_id)
);

CREATE INDEX idx_pipeline_activation_events_recorded
    ON pipeline_activation_events (tenant_id, recorded_at DESC, event_id);

-- One permanent owner for each submission id. A pipeline row is committed
-- with its run; a legacy row is claimed before the legacy path's first
-- write, so no legacy submission row exists yet and there is no foreign key
-- to trace_submissions. The run foreign key is deferred because the receipt
-- transaction inserts this row before the run.
CREATE TABLE pipeline_receipt_ownership (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    owner TEXT NOT NULL CHECK (owner IN ('legacy', 'pipeline')),
    run_id UUID,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id),
    CHECK (
        (owner = 'pipeline' AND run_id IS NOT NULL)
        OR (owner = 'legacy' AND run_id IS NULL)
    ),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE
        DEFERRABLE INITIALLY DEFERRED
);

-- Immutable, the V107 shape: no UPDATE and no direct DELETE; a DELETE that
-- arrives through a cascade (the tenant was deleted) is let through.
CREATE FUNCTION reject_pipeline_activation_record_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'pipeline activation records are immutable';
END;
$$;

CREATE TRIGGER pipeline_activation_events_reject_update
    BEFORE UPDATE ON pipeline_activation_events
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_record_mutation();
CREATE TRIGGER pipeline_activation_events_reject_delete
    BEFORE DELETE ON pipeline_activation_events
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_record_mutation();
CREATE TRIGGER pipeline_receipt_ownership_reject_update
    BEFORE UPDATE ON pipeline_receipt_ownership
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_record_mutation();
CREATE TRIGGER pipeline_receipt_ownership_reject_delete
    BEFORE DELETE ON pipeline_receipt_ownership
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_activation_record_mutation();

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

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V110: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_tenant_routing: each upload reads it; an operator action writes
-- it through an admin route (insert, or update of every column but the key).
GRANT SELECT, INSERT ON pipeline_tenant_routing TO trace_ingest_runtime;
GRANT UPDATE (routing_state, activation_record_id, actor_principal_ref,
              reason_code, evidence_hash, recorded_at)
    ON pipeline_tenant_routing TO trace_ingest_runtime;

-- pipeline_activation_events and pipeline_receipt_ownership: append-only.
GRANT SELECT, INSERT ON pipeline_activation_events TO trace_ingest_runtime;
GRANT SELECT, INSERT ON pipeline_receipt_ownership TO trace_ingest_runtime;
