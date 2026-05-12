-- Additive: surface the vector index entry id on gate-decision rows so the
-- revocation worker can load a decision row and call invalidate_vector_entry
-- without requiring the operator to store the id externally.
--
-- Nullable: decisions where both gates did not pass, or rows produced by the
-- deterministic/legacy gate service that never inserts into a real index,
-- carry NULL. Only rows where the enclave gate service inserted an embedding
-- carry a non-NULL value.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS vector_entry_id UUID;
