-- Phase A5: surface why credit was withheld on a passing gate decision.
--
-- When the gate worker route emits a passing GateDecision, the server attempts
-- to mint a `novelty_utility` credit event. The credit can be withheld for
-- ABAC reasons (policy mismatch, central-issuer denial, non-production gate);
-- when it is, we still want the trace_gate_decisions audit row to land so
-- operators can see the gate decision plus the reason credit did not mint.
--
-- Nullable. NULL on legacy rows and on rows where credit was actually emitted
-- or the gate failed. Populated with a stable label-only string when credit
-- was withheld despite the gate passing: "policy_mismatch",
-- "central_issuer_denied", "non_production_gate", "submission_not_accepted".

ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS credit_withheld_reason TEXT;
