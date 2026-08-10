-- Contributor-initiated trace withdrawal.
--
-- Backs POST /v1/account/traces/{submission_id}/withdraw, described in
-- docs/superpowers/specs/2026-08-08-trace-withdrawal-design.md. Withdrawal
-- deletes the stored content and the encrypted artifact and retains only a
-- hash-only/label-only tombstone.
--
-- Numbering: V42 is taken by the unmerged db-authoritative-invites branch, so
-- this slice takes V43. Each block in the hand-rolled run_migrations gates on
-- its own version number, not on sequence position, so an out-of-order apply
-- against a long-lived pilot is safe.

-- Marks the submission row as contributor-withdrawn. The status itself moves to
-- 'revoked', which every existing export / training / consumer predicate already
-- excludes; withdrawn_at is what distinguishes a contributor withdrawal from an
-- operator or policy revocation.
ALTER TABLE trace_submissions
    ADD COLUMN IF NOT EXISTS withdrawn_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_trace_submissions_withdrawn
    ON trace_submissions (tenant_id, withdrawn_at DESC)
    WHERE withdrawn_at IS NOT NULL;

-- The withdrawal tombstone. Deliberately five columns and no more:
--
--   * no object_key / path       -- the content is gone; a path would outlive it
--   * no auth_principal_ref      -- no contributor identity in a retained row
--   * no content, summary, body  -- the whole point of withdrawal
--
-- `distribution_reach` is a closed label set recording which of the three tiers
-- applied at the moment of withdrawal, so the honest answer stays available
-- after the fact without recomputing membership against mutated export state.
--
-- There is intentionally NO foreign key to trace_submissions: the tombstone
-- must outlive any future hard-delete of the submission row. The tenant FK is
-- kept so tenant teardown still reclaims the rows.
CREATE TABLE IF NOT EXISTS trace_withdrawals (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    withdrawn_at TIMESTAMPTZ NOT NULL,
    prior_status TEXT NOT NULL,
    distribution_reach TEXT NOT NULL CHECK (
        distribution_reach IN (
            'not_distributed',
            'commons_not_distributed',
            'commons_distributed'
        )
    ),
    PRIMARY KEY (tenant_id, submission_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_withdrawals_withdrawn_at
    ON trace_withdrawals (tenant_id, withdrawn_at DESC);

ALTER TABLE trace_withdrawals ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_withdrawals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_withdrawals;
CREATE POLICY trace_corpus_tenant_isolation ON trace_withdrawals
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
