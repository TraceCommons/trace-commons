-- Dstack enclave foundation: novelty_utility credit event, gate-decision audit
-- table, and gate-version stamps on credit and vector rows.
--
-- Strictly additive. No existing rows are migrated; legacy rows carry NULL
-- gate-version fields and the legacy_deterministic gate-policy-version default
-- on vector entries. Behavior is preserved until a deployment opts into a
-- non-default TRACE_COMMONS_GATE_SERVICE.

-- ---------------------------------------------------------------------------
-- trace_credit_ledger: stamp credit events with the gate version that
-- produced them. Legacy events have neither stamp.
-- ---------------------------------------------------------------------------
ALTER TABLE trace_credit_ledger
    ADD COLUMN IF NOT EXISTS gate_policy_version TEXT,
    ADD COLUMN IF NOT EXISTS gate_version_hash TEXT;

-- ---------------------------------------------------------------------------
-- trace_vector_entries: pin the gate policy / embedder model / attestation
-- chain that produced each indexed row. Legacy rows are stamped with the
-- "legacy_deterministic" default so reads can tell pre-enclave rows apart
-- from enclave-produced rows on inspection without a NULL check.
-- ---------------------------------------------------------------------------
ALTER TABLE trace_vector_entries
    ADD COLUMN IF NOT EXISTS gate_policy_version TEXT NOT NULL DEFAULT 'legacy_deterministic',
    ADD COLUMN IF NOT EXISTS embedder_model_id TEXT NOT NULL DEFAULT 'legacy_deterministic',
    ADD COLUMN IF NOT EXISTS attestation_chain_hash TEXT;

-- ---------------------------------------------------------------------------
-- trace_gate_decisions: hash-only audit row written by TraceGateService for
-- every evaluated trace, regardless of pass/fail. RLS forced and tenant
-- predicate routed through trace_current_tenant_id() to match the rest of
-- the Trace Commons surface.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS trace_gate_decisions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    decision_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    gate_policy_version TEXT NOT NULL,
    gate_version_hash TEXT NOT NULL,
    perplexity_micros BIGINT NOT NULL,
    tail_fraction_micros BIGINT NOT NULL,
    perplexity_passed BOOLEAN NOT NULL,
    novelty_score_micros BIGINT NOT NULL,
    nearest_neighbor_hash TEXT NOT NULL,
    novelty_passed BOOLEAN NOT NULL,
    embedding_evidence_hash TEXT NOT NULL,
    attestation_chain_hash TEXT NOT NULL,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, decision_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_gate_decisions_submission
    ON trace_gate_decisions (tenant_id, submission_id, decided_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_gate_decisions_decided_at
    ON trace_gate_decisions (tenant_id, decided_at DESC);

ALTER TABLE trace_gate_decisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_gate_decisions FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_gate_decisions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_gate_decisions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
