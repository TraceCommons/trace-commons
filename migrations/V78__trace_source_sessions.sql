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
-- another session or account, even when the content row is gone. The single
-- exception is an executed account merge: the mapping follows the absorbed
-- account into the surviving one, keeping its submission and session, so a
-- withdrawal made by either identity still reaches every mapped version.
CREATE FUNCTION trace_submission_session_immutable() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id = OLD.tenant_id
       AND NEW.submission_id = OLD.submission_id
       AND NEW.session_digest = OLD.session_digest
       AND NEW.account_id <> OLD.account_id
       AND EXISTS (
           SELECT 1 FROM trace_account_merge_proposals p
            WHERE p.tenant_id = OLD.tenant_id
              AND p.absorbed_account_id = OLD.account_id
              AND p.surviving_account_id = NEW.account_id
              AND p.consumed_at IS NOT NULL
       ) THEN
        RETURN NEW;
    END IF;
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
-- Ownership also follows executed account merges: a submission bound to an
-- absorbed account belongs to the survivor. Only the merge edge is readable.
GRANT SELECT (tenant_id, absorbed_account_id, surviving_account_id, consumed_at)
    ON trace_account_merge_proposals TO trace_account_admission_runtime;
