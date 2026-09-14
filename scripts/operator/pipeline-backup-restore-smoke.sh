#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT="${TRACE_COMMONS_PIPELINE_RESTORE_PORT:-3920}"
PG_PORT="${TRACE_COMMONS_PIPELINE_RESTORE_PG_PORT:-55441}"
CONTAINER="trace-commons-pipeline-restore-$$"
SOURCE_ROOT="${ROOT}/.local/pipeline-restore-source"
RESTORED_ROOT="${ROOT}/.local/pipeline-restore-copy"
REPORT="${ROOT}/.local/pipeline-restore-report-v7.json"
SERVER_LOG="${ROOT}/.local/pipeline-restore-server.log"
SERVER_PID=""

cleanup() {
  if [[ -n "${SERVER_PID}" ]]; then
    kill "${SERVER_PID}" 2>/dev/null || true
    wait "${SERVER_PID}" 2>/dev/null || true
  fi
  docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

mkdir -p "${ROOT}/.local"
rm -rf "${SOURCE_ROOT}" "${RESTORED_ROOT}"
rm -f "${REPORT}" "${SERVER_LOG}"

docker run --rm --detach \
  --name "${CONTAINER}" \
  -e POSTGRES_PASSWORD=phase7-admin \
  -p "127.0.0.1:${PG_PORT}:5432" \
  postgres:17-alpine >/dev/null

for _ in $(seq 1 60); do
  if docker exec "${CONTAINER}" pg_isready -U postgres >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
docker exec "${CONTAINER}" pg_isready -U postgres >/dev/null

cd "${ROOT}"
cargo build -p trace-commons-server --bin trace-commons-pipeline-local

export TRACE_COMMONS_PIPELINE_MASTER_KEY="phase-7-restore-master-key-material-32-bytes"
export TRACE_COMMONS_PIPELINE_TOKENS="contributor-token,tenant-phase7,principal_sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,contributor;reviewer-token,tenant-phase7,reviewer_sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee,reviewer;worker-token,tenant-phase7,worker_sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,worker;operator-token,tenant-phase7,operator_sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc,operator;exporter-token,tenant-phase7,exporter_sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff,exporter;lifecycle-token,tenant-phase7,lifecycle_worker_sha256:9999999999999999999999999999999999999999999999999999999999999999,lifecycle_worker"

"${ROOT}/target/debug/trace-commons-pipeline-local" serve \
  --database-url "postgres://postgres:phase7-admin@127.0.0.1:${PG_PORT}/postgres" \
  --bind "127.0.0.1:${PORT}" \
  --artifact-root "${SOURCE_ROOT}" \
  --allow-minimal-policies \
  --compatibility-policies >"${SERVER_LOG}" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 120); do
  if curl --fail --silent "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
curl --fail --silent "http://127.0.0.1:${PORT}/healthz" >/dev/null

"${ROOT}/target/debug/trace-commons-pipeline-local" corpus \
  --base-url "http://127.0.0.1:${PORT}" \
  --submit-token contributor-token \
  --worker-token worker-token \
  --reviewer-token reviewer-token \
  --inspect-token operator-token \
  --json-report "${ROOT}/.local/pipeline-restore-corpus.json" \
  --markdown-report "${ROOT}/.local/pipeline-restore-corpus.md"

kill "${SERVER_PID}"
wait "${SERVER_PID}" 2>/dev/null || true
SERVER_PID=""

PENDING_RUN_ID="00000000-0000-4000-8000-000000000701"
docker exec "${CONTAINER}" psql -U postgres -v ON_ERROR_STOP=1 -c "
  INSERT INTO pipeline_runs (
      tenant_id, run_id, submission_id, trace_id, bundle_id,
      request_idempotency_key, request_content_hash, source_object_ref_id,
      next_phase, state
  )
  SELECT tenant_id, '${PENDING_RUN_ID}'::uuid, submission_id, trace_id, bundle_id,
         'sha256:' || repeat('7', 64), 'sha256:' || repeat('8', 64),
         source_object_ref_id, 'review', 'pending'
    FROM pipeline_runs
   WHERE tenant_id = 'tenant-phase7'
   ORDER BY created_at
   LIMIT 1;
" >/dev/null

fingerprint() {
  local database="$1"
  docker exec "${CONTAINER}" psql -U postgres -d "${database}" -Atc "
    SELECT COALESCE((
          SELECT string_agg(
            concat_ws('|', run_id, submission_id, trace_id, bundle_id,
                      request_idempotency_key, request_content_hash,
                      source_object_ref_id, next_phase, state,
                      COALESCE(index_command_hash, ''),
                      COALESCE(credit_event_id::text, ''),
                      COALESCE(settlement_batch_id::text, '')),
            E'\n' ORDER BY run_id
          )
          FROM pipeline_runs
        ), '') || E'\n--outcomes--\n' || COALESCE((
          SELECT string_agg(
            concat_ws('|', outcome_id, run_id, phase, bundle_id,
                      outcome_schema_id, outcome_schema_version,
                      decision::text, evidence::text, evaluation::text),
            E'\n' ORDER BY run_id, phase
          )
          FROM phase_outcomes
        ), '') || E'\n--packages--\n' || COALESCE((
          SELECT string_agg(
            concat_ws('|', tenant_id, bundle_id, package::text),
            E'\n' ORDER BY tenant_id, bundle_id
          )
          FROM pipeline_bundle_packages
        ), '');
  " | shasum -a 256 | awk '{print $1}'
}

SOURCE_FINGERPRINT="$(fingerprint postgres)"
docker exec "${CONTAINER}" pg_dump -U postgres -Fc -f /tmp/phase7.dump postgres
docker exec "${CONTAINER}" createdb -U postgres restored
docker exec "${CONTAINER}" pg_restore \
  -U postgres -d restored --no-owner --no-privileges /tmp/phase7.dump
RESTORED_FINGERPRINT="$(fingerprint restored)"
[[ "${SOURCE_FINGERPRINT}" == "${RESTORED_FINGERPRINT}" ]]

cp -R "${SOURCE_ROOT}" "${RESTORED_ROOT}"
diff -qr "${SOURCE_ROOT}" "${RESTORED_ROOT}" >/dev/null
ARTIFACT_FINGERPRINT="$(
  python3 - "${RESTORED_ROOT}" <<'PY'
import hashlib
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
digest = hashlib.sha256()
for path in sorted(item for item in root.rglob("*") if item.is_file()):
    digest.update(path.relative_to(root).as_posix().encode())
    digest.update(hashlib.sha256(path.read_bytes()).digest())
print(digest.hexdigest())
PY
)"

PENDING_COUNT="$(docker exec "${CONTAINER}" psql -U postgres -d restored -Atc \
  "SELECT COUNT(*) FROM pipeline_runs WHERE run_id = '${PENDING_RUN_ID}' AND state = 'pending'")"
[[ "${PENDING_COUNT}" == "1" ]]

"${ROOT}/target/debug/trace-commons-pipeline-local" serve \
  --database-url "postgres://postgres:phase7-admin@127.0.0.1:${PG_PORT}/restored" \
  --bind "127.0.0.1:${PORT}" \
  --artifact-root "${RESTORED_ROOT}" \
  --allow-minimal-policies \
  --compatibility-policies \
  --skip-migrations >"${SERVER_LOG}" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 120); do
  if curl --fail --silent "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
curl --fail --silent "http://127.0.0.1:${PORT}/healthz" >/dev/null
for _ in $(seq 1 4); do
  curl --fail --silent \
    -X POST \
    -H "Authorization: Bearer worker-token" \
    "http://127.0.0.1:${PORT}/v1/pipeline/worker?limit=1" >/dev/null
done
RESTORED_STATE="$(
  curl --fail --silent \
    -H "Authorization: Bearer operator-token" \
    "http://127.0.0.1:${PORT}/v1/pipeline/runs/${PENDING_RUN_ID}" |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["run"]["state"])'
)"
[[ "${RESTORED_STATE}" == "complete" ]]
kill "${SERVER_PID}"
wait "${SERVER_PID}" 2>/dev/null || true
SERVER_PID=""

python3 - "${REPORT}" "${SOURCE_FINGERPRINT}" "${ARTIFACT_FINGERPRINT}" "${PENDING_RUN_ID}" <<'PY'
import hashlib
import json
import pathlib
import sys
from datetime import datetime, timezone

report_path, database_hash, artifact_hash, pending_run_id = sys.argv[1:]
evidence = hashlib.sha256(
    f"{database_hash}:{artifact_hash}:{pending_run_id}".encode()
).hexdigest()
report = {
    "schema": "trace_commons.pipeline_restore_report.v1",
    "generated_at": datetime.now(timezone.utc).isoformat(),
    "status": "pass",
    "database_fingerprint": f"sha256:{database_hash}",
    "artifact_fingerprint": f"sha256:{artifact_hash}",
    "pending_run_id": pending_run_id,
    "preserved": [
        "run_and_outcome_ids",
        "bundle_and_content_hashes",
        "outcome_order",
        "package_bytes",
        "pending_operation_identity",
        "pending_operation_recovery",
        "encrypted_object_bytes",
    ],
    "evidence_hash": f"sha256:{evidence}",
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
PY

echo "PipelineBackupRestoreOK: report=${REPORT}"
