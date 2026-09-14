#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT="${TRACE_COMMONS_PIPELINE_PORT:-3918}"
PG_PORT="${TRACE_COMMONS_PIPELINE_PG_PORT:-55439}"
CONTAINER="trace-commons-pipeline-phase3-$$"
ARTIFACT_ROOT="${ROOT}/.local/pipeline-phase3-artifacts"
JSON_REPORT="${ROOT}/.local/pipeline-report-v3.json"
MARKDOWN_REPORT="${ROOT}/.local/pipeline-report-v3.md"
SERVER_LOG="${ROOT}/.local/pipeline-phase3-server.log"
MIGRATION_LOG="${ROOT}/.local/pipeline-phase3-migration.log"
SERVER_PID=""
MIGRATION_PID=""

if command -v lsof >/dev/null 2>&1; then
  if lsof -nP -iTCP:"${PORT}" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "pipeline corpus port ${PORT} is already in use" >&2
    exit 1
  fi
  if lsof -nP -iTCP:"${PG_PORT}" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "pipeline corpus postgres port ${PG_PORT} is already in use" >&2
    exit 1
  fi
fi

cleanup() {
  if [[ -n "${SERVER_PID}" ]]; then
    kill "${SERVER_PID}" 2>/dev/null || true
    wait "${SERVER_PID}" 2>/dev/null || true
  fi
  if [[ -n "${MIGRATION_PID}" ]]; then
    kill "${MIGRATION_PID}" 2>/dev/null || true
    wait "${MIGRATION_PID}" 2>/dev/null || true
  fi
  docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

mkdir -p "${ROOT}/.local"
rm -rf "${ARTIFACT_ROOT}"
rm -f "${JSON_REPORT}" "${MARKDOWN_REPORT}" "${SERVER_LOG}" "${MIGRATION_LOG}"

docker run --rm --detach \
  --name "${CONTAINER}" \
  -e POSTGRES_PASSWORD=phase3-admin \
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

export TRACE_COMMONS_PIPELINE_MASTER_KEY="phase-3-local-master-key-material-32-bytes"
export TRACE_COMMONS_PIPELINE_TOKENS="contributor-token,tenant-phase3,principal_sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,contributor;worker-token,tenant-phase3,worker_sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,worker;operator-token,tenant-phase3,operator_sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc,operator;other-operator-token,tenant-other,operator_sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd,operator"

"${ROOT}/target/debug/trace-commons-pipeline-local" serve \
  --database-url "postgres://postgres:phase3-admin@127.0.0.1:${PG_PORT}/postgres" \
  --bind "127.0.0.1:${PORT}" \
  --artifact-root "${ARTIFACT_ROOT}" \
  --allow-minimal-policies >"${MIGRATION_LOG}" 2>&1 &
MIGRATION_PID=$!

for _ in $(seq 1 120); do
  if ! kill -0 "${MIGRATION_PID}" 2>/dev/null; then
    echo "migration server exited before health checks" >&2
    cat "${MIGRATION_LOG}" >&2 || true
    exit 1
  fi
  if curl --fail --silent "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1 \
    && docker exec "${CONTAINER}" psql -U postgres -tAc "SELECT 1 FROM pg_roles WHERE rolname = 'pipeline_claimer'" 2>/dev/null | grep -q 1; then
    break
  fi
  sleep 0.25
done
curl --fail --silent "http://127.0.0.1:${PORT}/healthz" >/dev/null
docker exec "${CONTAINER}" psql -U postgres -tAc "SELECT 1 FROM pg_roles WHERE rolname = 'pipeline_claimer'" | grep -q 1
kill "${MIGRATION_PID}"
wait "${MIGRATION_PID}" 2>/dev/null || true
MIGRATION_PID=""

docker exec "${CONTAINER}" psql -U postgres -v ON_ERROR_STOP=1 -c \
  "CREATE ROLE pipeline_runtime LOGIN PASSWORD 'phase3-runtime' NOBYPASSRLS;
   GRANT pipeline_claimer TO pipeline_runtime;
   GRANT CONNECT ON DATABASE postgres TO pipeline_runtime;
   GRANT USAGE ON SCHEMA public TO pipeline_runtime;
   GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO pipeline_runtime;
   GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO pipeline_runtime;
   GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA public TO pipeline_runtime;" >/dev/null

"${ROOT}/target/debug/trace-commons-pipeline-local" serve \
  --database-url "postgres://pipeline_runtime:phase3-runtime@127.0.0.1:${PG_PORT}/postgres" \
  --bind "127.0.0.1:${PORT}" \
  --artifact-root "${ARTIFACT_ROOT}" \
  --allow-minimal-policies \
  --skip-migrations >"${SERVER_LOG}" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 120); do
  if ! kill -0 "${SERVER_PID}" 2>/dev/null; then
    echo "runtime server exited before health checks" >&2
    cat "${SERVER_LOG}" >&2 || true
    exit 1
  fi
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
  --inspect-token operator-token \
  --json-report "${JSON_REPORT}" \
  --markdown-report "${MARKDOWN_REPORT}"

RUN_ID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["fixtures"][0]["run_id"])' "${JSON_REPORT}")"
STATUS="$(curl --silent --output /dev/null --write-out '%{http_code}' \
  -H 'Authorization: Bearer other-operator-token' \
  "http://127.0.0.1:${PORT}/v1/pipeline/runs/${RUN_ID}")"
[[ "${STATUS}" == "404" ]]

SECRET='ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890'
if grep -F -q "${SECRET}" "${JSON_REPORT}" "${MARKDOWN_REPORT}" "${SERVER_LOG}"; then
  echo "secret probe appeared in pipeline output" >&2
  exit 1
fi

echo "JSON report: ${JSON_REPORT}"
echo "Markdown report: ${MARKDOWN_REPORT}"
