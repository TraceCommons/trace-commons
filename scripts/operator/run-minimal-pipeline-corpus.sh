#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT="${TRACE_COMMONS_PIPELINE_PORT:-3917}"
PG_PORT="${TRACE_COMMONS_PIPELINE_PG_PORT:-55439}"
CONTAINER="trace-commons-pipeline-phase2-$$"
ARTIFACT_ROOT="${ROOT}/.local/pipeline-phase2-artifacts"
JSON_REPORT="${ROOT}/.local/pipeline-report-v2.json"
MARKDOWN_REPORT="${ROOT}/.local/pipeline-report-v2.md"
SERVER_LOG="${ROOT}/.local/pipeline-phase2-server.log"
MIGRATION_LOG="${ROOT}/.local/pipeline-phase2-migration.log"
SERVER_PID=""
MIGRATION_PID=""

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
  -e POSTGRES_PASSWORD=phase2-admin \
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

export TRACE_COMMONS_PIPELINE_MASTER_KEY="phase-2-local-master-key-material-32-bytes"
export TRACE_COMMONS_PIPELINE_TOKENS="contributor-token,tenant-phase2,principal_sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,contributor;worker-token,tenant-phase2,worker_sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,worker;operator-token,tenant-phase2,operator_sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc,operator;other-operator-token,tenant-other,operator_sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd,operator"

"${ROOT}/target/debug/trace-commons-pipeline-local" serve \
  --database-url "postgres://postgres:phase2-admin@127.0.0.1:${PG_PORT}/postgres" \
  --bind "127.0.0.1:${PORT}" \
  --artifact-root "${ARTIFACT_ROOT}" \
  --allow-minimal-policies >"${MIGRATION_LOG}" 2>&1 &
MIGRATION_PID=$!

for _ in $(seq 1 120); do
  if curl --fail --silent "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
curl --fail --silent "http://127.0.0.1:${PORT}/healthz" >/dev/null
kill "${MIGRATION_PID}"
wait "${MIGRATION_PID}" 2>/dev/null || true
MIGRATION_PID=""

docker exec "${CONTAINER}" psql -U postgres -v ON_ERROR_STOP=1 -c \
  "CREATE ROLE pipeline_runtime LOGIN PASSWORD 'phase2-runtime' NOBYPASSRLS;
   GRANT pipeline_claimer TO pipeline_runtime;
   GRANT CONNECT ON DATABASE postgres TO pipeline_runtime;
   GRANT USAGE ON SCHEMA public TO pipeline_runtime;
   GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO pipeline_runtime;
   GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO pipeline_runtime;
   GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA public TO pipeline_runtime;" >/dev/null

"${ROOT}/target/debug/trace-commons-pipeline-local" serve \
  --database-url "postgres://pipeline_runtime:phase2-runtime@127.0.0.1:${PG_PORT}/postgres" \
  --bind "127.0.0.1:${PORT}" \
  --artifact-root "${ARTIFACT_ROOT}" \
  --allow-minimal-policies \
  --skip-migrations >"${SERVER_LOG}" 2>&1 &
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
  --inspect-token operator-token \
  --json-report "${JSON_REPORT}" \
  --markdown-report "${MARKDOWN_REPORT}"

RUN_ID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["fixtures"][0]["run_id"])' "${JSON_REPORT}")"
STATUS="$(curl --silent --output /dev/null --write-out '%{http_code}' \
  -H 'Authorization: Bearer other-operator-token' \
  "http://127.0.0.1:${PORT}/v1/pipeline/runs/${RUN_ID}")"
[[ "${STATUS}" == "404" ]]

SECRET='ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890'
if rg --fixed-strings --quiet "${SECRET}" "${JSON_REPORT}" "${MARKDOWN_REPORT}" "${SERVER_LOG}"; then
  echo "secret probe appeared in pipeline output" >&2
  exit 1
fi

echo "JSON report: ${JSON_REPORT}"
echo "Markdown report: ${MARKDOWN_REPORT}"
