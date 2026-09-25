-- Durable invite trust belongs to an existing, verified account. No admission
-- policy is activated by these rows.
CREATE TABLE trace_account_trust (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    authority TEXT NOT NULL CHECK (authority = 'invited'),
    trust_version BIGINT NOT NULL CHECK (trust_version > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, account_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);

CREATE TABLE trace_account_invite_grants (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    invite_subject_hash TEXT NOT NULL CHECK (invite_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    trust_version BIGINT NOT NULL CHECK (trust_version > 0),
    granted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, account_id, invite_subject_hash),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);

CREATE TABLE trace_account_trust_events (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    source_event_id UUID NOT NULL,
    request_digest TEXT NOT NULL CHECK (request_digest ~ '^sha256:[0-9a-f]{64}$'),
    invite_subject_hash TEXT NOT NULL CHECK (invite_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    trust_version BIGINT NOT NULL CHECK (trust_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, account_id, source_event_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);

ALTER TABLE trace_account_trust ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_trust FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_trust;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_trust
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_account_invite_grants ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_invite_grants FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_invite_grants;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_invite_grants
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_account_trust_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_trust_events FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_trust_events;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_trust_events
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The ingest login receives this role explicitly at deployment. These are
-- only the columns needed by the account/invite transaction; the role is not
-- an invite issuer, a registry reader, or a BYPASSRLS escape hatch.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_account_invite_runtime') THEN
        CREATE ROLE trace_account_invite_runtime NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_account_invite_runtime;
GRANT SELECT (tenant_id, account_id, closed_at), UPDATE (account_id)
    ON trace_accounts TO trace_account_invite_runtime;
GRANT SELECT (tenant_id, account_id)
    ON trace_near_account_anchors TO trace_account_invite_runtime;
GRANT SELECT (invite_subject_hash, consumed_uses, max_uses, revoked_at, expires_at),
      UPDATE (consumed_uses, updated_at)
    ON onboarding_invite_grants TO trace_account_invite_runtime;
GRANT SELECT, INSERT, UPDATE ON trace_account_trust TO trace_account_invite_runtime;
GRANT SELECT, INSERT ON trace_account_invite_grants, trace_account_trust_events
    TO trace_account_invite_runtime;
