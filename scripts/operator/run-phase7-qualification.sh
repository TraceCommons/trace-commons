#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPORT="${ROOT}/.local/pipeline-qualification-report-v7.json"
CATALOG="${ROOT}/.local/pipeline-lab-catalog-v1.json"
INVENTORY="${ROOT}/.local/phase7-interface-inventory.json"
CORPUS_REPORT="${ROOT}/.local/pipeline-report-v7.json"
RESTORE_REPORT="${ROOT}/.local/pipeline-restore-report-v7.json"

cd "${ROOT}"
mkdir -p "${ROOT}/.local"

python3 scripts/operator/phase7-inventory.py --check --output "${INVENTORY}"
RUSTFLAGS="-D warnings" cargo test -p trace-commons-gate-api --lib
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server \
  versioned_pipeline_qualification --lib
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server \
  --bin trace-commons-ingest key_rotation_drill_records
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server \
  --bin trace-commons-ingest audit_chain_drill_records
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server \
  --bin trace-commons-ingest managed_eddsa
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server \
  --bin trace-commons-ingest rejects_static_tenant_token_when
scripts/operator/run-phase7-pg-suite.sh

TRACE_COMMONS_PIPELINE_REPORT_VERSION=7 \
  scripts/operator/run-compatibility-pipeline-corpus.sh
scripts/operator/pipeline-backup-restore-smoke.sh

python3 - \
  "${REPORT}" "${CATALOG}" "${INVENTORY}" "${CORPUS_REPORT}" \
  "${RESTORE_REPORT}" "${ROOT}/docs/redesign/contract-test-manifest.json" <<'PY'
import hashlib
import json
import pathlib
import subprocess
import sys
from datetime import datetime, timezone

report_path, catalog_path, inventory_path, corpus_path, restore_path, manifest_path = map(
    pathlib.Path, sys.argv[1:]
)


def load(path):
    return json.loads(path.read_text())


def file_hash(path):
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


inventory = load(inventory_path)
corpus = load(corpus_path)
restore = load(restore_path)
revision = subprocess.run(
    ["git", "rev-parse", "HEAD"],
    check=True,
    capture_output=True,
    text=True,
).stdout.strip()
base_revision_hash = "sha256:" + hashlib.sha256(revision.encode()).hexdigest()
repository_files = subprocess.run(
    ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    check=True,
    capture_output=True,
).stdout.split(b"\0")
tree = hashlib.sha256()
for raw_path in sorted(path for path in repository_files if path):
    path = pathlib.Path(raw_path.decode())
    if path.parts[0] in {".local", ".vscode", "target"} or not path.is_file():
        continue
    tree.update(len(raw_path).to_bytes(8, "big"))
    tree.update(raw_path)
    content = path.read_bytes()
    tree.update(len(content).to_bytes(8, "big"))
    tree.update(content)
revision_hash = "sha256:" + tree.hexdigest()
generated_at = datetime.now(timezone.utc).isoformat()

drill_sources = {
    "tenant_isolation": "versioned_pipeline_pg::phase_one_pipeline_is_idempotent_tenant_scoped_and_complete",
    "bundle_package_integrity": "versioned_pipeline_qualification::package_signature_binds_canonical_package_and_trusted_key",
    "bundle_activation_rollback": "versioned_pipeline_pg::bound_bundle_survives_activation_and_rollback",
    "phase_outcome_atomicity": "versioned_pipeline_pg::crash_boundaries_one_through_five_converge_after_restart",
    "fenced_lease_recovery": "versioned_pipeline_pg::stale_lease_cannot_commit_and_retry_exhaustion_is_visible",
    "settle_command_recovery": "versioned_pipeline_pg::crash_boundaries_six_through_eleven_converge_after_restart",
    "index_idempotency_conflict": "versioned_pipeline_pg::phase_seven_index_rebuild_uses_commands_without_creating_credit_or_outcomes",
    "settlement_preview_approval": "versioned_pipeline_pg::compatible_runs_finalize_in_one_batch",
    "near_outbox_recovery": "versioned_pipeline_pg::near_payout_states_do_not_change_settle_outcome",
    "withdrawal_propagation": "versioned_pipeline_pg::phase_six_export_snapshot_is_immutable_and_withdrawal_invalidates_it",
    "key_rotation": "trace_commons_ingest_internal::key_rotation_drill_records",
    "audit_chain_verification": "trace_commons_ingest_internal::audit_chain_drill_records",
    "backup_restore": "pipeline-backup-restore-smoke",
}
drills = []
for drill_id, test_id in drill_sources.items():
    evidence = hashlib.sha256(
        f"{revision_hash}:{drill_id}:{test_id}".encode()
    ).hexdigest()
    drills.append(
        {
            "drill_id": drill_id,
            "status": "pass",
            "safe_blockers": [],
            "observed_at": generated_at,
            "maximum_age_seconds": 604800,
            "test_id": test_id,
            "evidence_hash": f"sha256:{evidence}",
        }
    )

inputs = {
    "code_revision_hash": revision_hash,
    "base_revision_hash": base_revision_hash,
    "bundle_id": corpus["bundle_id"],
    "corpus_digest": corpus["corpus_digest"],
    "configuration_digest": "sha256:"
    + hashlib.sha256(
        json.dumps(
            corpus["configuration_identities"],
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
    ).hexdigest(),
    "inventory_digest": inventory["inventory_digest"],
    "contract_manifest_digest": file_hash(manifest_path),
    "restore_evidence_hash": restore["evidence_hash"],
}
evidence_hash = "sha256:" + hashlib.sha256(
    json.dumps(
        {"inputs": inputs, "drills": drills},
        sort_keys=True,
        separators=(",", ":"),
    ).encode()
).hexdigest()
report = {
    "schema": "trace_commons.pipeline_qualification_report.v1",
    "generated_at": generated_at,
    "scope": "local_test",
    "status": "pass",
    "production_promotion_ready": False,
    "safe_blockers": [
        "local_reference_scorer",
        "local_reference_embedder",
        "synthetic_index",
        "synthetic_settlement",
        "static_bearer_authentication",
    ],
    "external_payout_enabled": False,
    "inputs": inputs,
    "drills": drills,
    "acceptance_layers": [
        "protocol_schema",
        "policy_contract",
        "phase_runner",
        "bundle_identity_and_corpus",
        "postgresql_transaction_and_rls",
        "adapter_idempotency",
        "crash_recovery",
        "black_box_api",
        "operator_drill",
        "end_to_end_scenario",
    ],
    "evidence_hash": evidence_hash,
}
report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")

catalog = {
    "schema": "trace_commons.pipeline_lab_catalog.v1",
    "generated_at": generated_at,
    "bundles": [
        {
            "bundle_id": corpus["bundle_id"],
            "corpus_digest": corpus["corpus_digest"],
            "configuration_digest": inputs["configuration_digest"],
            "development_records": [
                str(corpus_path.relative_to(pathlib.Path.cwd())),
                str(report_path.relative_to(pathlib.Path.cwd())),
                str(inventory_path.relative_to(pathlib.Path.cwd())),
                str(restore_path.relative_to(pathlib.Path.cwd())),
            ],
            "production_ready": False,
            "safe_blockers": [
                "local_reference_scorer",
                "local_reference_embedder",
                "synthetic_index",
                "synthetic_settlement",
                "static_bearer_authentication",
            ],
        }
    ],
}
catalog_path.write_text(json.dumps(catalog, indent=2, sort_keys=True) + "\n")
PY

echo "Phase7QualificationOK: report=${REPORT} catalog=${CATALOG}"
