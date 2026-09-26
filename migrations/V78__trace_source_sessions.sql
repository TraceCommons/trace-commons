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
           SELECT 1 FROM public.trace_account_merge_proposals p
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

-- Account merges carry source sessions and their submission mappings to the
-- surviving account. The login executing a merge holds no privilege on these
-- tables and gains none: this SECURITY DEFINER function, owned by a NOLOGIN
-- NOBYPASSRLS guard, is the only way in. It acts only for the caller's
-- tenant context, and only for a proposal whose consuming UPDATE is this very
-- transaction (the same xmin proof trace_reward_accounts_merge uses), so it
-- cannot be invoked to move sessions outside an executing merge. Forced RLS
-- still applies to the guard.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_source_session_merge_guard') THEN
        CREATE ROLE trace_source_session_merge_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_source_session_merge_guard NOLOGIN;
GRANT trace_source_session_merge_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_source_session_merge_guard;
GRANT SELECT ON trace_account_merge_proposals TO trace_source_session_merge_guard;
GRANT SELECT, INSERT, UPDATE (withdrawn_at)
    ON trace_source_sessions TO trace_source_session_merge_guard;
GRANT SELECT, UPDATE (account_id)
    ON trace_submission_sessions TO trace_source_session_merge_guard;

CREATE FUNCTION public.trace_source_sessions_merge(
    p_tenant TEXT, p_surviving_account UUID, p_absorbed_account UUID, p_proposal UUID
) RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE v_moved BIGINT;
BEGIN
    IF p_tenant IS NULL OR p_tenant = ''
        OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_surviving_account IS NULL OR p_absorbed_account IS NULL
        OR p_surviving_account = p_absorbed_account OR p_proposal IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'source_session_merge_unauthorized';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM public.trace_account_merge_proposals proposal
         WHERE proposal.tenant_id = p_tenant
           AND proposal.proposal_id = p_proposal
           AND proposal.surviving_account_id = p_surviving_account
           AND proposal.absorbed_account_id = p_absorbed_account
           AND proposal.consumed_at IS NOT NULL
           -- See trace_reward_accounts_merge: the consuming UPDATE must be
           -- this transaction proper, not a subtransaction.
           AND proposal.xmin = pg_catalog.pg_current_xact_id()::xid
    ) THEN
        RAISE EXCEPTION USING MESSAGE = 'source_session_merge_unauthorized';
    END IF;
    -- Withdrawal wins when both accounts hold the same session.
    INSERT INTO public.trace_source_sessions AS existing
        (tenant_id, account_id, session_digest, created_at, withdrawn_at)
    SELECT tenant_id, p_surviving_account, session_digest, created_at, withdrawn_at
      FROM public.trace_source_sessions
     WHERE tenant_id = p_tenant AND account_id = p_absorbed_account
    ON CONFLICT (tenant_id, account_id, session_digest) DO UPDATE
       SET withdrawn_at = COALESCE(existing.withdrawn_at, EXCLUDED.withdrawn_at);
    GET DIAGNOSTICS v_moved = ROW_COUNT;
    -- The immutability trigger admits this re-key only for a consumed merge.
    -- The absorbed account's session rows stay, unreferenced, on the closed
    -- account.
    UPDATE public.trace_submission_sessions
       SET account_id = p_surviving_account
     WHERE tenant_id = p_tenant AND account_id = p_absorbed_account;
    RETURN v_moved;
END $$;
GRANT CREATE ON SCHEMA public TO trace_source_session_merge_guard;
ALTER FUNCTION public.trace_source_sessions_merge(TEXT, UUID, UUID, UUID)
    OWNER TO trace_source_session_merge_guard;
REVOKE CREATE ON SCHEMA public FROM trace_source_session_merge_guard;
-- Callable by any merge-executing login, as trace_reward_accounts_merge is:
-- the in-transaction consumed-proposal proof is the authorization.
GRANT EXECUTE ON FUNCTION public.trace_source_sessions_merge(TEXT, UUID, UUID, UUID) TO PUBLIC;
REVOKE trace_source_session_merge_guard FROM CURRENT_USER;
