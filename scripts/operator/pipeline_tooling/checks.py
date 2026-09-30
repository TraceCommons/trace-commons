"""Check definitions for `pipeline.py test` and `pipeline.py qualify`.

`CheckSpec` is the generic (check id, digests-required) pair
`require_current_pass_results` validates a required check against.
`TEST_CHECKS` holds the cargo/python steps behind `pipeline.py test`'s three
groups. `REQUIRED_DATABASE_CHECKS` and `REQUIRED_CHECK_IDS` are what
`qualify` requires a current pass result for (P4-D5): each database check
is one exact test that emits its own check id after its assertions, and the
three corpus checks and the restore drill are emitted by the harnesses
`run` and `restore-drill` start. `REQUIRED_CHECK_IDS` stays a subset of the
Rust `PROMOTION_REQUIRED_CHECKS` (a self-test reads it out of the source);
`pipeline_http_corpus_package` is in neither list (ruling PF-1).
"""

from __future__ import annotations

import sys
from dataclasses import dataclass
from typing import Tuple

from .environment import ROOT


@dataclass(frozen=True)
class CheckSpec:
    check_id: str
    digests_required: bool


@dataclass(frozen=True)
class TestStep:
    """One step of `pipeline.py test`. `kind` is `"cargo"` (run through the
    zero-match guard) or `"python"` (run directly through `run_child`)."""

    step: str
    kind: str
    cargo_args: Tuple[str, ...] = ()
    test_filter: str = ""
    exact: bool = False
    ignored: bool = False
    argv: Tuple[str, ...] = ()
    # postgres-only: which environment variable(s) and scenario database(s)
    # this step needs.
    needs_upgrade_db: bool = False
    needs_login_resolver: bool = False
    needs_pilot_check: bool = False


# `qualify` runs this step with `--output <run dir>/inventory.json` and reads
# its `inventory_digest` (ruling T11-3).
INVENTORY_STEP = "contracts_deployment_inventory"

CONTRACTS_STEPS = (
    TestStep(
        "contracts_gate_api_pipeline",
        "cargo",
        cargo_args=("-p", "trace-commons-gate-api", "--lib"),
        test_filter="pipeline::",
    ),
    TestStep(
        "contracts_pipeline_bundle",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--lib"),
        test_filter="versioned_pipeline_bundle",
    ),
    TestStep(
        "contracts_pipeline_qualification",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--lib"),
        test_filter="versioned_pipeline_qualification",
    ),
    TestStep(
        INVENTORY_STEP,
        "python",
        argv=(sys.executable, str(ROOT / "scripts/operator/pipeline-deployment-inventory.py"), "--check"),
    ),
)

RUNTIME_STEPS = (
    TestStep(
        "runtime_versioned_pipeline",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--lib"),
        test_filter="versioned_pipeline",
    ),
    TestStep(
        "runtime_pipeline_runtime_bin",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--bin", "trace-commons-ingest"),
        test_filter="pipeline_runtime",
    ),
    TestStep(
        "runtime_tooling_self_test",
        "python",
        argv=(sys.executable, str(ROOT / "scripts/operator/test_pipeline_tooling.py")),
    ),
)

# A binding check of `qualify` too, in its own scenario.
MIGRATION_ATOMICITY_STEP = TestStep(
    "postgres_migration_atomicity",
    "cargo",
    cargo_args=("-p", "trace-commons-server", "--test", "migration_atomicity_pg"),
    test_filter="",
)

# Each step below is CI's own `postgres-suites` / `ingest-bin-postgres` step
# shape, one scenario per step (never shared, unlike the CI job that reuses
# one database across several of them).
POSTGRES_STEPS = (
    TestStep(
        "postgres_pipeline_storage_upgrade",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--lib"),
        test_filter="pipeline_upgrade",
        ignored=True,
        needs_upgrade_db=True,
    ),
    MIGRATION_ATOMICITY_STEP,
    TestStep(
        "postgres_versioned_pipeline_runtime",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--test", "versioned_pipeline_runtime_pg"),
        test_filter="",
    ),
    TestStep(
        "postgres_real_http_receipt",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--bin", "trace-commons-ingest"),
        test_filter="real_http_receipt_completes_and_resumes_after_restart",
        needs_pilot_check=True,
    ),
    TestStep(
        "postgres_compatibility_http",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--bin", "trace-commons-ingest"),
        test_filter="compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export",
        needs_login_resolver=True,
        needs_pilot_check=True,
    ),
    TestStep(
        "postgres_legacy_parity",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--bin", "trace-commons-ingest"),
        test_filter="legacy_and_pipeline_tenants_match_under_equivalent_configuration",
        needs_login_resolver=True,
        needs_pilot_check=True,
    ),
)

TEST_CHECKS = {
    "contracts": CONTRACTS_STEPS,
    "runtime": RUNTIME_STEPS,
    "postgres": POSTGRES_STEPS,
}


@dataclass(frozen=True)
class DatabaseCheck:
    """One required check `qualify` runs against a fresh scenario: the exact
    test (`cargo_args` and `test_name`, run with `--exact`) that emits
    `check_id` after its assertions. `database` names what the test is
    pointed at: `upgrade` (`TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL`),
    `runtime` (`TRACE_COMMONS_PG_TEST_DATABASE_URL`, used as given), or
    `pilot` (the same variable; the ingest suite runs in `<db>_pilot`, where
    the transaction guard looks). `digests` says whether the result must
    carry the three package digests."""

    check_id: str
    cargo_args: Tuple[str, ...]
    test_name: str
    database: str
    ignored: bool
    digests: bool


_LIB = ("-p", "trace-commons-server", "--lib")
_RUNTIME_SUITE = ("-p", "trace-commons-server", "--test", "versioned_pipeline_runtime_pg")
_INGEST_BIN = ("-p", "trace-commons-server", "--bin", "trace-commons-ingest")
_HTTP_TESTS = "tests::pipeline_http_pg_tests::"


def _runtime(check_id, test_name):
    return DatabaseCheck(check_id, _RUNTIME_SUITE, test_name, "runtime", False, True)


REQUIRED_DATABASE_CHECKS = (
    DatabaseCheck(
        "pipeline_storage_upgrade_rls",
        _LIB,
        "db::postgres::pipeline_upgrade_tests::pipeline_upgrade_from_v91_installs_forced_rls_storage",
        "upgrade",
        True,
        False,
    ),
    _runtime("pipeline_crash_matrix", "crash_matrix_produces_one_logical_effect_per_point"),
    _runtime(
        "pipeline_independent_instruments", "independent_instruments_retry_without_repeating_a_completed_one"
    ),
    _runtime("pipeline_stale_lease_fence", "stale_lease_cannot_commit_after_reclaim"),
    _runtime("pipeline_lease_renewal", "a_score_longer_than_its_lease_completes_once_with_two_workers"),
    _runtime("pipeline_receipt_replay_exact", "receipt_replay_and_conflict_are_exact"),
    _runtime("pipeline_payout_recovery", "payout_crash_between_submit_and_confirm_submits_once"),
    _runtime("pipeline_index_rebuild", "index_rebuild_uses_sealed_commands_without_new_credit_or_outcomes"),
    _runtime("pipeline_orphan_sweep", "a_refused_score_commit_leaves_staged_rows_the_sweep_removes"),
    _runtime("pipeline_bundle_qualification", "qualification_inspects_the_objects_the_constructor_receives"),
    DatabaseCheck(
        "pipeline_http_restart_recovery",
        _INGEST_BIN,
        _HTTP_TESTS + "real_http_receipt_completes_and_resumes_after_restart",
        "pilot",
        False,
        True,
    ),
    DatabaseCheck(
        "pipeline_http_receipt_ownership",
        _INGEST_BIN,
        _HTTP_TESTS + "real_http_pipeline_receipt_checks_ownership_on_replay",
        "pilot",
        False,
        True,
    ),
)

# The corpus checks `qualify` runs through `run`'s own code path, in this
# order (ruling T11-4), and the check the restore drill's resume emits.
REQUIRED_CORPUS_CHECK_IDS = (
    "pipeline_http_corpus_minimal",
    "pipeline_http_corpus_compatibility",
    "pipeline_http_corpus_hf_local",
)
RESTORE_CHECK_ID = "pipeline_restore_drill"

REQUIRED_CHECK_IDS = frozenset(
    {check.check_id for check in REQUIRED_DATABASE_CHECKS} | set(REQUIRED_CORPUS_CHECK_IDS) | {RESTORE_CHECK_ID}
)


def required_specs():
    """`require_current_pass_results`'s `required` for `qualify`: every
    required check id, with the digests its row asks for (the corpus checks
    and the restore drill always carry them)."""
    specs = {check.check_id: CheckSpec(check.check_id, check.digests) for check in REQUIRED_DATABASE_CHECKS}
    for check_id in (*REQUIRED_CORPUS_CHECK_IDS, RESTORE_CHECK_ID):
        specs[check_id] = CheckSpec(check_id, digests_required=True)
    return specs
