-- Large-trace chunked scoring (strictly additive).
--
-- New nullable columns on trace_gate_decisions: peak (most-novel-region)
-- values plus chunk bookkeeping. Existing rows read as single-chunk traces
-- via NULL semantics (chunk_count NULL => 1, peak NULL => representative,
-- chunks_capped NULL => false). No existing rows are migrated or re-scored.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS peak_perplexity_micros BIGINT,
    ADD COLUMN IF NOT EXISTS peak_novelty_micros BIGINT,
    ADD COLUMN IF NOT EXISTS chunk_count INT,
    ADD COLUMN IF NOT EXISTS chunks_capped BOOLEAN;

-- Per-chunk vector-index entries. One row per inserted chunk embedding,
-- keyed (submission_id, chunk_index) per the design; decision_id ties the
-- set to the audit row that produced it. The decision row's legacy
-- vector_entry_id column (V24) keeps holding the FIRST inserted entry so
-- existing single-entry consumers (vector replay, operator flows) continue
-- to work; this table is the complete authoritative set for revocation.
-- Existing pre-V37 entries are treated as chunk_index = 0 and remain
-- reachable via the decision row's vector_entry_id.
CREATE TABLE IF NOT EXISTS trace_gate_chunk_vector_entries (
    tenant_id       TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    decision_id     UUID NOT NULL,
    submission_id   UUID NOT NULL,
    chunk_index     INT NOT NULL CHECK (chunk_index >= 0),
    vector_entry_id UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, decision_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_trace_gate_chunk_vector_entries_submission
    ON trace_gate_chunk_vector_entries (tenant_id, submission_id);

ALTER TABLE trace_gate_chunk_vector_entries ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_gate_chunk_vector_entries FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_gate_chunk_vector_entries;
CREATE POLICY trace_corpus_tenant_isolation ON trace_gate_chunk_vector_entries
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
