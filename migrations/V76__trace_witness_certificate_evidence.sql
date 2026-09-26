-- Exact verified witness headers are private evidence for one submitted body.
-- There is deliberately no transcript or request body column here.
CREATE TABLE trace_witness_certificate_evidence (
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    certificate_json BYTEA NOT NULL,
    signature_header BYTEA NOT NULL,
    raw_body_sha256 TEXT NOT NULL CHECK (raw_body_sha256 ~ '^[0-9a-f]{64}$'),
    artifact_sha256 TEXT NOT NULL CHECK (artifact_sha256 ~ '^[0-9a-f]{64}$'),
    certificate_version SMALLINT NOT NULL CHECK (certificate_version IN (1, 2)),
    inference_class TEXT NOT NULL CHECK (inference_class IN
        ('unattested', 'provider_tee_final_call', 'gateway_final_call')),
    bound_model TEXT,
    receipt_signer TEXT,
    issued_at TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id) ON DELETE CASCADE,
    CHECK (
        (inference_class = 'unattested' AND bound_model IS NULL AND receipt_signer IS NULL)
        OR (certificate_version = 2 AND inference_class <> 'unattested'
            AND receipt_signer IS NOT NULL)
    )
);

ALTER TABLE trace_witness_certificate_evidence ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_witness_certificate_evidence FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_witness_certificate_evidence;
CREATE POLICY trace_corpus_tenant_isolation ON trace_witness_certificate_evidence
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_witness_evidence_runtime') THEN
        CREATE ROLE trace_witness_evidence_runtime NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_witness_evidence_runtime NOLOGIN;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_witness_evidence_runtime'
               AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'V76: witness evidence role is privileged';
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_witness_evidence_runtime;
GRANT SELECT, INSERT ON trace_witness_certificate_evidence
    TO trace_witness_evidence_runtime;
-- Only the derived selected-object association may move after a retry of the
-- exact signed source. Certificate/signature/body identity stays immutable.
GRANT UPDATE (artifact_sha256) ON trace_witness_certificate_evidence
    TO trace_witness_evidence_runtime;

-- The column grants above bind only sessions that run as the runtime role. A
-- deployment whose ingest login owns the table (or is the migrator) holds full
-- UPDATE on it, so the grants alone do not make the signed source immutable.
-- This trigger does, for every role: an UPDATE may move only the derived
-- artifact_sha256 link. Removal is a separate operation: evidence goes with its
-- submission (ON DELETE CASCADE), or is replaced when a quarantined submission
-- is remediated with a new body.
CREATE OR REPLACE FUNCTION trace_witness_evidence_signed_source_immutable()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (NEW.tenant_id, NEW.submission_id, NEW.certificate_json, NEW.signature_header,
        NEW.raw_body_sha256, NEW.certificate_version, NEW.inference_class,
        NEW.bound_model, NEW.receipt_signer, NEW.issued_at, NEW.received_at)
       IS DISTINCT FROM
       (OLD.tenant_id, OLD.submission_id, OLD.certificate_json, OLD.signature_header,
        OLD.raw_body_sha256, OLD.certificate_version, OLD.inference_class,
        OLD.bound_model, OLD.receipt_signer, OLD.issued_at, OLD.received_at)
    THEN
        RAISE EXCEPTION 'witness evidence signed source is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION trace_witness_evidence_signed_source_immutable() FROM PUBLIC;
DROP TRIGGER IF EXISTS trace_witness_evidence_signed_source_immutable
    ON trace_witness_certificate_evidence;
CREATE TRIGGER trace_witness_evidence_signed_source_immutable
    BEFORE UPDATE ON trace_witness_certificate_evidence
    FOR EACH ROW EXECUTE FUNCTION trace_witness_evidence_signed_source_immutable();

-- Quarantine remediation replaces a quarantined submission's evidence in the
-- submission's own transaction. The runtime code restricts the DELETE to a row
-- whose submission is stored as quarantined.
GRANT DELETE ON trace_witness_certificate_evidence
    TO trace_witness_evidence_runtime;
