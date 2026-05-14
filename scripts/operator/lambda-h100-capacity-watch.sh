#!/usr/bin/env bash
#
# lambda-h100-capacity-watch.sh — poll the Lambda Cloud API for H100
# instance availability and print when capacity returns. Use this before
# scheduling an A.5a pre-flight or A2.7 re-bake-off so the operator can
# stop refreshing the dashboard.
#
# Usage:
#   ./scripts/operator/lambda-h100-capacity-watch.sh
#
#   Credentials: reads ~/.netrc for cloud.lambdalabs.com via `curl --netrc`.
#                The script never sees or logs the API key. If you don't
#                already have a netrc entry, add one like:
#
#                    machine cloud.lambdalabs.com
#                      login api
#                      password <your-lambda-api-key>
#
#                then `chmod 600 ~/.netrc`.
#
#   Optional env vars:
#     LAMBDA_POLL_SECONDS    — interval between polls (default 300 = 5 min)
#     LAMBDA_INSTANCE_GLOB   — case-insensitive substring filter on instance
#                              type name. Default: 'h100'. Override to
#                              'gpu_1x_h100' for a narrower watch or
#                              'h100|h200' (regex) to widen.
#
# Behavior:
#   - Polls https://cloud.lambdalabs.com/api/v1/instance-types every
#     LAMBDA_POLL_SECONDS until at least one matching instance type
#     reports a non-empty regions_with_capacity_available list.
#   - On match: prints one line per available (type, region) pair and
#     exits 0. The operator can then provision via the dashboard or
#     `lambda-cloud launch ...`.
#   - On API auth failure: exits 2 with a one-line stderr message.
#   - Hash-only audit: credentials live in ~/.netrc; the script's process
#     listing does not include the key, and the polled payload is never
#     written to disk.
#
# Stop with Ctrl-C.

set -euo pipefail

POLL=${LAMBDA_POLL_SECONDS:-300}
GLOB=${LAMBDA_INSTANCE_GLOB:-h100}
API_URL="https://cloud.lambdalabs.com/api/v1/instance-types"

if [[ ! -f "${HOME}/.netrc" ]]; then
  echo "error: ~/.netrc not found — see header comment for the expected entry" >&2
  exit 2
fi

echo "watching for matching capacity (filter: ${GLOB}, poll: ${POLL}s)..."

while true; do
  payload=$(curl -sS --netrc "${API_URL}" || true)

  if [[ -z "${payload}" ]]; then
    echo "$(date -u +%H:%M:%S) poll failed (empty response); retrying" >&2
    sleep "${POLL}"
    continue
  fi

  # Detect auth failures explicitly so the watcher exits instead of
  # spinning silently on a missing or bad credential.
  if echo "${payload}" | grep -qE '"code"[[:space:]]*:[[:space:]]*"global/invalid-api-key"'; then
    echo "error: Lambda API rejected the credential (check ~/.netrc entry for cloud.lambdalabs.com)" >&2
    exit 2
  fi

  available=$(python3 - <<PYEOF
import json, re, sys
try:
    d = json.loads("""${payload}""")
except Exception as exc:
    print(f"PARSE_ERROR: {exc}", file=sys.stderr)
    sys.exit(0)

types = d.get("data", {})
pattern = re.compile(r"""${GLOB}""", re.IGNORECASE)
hits = []
for name, entry in types.items():
    if not pattern.search(name):
        continue
    regions = entry.get("regions_with_capacity_available") or []
    if not regions:
        continue
    for r in regions:
        region_name = r.get("name") if isinstance(r, dict) else str(r)
        hits.append((name, region_name))

for name, region in hits:
    print(f"{name}\t{region}")
PYEOF
)

  if [[ -n "${available}" ]]; then
    echo ""
    echo "$(date -u +%FT%TZ) CAPACITY AVAILABLE:"
    echo "${available}" | awk -F'\t' '{ printf "  - %s in %s\n", $1, $2 }'
    exit 0
  fi

  echo "$(date -u +%H:%M:%S) no capacity yet"
  sleep "${POLL}"
done
