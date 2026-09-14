#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PG_PORT="${TRACE_COMMONS_PHASE7_TEST_PG_PORT:-55442}"
CONTAINER="trace-commons-phase7-tests-$$"

cleanup() {
  docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run --rm --detach \
  --name "${CONTAINER}" \
  -e POSTGRES_PASSWORD=phase7-test \
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
TRACE_COMMONS_PG_TEST_DATABASE_URL="postgres://postgres:phase7-test@127.0.0.1:${PG_PORT}/postgres" \
  RUSTFLAGS="-D warnings" \
  cargo test -p trace-commons-server --test versioned_pipeline_pg
