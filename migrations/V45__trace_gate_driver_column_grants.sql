-- Retrofit trace_gate_driver to the V38 column-scoped grant convention.
--
-- V36 created the role with table-wide SELECT plus USING(true) cross-tenant
-- policies. Cross-tenant enumeration remains deliberate and stays authorized by
-- those policies; this migration only narrows grant *width*.
--
-- Column lists are exactly the columns the gate-driver pool queries in
-- crates/trace-commons-server/src/db/postgres.rs select, join on, filter by,
-- or order by:
--   list_submissions_needing_gate_decision
--   list_submissions_with_gate_decision
--   list_gate_decisions_for_credit_scoring
--   list_dedup_signals
--   list_contributor_cap_signals
--   list_scores_by_submission_ids
--   list_own_gate_decision_scores
-- A compromised driver credential must not enumerate object keys, encryption
-- key refs, redaction payloads, attestation hashes, or other cross-tenant
-- columns outside that surface.

REVOKE ALL ON trace_submissions FROM trace_gate_driver;
REVOKE ALL ON trace_gate_decisions FROM trace_gate_driver;
REVOKE ALL ON trace_object_refs FROM trace_gate_driver;
REVOKE ALL ON trace_gate_evaluation_attempts FROM trace_gate_driver;

GRANT SELECT (tenant_id, submission_id, received_at, auth_principal_ref)
    ON trace_submissions TO trace_gate_driver;
GRANT SELECT (
        tenant_id,
        submission_id,
        decision_id,
        decided_at,
        perplexity_micros,
        peak_perplexity_micros,
        novelty_score_micros,
        perplexity_passed,
        novelty_passed,
        credit_quality_micros,
        dedup_cluster_id,
        dedup_simhash,
        dedup_cluster_size
    )
    ON trace_gate_decisions TO trace_gate_driver;
GRANT SELECT (tenant_id, submission_id, artifact_kind, invalidated_at, deleted_at)
    ON trace_object_refs TO trace_gate_driver;
GRANT SELECT (tenant_id, submission_id, attempts, last_attempt_at)
    ON trace_gate_evaluation_attempts TO trace_gate_driver;
