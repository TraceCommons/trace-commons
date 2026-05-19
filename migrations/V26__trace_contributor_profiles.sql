-- Community surface — contributor public profiles.
--
-- Backs the public_attribution opt-in flow described in
-- docs/superpowers/specs/2026-05-19-community-analytics-leaderboard-design.md.
-- Each row maps a tenant's pseudonymous principal_ref to a self-declared
-- display handle that the contributor has explicitly consented to publish.
-- Withdrawal is soft-delete (withdrawn_at), so the audit trail survives
-- an opt-in/out cycle and the existing analytics aggregations can still
-- reference the principal_ref without re-exposing the handle.
--
-- RLS-enforced like the rest of the corpus; the tenant predicate uses
-- trace_current_tenant_id() (defined in V18).

CREATE TABLE IF NOT EXISTS trace_contributor_profiles (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    principal_ref TEXT NOT NULL,
    display_handle TEXT NOT NULL,
    handle_normalized TEXT NOT NULL,
    bio TEXT,
    public_since TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    withdrawn_at TIMESTAMPTZ,
    last_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    update_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, principal_ref),
    UNIQUE (tenant_id, handle_normalized)
);

CREATE INDEX IF NOT EXISTS idx_trace_contributor_profiles_public
    ON trace_contributor_profiles (tenant_id, public_since DESC)
    WHERE withdrawn_at IS NULL;

CREATE TABLE IF NOT EXISTS trace_contributor_profile_audit (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    audit_sequence BIGSERIAL NOT NULL,
    principal_ref TEXT NOT NULL,
    action TEXT NOT NULL,
    handle_normalized TEXT,
    reason TEXT,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, audit_sequence)
);

CREATE INDEX IF NOT EXISTS idx_trace_contributor_profile_audit_principal
    ON trace_contributor_profile_audit (tenant_id, principal_ref, decided_at DESC);

ALTER TABLE trace_contributor_profiles ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_contributor_profiles FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_contributor_profiles;
CREATE POLICY trace_corpus_tenant_isolation ON trace_contributor_profiles
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_contributor_profile_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_contributor_profile_audit FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_contributor_profile_audit;
CREATE POLICY trace_corpus_tenant_isolation ON trace_contributor_profile_audit
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
