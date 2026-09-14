-- Phase 7 production package qualification.
--
-- Detailed lab and drill reports remain outside the ingest database. This
-- table stores only the immutable package trust and evidence identities that
-- a production activation gate needs.

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
    evidence_hash TEXT NOT NULL CHECK (
        evidence_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    qualified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, bundle_id),
    FOREIGN KEY (tenant_id, bundle_id)
        REFERENCES pipeline_bundle_packages (tenant_id, bundle_id)
        ON DELETE RESTRICT
);

CREATE FUNCTION reject_pipeline_bundle_qualification_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
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
