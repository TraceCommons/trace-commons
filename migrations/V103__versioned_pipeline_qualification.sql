-- Production package qualification for the versioned pipeline (delivery
-- PR 4). Detailed lab and drill reports stay outside the ingest database:
-- this table stores only the immutable package trust and evidence
-- identities that the PR 5 activation gate reads.

CREATE TABLE pipeline_bundle_qualifications (
    tenant_id TEXT NOT NULL,
    bundle_id TEXT NOT NULL CHECK (bundle_id ~ '^sha256:[0-9a-f]{64}$'),
    package_hash TEXT NOT NULL CHECK (package_hash ~ '^sha256:[0-9a-f]{64}$'),
    signing_key_id TEXT NOT NULL CHECK (
        signing_key_id ~ '^[A-Za-z0-9_.:-]{1,128}$'
    ),
    signature_hash TEXT NOT NULL CHECK (
        signature_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    corpus_digest TEXT NOT NULL CHECK (
        corpus_digest ~ '^sha256:[0-9a-f]{64}$'
    ),
    input_digest TEXT NOT NULL CHECK (
        input_digest ~ '^sha256:[0-9a-f]{64}$'
    ),
    configuration_digest TEXT NOT NULL CHECK (
        configuration_digest ~ '^sha256:[0-9a-f]{64}$'
    ),
    code_revision_hash TEXT NOT NULL CHECK (
        code_revision_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    runtime_dependency_digest TEXT NOT NULL CHECK (
        runtime_dependency_digest ~ '^sha256:[0-9a-f]{64}$'
    ),
    evidence_hash TEXT NOT NULL CHECK (
        evidence_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    qualified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, bundle_id),
    -- A qualification leaves only with its package, and a package leaves
    -- only with its tenant (V93's trigger lets only a cascade through).
    FOREIGN KEY (tenant_id, bundle_id)
        REFERENCES pipeline_bundle_packages (tenant_id, bundle_id)
        ON DELETE CASCADE
);

-- Immutable, the same shape as `pipeline_review_assessments` (V101): neither
-- an UPDATE nor a direct DELETE is ever allowed, but a DELETE arriving
-- through a cascade (the package this qualification belongs to was deleted,
-- taking its tenant with it) is let through. See
-- `reject_pipeline_review_assessment_mutation`'s comment (V101) for why
-- `pg_trigger_depth() > 1` is the direct/cascade boundary.
CREATE FUNCTION reject_pipeline_bundle_qualification_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'pipeline bundle qualifications are immutable';
END;
$$;

CREATE TRIGGER pipeline_bundle_qualifications_reject_update
    BEFORE UPDATE ON pipeline_bundle_qualifications
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_bundle_qualification_mutation();

CREATE TRIGGER pipeline_bundle_qualifications_reject_delete
    BEFORE DELETE ON pipeline_bundle_qualifications
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_bundle_qualification_mutation();

ALTER TABLE pipeline_bundle_qualifications ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_bundle_qualifications FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_bundle_qualifications;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_bundle_qualifications
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

-- pipeline_bundle_qualifications: append-only. A qualification inserts once
-- per bundle and the worker/API paths read it; no UPDATE or DELETE grant --
-- the triggers above refuse both directly, and a qualification leaves only
-- with its package, through the foreign key's cascade, which runs as the
-- table owner.
GRANT SELECT, INSERT ON pipeline_bundle_qualifications TO trace_ingest_runtime;
