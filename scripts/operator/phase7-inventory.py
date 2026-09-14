#!/usr/bin/env python3
"""Build and validate the Phase 7 deployment interface inventory."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]

ROUTE_RULES = (
    (re.compile(r"^/(healthz?|ready)$"), ("OPS-001", "CMP-001")),
    (re.compile(r"^/\.well-known/"), ("STA-005", "CMP-001")),
    (re.compile(r"^/v1/community/"), ("COM-001", "COM-002")),
    (re.compile(r"^/v1/admin/community/"), ("COM-001", "LIF-003")),
    (re.compile(r"^/(v1/)?account/"), ("AUTH-001", "AUTH-004")),
    (re.compile(r"^/v1/contributors/"), ("STA-001", "CRD-004")),
    (re.compile(r"^/v1/traces"), ("SUB-001", "LIF-001")),
    (re.compile(r"^/v1/exports"), ("EXP-001", "EXP-003")),
    (re.compile(r"^/v1/pipeline/"), ("RUN-001", "OPS-005")),
    (re.compile(r"^/v1/review/"), ("REV-003", "AUTH-005")),
    (re.compile(r"^/v1/analytics/"), ("STA-004", "OPS-003")),
    (re.compile(r"^/v1/datasets/"), ("EXP-004", "CMP-001")),
    (re.compile(r"^/v1/benchmarks/"), ("EXP-004", "AUTH-004")),
    (re.compile(r"^/v1/ranker/"), ("EXP-004", "AUTH-004")),
    (re.compile(r"^/v1/workers/"), ("OPS-002", "AUTH-005")),
    (re.compile(r"^/v1/admin/"), ("OPS-003", "AUTH-004")),
    (re.compile(r"^/v1/audit/"), ("SYS-009", "OPS-005")),
)

TABLE_RULES = (
    ("pipeline_", ("SYS-002", "RUN-001")),
    ("phase_outcomes", ("SYS-007", "RUN-001")),
    ("trace_account", ("AUTH-004", "SYS-002")),
    ("trace_session", ("AUTH-004", "SYS-002")),
    ("trace_webauthn", ("AUTH-004", "SYS-002")),
    ("trace_login", ("AUTH-004", "SYS-002")),
    ("trace_near_ident", ("AUTH-004", "SYS-002")),
    ("device_keys", ("AUTH-003", "SYS-002")),
    ("onboarding_", ("AUTH-001", "SYS-002")),
    ("trace_credit", ("CRD-001", "SYS-002")),
    ("trace_near_credit", ("STL-003", "SYS-002")),
    ("trace_export", ("EXP-001", "SYS-002")),
    ("trace_ranking", ("EXP-004", "SYS-002")),
    ("trace_benchmark", ("EXP-004", "SYS-002")),
    ("trace_", ("SYS-002", "SYS-007")),
)

ADAPTERS = (
    ("postgresql", "authoritative_metadata", ("SYS-002", "RUN-005")),
    ("encrypted_object_store", "artifact_store", ("SYS-003", "OPS-006")),
    ("key_wrapper", "key_management", ("SYS-003", "OPS-006")),
    ("perplexity_scorer", "score", ("SCR-001", "SCN-005")),
    ("embedder", "score", ("SCR-002", "STL-001")),
    ("vector_index_reader", "score", ("SCR-002", "STL-001")),
    ("vector_index_writer", "settle", ("STL-001", "STL-003")),
    ("near_outbox_adapter", "payout", ("STL-003", "SCN-011")),
    ("export_artifact_writer", "export", ("EXP-003", "LIF-003")),
)

APPLICATION_ROLES = (
    "contributor",
    "reviewer",
    "operator",
    "admin",
    "exporter",
    "lifecycle_worker",
    "utility_worker",
    "review_worker",
    "retention_worker",
    "vector_worker",
    "benchmark_worker",
    "process_evaluation_worker",
    "revocation_worker",
    "export_worker",
    "revocation_propagation_worker",
)


def digest_json(value: object) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def classify_route(path: str) -> tuple[str, ...] | None:
    for pattern, contracts in ROUTE_RULES:
        if pattern.search(path):
            return contracts
    return None


def classify_table(name: str) -> tuple[str, ...] | None:
    for prefix, contracts in TABLE_RULES:
        if name.startswith(prefix):
            return contracts
    return None


def source_routes(path: Path) -> list[str]:
    source = path.read_text()
    return sorted(set(re.findall(r'\.route\(\s*"([^"]+)"', source, re.DOTALL)))


def migration_inventory() -> tuple[list[str], list[str]]:
    tables: set[str] = set()
    roles: set[str] = set()
    for migration in sorted((ROOT / "migrations").glob("V*.sql")):
        source = migration.read_text()
        tables.update(
            re.findall(
                r"CREATE TABLE(?: IF NOT EXISTS)?\s+([a-z][a-z0-9_]*)",
                source,
                re.IGNORECASE,
            )
        )
        roles.update(re.findall(r"rolname\s*=\s*'([a-z][a-z0-9_]*)'", source))
        roles.update(
            re.findall(
                r"CREATE ROLE\s+([a-z][a-z0-9_]*)", source, re.IGNORECASE
            )
        )
    return sorted(name.lower() for name in tables), sorted(roles)


def object_namespaces() -> list[dict[str, object]]:
    source = (
        ROOT
        / "crates/trace-commons-server/src/trace_artifact_store.rs"
    ).read_text()
    match = re.search(
        r"pub enum TraceArtifactKind\s*\{(?P<body>.*?)\n\}", source, re.DOTALL
    )
    if not match:
        raise ValueError("TraceArtifactKind inventory is unavailable")
    variants = re.findall(r"^\s*([A-Z][A-Za-z0-9]+),", match.group("body"), re.MULTILINE)
    return [
        {"name": variant, "contracts": ["SYS-003", "LIF-003"]} for variant in variants
    ]


def build_inventory() -> dict[str, object]:
    components = (
        "crates/trace-commons-server/src/bin/trace-commons-ingest.rs",
        "crates/trace-commons-server/src/bin/trace-commons-pipeline-local.rs",
    )
    routes: list[dict[str, object]] = []
    unknown_routes: list[str] = []
    for component in components:
        for route in source_routes(ROOT / component):
            contracts = classify_route(route)
            if contracts is None:
                unknown_routes.append(f"{component}:{route}")
                continue
            routes.append(
                {
                    "component": Path(component).name,
                    "path": route,
                    "contracts": list(contracts),
                }
            )
    tables, roles = migration_inventory()
    classified_tables: list[dict[str, object]] = []
    unknown_tables: list[str] = []
    for table in tables:
        contracts = classify_table(table)
        if contracts is None:
            unknown_tables.append(table)
            continue
        classified_tables.append({"name": table, "contracts": list(contracts)})
    inventory: dict[str, object] = {
        "schema": "trace_commons.phase7_interface_inventory.v1",
        "routes": routes,
        "worker_operations": [
            item
            for item in routes
            if str(item["path"]).startswith("/v1/workers/")
            or item["path"] == "/v1/pipeline/worker"
        ],
        "adapters": [
            {"name": name, "kind": kind, "contracts": list(contracts)}
            for name, kind, contracts in ADAPTERS
        ],
        "tables": classified_tables,
        "object_namespaces": object_namespaces(),
        "telemetry_destinations": [
            {
                "name": "structured_log_stdout",
                "contracts": ["SYS-004", "OPS-003"],
            },
            {"name": "audit_database", "contracts": ["SYS-009", "OPS-005"]},
            {"name": "audit_file_legacy", "contracts": ["SYS-009", "CMP-002"]},
            {
                "name": "operator_http_summary",
                "contracts": ["SYS-004", "OPS-003"],
            },
        ],
        "roles": [
            {
                "name": f"database:{role}",
                "kind": "database",
                "contracts": ["SYS-002", "AUTH-005"],
            }
            for role in roles
        ]
        + [
            {
                "name": f"application:{role}",
                "kind": "application",
                "contracts": ["AUTH-004", "AUTH-005"],
            }
            for role in APPLICATION_ROLES
        ],
    }
    if unknown_routes or unknown_tables:
        details = ", ".join(unknown_routes + unknown_tables)
        raise ValueError(f"unclassified deployment interface: {details}")
    inventory["inventory_digest"] = digest_json(inventory)
    return inventory


def validate(inventory: dict[str, object]) -> None:
    for category in (
        "routes",
        "worker_operations",
        "adapters",
        "tables",
        "object_namespaces",
        "telemetry_destinations",
        "roles",
    ):
        values = inventory.get(category)
        if not isinstance(values, list) or not values:
            raise ValueError(f"inventory category is empty: {category}")
        for item in values:
            if not isinstance(item, dict) or not item.get("contracts"):
                raise ValueError(f"inventory item has no contract mapping: {category}")


def validate_contract_manifest() -> None:
    path = ROOT / "docs/redesign/contract-test-manifest.json"
    manifest = json.loads(path.read_text())
    if manifest.get("updated_for_phase") != 8:
        raise ValueError("contract manifest is not updated for Phase 8")
    allowed_statuses = set(manifest.get("status_values", []))
    seen_contracts: set[str] = set()
    for group in manifest.get("contract_groups", []):
        status = group.get("evidence_status")
        if status not in allowed_statuses:
            raise ValueError("contract group has an invalid evidence status")
        contract_ids = group.get("contract_ids", [])
        if not contract_ids or seen_contracts.intersection(contract_ids):
            raise ValueError("contract group is empty or duplicated")
        seen_contracts.update(contract_ids)
        if status != "deferred" and not group.get("test_ids"):
            raise ValueError(f"contract group has no tests: {contract_ids}")
        if status == "deferred" and not group.get("deferral"):
            raise ValueError(f"deferred contract has no reason: {contract_ids}")
    scenarios = manifest.get("scenarios", [])
    expected = {f"SCN-{number:03d}" for number in range(1, 16)}
    actual = {scenario.get("scenario_id") for scenario in scenarios}
    if actual != expected:
        raise ValueError("contract manifest does not contain all 15 scenarios")
    for scenario in scenarios:
        if scenario.get("evidence_status") not in allowed_statuses:
            raise ValueError("scenario has an invalid evidence status")
        if not scenario.get("test_ids"):
            raise ValueError(f"scenario has no tests: {scenario.get('scenario_id')}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    inventory = build_inventory()
    validate(inventory)
    validate_contract_manifest()
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(inventory, indent=2, sort_keys=True) + "\n")
    if args.check:
        print(
            "Phase7InventoryOK:"
            f" routes={len(inventory['routes'])}"
            f" workers={len(inventory['worker_operations'])}"
            f" tables={len(inventory['tables'])}"
            f" digest={inventory['inventory_digest']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
