-- Durable, account-scoped source-session tombstones. Native IDs and source
-- paths never enter these tables. Submission mappings outlive content deletion.
CREATE TABLE trace_source_sessions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id UUID NOT NULL,
    session_digest BYTEA NOT NULL CHECK (length(session_digest) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    withdrawn_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, account_id, session_digest),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts (tenant_id, account_id)
        ON DELETE CASCADE
);

CREATE TABLE trace_submission_sessions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    account_id UUID NOT NULL,
    session_digest BYTEA NOT NULL CHECK (length(session_digest) = 32),
    PRIMARY KEY (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, account_id, session_digest)
        REFERENCES trace_source_sessions (tenant_id, account_id, session_digest)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_submission_sessions_source
    ON trace_submission_sessions (tenant_id, account_id, session_digest);

-- A submission can be claimed once. No future writer may retarget it to
-- another session or account, even when the content row is gone.
CREATE FUNCTION trace_submission_session_immutable() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'trace submission session mapping is immutable';
END;
$$;

CREATE TRIGGER trace_submission_session_immutable
    BEFORE UPDATE ON trace_submission_sessions
    FOR EACH ROW EXECUTE FUNCTION trace_submission_session_immutable();

ALTER TABLE trace_source_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_source_sessions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_source_sessions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_source_sessions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_submission_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_submission_sessions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_submission_sessions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_submission_sessions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Source ownership checks compare a retained V59 anchor to its account.
-- This hash-only column is the sole additional account-runtime privilege.
GRANT SELECT (anchor_hash) ON trace_near_account_anchors
    TO trace_account_admission_runtime;
