// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The pilot's hand-made grants to its ingest runtime group,
//! `trace_ingest_runtime`, shared by every PostgreSQL suite that reproduces a
//! least-privilege deployment. Each suite includes this file with `#[path]`,
//! so the suites share one definition without depending on each other.

/// The pilot's hand-made runtime grants, taken once when the schema was at V62
/// (from the cutover rehearsal harness). `pg_default_acl` is empty there, so
/// every table a later migration creates is invisible to the group until
/// something grants on it.
pub const PILOT_V62_RUNTIME_GRANTS: &str = "
    GRANT USAGE ON SCHEMA public TO trace_ingest_runtime;
    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO trace_ingest_runtime;
    REVOKE ALL ON trace_admission_receipts, trace_admission_global_budget FROM trace_ingest_runtime;
    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO trace_ingest_runtime;
    GRANT EXECUTE ON FUNCTION
        trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT),
        trace_transition_admission(TEXT,UUID,UUID,TEXT)
        TO trace_ingest_runtime;
    GRANT EXECUTE ON FUNCTION trace_prune_onboarding_expiry(TEXT,INTEGER,BOOLEAN)
        TO trace_ingest_runtime;
    GRANT CREATE ON SCHEMA public TO trace_ingest_runtime;";

/// The one runtime grant the pilot took by hand at V74, per
/// `docs/operator/deployment.md`. Apply it once V74 has created the role.
pub const PILOT_V74_RUNTIME_GRANT: &str = "GRANT trace_public_run_runtime TO trace_ingest_runtime;";
