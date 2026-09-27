-- Versioned-pipeline durability: fenced leases, retained bundles, immutable
-- run identity, and receipt-orphan tracking.

ALTER TABLE pipeline_runs
    DROP CONSTRAINT pipeline_runs_state_check;
ALTER TABLE pipeline_runs
    -- `awaiting_review`: a run Review quarantined with no human assessment
    -- yet. No claim query selects it (they list only `pending`, `retry`,
    -- and expired `leased`), so a parked run does not retry hourly forever.
    ADD CONSTRAINT pipeline_runs_state_check CHECK (
        state IN ('pending', 'leased', 'retry', 'awaiting_review', 'complete', 'failed')
    ),
    ADD COLUMN lease_token UUID,
    ADD COLUMN lease_expires_at TIMESTAMPTZ,
    ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    ADD COLUMN max_attempts INTEGER NOT NULL DEFAULT 5 CHECK (max_attempts > 0),
    ADD COLUMN next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD COLUMN phase_started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD CONSTRAINT pipeline_runs_attempt_limit CHECK (attempt_count <= max_attempts),
    ADD CONSTRAINT pipeline_runs_lease_shape CHECK (
        (state = 'leased' AND lease_token IS NOT NULL AND lease_expires_at IS NOT NULL)
        OR
        (state <> 'leased' AND lease_token IS NULL AND lease_expires_at IS NULL)
    ),
    ADD CONSTRAINT pipeline_runs_safe_error_label CHECK (
        last_error_label IS NULL
        OR last_error_label ~ '^[a-z0-9_]{1,64}$'
    );

-- The worker claims per tenant (`claim_next` filters `tenant_id = $1`), so
-- the work index leads with the tenant.
DROP INDEX idx_pipeline_runs_work;
CREATE INDEX idx_pipeline_runs_work
    ON pipeline_runs (
        tenant_id,
        state,
        next_attempt_at,
        lease_expires_at,
        created_at ASC,
        run_id ASC
    );

CREATE FUNCTION reject_pipeline_run_identity_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.run_id IS DISTINCT FROM OLD.run_id
       OR NEW.submission_id IS DISTINCT FROM OLD.submission_id
       OR NEW.trace_id IS DISTINCT FROM OLD.trace_id
       OR NEW.bundle_id IS DISTINCT FROM OLD.bundle_id
       OR NEW.request_idempotency_key IS DISTINCT FROM OLD.request_idempotency_key
       OR NEW.request_content_hash IS DISTINCT FROM OLD.request_content_hash
       OR NEW.source_object_ref_id IS DISTINCT FROM OLD.source_object_ref_id
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'pipeline run identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER pipeline_runs_reject_identity_update
    BEFORE UPDATE ON pipeline_runs
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_run_identity_mutation();

CREATE TABLE pipeline_bundle_packages (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL CHECK (bundle_id ~ '^sha256:[0-9a-f]{64}$'),
    manifest_format_version INTEGER NOT NULL CHECK (manifest_format_version > 0),
    package JSONB NOT NULL,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, bundle_id)
);

CREATE TABLE pipeline_active_bundles (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL,
    selected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id),
    -- `NO ACTION`, not `RESTRICT` -- PostgreSQL never defers a `RESTRICT`
    -- action no matter what the `DEFERRABLE` clause says; `NO ACTION` is the
    -- same check, deferrable. Deferred to end of transaction because this
    -- row also cascades straight from `trace_tenants` on its own
    -- `tenant_id` column above, a sibling of `pipeline_bundle_packages`'
    -- cascade through the same parent, so a tenant delete can reach either
    -- branch first. By commit time this row is already gone whenever the
    -- whole tenant is going away; a package deleted on its own, while still
    -- the tenant's active selection, is still refused.
    FOREIGN KEY (tenant_id, bundle_id)
        REFERENCES pipeline_bundle_packages (tenant_id, bundle_id)
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE pipeline_bundle_policy_status (
    tenant_id TEXT NOT NULL,
    bundle_id TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (
        phase IN ('admission', 'review', 'score', 'settle')
    ),
    runnable BOOLEAN NOT NULL DEFAULT TRUE,
    error_label TEXT CHECK (
        error_label IS NULL OR error_label ~ '^[a-z0-9_]{1,64}$'
    ),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, bundle_id, phase),
    -- Cascade, not restrict: unlike `pipeline_active_bundles` above, this
    -- table has no foreign key of its own straight to `trace_tenants`, so a
    -- package's per-phase status rows have no other path out when their
    -- package goes away. `register_bundle` seeds one row per phase for
    -- every registered package (never a partial set), so this table only
    -- ever holds rows a live package owns.
    FOREIGN KEY (tenant_id, bundle_id)
        REFERENCES pipeline_bundle_packages (tenant_id, bundle_id)
        ON DELETE CASCADE
);

-- One row per receipt attempt, committed in its own transaction before the
-- attempt writes its object, and naming that object (key and ciphertext
-- hash). The receipt's final transaction moves the row to `committed`
-- together with the object ref; a row still `staged` after `cleanup_after`
-- names an object no run references, which the sweeper deletes with the
-- row. Each attempt writes its own object, so a key can have several
-- attempts but at most one committed one.
--
-- Only `tenant_id` has a foreign key here, to `trace_tenants` -- there is
-- no foreign key from this table to `pipeline_runs`, because a staged row
-- is written before the run it will eventually name exists. Deleting a
-- single submission (rather than its whole tenant) therefore leaves its
-- committed row behind: a later re-submission of the same key then gets a
-- content-conflict 409 (different content) or an error on
-- `idx_pipeline_receipt_artifacts_one_committed` (same content, a second
-- committed row for the same key). No product path deletes a single
-- submission.
CREATE TABLE pipeline_receipt_artifacts (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    run_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    request_idempotency_key TEXT NOT NULL CHECK (
        request_idempotency_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    request_content_hash TEXT NOT NULL CHECK (
        request_content_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    object_key TEXT NOT NULL CHECK (object_key <> ''),
    ciphertext_sha256 TEXT NOT NULL CHECK (ciphertext_sha256 ~ '^[0-9a-f]{64}$'),
    state TEXT NOT NULL DEFAULT 'staged' CHECK (state IN ('staged', 'committed')),
    cleanup_after TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '1 hour',
    staged_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    committed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, run_id, attempt_id),
    UNIQUE (tenant_id, object_key),
    CHECK (
        (state = 'staged' AND committed_at IS NULL)
        OR (state = 'committed' AND committed_at IS NOT NULL)
    )
);

-- At most one committed attempt per run: the one whose object the run's
-- source object ref names.
CREATE UNIQUE INDEX idx_pipeline_receipt_artifacts_one_committed
    ON pipeline_receipt_artifacts (tenant_id, run_id)
    WHERE state = 'committed';
-- The receipt's content-conflict check reads a key's attempts.
CREATE INDEX idx_pipeline_receipt_artifacts_key
    ON pipeline_receipt_artifacts (tenant_id, request_idempotency_key);
-- The sweeper reads a tenant's staged attempts by due time.
CREATE INDEX idx_pipeline_receipt_artifacts_due
    ON pipeline_receipt_artifacts (tenant_id, cleanup_after)
    WHERE state = 'staged';

-- Bundle packages are append-only, the same append-only shape as phase
-- outcomes (`reject_phase_outcome_mutation` above): neither an UPDATE nor a
-- direct DELETE is ever allowed, but a DELETE arriving through a cascade
-- (the tenant that owns this package was deleted) is let through so a
-- tenant with registered bundles can still be removed. See that function's
-- comment for why `pg_trigger_depth() > 1` is the direct/cascade boundary.
CREATE FUNCTION reject_pipeline_bundle_package_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'pipeline bundle packages are immutable';
END;
$$;

CREATE TRIGGER pipeline_bundle_packages_reject_update
    BEFORE UPDATE ON pipeline_bundle_packages
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_bundle_package_mutation();

CREATE TRIGGER pipeline_bundle_packages_reject_delete
    BEFORE DELETE ON pipeline_bundle_packages
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_bundle_package_mutation();

ALTER TABLE pipeline_bundle_packages ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_bundle_packages FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_bundle_packages;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_bundle_packages
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_active_bundles ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_active_bundles FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_active_bundles;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_active_bundles
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_bundle_policy_status ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_bundle_policy_status FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_bundle_policy_status;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_bundle_policy_status
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_receipt_artifacts ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_receipt_artifacts FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_receipt_artifacts;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_receipt_artifacts
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
