-- Contributor accounts, principal union-scoping, login links, and sessions.
--
-- These tables introduce a durable pseudonymous contributor identity
-- (trace_accounts) plus the primitives that let many auth principals
-- (device keys, web sessions) resolve to the same account for trace
-- read-back. No PII is stored; principals are referenced by opaque
-- principal_ref and secrets are recorded hash-only (sha256:...).
--
-- trace_login_links is deliberately a SEPARATE table from
-- onboarding_invites (V29). They share a "hashed single-use code" shape
-- but differ in lifecycle, actor, and TTL: onboarding_invites are
-- operator-minted, account-less, long-lived invite counters gating
-- /v1/onboard; login links are device-minted, account-bearing, ~5min
-- ephemeral hand-offs that mint a web session on redemption. Conflating
-- them would couple two independent lifecycles onto one row shape.
--
-- Convention (matches V1/V26/V29): tenant_id TEXT FK to
-- trace_tenants(tenant_id) as column 1; locally-owned ids are UUID;
-- ENABLE then FORCE ROW LEVEL SECURITY with the shared
-- trace_corpus_tenant_isolation policy; tenant-leading indexes;
-- sha256:-shaped CHECKs on hash columns; nullable-TIMESTAMPTZ
-- soft-deletes. No GRANTs: FORCE RLS + trace_current_tenant_id() is the
-- whole access model.

-- trace_accounts: durable pseudonymous identity (no PII).
CREATE TABLE IF NOT EXISTS trace_accounts (
    tenant_id  TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at  TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, account_id)
);

ALTER TABLE trace_accounts ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_accounts FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_accounts;
CREATE POLICY trace_corpus_tenant_isolation ON trace_accounts
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- trace_account_principals: the union-scoping primitive. A principal links
-- to AT MOST ONE account per tenant; unlinked_at IS NULL marks active links.
CREATE TABLE IF NOT EXISTS trace_account_principals (
    tenant_id     TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id    UUID NOT NULL,
    principal_ref TEXT NOT NULL,
    linked_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    unlinked_at   TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, account_id, principal_ref),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id),
    UNIQUE (tenant_id, principal_ref)
);

CREATE INDEX IF NOT EXISTS idx_trace_account_principals_active
    ON trace_account_principals (tenant_id, account_id)
    WHERE unlinked_at IS NULL;

ALTER TABLE trace_account_principals ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_principals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_principals;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_principals
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- trace_login_links: device-minted, ephemeral (~5min) single-use codes.
CREATE TABLE IF NOT EXISTS trace_login_links (
    tenant_id             TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    link_id               UUID NOT NULL,
    account_id            UUID NOT NULL,
    code_hash             TEXT NOT NULL CHECK (code_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_principal_ref TEXT NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at            TIMESTAMPTZ NOT NULL,
    consumed_at           TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, link_id),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id),
    UNIQUE (code_hash)
);

CREATE INDEX IF NOT EXISTS idx_trace_login_links_unconsumed
    ON trace_login_links (code_hash)
    WHERE consumed_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_trace_login_links_active
    ON trace_login_links (tenant_id, created_principal_ref)
    WHERE consumed_at IS NULL;

ALTER TABLE trace_login_links ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_login_links FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_login_links;
CREATE POLICY trace_corpus_tenant_isolation ON trace_login_links
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- trace_sessions: ~7d revocable browser/device sessions.
CREATE TABLE IF NOT EXISTS trace_sessions (
    tenant_id    TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    session_id   UUID NOT NULL,
    account_id   UUID NOT NULL,
    token_hash   TEXT NOT NULL CHECK (token_hash ~ '^sha256:[0-9a-f]{64}$'),
    client_kind  TEXT NOT NULL DEFAULT 'web' CHECK (client_kind IN ('web', 'device')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at   TIMESTAMPTZ NOT NULL,
    revoked_at   TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, session_id),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id),
    UNIQUE (token_hash)
);

CREATE INDEX IF NOT EXISTS idx_trace_sessions_account
    ON trace_sessions (tenant_id, account_id)
    WHERE revoked_at IS NULL;

ALTER TABLE trace_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_sessions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_sessions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_sessions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- trace_account_audit: hash-only / label-only account-action audit trail.
CREATE TABLE IF NOT EXISTS trace_account_audit (
    tenant_id      TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    audit_sequence BIGINT NOT NULL,
    action         TEXT NOT NULL,
    actor_ref      TEXT NOT NULL,
    outcome        TEXT NOT NULL,
    safe_metadata  JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, audit_sequence)
);

ALTER TABLE trace_account_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_audit FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_audit;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_audit
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
