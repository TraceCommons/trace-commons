-- Persist the pre-cap chunk total so a gate decision can state true coverage.
--
-- V37 records chunk_count (how many chunks were scored) and chunks_capped
-- (whether the per-trace cap dropped the rest), but not the denominator. A
-- capped decision could therefore say "16 chunks were scored" and never say
-- "of 61" — and the signed score attestation inherited that silence.
--
-- We store the pre-cap TOTAL rather than the dropped count: chunk_count is
-- already the numerator, so a total makes coverage readable as "16 of 61"
-- with no arithmetic, and it degrades cleanly (total = chunk_count whenever
-- nothing was dropped). Strictly additive and nullable: NULL means the total
-- was never recorded. Decisions written before this migration keep a NULL
-- here forever, and readers MUST report that as an unknown denominator
-- rather than estimating one. No rows are migrated or re-scored.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS total_chunk_count INT;

-- Column-level grants for the gate-driver reader role.
--
-- trace_gate_driver holds COLUMN-level SELECT grants (V45), which live in
-- pg_attribute.attacl, not pg_class.relacl — so the table looks ungranted
-- while individual columns are granted. V37 added chunk_count and
-- chunks_capped without extending those grants, and the gate-driver role
-- could not read either column; that was hand-patched on the pilot host.
-- Grant all three here so the repo migration repairs the drift instead of
-- leaving one host special. GRANT is idempotent and safe to re-run.
GRANT SELECT (total_chunk_count) ON trace_gate_decisions TO trace_gate_driver;
GRANT SELECT (chunk_count) ON trace_gate_decisions TO trace_gate_driver;
GRANT SELECT (chunks_capped) ON trace_gate_decisions TO trace_gate_driver;

-- RLS is untouched: trace_gate_decisions keeps ENABLE + FORCE ROW LEVEL
-- SECURITY and its existing policies. A column grant is not a policy.
