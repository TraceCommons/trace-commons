"""Check definitions for `pipeline.py test`.

`CheckSpec` is the generic (check id, digests-required) pair
`require_current_pass_results` validates a required check against; later
tasks (run, restore-drill, promote) build `dict[str, CheckSpec]` values from
`PROMOTION_REQUIRED_CHECKS`-shaped data. `TEST_CHECKS` is this task's own
concern: the cargo/python steps behind `pipeline.py test`'s three groups.
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
        "contracts_deployment_inventory",
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
    TestStep(
        "postgres_migration_atomicity",
        "cargo",
        cargo_args=("-p", "trace-commons-server", "--test", "migration_atomicity_pg"),
        test_filter="",
    ),
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
