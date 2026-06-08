-- Agent-driven onboarding invite consumption counters.
--
-- The file-backed allowlist remains the operator-edited source of valid
-- invite hashes and tenant routing for the pilot. This table records atomic
-- consumption under tenant RLS so single-use and max_uses semantics survive
-- concurrent `/v1/onboard` calls.

CREATE TABLE IF NOT EXISTS onboarding_invites (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    invite_subject_hash TEXT NOT NULL,
    max_uses INTEGER NOT NULL DEFAULT 1 CHECK (max_uses > 0),
    consumed_uses INTEGER NOT NULL DEFAULT 0 CHECK (consumed_uses >= 0),
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, invite_subject_hash),
    CHECK (invite_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (consumed_uses <= max_uses)
);

CREATE INDEX IF NOT EXISTS idx_onboarding_invites_tenant_active
    ON onboarding_invites (tenant_id, updated_at DESC)
    WHERE revoked_at IS NULL;

ALTER TABLE onboarding_invites ENABLE ROW LEVEL SECURITY;
ALTER TABLE onboarding_invites FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON onboarding_invites;
CREATE POLICY trace_corpus_tenant_isolation ON onboarding_invites
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
