#!/usr/bin/env bash
# Reset perplexity-gate retry bookkeeping so a freshly-deployed gate binary
# re-attempts previously-failed submissions from attempts=0 — instead of us
# raising TRACE_COMMONS_PERPLEXITY_DRIVER_MAX_ATTEMPTS to sidestep the cap.
#
# Deletes rows from trace_gate_evaluation_attempts for submissions that still
# have NO gate decision (the stuck set). Successfully-scored submissions carry
# a decision row and are never touched.
#
# Uses two roles that already exist (no new grants, no migration):
#   - trace_gate_driver  (cross-tenant SELECT, from the driver's own URL) to
#     enumerate the stuck (tenant, submission) pairs
#   - app                (per-tenant DML, NOBYPASSRLS) to delete, with the
#     transaction-local tenant GUC set so RLS authorizes the delete
#
# Usage (on tc-pilot-host):
#   sudo deploy/pilot-gcp/reset-gate-attempts.sh                 # all stuck rows
#   sudo deploy/pilot-gcp/reset-gate-attempts.sh <submission_id> # one submission
#
# Env overrides: TC_INGEST_ENV (default /etc/tracecommons/ingest.env).
set -euo pipefail

ENV_FILE="${TC_INGEST_ENV:-/etc/tracecommons/ingest.env}"
APP_URL="$(grep -m1 '^DATABASE_URL=' "$ENV_FILE" | cut -d= -f2-)"
DRIVER_URL="$(grep -m1 '^TRACE_COMMONS_GATE_DRIVER_DATABASE_URL=' "$ENV_FILE" | cut -d= -f2-)"
[ -n "$APP_URL" ]    || { echo "DATABASE_URL not found in $ENV_FILE" >&2; exit 1; }
[ -n "$DRIVER_URL" ] || { echo "TRACE_COMMONS_GATE_DRIVER_DATABASE_URL not found in $ENV_FILE" >&2; exit 1; }

ONLY_SUB="${1:-}"
if [ -n "$ONLY_SUB" ] && ! [[ "$ONLY_SUB" =~ ^[0-9a-fA-F-]{8,36}$ ]]; then
  echo "argument must be a submission UUID, got: $ONLY_SUB" >&2
  exit 1
fi

# Fail closed if the app role cannot delete — then a grant migration is needed.
CAN_DELETE="$(psql "$APP_URL" -Atc \
  "select has_table_privilege(current_user, 'trace_gate_evaluation_attempts', 'DELETE');")"
if [ "$CAN_DELETE" != "t" ]; then
  echo "app role lacks DELETE on trace_gate_evaluation_attempts; add a grant migration first" >&2
  exit 2
fi

stuck_sql() {
  # Stuck = has an attempt row but no decision. $1 is an optional extra predicate.
  echo "select a.tenant_id || ' ' || a.submission_id
        from trace_gate_evaluation_attempts a
        left join trace_gate_decisions d
          on d.tenant_id = a.tenant_id and d.submission_id = a.submission_id
        where d.decision_id is null ${1:-}
        order by a.tenant_id, a.submission_id;"
}

filter=""
[ -n "$ONLY_SUB" ] && filter="and a.submission_id = '$ONLY_SUB'"
mapfile -t pairs < <(psql "$DRIVER_URL" -Atc "$(stuck_sql "$filter")")

if [ "${#pairs[@]}" -eq 0 ]; then
  echo "no stuck attempt rows to reset"
  exit 0
fi
before="${#pairs[@]}"
echo "found $before stuck submission(s) to reset"

for pair in "${pairs[@]}"; do
  tenant="${pair%% *}"
  sub="${pair##* }"
  # One transaction: set the transaction-local tenant GUC, then delete under RLS.
  psql "$APP_URL" -Atq >/dev/null <<SQL
begin;
select set_config('trace_commons.trace_tenant_id', '$tenant', true);
delete from trace_gate_evaluation_attempts
  where tenant_id = '$tenant' and submission_id = '$sub';
commit;
SQL
  echo "reset ${sub:0:8}"
done

after="$(psql "$DRIVER_URL" -Atc "select count(*) from ($(stuck_sql "")) s;")"
echo "done; stuck submissions before=$before after=$after"
