-- V139: a trace_gate_decisions row may hold no perplexity and no tail
-- fraction (owner decision 2026-10-26, substance pilot).
--
-- V23 made perplexity_micros and tail_fraction_micros NOT NULL, because the
-- legacy gate measures both on every row it scores. The substance Score
-- policy measures neither. Its Settle row (source 'pipeline_settle',
-- credit_quality_calibration_version 4, or NULL for a duplicate-withheld
-- run) had to store a placeholder 0 in both columns, and every reader of
-- these columns then read that 0 as a measured perplexity. A substance row
-- now stores NULL in both. NULL means "no perplexity was measured", as it
-- already means for the per-author perplexity columns (V73) and
-- qualifying_token_fraction_micros (V54), and a reader must not default it.
--
-- READERS. Each reader that reads either column as a measured value reads
-- only rows that hold one (IS NOT NULL): the score attestation, the admin
-- score read-back, the tail-floor calibration and the gate outcome export.
-- A legacy writer still writes a value, so no legacy row changes.
--
-- COPIES. No SQL function, trigger, view or other table reads either
-- column, so nothing copies a NULL into a NOT NULL column.
--
-- NUMBERING. V134 to V138 are taken on main. V139 is the next number free.
--
-- GRANTS. None change: trace_gate_driver already reads perplexity_micros
-- (V45), and no role reads tail_fraction_micros through a column grant.

ALTER TABLE trace_gate_decisions
    ALTER COLUMN perplexity_micros DROP NOT NULL,
    ALTER COLUMN tail_fraction_micros DROP NOT NULL;
