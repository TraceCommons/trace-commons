-- Per-author perplexity, shadow mode.
--
-- Whole-trace perplexity is token-weighted, so in an agent session it is set
-- by tool output and pasted input. These columns record the same logprobs
-- split by who authored each token. Nothing reads them to decide anything.
-- See docs/superpowers/specs/2026-09-18-per-author-perplexity-shadow-design.md.
--
-- Nullable with no default value on purpose: NULL is "not computed" -- a row
-- written before this migration, or scored by a backend that reports no
-- token lengths. A perplexity column is also NULL when that author has no
-- attributed tokens; its token column is then 0. Readers MUST NOT substitute
-- a value for NULL, or calibration reads every unmeasured row as a real
-- observation.
--
-- No RLS change: same table, same forced policies. No column-level SELECT
-- grant to trace_gate_driver, as with V53/V54: nothing the narrow role runs
-- reads these, and calibration reads them through an operator connection.

ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS agent_prose_perplexity_micros BIGINT;
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS agent_prose_tokens BIGINT;
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS tool_result_perplexity_micros BIGINT;
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS tool_result_tokens BIGINT;
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS attributed_token_fraction_micros BIGINT;
