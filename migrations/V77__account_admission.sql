-- Account-authority reservations are distinct from legacy evidence reservations.
-- The policy is supplied explicitly at activation; this migration sets no quota.
ALTER TABLE trace_account_trust DROP CONSTRAINT trace_account_trust_authority_check;
ALTER TABLE trace_account_trust ADD CONSTRAINT trace_account_trust_authority_check
    CHECK (authority IN ('bounded', 'invited'));
ALTER TABLE trace_account_invite_grants ADD COLUMN revoked_at TIMESTAMPTZ;

CREATE TABLE trace_account_admission_budget (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    period_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    cost_limit BIGINT NOT NULL CHECK (cost_limit > 0),
    cost_bound BIGINT NOT NULL CHECK (cost_bound > 0),
    cost_used BIGINT NOT NULL DEFAULT 0 CHECK (cost_used >= 0),
    PRIMARY KEY (tenant_id, account_id, period_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
CREATE TABLE trace_account_admission_submissions (
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    account_id UUID NOT NULL,
    body_hash TEXT NOT NULL CHECK (body_hash ~ '^[0-9a-f]{64}$'),
    authority_kind TEXT NOT NULL CHECK (authority_kind = 'account'),
    trust_version BIGINT NOT NULL CHECK (trust_version > 0),
    policy_version TEXT NOT NULL,
    period_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('reserved','processing','completed','released')),
    lease_id UUID NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    last_cost_bound BIGINT NOT NULL CHECK (last_cost_bound > 0),
    last_charged BOOLEAN NOT NULL,
    ever_processed BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
CREATE INDEX trace_account_admission_submissions_account
    ON trace_account_admission_submissions(tenant_id, account_id);

ALTER TABLE trace_account_admission_budget ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_admission_budget FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_admission_budget
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
ALTER TABLE trace_account_admission_submissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_admission_submissions FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_admission_submissions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Ingest must be granted this NOLOGIN role by the deployer. It cannot mint an
-- invite, enumerate the registry, or bypass tenant RLS.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_account_admission_runtime') THEN
        CREATE ROLE trace_account_admission_runtime NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id, closed_at), UPDATE (account_id)
    ON trace_accounts TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id, principal_ref, unlinked_at)
    ON trace_account_principals TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id, principal_ref, device_key_id)
    ON trace_near_provisioned_devices TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, device_key_id, revoked_at, onboarding_origin)
    ON device_keys TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id)
    ON trace_near_account_anchors TO trace_account_admission_runtime;
GRANT SELECT, INSERT, UPDATE ON trace_account_trust TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id, revoked_at)
    ON trace_account_invite_grants TO trace_account_admission_runtime;
GRANT SELECT, INSERT, UPDATE ON trace_account_admission_budget,
    trace_account_admission_submissions TO trace_account_admission_runtime;
