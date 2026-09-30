-- Product projections and immutable customer export snapshots.
--
-- Submission status remains derived from authoritative pipeline, outcome,
-- credit, and outbox records. It intentionally has no read-model table.

CREATE TABLE pipeline_export_snapshots (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    snapshot_id UUID NOT NULL,
    request_idempotency_key TEXT NOT NULL CHECK (
        request_idempotency_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    requester_principal_ref TEXT NOT NULL CHECK (
        requester_principal_ref ~ '^(principal|exporter)_sha256:[a-z0-9]{1,64}$'
    ),
    allowed_use TEXT NOT NULL CHECK (allowed_use ~ '^[a-z0-9_]{1,64}$'),
    purpose_hash TEXT NOT NULL CHECK (purpose_hash ~ '^sha256:[0-9a-f]{64}$'),
    selection_policy_id TEXT NOT NULL CHECK (
        selection_policy_id ~ '^[a-z0-9_.-]{1,128}$'
    ),
    source_list_hash TEXT NOT NULL CHECK (
        source_list_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    item_count INTEGER NOT NULL CHECK (item_count >= 0 AND item_count <= 500),
    state TEXT NOT NULL DEFAULT 'ready' CHECK (
        state IN ('ready', 'complete', 'invalidated')
    ),
    export_manifest_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    invalidated_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, snapshot_id),
    UNIQUE (tenant_id, request_idempotency_key),
    CHECK (
        (state = 'ready' AND export_manifest_id IS NULL AND completed_at IS NULL)
        OR
        (state = 'complete' AND export_manifest_id IS NOT NULL AND completed_at IS NOT NULL)
        OR
        (state = 'invalidated' AND invalidated_at IS NOT NULL)
    )
);

CREATE TABLE pipeline_export_snapshot_items (
    tenant_id TEXT NOT NULL,
    snapshot_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0 AND ordinal < 500),
    run_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    registry_revision_id UUID NOT NULL,
    source_object_ref_id UUID NOT NULL,
    source_content_hash TEXT NOT NULL CHECK (
        source_content_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    bundle_id TEXT NOT NULL CHECK (bundle_id ~ '^sha256:[0-9a-f]{64}$'),
    outcome_schema_id TEXT NOT NULL,
    outcome_schema_version INTEGER NOT NULL CHECK (outcome_schema_version > 0),
    authorized_view_schema_id TEXT NOT NULL,
    consent_scopes JSONB NOT NULL CHECK (jsonb_typeof(consent_scopes) = 'array'),
    allowed_uses JSONB NOT NULL CHECK (jsonb_typeof(allowed_uses) = 'array'),
    invalidated_at TIMESTAMPTZ,
    invalidation_reason TEXT CHECK (
        invalidation_reason IS NULL
        OR invalidation_reason IN ('withdrawn', 'revoked', 'expired', 'purged')
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, snapshot_id, registry_revision_id),
    UNIQUE (tenant_id, snapshot_id, ordinal),
    -- `NO ACTION`, not `RESTRICT` -- PostgreSQL never defers a `RESTRICT`
    -- action no matter what the `DEFERRABLE` clause says; `NO ACTION` is the
    -- same check, deferrable. Deferred, not cascade: nothing guards a direct
    -- `DELETE FROM pipeline_runs` (no trigger sits on that table the way one
    -- sits on `phase_outcomes` or the export tables), so if this key were
    -- cascade a lone run delete -- no submission or tenant delete involved
    -- -- could carry a retained export item away with it. The item is
    -- instead removed by its submission's own cascade below; deferring this
    -- check means a submission or tenant delete (which also removes
    -- `pipeline_runs`, via `trace_submissions`, a sibling of the item's own
    -- cascade through the same parent) can reach either branch first, and
    -- by commit time the item is already gone either way. A lone run
    -- delete, with a live item still referencing it, is still refused at
    -- commit.
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED,
    -- `NO ACTION`, not `RESTRICT` -- PostgreSQL never defers a `RESTRICT`
    -- action no matter what the `DEFERRABLE` clause says; `NO ACTION` is the
    -- same check, deferrable. Deferred because the item is already removed
    -- by its submission's own cascade below, and a tenant delete also
    -- cascades `pipeline_export_snapshots` straight from `trace_tenants`, a
    -- sibling of the submission's own cascade chain up to the same tenant,
    -- so a tenant delete can reach either branch first. By commit time the
    -- item is already gone whenever the whole tenant is going away
    -- together; a snapshot deleted on its own is still refused regardless
    -- of this key, by the trigger on `pipeline_export_snapshots` itself
    -- below.
    FOREIGN KEY (tenant_id, snapshot_id)
        REFERENCES pipeline_export_snapshots (tenant_id, snapshot_id)
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED,
    -- Cascade, not restrict: `trace_submissions` is the item's shared
    -- ancestor with its run (edc52070's sibling-path reasoning), and the one
    -- foreign key here with no other row racing to remove it out from under
    -- this one -- a submission delete removes this row directly, and a
    -- tenant delete cascades it from `trace_tenants`. The reference foreign
    -- keys above and below defer their checks instead, because each of
    -- their targets is also removed by a separate path from that same
    -- submission or tenant, racing this one.
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE,
    -- `NO ACTION`, not `RESTRICT`, for the same reason as the run and
    -- snapshot keys above: the item is already removed by its submission's
    -- own cascade above, and `trace_object_refs` cascades straight from
    -- `trace_submissions`, a sibling of that same cascade, so a submission
    -- or tenant delete can reach either branch first.
    FOREIGN KEY (tenant_id, submission_id, source_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (
        (invalidated_at IS NULL AND invalidation_reason IS NULL)
        OR (invalidated_at IS NOT NULL AND invalidation_reason IS NOT NULL)
    )
);

CREATE INDEX idx_pipeline_export_snapshot_items_submission
    ON pipeline_export_snapshot_items (
        tenant_id, submission_id, invalidated_at, snapshot_id
    );

CREATE FUNCTION reject_pipeline_export_snapshot_identity_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.snapshot_id IS DISTINCT FROM OLD.snapshot_id
       OR NEW.request_idempotency_key IS DISTINCT FROM OLD.request_idempotency_key
       OR NEW.requester_principal_ref IS DISTINCT FROM OLD.requester_principal_ref
       OR NEW.allowed_use IS DISTINCT FROM OLD.allowed_use
       OR NEW.purpose_hash IS DISTINCT FROM OLD.purpose_hash
       OR NEW.selection_policy_id IS DISTINCT FROM OLD.selection_policy_id
       OR NEW.source_list_hash IS DISTINCT FROM OLD.source_list_hash
       OR NEW.item_count IS DISTINCT FROM OLD.item_count
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'pipeline export snapshot identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER pipeline_export_snapshots_reject_identity_update
    BEFORE UPDATE ON pipeline_export_snapshots
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_export_snapshot_identity_mutation();

CREATE FUNCTION reject_pipeline_export_snapshot_item_identity_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.snapshot_id IS DISTINCT FROM OLD.snapshot_id
       OR NEW.ordinal IS DISTINCT FROM OLD.ordinal
       OR NEW.run_id IS DISTINCT FROM OLD.run_id
       OR NEW.submission_id IS DISTINCT FROM OLD.submission_id
       OR NEW.trace_id IS DISTINCT FROM OLD.trace_id
       OR NEW.registry_revision_id IS DISTINCT FROM OLD.registry_revision_id
       OR NEW.source_object_ref_id IS DISTINCT FROM OLD.source_object_ref_id
       OR NEW.source_content_hash IS DISTINCT FROM OLD.source_content_hash
       OR NEW.bundle_id IS DISTINCT FROM OLD.bundle_id
       OR NEW.outcome_schema_id IS DISTINCT FROM OLD.outcome_schema_id
       OR NEW.outcome_schema_version IS DISTINCT FROM OLD.outcome_schema_version
       OR NEW.authorized_view_schema_id IS DISTINCT FROM OLD.authorized_view_schema_id
       OR NEW.consent_scopes IS DISTINCT FROM OLD.consent_scopes
       OR NEW.allowed_uses IS DISTINCT FROM OLD.allowed_uses
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'pipeline export snapshot item identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER pipeline_export_snapshot_items_reject_identity_update
    BEFORE UPDATE ON pipeline_export_snapshot_items
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_export_snapshot_item_identity_mutation();

-- Snapshots and their items are retained, the same append-only intent as
-- `phase_outcomes` (V92): a direct DELETE is refused, but one arriving
-- through a cascade (the tenant a snapshot belongs to was deleted, or -- for
-- an item -- the run it was derived from was deleted, taking the whole
-- submission or tenant with it) is let through, so a tenant or a submission
-- with export history can still be removed. See
-- `reject_phase_outcome_mutation`'s comment (V92) for why
-- `pg_trigger_depth() > 1` is the direct/cascade boundary. This function is
-- shared by both tables' delete triggers below.
CREATE FUNCTION reject_pipeline_export_snapshot_delete()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'pipeline export snapshots are retained';
END;
$$;

CREATE TRIGGER pipeline_export_snapshots_reject_delete
    BEFORE DELETE ON pipeline_export_snapshots
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_export_snapshot_delete();

CREATE TRIGGER pipeline_export_snapshot_items_reject_delete
    BEFORE DELETE ON pipeline_export_snapshot_items
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_export_snapshot_delete();

ALTER TABLE pipeline_export_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_export_snapshots FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_export_snapshots;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_export_snapshots
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_export_snapshot_items ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_export_snapshot_items FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_export_snapshot_items;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_export_snapshot_items
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The ingest runtime's grants on what this migration adds, on V92's terms:
-- what the pipeline code reads and writes, and nothing broader. A table or
-- column the code does not use gets no grant.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V103: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_export_snapshots: export creation inserts a snapshot and reads
-- it back (also by its request key); completion records the delivery
-- (state, export_manifest_id, completed_at); the lifecycle and operational
-- summaries count the tenant's snapshots by state; a withdrawal reads
-- whether a snapshot carrying the submission was delivered, and it
-- invalidates every snapshot that carries the submission (state,
-- invalidated_at). Nothing updates a snapshot's identity, which the trigger
-- above refuses too, and nothing deletes one here.
GRANT SELECT, INSERT ON pipeline_export_snapshots TO trace_ingest_runtime;
GRANT UPDATE (state, export_manifest_id, completed_at, invalidated_at)
    ON pipeline_export_snapshots TO trace_ingest_runtime;

-- pipeline_export_snapshot_items: export creation inserts a snapshot's
-- items and reads them back; a withdrawal reads the items that carry the
-- submission and invalidates them (invalidated_at, invalidation_reason).
-- Nothing updates an item's identity, which the trigger above refuses too,
-- and nothing deletes one here.
GRANT SELECT, INSERT ON pipeline_export_snapshot_items TO trace_ingest_runtime;
GRANT UPDATE (invalidated_at, invalidation_reason)
    ON pipeline_export_snapshot_items TO trace_ingest_runtime;
