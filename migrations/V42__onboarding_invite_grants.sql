-- V42: DB-authoritative contributor invites.
--
-- Deliberately tenant-less: an invite has no tenant until it is redeemed, and
-- lookup is by invite hash alone. V29 `onboarding_invites` is untouched and
-- keeps counting redemptions per tenant. This table answers "may this code be
-- redeemed"; V29 answers "how many times has it been redeemed under this
-- tenant".
--
-- Hash-only: no raw invite codes, no contributor identity, no credential
-- values. `note_label` and `issued_by_label` are operator free text and are
-- never returned to non-admin callers.

CREATE TABLE IF NOT EXISTS onboarding_invite_grants (
    invite_subject_hash     TEXT PRIMARY KEY
        CHECK (invite_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    policy_label            TEXT NOT NULL,
    tenant_mode             TEXT NOT NULL CHECK (tenant_mode IN ('fixed', 'derived')),
    fixed_tenant_id         TEXT,
    tenant_template_id      TEXT,
    policy_version          TEXT NOT NULL,
    allowed_consent_scopes  TEXT[] NOT NULL DEFAULT '{}',
    allowed_uses            TEXT[] NOT NULL DEFAULT '{}',
    max_uses                INTEGER NOT NULL DEFAULT 3 CHECK (max_uses > 0),
    expires_at              TIMESTAMPTZ,
    issuance_source         TEXT NOT NULL,
    issued_by_label         TEXT,
    credential_binding_hash TEXT
        CHECK (credential_binding_hash IS NULL
               OR credential_binding_hash ~ '^sha256:[0-9a-f]{64}$'),
    note_label              TEXT,
    revoked_at              TIMESTAMPTZ,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Enforce the tenant_mode / tenant-column pairing in BOTH directions so
    -- neither column can be populated for the wrong mode.
    CONSTRAINT onboarding_invite_grants_tenant_mode_pairing CHECK (
        (tenant_mode = 'fixed'
            AND fixed_tenant_id IS NOT NULL
            AND tenant_template_id IS NULL)
        OR
        (tenant_mode = 'derived'
            AND tenant_template_id IS NOT NULL
            AND fixed_tenant_id IS NULL)
    )
);

-- One verified credential yields at most one live invite per pool. Revoking
-- frees the binding for reissue.
CREATE UNIQUE INDEX IF NOT EXISTS idx_onboarding_invite_grants_credential
    ON onboarding_invite_grants (policy_label, credential_binding_hash)
    WHERE credential_binding_hash IS NOT NULL AND revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_onboarding_invite_grants_live
    ON onboarding_invite_grants (policy_label, created_at DESC)
    WHERE revoked_at IS NULL;

-- Redemption-path predicate. The issuer sets this GUC to the hash of the code
-- the caller actually presented, transaction-locally. A lookup can therefore
-- only ever return the row for a code the caller already knows: the hot path
-- cannot enumerate live invites. Mirrors V35's trace_current_instance_subject().
CREATE OR REPLACE FUNCTION trace_current_invite_subject()
RETURNS TEXT
LANGUAGE SQL
STABLE
AS $$
    SELECT NULLIF(current_setting('trace_commons.invite_subject', true), '');
$$;

ALTER TABLE onboarding_invite_grants ENABLE ROW LEVEL SECURITY;
ALTER TABLE onboarding_invite_grants FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS invite_lookup ON onboarding_invite_grants;
CREATE POLICY invite_lookup ON onboarding_invite_grants
    FOR SELECT
    USING (invite_subject_hash = trace_current_invite_subject());

-- Cross-invite reader/writer role for the registry cache refresh and the admin
-- API. NOBYPASSRLS is load-bearing: the permissive policy below is what
-- authorizes the role, not a bypass, so the runtime/PUBLIC role stays confined
-- to the GUC predicate above. Mirrors trace_gate_driver (V36).
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_invite_registry') THEN
        CREATE ROLE trace_invite_registry NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_invite_registry SET statement_timeout = '5s';

GRANT SELECT, INSERT, UPDATE ON onboarding_invite_grants TO trace_invite_registry;

DROP POLICY IF EXISTS trace_invite_registry_all ON onboarding_invite_grants;
CREATE POLICY trace_invite_registry_all ON onboarding_invite_grants
    FOR ALL TO trace_invite_registry
    USING (true) WITH CHECK (true);
