-- Earned account trust, M5 (docs/superpowers/specs/2026-09-26-earned-account-trust-design.md):
-- each account reservation can name the earned tier and the evaluation it
-- relied on, so an operator can reconstruct every admission decision.
--
-- Both columns are nullable. NULL means no growth rule was applied, which is
-- every reservation while growth is shadow-only: the production policy keeps
-- "growth_rule": "none", and admission reads no evaluation. A later tier-0
-- read with no evaluation behind it records earned_tier 0 and a NULL digest,
-- so the two are not tied together here.
--
-- The digest is hash-only, in the same form as
-- trace_account_trust_evaluations.facts_digest. No new grant: the admission
-- runtime already holds table-wide INSERT and UPDATE on this table (V77).
ALTER TABLE trace_account_admission_submissions
    ADD COLUMN earned_tier INTEGER CHECK (earned_tier >= 0),
    ADD COLUMN trust_evaluation_digest TEXT
        CHECK (trust_evaluation_digest ~ '^sha256:[0-9a-f]{64}$');
