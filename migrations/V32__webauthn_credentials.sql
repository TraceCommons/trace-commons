-- WebAuthn (passkey) credentials and session-rotation columns (Slice 2).
--
-- trace_webauthn_credentials stores registered passkeys for a contributor
-- account. The serialized webauthn-rs Passkey (public key, counter, AAGUID,
-- transports) lives in `passkey JSONB`; no PII is stored. credential_id is the
-- base64url WebAuthn credential id, globally UNIQUE so an unauthenticated
-- assertion can resolve credential_id -> tenant_id via the login resolver
-- before any tenant context exists (mirrors trace_login_links code_hash in V30).
--
-- trace_sessions gains rotation columns so web sessions can rotate their bearer
-- token without re-auth: prev_token_hash / prev_token_valid_until form a short
-- grace window for the prior token during rotation, auth_credential_id records
-- which passkey authenticated the session (label-only, the base64url id), and
-- token_issued_at tracks the CURRENT token's issue time for rotation cadence
-- (distinct from created_at, which is session birth and never moves).
--
-- client_kind is widened to admit 'passkey' alongside 'web' and 'device'.
--
-- Convention (matches V1/V26/V29/V30): tenant_id TEXT FK to
-- trace_tenants(tenant_id) as column 1; locally-owned ids are UUID; ENABLE then
-- FORCE ROW LEVEL SECURITY with the shared trace_corpus_tenant_isolation policy;
-- tenant-leading indexes; nullable-TIMESTAMPTZ soft-deletes. The login-resolver
-- role gets a narrow column-scoped SELECT plus a role-scoped permissive policy
-- for the unauthenticated credential_id -> tenant_id resolve, exactly as V30
-- does for trace_login_links.

-- trace_webauthn_credentials: registered passkeys per account (no PII).
CREATE TABLE IF NOT EXISTS trace_webauthn_credentials (
    tenant_id     TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    credential_id TEXT NOT NULL,
    account_id    UUID NOT NULL,
    passkey       JSONB NOT NULL,
    label         TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at  TIMESTAMPTZ,
    revoked_at    TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, credential_id),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE,
    UNIQUE (credential_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_webauthn_credentials_active
    ON trace_webauthn_credentials (tenant_id, account_id)
    WHERE revoked_at IS NULL;

ALTER TABLE trace_webauthn_credentials ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_webauthn_credentials FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_webauthn_credentials;
CREATE POLICY trace_corpus_tenant_isolation ON trace_webauthn_credentials
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Session rotation columns. token_issued_at tracks the CURRENT token's issue
-- time for rotation cadence and MUST NOT be conflated with created_at.
ALTER TABLE trace_sessions
    ADD COLUMN prev_token_hash TEXT CHECK (prev_token_hash ~ '^sha256:[0-9a-f]{64}$'),
    ADD COLUMN prev_token_valid_until TIMESTAMPTZ,
    ADD COLUMN auth_credential_id TEXT,
    ADD COLUMN token_issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Widen client_kind to admit 'passkey'. V30 declared the constraint inline, so
-- the system-generated name is trace_sessions_client_kind_check.
ALTER TABLE trace_sessions
    DROP CONSTRAINT trace_sessions_client_kind_check,
    ADD CONSTRAINT trace_sessions_client_kind_check
        CHECK (client_kind IN ('web', 'device', 'passkey'));

-- Resolver extension: an unauthenticated assertion arrives with NO tenant
-- context, so it must map a globally-UNIQUE credential_id -> tenant_id across
-- tenants before any tenant predicate can apply. Forced RLS would filter every
-- row out for a role with no tenant context, so add a role-scoped permissive
-- SELECT policy. Permissive policies OR together: for trace_login_resolver the
-- effective USING becomes (tenant-isolation OR true) = true; the runtime /
-- PUBLIC role is unaffected (this policy is `TO trace_login_resolver` only) and
-- keeps full tenant isolation. The column GRANT limits the resolver to
-- (tenant_id, credential_id), keeping this a one-table, two-column read. This
-- mirrors V30's trace_login_resolver_cross_tenant_read on trace_login_links.
GRANT SELECT (tenant_id, credential_id) ON trace_webauthn_credentials TO trace_login_resolver;
DROP POLICY IF EXISTS trace_login_resolver_credential_read ON trace_webauthn_credentials;
CREATE POLICY trace_login_resolver_credential_read ON trace_webauthn_credentials
    FOR SELECT TO trace_login_resolver
    USING (true);
