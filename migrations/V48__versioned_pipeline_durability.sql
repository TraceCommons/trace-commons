-- Phase 2 durability for the versioned pipeline: fenced leases, retained
-- bundles, immutable run identity, and receipt-orphan tracking.

ALTER TABLE pipeline_runs
    DROP CONSTRAINT pipeline_runs_state_check;
ALTER TABLE pipeline_runs
    ADD CONSTRAINT pipeline_runs_state_check CHECK (
        state IN ('pending', 'leased', 'retry', 'complete', 'failed')
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

DROP INDEX idx_pipeline_runs_work;
CREATE INDEX idx_pipeline_runs_work
    ON pipeline_runs (
        state,
        next_attempt_at,
        lease_expires_at,
        created_at ASC,
        run_id ASC
    );

ALTER TABLE pipeline_runs FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_runs;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_runs
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE phase_outcomes FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON phase_outcomes;
CREATE POLICY trace_corpus_tenant_isolation ON phase_outcomes
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

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
    FOREIGN KEY (tenant_id, bundle_id)
        REFERENCES pipeline_bundle_packages (tenant_id, bundle_id)
        ON DELETE RESTRICT
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
    FOREIGN KEY (tenant_id, bundle_id)
        REFERENCES pipeline_bundle_packages (tenant_id, bundle_id)
        ON DELETE RESTRICT
);

CREATE TABLE pipeline_receipt_artifacts (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    run_id UUID NOT NULL,
    request_idempotency_key TEXT NOT NULL CHECK (
        request_idempotency_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    request_content_hash TEXT NOT NULL CHECK (
        request_content_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    object_key TEXT,
    ciphertext_sha256 TEXT CHECK (
        ciphertext_sha256 IS NULL OR ciphertext_sha256 ~ '^[0-9a-f]{64}$'
    ),
    state TEXT NOT NULL DEFAULT 'staged' CHECK (state IN ('staged', 'committed')),
    cleanup_after TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '1 hour',
    staged_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    committed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, run_id),
    UNIQUE (tenant_id, request_idempotency_key),
    CHECK (
        (state = 'staged' AND committed_at IS NULL)
        OR (state = 'committed' AND committed_at IS NOT NULL)
    )
);

CREATE FUNCTION reject_pipeline_bundle_package_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
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

-- This is the only cross-tenant queue operation. Its result contains only
-- trusted routing metadata; policy and artifact reads use a tenant-scoped
-- transaction after this function returns.
CREATE FUNCTION claim_pipeline_run(
    requested_lease_token UUID,
    requested_lease_seconds INTEGER
)
RETURNS TABLE (
    tenant_id TEXT,
    run_id UUID,
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    IF requested_lease_seconds < 1 OR requested_lease_seconds > 300 THEN
        RAISE EXCEPTION 'pipeline lease duration is invalid';
    END IF;

    UPDATE pipeline_runs p
       SET state = 'failed',
           lease_token = NULL,
           lease_expires_at = NULL,
           last_error_label = 'attempts_exhausted',
           updated_at = NOW()
     WHERE p.state = 'leased'
       AND p.lease_expires_at <= NOW()
       AND p.attempt_count >= p.max_attempts;

    RETURN QUERY
    WITH candidate AS (
        SELECT p.tenant_id, p.run_id
          FROM pipeline_runs p
         WHERE p.next_phase <> 'none'
           AND p.attempt_count < p.max_attempts
           AND (
                (p.state IN ('pending', 'retry') AND p.next_attempt_at <= NOW())
                OR (p.state = 'leased' AND p.lease_expires_at <= NOW())
           )
         ORDER BY p.next_attempt_at ASC, p.created_at ASC, p.run_id ASC
         FOR UPDATE SKIP LOCKED
         LIMIT 1
    )
    UPDATE pipeline_runs p
       SET state = 'leased',
           lease_token = requested_lease_token,
           lease_expires_at = NOW() + make_interval(secs => requested_lease_seconds),
           attempt_count = p.attempt_count + 1,
           last_error_label = NULL,
           updated_at = NOW()
      FROM candidate
     WHERE p.tenant_id = candidate.tenant_id
       AND p.run_id = candidate.run_id
    RETURNING p.tenant_id, p.run_id, p.lease_token, p.lease_expires_at;
END;
$$;

REVOKE ALL ON FUNCTION claim_pipeline_run(UUID, INTEGER) FROM PUBLIC;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pipeline_claimer') THEN
        CREATE ROLE pipeline_claimer NOLOGIN NOBYPASSRLS;
    END IF;
END
$$;

GRANT USAGE ON SCHEMA public TO pipeline_claimer;
GRANT EXECUTE ON FUNCTION claim_pipeline_run(UUID, INTEGER) TO pipeline_claimer;
