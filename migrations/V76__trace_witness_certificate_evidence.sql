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
    receipt_sha256 TEXT CHECK (receipt_sha256 IS NULL OR receipt_sha256 ~ '^[0-9a-f]{64}$'),
    issued_at TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id) ON DELETE CASCADE,
    CHECK (
        (inference_class = 'unattested' AND bound_model IS NULL AND receipt_signer IS NULL AND receipt_sha256 IS NULL)
        OR (certificate_version = 2 AND inference_class <> 'unattested'
            AND receipt_signer IS NOT NULL AND receipt_sha256 IS NOT NULL)
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
