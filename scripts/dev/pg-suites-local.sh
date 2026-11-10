#!/usr/bin/env bash
#
# Run CI's PostgreSQL suites locally -- the `postgres-suites` job's, plus the
# ingest lib slices that CI runs in `ingest-bin-postgres` -- up to N at a
# time, each lane on a throwaway PostgreSQL cluster of its own.
#
#   scripts/dev/pg-suites-local.sh [--jobs N] [--only a,b,...] [--keep] [--list]
#
# Run it from the checkout whose code you want to test; it acts on the
# workspace that contains the current directory. Needs `initdb`, `pg_ctl`,
# `psql` and `createdb` on PATH (Homebrew `postgresql@16` provides them); it
# does not use, or need, a PostgreSQL server that is already running.
#
# What it does, in order:
#   1. Builds every test binary it needs once, with `cargo test --no-run`,
#      through the build lock (`turn.sh build`). Nothing compiles later.
#   2. Starts one cluster per lane, shaped like CI's service container:
#      superuser `trace`, trust auth, listening on 127.0.0.1 only on a free
#      port, no Unix socket, data under target/pg-suites-local/<run>/. It
#      creates CI's two login-resolver roles in each.
#   3. Deals the suites round-robin across the lanes. A lane runs its suites
#      one after another, each on a fresh database with CI's name, by
#      executing the prebuilt test binary directly (not `cargo test`, which
#      would hold cargo's build-directory lock and so serialise the lanes),
#      through the test lock (`turn.sh test`), into a log of its own.
#   4. Refuses a suite whose database shows fewer than 5 committed
#      transactions, as CI's "Refuse a run that executed nothing" does: a suite
#      that skipped for want of a variable reports green and commits nothing.
#      Rows that check a log line instead (CI does the same for them) say so.
#   5. Prints a summary, stops the clusters and deletes their data (also on
#      Ctrl-C), unless --keep, which leaves them running until the next run
#      (each run first stops any cluster whose run has exited). Logs stay
#      under target/pg-suites-local/<run>/.
#
# Why a cluster per lane and not a database per suite on one cluster: roles
# and role memberships are cluster-wide, and the migrations write them (CREATE
# ROLE IF NOT EXISTS, ALTER ROLE, GRANT <guard> TO CURRENT_USER ... REVOKE)
# every time they run on a fresh database. Two suites migrating two databases
# of one cluster at the same moment race on those catalog rows; measured on
# 2026-11-10 with `witness_blocked_pg` beside two other suites:
#   apply migrations: ... code: SqlState(EXX000), message: "tuple concurrently updated"
# The migration runner's advisory lock and tests/support/role_ddl_lock.rs are
# per database, so neither serialises across databases. A separate cluster
# per lane removes the shared state instead of scheduling around it, and
# matches CI, where the job's suites share one container one after another.
#
# Not run here: `nearai_ceremony_pg_tests` (needs a Linux Secret Service) and
# the Python token-distribution tests (no database). CI's two role-existence
# checks (trace_rls_assertion_runtime, trace_pipeline_runtime_test) run at the
# end of each lane that ran the suite concerned. The whole-lib ingest job
# (`ingest-bin-postgres`) is separate; see CLAUDE.md.
#
# Environment:
#   PG_SUITES_TEST_THREADS  libtest threads per suite (default 1).
#   TURN_SH             the turn lock (default ~/.claude/turn-locks/turn.sh;
#                       commands run unwrapped if it does not exist).

set -euo pipefail

JOBS=2
ONLY=""
KEEP=0
LIST=0
while [ $# -gt 0 ]; do
  case "$1" in
    --jobs) JOBS=${2:?--jobs needs a number}; shift 2 ;;
    --jobs=*) JOBS=${1#*=}; shift ;;
    --only) ONLY=${2:?--only needs a list}; shift 2 ;;
    --only=*) ONLY=${1#*=}; shift ;;
    --keep) KEEP=1; shift ;;
    --list) LIST=1; shift ;;
    -h|--help) sed -n '2,56p' "$0"; exit 0 ;;
    *) echo "pg-suites-local: unknown argument $1" >&2; exit 2 ;;
  esac
done
case "$JOBS" in ''|*[!0-9]*|0) echo "pg-suites-local: --jobs must be a positive integer" >&2; exit 2 ;; esac

# name|package|target|test args|databases|env|required pattern|forbidden pattern
#
#   package    S = trace-commons-server, I = trace-commons-ingest
#   target     `lib`, or the integration test target name
#   databases  created fresh before the suite; `~name` is created but not held
#              to the transaction count, because the test works in a
#              `<name>_pilot` database of its own and CI checks its log instead
#   env        VAR=@db (URL to that database), VAR=@resolver:db (the login
#              resolver's URL to it), VAR=@postgres, or VAR=literal
#
# Mirrors the `postgres-suites` job in .github/workflows/ci.yml, one row per
# step that runs a test; keep the two in step.
SUITES='
account_onboarding_pg|S|account_onboarding_pg||admission_test_onboarding|TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL=@admission_test_onboarding||
account_trust_pg|S|account_trust_pg||admission_test_account_trust|TRACE_COMMONS_ACCOUNT_TRUST_PG_TEST_DATABASE_URL=@admission_test_account_trust||
account_trust_growth_pg|S|account_trust_growth_pg||admission_test_account_trust_growth|TRACE_COMMONS_ACCOUNT_TRUST_GROWTH_PG_TEST_DATABASE_URL=@admission_test_account_trust_growth||
upload_trust_frontier_plan_pg|S|upload_trust_frontier_plan_pg||admission_test_upload_trust_plan_ci|TRACE_COMMONS_UPLOAD_TRUST_PLAN_PG_TEST_URL=@admission_test_upload_trust_plan_ci||
upload_trust_upgrade|S|lib|v137_invalidates_already_enabled_pre_migration_applied_generations_once --ignored|admission_test_upload_trust_upgrade_ci|TRACE_COMMONS_UPLOAD_TRUST_PG_UPGRADE_TEST_URL=@admission_test_upload_trust_upgrade_ci||
upload_trust_scheduler|I|lib|client_upload_trust_scheduler_pg_tests --ignored|admission_test_upload_trust_scheduler_ci|TRACE_COMMONS_CLIENT_UPLOAD_TRUST_PG_TEST_URL=@admission_test_upload_trust_scheduler_ci||
graded_corpus|I|lib|a_graded_production_package_passes_the_corpus_check --ignored|admission_test_graded_corpus|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_graded_corpus||
starter_plausibility_pg|S|starter_plausibility_pg||admission_test_starter|TRACE_COMMONS_STARTER_PG_TEST_DATABASE_URL=@admission_test_starter||
account_trust_live_pg|S|account_trust_live_pg||admission_test_account_trust_live|TRACE_COMMONS_ACCOUNT_TRUST_LIVE_PG_TEST_DATABASE_URL=@admission_test_account_trust_live||
account_invite_route|I|lib|account_invite_route_requires_session_and_rejects_cross_site_cookie|admission_test_account_trust_route|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_account_trust_route||
legacy_invite_link_pg|S|legacy_invite_link_pg||admission_test_legacy_invite_link|TRACE_COMMONS_LEGACY_INVITE_LINK_PG_TEST_DATABASE_URL=@admission_test_legacy_invite_link||
legacy_payout_carry_pg|S|legacy_payout_carry_pg||admission_test_legacy_payout_carry|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_legacy_payout_carry||
device_invite_subject_pg|S|device_invite_subject_pg||admission_test_device_invite_subject|TRACE_COMMONS_DEVICE_INVITE_SUBJECT_PG_TEST_DATABASE_URL=@admission_test_device_invite_subject||
inference_connection_pg|S|inference_connection_pg||admission_test_inference_connection|TRACE_COMMONS_INFERENCE_CONNECTION_PG_TEST_DATABASE_URL=@admission_test_inference_connection||
referral_rewards_pg|S|referral_rewards_pg||admission_test_referral_rewards|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_referral_rewards||
referral_codes_pg|S|referral_codes_pg||admission_test_referral_codes|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_referral_codes||
referral_bonus_pg|S|referral_bonus_pg||admission_test_referral_bonus|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_referral_bonus||
payout_rails_pg|S|payout_rails_pg||admission_test_payout_rails|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_payout_rails||
first_run_stats_pg|S|first_run_stats_pg||admission_test_first_run_stats|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_first_run_stats||
witness_blocked_pg|S|witness_blocked_pg||admission_test_witness_blocked|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_witness_blocked||
payout_pass_pg|S|payout_pass_pg||admission_test_payout_pass|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_payout_pass||
referral_stream_pg|S|referral_stream_pg||admission_test_referral_stream|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_referral_stream||
inference_connection_route|I|lib|inference_connection_requires_explicit_account_session_selection|admission_test_inference_connection_route|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_inference_connection_route||
migration_atomicity_pg|S|migration_atomicity_pg||admission_test_migration|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_migration||
trace_invite_registry_pg|S|trace_invite_registry_pg||admission_test_invite|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_invite||
account_merge_pg|S|account_merge_pg||admission_test_merge|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_merge||
trace_corpus_pg_rls|S|trace_corpus_pg_rls||admission_test_rls|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_rls||
token_bundle_pg|S|token_bundle_pg||admission_test_tokens|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_tokens||
token_bundle_http_journey|I|lib|token_bundle_http_journey|admission_test_tokens|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_tokens TRACE_COMMONS_BUNDLE_SERVER_ID=journey-server||
trace_corpus_pg_store|S|trace_corpus_pg_store||admission_test_store|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_store||
witness_evidence_pg|S|witness_evidence_pg||admission_test_witness_evidence|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_witness_evidence||
trace_session_withdrawal_pg|S|trace_session_withdrawal_pg||admission_test_source_sessions|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_source_sessions||
upload_principal_ownership_pg|S|upload_principal_ownership_pg||admission_test_upload_ownership|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_upload_ownership||
credit_hold_ownership|S|lib|credit_hold_ownership::tests|admission_test_credit_hold_ownership|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_credit_hold_ownership||
activity_missions_pg|S|activity_missions_pg||admission_test_activity_missions|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_activity_missions||
model_training_consent_pg|S|model_training_consent_pg||admission_test_model_training_consent|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_model_training_consent||
r1_pipeline_measurement|S|r1_pipeline_measurement|--nocapture||TRACE_COMMONS_R1_MEASUREMENT_PG_TEST_URL=@postgres|test result: ok\.|^skipping:
legacy_invite_counts|S|legacy_invite_counts|--nocapture||TRACE_COMMONS_LEGACY_INVITE_COUNTS_PG_TEST_URL=@postgres|test result: ok\.|^skipping:
pipeline_upgrade|S|lib|pipeline_upgrade --ignored|pipeline_test_upgrade|TRACE_COMMONS_PIPELINE_PG_UPGRADE_TEST_URL=@pipeline_test_upgrade||
versioned_pipeline_runtime_pg|S|versioned_pipeline_runtime_pg||admission_test_pipeline|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_pipeline||
pipeline_real_http|I|lib|real_http_receipt_completes_and_resumes_after_restart|~admission_test_pipeline_http|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_pipeline_http|test result: ok\. 1 passed; 0 failed|
pipeline_compat|I|lib|compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export|~admission_test_pipeline_compat|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_pipeline_compat TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=@resolver:admission_test_pipeline_compat|test result: ok\. 1 passed; 0 failed|
pipeline_parity|I|lib|legacy_and_pipeline_tenants_match_except_pinned_differences|~admission_test_pipeline_parity|TRACE_COMMONS_PG_TEST_DATABASE_URL=@admission_test_pipeline_parity TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=@resolver:admission_test_pipeline_parity|test result: ok\. 1 passed; 0 failed|
mission_rewards_pg|S|mission_rewards_pg|--ignored|reward_test_missions|TRACE_COMMONS_REWARDS_PG_TEST_URL=@reward_test_missions||
reward_v69_upgrade|S|lib|reward_v69_upgrade --ignored|reward_test_missions_upgrade|TRACE_COMMONS_REWARDS_PG_UPGRADE_TEST_URL=@reward_test_missions_upgrade||
reward_participant_pg|S|reward_participant_pg|--ignored|reward_test_participants|TRACE_COMMONS_REWARDS_PG_TEST_URL=@reward_test_participants||
reward_participant_http|I|lib|reward_participant_http_contract --ignored|reward_test_participant_http|TRACE_COMMONS_REWARDS_PG_TEST_URL=@reward_test_participant_http||
mission_publication_pg|S|mission_publication_pg|--ignored|reward_test_mission_packages|TRACE_COMMONS_REWARDS_PG_TEST_URL=@reward_test_mission_packages||
mission_catalog_http|I|lib|public_mission_catalog_http_is_bounded_anonymous_and_side_effect_free --ignored|reward_test_mission_http|TRACE_COMMONS_REWARDS_PG_TEST_URL=@reward_test_mission_http||
admission_ledger_pg|S|admission_ledger_pg|--ignored|admission_test_ledger admission_test_account_ledger|TRACE_COMMONS_ADMISSION_PG_TEST_URL=@admission_test_ledger TRACE_COMMONS_ACCOUNT_ADMISSION_PG_TEST_URL=@admission_test_account_ledger||
account_trust_external_pg|S|account_trust_external_pg|--ignored|admission_test_account_trust_external|TRACE_COMMONS_ACCOUNT_ADMISSION_PG_TEST_URL=@admission_test_account_trust_external||
admission_pg_tests|I|lib|admission_pg_tests --ignored|admission_test_ingest|TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL=@admission_test_ingest||
wallet_v2_pg_tests|I|lib|wallet_v2_pg_tests --ignored|admission_test_walletv2|TRACE_COMMONS_WALLET_V2_PG_TEST_URL=@admission_test_walletv2||
near_ai_rebind_pg_tests|I|lib|near_ai_rebind_pg_tests --ignored|admission_test_rebind|TRACE_COMMONS_REBIND_PG_TEST_URL=@admission_test_rebind||
'

if [ "$LIST" = 1 ]; then
  printf '%s\n' "$SUITES" | sed '/^$/d' | cut -d'|' -f1
  exit 0
fi

SEL=""
if [ -n "$ONLY" ]; then
  for want in $(printf '%s' "$ONLY" | tr ',' ' '); do
    row=$(printf '%s\n' "$SUITES" | grep "^$want|" || true)
    [ -n "$row" ] || { echo "pg-suites-local: no suite named $want (see --list)" >&2; exit 2; }
    SEL="$SEL$row
"
  done
else
  SEL=$(printf '%s\n' "$SUITES" | sed '/^$/d')
fi

for tool in initdb pg_ctl psql createdb python3; do
  command -v "$tool" >/dev/null || { echo "pg-suites-local: $tool is not on PATH" >&2; exit 1; }
done

ROOT=$(git rev-parse --show-toplevel)
THREADS=${PG_SUITES_TEST_THREADS:-1}
TURN=${TURN_SH:-$HOME/.claude/turn-locks/turn.sh}
RUN="$(date +%Y%m%dT%H%M%S)-$$"
WORK="$ROOT/target/pg-suites-local/$RUN"
mkdir -p "$WORK"
echo $$ > "$WORK/owner.pid"

turn() { # turn <build|test> <label> -- cmd...
  local kind=$1 label=$2; shift 3
  if [ -f "$TURN" ]; then bash "$TURN" "$kind" "$label" -- "$@"; else "$@"; fi
}

# A previous run killed before its EXIT trap leaves a postmaster behind.
# Stop any whose owning script is gone.
for pidfile in "$ROOT"/target/pg-suites-local/*/lane*/postmaster.pid; do
  [ -f "$pidfile" ] || continue
  lanedir=$(dirname "$pidfile"); rundir=$(dirname "$lanedir")
  owner=$(cat "$rundir/owner.pid" 2>/dev/null || echo "")
  if [ -z "$owner" ] || ! kill -0 "$owner" 2>/dev/null; then
    pg_ctl -D "$lanedir" -m immediate stop >/dev/null 2>&1 || true
    rm -rf "$lanedir"
    echo "pg-suites-local: stopped a cluster left by an earlier run ($lanedir)" >&2
  fi
done

# ---- 1. build once -------------------------------------------------------
tests=()
need_s=0; need_s_lib=0; need_i_lib=0
while IFS='|' read -r _name pkg target _rest; do
  [ -n "$pkg" ] || continue
  if [ "$target" = lib ]; then
    if [ "$pkg" = S ]; then need_s_lib=1; else need_i_lib=1; fi
  else
    need_s=1; tests+=(--test "$target")
  fi
done <<EOF
$SEL
EOF
build=(cargo test --no-run --message-format=json-render-diagnostics)
if [ "$need_s" = 1 ] || [ "$need_s_lib" = 1 ]; then build+=(-p trace-commons-server); fi
if [ "$need_i_lib" = 1 ]; then build+=(-p trace-commons-ingest); fi
if [ "$need_s_lib" = 1 ] || [ "$need_i_lib" = 1 ]; then build+=(--lib); fi
build+=(${tests[@]+"${tests[@]}"})
echo "pg-suites-local: building test binaries; logs in $WORK" >&2
(cd "$ROOT" && turn build "pg-suites-local build" -- "${build[@]}") \
  > "$WORK/build.json" 2> "$WORK/build.log" \
  || { tail -40 "$WORK/build.log" >&2; echo "pg-suites-local: build failed" >&2; exit 1; }

# "<S|I>:<lib|test name> <executable> <manifest>", from cargo's artifacts.
python3 -I - "$WORK/build.json" > "$WORK/executables.txt" <<'PY'
import json, sys
short = {"trace-commons-server": "S", "trace-commons-ingest": "I"}
for line in open(sys.argv[1]):
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") != "compiler-artifact" or not m.get("executable"):
        continue
    if not m.get("profile", {}).get("test"):
        continue
    t, pkg = m["target"], m["package_id"]
    tag = next((v for k, v in short.items()
                if "/" + k + "#" in pkg or "#" + k + "@" in pkg or pkg.startswith(k + " ")), None)
    if tag is None:
        continue
    kind = "lib" if "lib" in t["kind"] else t["name"]
    print(f"{tag}:{kind} {m['executable']} {m['manifest_path']}")
PY
exe_for() { awk -v k="$1" '$1==k {print $2; exit}' "$WORK/executables.txt"; }
dir_for() { awk -v k="$1" '$1==k {print $3; exit}' "$WORK/executables.txt" | xargs dirname; }

# ---- 2. one cluster per lane ---------------------------------------------
LANES=$JOBS
n_sel=$(printf '%s\n' "$SEL" | sed '/^$/d' | wc -l | tr -d ' ')
[ "$n_sel" -lt "$LANES" ] && LANES=$n_sel

PORTS=()
cleanup() {
  local k
  for k in $(seq 1 "$LANES"); do
    [ -d "$WORK/lane$k" ] || continue
    if [ "$KEEP" = 1 ]; then
      echo "pg-suites-local: kept lane $k on 127.0.0.1:${PORTS[$((k - 1))]:-?} (pg_ctl -D $WORK/lane$k stop)" >&2
    else
      pg_ctl -D "$WORK/lane$k" -m fast stop >/dev/null 2>&1 || pg_ctl -D "$WORK/lane$k" -m immediate stop >/dev/null 2>&1 || true
      rm -rf "$WORK/lane$k"
    fi
  done
}
trap cleanup EXIT
trap 'exit 130' INT TERM HUP

free_port() {
  python3 -I -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

for k in $(seq 1 "$LANES"); do
  dir="$WORK/lane$k"
  initdb -D "$dir" -U trace --auth=trust -E UTF8 --no-locale -N >"$WORK/lane$k.initdb.log" 2>&1 \
    || { cat "$WORK/lane$k.initdb.log" >&2; exit 1; }
  started=0
  for _ in 1 2 3; do
    port=$(free_port)
    if pg_ctl -D "$dir" -w -t 30 -l "$WORK/lane$k.server.log" \
         -o "-c listen_addresses=127.0.0.1 -c port=$port -c unix_socket_directories='' -c fsync=off -c synchronous_commit=off -c full_page_writes=off" \
         start >/dev/null 2>&1; then
      started=1; break
    fi
  done
  [ "$started" = 1 ] || { tail -20 "$WORK/lane$k.server.log" >&2; echo "pg-suites-local: lane $k cluster did not start" >&2; exit 1; }
  PORTS+=("$port")
  # CI creates these before any suite migrates (V30 creates the group only
  # if it is absent).
  psql -X -q -h 127.0.0.1 -p "$port" -U trace -d postgres -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
CREATE ROLE trace_login_resolver NOLOGIN NOBYPASSRLS;
CREATE ROLE tc_login_resolver_login LOGIN NOBYPASSRLS;
GRANT trace_login_resolver TO tc_login_resolver_login;
SQL
done

# ---- 3. run --------------------------------------------------------------
# run_suite <record> <port>: writes "$WORK/<name>.status"
run_suite() {
  local rec=$1 port=$2 name pkg target targs dbs envs req forbid
  IFS='|' read -r name pkg target targs dbs envs req forbid <<EOF
$rec
EOF
  local key="$pkg:$target" exe dir log="$WORK/$name.log" status=PASS reason="" d n kv
  local q=(psql -X -qtA -h 127.0.0.1 -p "$port" -U trace -d postgres)
  exe=$(exe_for "$key"); dir=$(dir_for "$key")
  if [ -z "$exe" ]; then
    echo "FAIL|no test binary for $key|0" > "$WORK/$name.status"; return
  fi
  for d in $dbs; do
    d=${d#\~}
    "${q[@]}" -c "DROP DATABASE IF EXISTS \"$d\" WITH (FORCE)" >/dev/null 2>&1 || true
    "${q[@]}" -c "CREATE DATABASE \"$d\"" >/dev/null
  done
  local envargs=() spec
  for kv in $envs; do
    spec=${kv#*=}
    case "$spec" in
      @resolver:*) spec="postgres://tc_login_resolver_login@127.0.0.1:$port/${spec#@resolver:}" ;;
      @*) spec="postgres://trace@127.0.0.1:$port/${spec#@}" ;;
    esac
    envargs+=("${kv%%=*}=$spec")
  done
  local start end rc
  start=$(date +%s)
  set +e
  # shellcheck disable=SC2086 # $targs is a word list: filter and flags
  (cd "$dir" && turn test "pg-suites-local $name" -- \
     env CARGO_MANIFEST_DIR="$dir" ${envargs[@]+"${envargs[@]}"} \
     "$exe" $targs --test-threads="$THREADS") > "$log" 2>&1
  rc=$?
  set -e
  end=$(date +%s)
  if [ $rc -ne 0 ]; then status=FAIL; reason="exit $rc"; fi
  if [ "$status" = PASS ] && [ -n "$req" ] && ! grep -Eq "$req" "$log"; then
    status=FAIL; reason="log lacks /$req/"
  fi
  if [ "$status" = PASS ] && [ -n "$forbid" ] && grep -Eq "$forbid" "$log"; then
    status=FAIL; reason="log has /$forbid/: the suite skipped"
  fi
  if [ "$status" = PASS ]; then
    for d in $dbs; do
      case "$d" in \~*) continue ;; esac
      n=$("${q[@]}" -c "SELECT xact_commit FROM pg_stat_database WHERE datname = '$d'" 2>/dev/null || echo 0)
      if [ "${n:-0}" -lt 5 ]; then status=FAIL; reason="$d committed ${n:-0} transactions: the suite skipped"; fi
    done
  fi
  # CI's two role-existence checks: the role is created only on the path the
  # suite must take, so its absence means the assertions never ran.
  case "$name" in
    trace_corpus_pg_rls) want_role=trace_rls_assertion_runtime ;;
    versioned_pipeline_runtime_pg) want_role=trace_pipeline_runtime_test ;;
    *) want_role="" ;;
  esac
  if [ "$status" = PASS ] && [ -n "$want_role" ] \
     && [ "$("${q[@]}" -c "SELECT count(*) FROM pg_roles WHERE rolname = '$want_role'")" != 1 ]; then
    status=FAIL; reason="$want_role was never provisioned: the suite's assertions did not run"
  fi
  [ -n "$reason" ] || reason=$(grep -E '^test result:' "$log" | tail -1)
  echo "$status|$reason|$((end - start))|lane on :$port" > "$WORK/$name.status"
}

lane() { # lane <k>
  local k=$1 port=${PORTS[$(($1 - 1))]} rec
  printf '%s\n' "$SEL" | sed '/^$/d' | awk -v n="$LANES" -v k="$k" '(NR - 1) % n == k - 1' \
    | while IFS= read -r rec; do run_suite "$rec" "$port" || true; done
}

T0=$(date +%s)
pids=()
for k in $(seq 1 "$LANES"); do
  lane "$k" & pids+=($!)
done
for p in "${pids[@]}"; do wait "$p" || true; done
T1=$(date +%s)

# ---- 4. summary ----------------------------------------------------------
pass=0; fail=0
printf '\n%-32s %-6s %6s  %s\n' SUITE RESULT SECS DETAIL
while IFS='|' read -r name _rest; do
  [ -n "$name" ] || continue
  if [ -f "$WORK/$name.status" ]; then
    IFS='|' read -r st why secs _where < "$WORK/$name.status"
  else
    st=FAIL; why="no result recorded (its lane stopped early)"; secs=0
  fi
  if [ "$st" = PASS ]; then pass=$((pass + 1)); else fail=$((fail + 1)); fi
  printf '%-32s %-6s %6s  %s\n' "$name" "$st" "$secs" "$why"
done <<EOF
$SEL
EOF
echo
echo "pg-suites-local: $pass passed, $fail failed, $((T1 - T0))s wall over $LANES lane(s); logs in $WORK"
[ "$fail" = 0 ]
