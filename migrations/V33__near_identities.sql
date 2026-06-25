-- NEAR login identities (Slice 3a).
--
-- trace_near_identities maps a NEAR access public key to a contributor account
-- so a wallet-signed assertion can authenticate. public_key is the NEAR access
-- key (ed25519:...) and is globally UNIQUE so an unauthenticated assertion can
-- resolve public_key -> tenant_id via the login resolver before any tenant
-- context exists (mirrors trace_webauthn_credentials.credential_id in V32 and
-- trace_login_links code_hash in V30). near_account_id is the human-readable
-- NEAR account label (attribution only); account_id binds to the local
-- contributor account. No PII is stored.
--
-- client_kind is widened to admit 'near' alongside 'web', 'device', and
-- 'passkey'.
--
-- Convention (matches V1/V26/V29/V30/V32): tenant_id TEXT FK to
-- trace_tenants(tenant_id) as column 1; locally-owned ids are UUID; ENABLE then
-- FORCE ROW LEVEL SECURITY with the shared trace_corpus_tenant_isolation policy;
-- tenant-leading indexes; nullable-TIMESTAMPTZ soft-deletes. The login-resolver
-- role gets a narrow column-scoped SELECT plus a role-scoped permissive policy
-- for the unauthenticated public_key -> tenant_id resolve, exactly as V32 does
-- for trace_webauthn_credentials.

-- trace_near_identities: NEAR access keys bound to a contributor account.
CREATE TABLE IF NOT EXISTS trace_near_identities (
    tenant_id       TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    public_key      TEXT NOT NULL,
    near_account_id TEXT NOT NULL,
    account_id      UUID NOT NULL,
    label           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at    TIMESTAMPTZ,
    revoked_at      TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, public_key),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE,
    UNIQUE (public_key)
);

CREATE INDEX IF NOT EXISTS idx_trace_near_identities_active
    ON trace_near_identities (tenant_id, account_id)
    WHERE revoked_at IS NULL;

ALTER TABLE trace_near_identities ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_near_identities FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_near_identities;
CREATE POLICY trace_corpus_tenant_isolation ON trace_near_identities
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Widen client_kind to admit 'near'. V30 declared the constraint inline, so the
-- system-generated name is trace_sessions_client_kind_check.
ALTER TABLE trace_sessions
    DROP CONSTRAINT trace_sessions_client_kind_check,
    ADD CONSTRAINT trace_sessions_client_kind_check
        CHECK (client_kind IN ('web', 'device', 'passkey', 'near'));

-- Resolver extension: an unauthenticated assertion arrives with NO tenant
-- context, so it must map a globally-UNIQUE public_key -> tenant_id across
-- tenants before any tenant predicate can apply. Forced RLS would filter every
-- row out for a role with no tenant context, so add a role-scoped permissive
-- SELECT policy. Permissive policies OR together: for trace_login_resolver the
-- effective USING becomes (tenant-isolation OR true) = true; the runtime /
-- PUBLIC role is unaffected (this policy is `TO trace_login_resolver` only) and
-- keeps full tenant isolation. The column GRANT limits the resolver to
-- (tenant_id, public_key), keeping this a one-table, two-column read. This
-- mirrors V32's trace_login_resolver_credential_read on
-- trace_webauthn_credentials.
GRANT SELECT (tenant_id, public_key) ON trace_near_identities TO trace_login_resolver;
DROP POLICY IF EXISTS trace_login_resolver_near_read ON trace_near_identities;
CREATE POLICY trace_login_resolver_near_read ON trace_near_identities
    FOR SELECT TO trace_login_resolver
    USING (true);
