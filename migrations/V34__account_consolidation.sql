-- Account consolidation (Slice 3b).
--
-- trace_account_merge_proposals records a proposed merge of two contributor
-- accounts within a tenant: an absorbed account folds into a surviving account.
-- proposal_id is a locally-owned UUID; surviving_account_id and
-- absorbed_account_id both bind to local contributor accounts. The proposal is
-- single-use (consumed_at) and time-bounded (expires_at). absorbed_principal_count
-- is an attribution-only count for operator review. No PII is stored.
--
-- This migration also adds payout designation: a contributor designates exactly
-- one active NEAR identity per account as the payout destination
-- (payout_designated_at on trace_near_identities, enforced by a partial-unique
-- index), and the settlement outbox (trace_near_credit_outbox, V2) carries the
-- resolved payout_near_account_id for that destination.
--
-- Convention (matches V1/V26/V29/V30/V32/V33): tenant_id TEXT FK to
-- trace_tenants(tenant_id) as column 1; locally-owned ids are UUID; ENABLE then
-- FORCE ROW LEVEL SECURITY with the shared trace_corpus_tenant_isolation policy;
-- tenant-leading indexes; nullable-TIMESTAMPTZ soft-deletes. This is an
-- authenticated-only surface: NO login-resolver grant is issued (unlike V32/V33,
-- there is no unauthenticated cross-tenant resolve here).

-- trace_account_merge_proposals: a proposed consolidation of two accounts.
CREATE TABLE IF NOT EXISTS trace_account_merge_proposals (
    tenant_id                TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    proposal_id              UUID NOT NULL,
    surviving_account_id     UUID NOT NULL,
    absorbed_account_id      UUID NOT NULL,
    absorbed_principal_count BIGINT NOT NULL DEFAULT 0,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at               TIMESTAMPTZ NOT NULL,
    consumed_at              TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, proposal_id),
    FOREIGN KEY (tenant_id, surviving_account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, absorbed_account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE
);

ALTER TABLE trace_account_merge_proposals ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_merge_proposals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_merge_proposals;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_merge_proposals
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Payout designation: at most one active NEAR identity per account may be the
-- payout destination. payout_designated_at is set when designated; the partial
-- unique index enforces a single active designation (revoked identities are
-- excluded so a fresh identity can take over after revocation).
ALTER TABLE trace_near_identities ADD COLUMN payout_designated_at TIMESTAMPTZ;
CREATE UNIQUE INDEX IF NOT EXISTS idx_trace_near_identities_one_payout
    ON trace_near_identities (tenant_id, account_id)
    WHERE payout_designated_at IS NOT NULL AND revoked_at IS NULL;

-- Settlement outbox (V2) carries the resolved payout NEAR account label for the
-- designated payout destination (attribution only).
ALTER TABLE trace_near_credit_outbox ADD COLUMN payout_near_account_id TEXT;
