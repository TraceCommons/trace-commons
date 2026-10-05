-- Qualified routing, activation history, and receipt ownership for the
-- versioned pipeline (delivery PR 5). Routing and ownership are explicit
-- records. Timestamps are audit metadata: they select no bundle and no owner.
-- The selected bundle stays in the active bundle table (V93); the routing row
-- holds only the state.

-- The history of routing changes. It is created first, because the routing
-- row names one of its rows. `routing_generation` is the generation of the
-- routing row that the event belongs to (see the routing table).
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
    routing_generation BIGINT NOT NULL CHECK (routing_generation >= 1),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, event_id)
);

-- The routing row is bound to one event (review round 1, point 1): no routing
-- state without an immutable event of the same tenant, state, and generation.
-- `activation_record_id` is the event's id. The key is deferred, because a
-- routing change writes the row first and its event after it, in one
-- transaction. `routing_generation` counts the versions of the row; the row
-- trigger below sets it on every update, and an insert may supply it, up to
-- a bound (a restore of the row, see the trigger).
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
    routing_generation BIGINT NOT NULL DEFAULT 1,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id),
    CONSTRAINT pipeline_tenant_routing_activation_event_fkey
        FOREIGN KEY (tenant_id, activation_record_id)
        REFERENCES pipeline_activation_events (tenant_id, event_id)
        DEFERRABLE INITIALLY DEFERRED
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

-- Sets the generation of each version of the routing row. An update gets the
-- old value plus 1, whatever the statement names. An insert keeps the value
-- it supplies when that is from 1 to 2^62, and gets 1 when it is null or
-- below 1 (the column default is 1, so an insert that names no generation
-- gets 1). A data-only restore loads a row with the generation it had,
-- beside its events; the commit check below still refuses a row whose
-- generation its event does not have. A tenant has one routing row and the
-- runtime has no DELETE on it, so an insert happens one time for a tenant.
--
-- An insert above 2^62 is refused. The update arm adds 1, so a first row at
-- the end of the type would make every later change of the tenant fail,
-- containment too. Below the bound the counter cannot reach the end: each
-- update adds 1, and 2^62 more updates do not happen. No other value of an
-- insert is refused: an upsert of an existing row fires the function for the
-- proposed insert and then for the update, so a check of the proposed value
-- against the stored row would refuse every upsert, and the proposed row's
-- value is discarded there (an upsert that proposes a value above the bound
-- is refused too; the code names no generation).
--
-- Because the trigger assigns on an update, the runtime needs no UPDATE
-- grant on the column. An update that keeps `activation_record_id` is
-- refused: every change of the row names a new event. The function reads
-- only OLD and NEW, so no other row and no clock can make it refuse a
-- change.
CREATE FUNCTION assign_pipeline_routing_generation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.routing_generation > 4611686018427387904 THEN
            RAISE EXCEPTION 'pipeline routing generation is out of range';
        END IF;
        IF NEW.routing_generation IS NULL OR NEW.routing_generation < 1 THEN
            NEW.routing_generation := 1;
        END IF;
    ELSE
        IF NEW.activation_record_id = OLD.activation_record_id THEN
            RAISE EXCEPTION 'pipeline routing change needs a new activation event';
        END IF;
        NEW.routing_generation := OLD.routing_generation + 1;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER pipeline_tenant_routing_assign_generation
    BEFORE INSERT OR UPDATE ON pipeline_tenant_routing
    FOR EACH ROW EXECUTE FUNCTION assign_pipeline_routing_generation();

-- At the commit, the event that a routing row names must have the row's state
-- and the row's generation. Each version of the row has a higher generation
-- than the versions before it, and an event is immutable, so an event matches
-- one version only: an earlier event cannot be named again. The check reads
-- the one event that NEW names, by its primary key. No time and no other
-- event of the tenant takes part (timestamps select nothing, as the header
-- says), and the events have no unique generation, so an event that no row
-- names cannot block a later change. The query names the tenant itself: an
-- owner session is not bound by the tenant policy. Table names carry no
-- schema; the function has invoker rights and no settings of its own, and
-- the tenant setting of the transaction is still in force at the commit.
--
-- What this does not check: that a qualified gate decided the change. A role
-- that may write the row may also write a matching event, so the gate stays
-- in the code.
CREATE FUNCTION check_pipeline_routing_event()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pipeline_activation_events e
         WHERE e.tenant_id = NEW.tenant_id
           AND e.event_id = NEW.activation_record_id
           AND e.resulting_state = NEW.routing_state
           AND e.routing_generation = NEW.routing_generation
    ) THEN
        RAISE EXCEPTION 'pipeline routing row does not match its activation event';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER pipeline_tenant_routing_event_match
    AFTER INSERT OR UPDATE ON pipeline_tenant_routing
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION check_pipeline_routing_event();

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
-- it through an admin route (insert, or update of every column but the key
-- and routing_generation, which the row trigger sets on an update).
GRANT SELECT, INSERT ON pipeline_tenant_routing TO trace_ingest_runtime;
GRANT UPDATE (routing_state, activation_record_id, actor_principal_ref,
              reason_code, evidence_hash, recorded_at)
    ON pipeline_tenant_routing TO trace_ingest_runtime;

-- pipeline_activation_events and pipeline_receipt_ownership: append-only.
GRANT SELECT, INSERT ON pipeline_activation_events TO trace_ingest_runtime;
GRANT SELECT, INSERT ON pipeline_receipt_ownership TO trace_ingest_runtime;
