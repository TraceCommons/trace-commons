#!/usr/bin/env bash
# The staging database backup and its restore check
# (deploy/staging-gcp/backup-db.sh, restore-check.sh; staging design 3.2),
# run against stubs. No network, no gcloud, no PostgreSQL, no root.
#
# This holds:
#
# - every command that reaches PostgreSQL (psql, pg_dump, pg_restore into a
#   database, createdb, dropdb) runs through `runuser -u postgres --`: every
#   Trace Commons table has FORCE ROW LEVEL SECURITY, which binds the owner
#   too, so only a superuser dumps or counts every row;
# - the backup refuses a session that is not a superuser, uploading nothing;
# - pg_dump runs on the snapshot the row counts were taken in, writes to a
#   mode-0600 file, and the dump, then the counts, then the sha256 sidecar
#   are uploaded, each with --if-generation-match=0, the sidecar last;
# - no local copy is left behind, and nothing but labels, sizes and hashes is
#   printed (no table name, no count);
# - a failed sidecar upload fails the backup, and the restore check then
#   takes the newest backup that has a sidecar;
# - the restore check verifies the sha256 before creating any database,
#   restores into a throwaway tc_restore_check_* database, compares every
#   table's row count with the counts taken at dump time, drops the throwaway
#   database whatever happened, and fails on a count mismatch, a hash
#   mismatch or a failed restore; drift against the live database is
#   reported, not failed;
# - both scripts count with the same SQL;
# - shellcheck, where installed, finds nothing.
#
# Seconds.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
backup="$repo/deploy/staging-gcp/backup-db.sh"
restore="$repo/deploy/staging-gcp/restore-check.sh"
units="$repo/deploy/staging-gcp/systemd"
failures=0
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

pass() {
  echo "ok: $*"
}

for f in "$backup" "$restore"; do
  [ -f "$f" ] || { echo "FAIL: $f is missing" >&2; exit 1; }
done

bucket=gs://tc-staging-artifacts-20261021
prefix=backups/postgres

# --- stubs -----------------------------------------------------------------
stubs="$scratch/stubs"
mkdir -p "$stubs"

# runuser: must be `-u postgres --`; marks what it runs as postgres.
cat >"$stubs/runuser" <<'STUB'
#!/usr/bin/env bash
if [ "${1:-}" != -u ] || [ "${2:-}" != postgres ] || [ "${3:-}" != -- ]; then
  printf '%s\n' "runuser BAD $*" >>"$TRACE"
  exit 97
fi
shift 3
AS_POSTGRES=1 exec "$@"
STUB

# A database command: logs who ran it and its arguments. Expanded in the
# stub, not here.
# shellcheck disable=SC2016
log_cmd='printf "%s as_postgres=%s %s\n" "$(basename "$0")" "${AS_POSTGRES:-0}" "$*" >>"$TRACE"'

# psql answers by marker, one line at a time, so the backup's held session
# (a pair of FIFOs) gets each answer as soon as it asks.
cat >"$stubs/psql" <<STUB
#!/usr/bin/env bash
$log_cmd
db=""
prev=""
for a; do
  [ "\$prev" = -d ] && db="\$a"
  case "\$a" in --dbname=*) db="\${a#--dbname=}" ;; esac
  prev="\$a"
done
counts_for() {
  case "\$1" in
    tc_restore_check_*) cat "\$FAKE_RESTORED_COUNTS" ;;
    *) cat "\$FAKE_LIVE_COUNTS" ;;
  esac
}
while IFS= read -r line; do
  case "\$line" in
    *pg_export_snapshot*) echo "00000003-0000001B-1" ;;
    *is_superuser*) echo "\${FAKE_SUPERUSER:-on}" ;;
    *datname*tc_restore_check_*) [ -z "\${FAKE_LEFTOVER_DBS:-}" ] || printf '%s\n' \$FAKE_LEFTOVER_DBS ;;
    *'\\gexec'*) counts_for "\$db" ;;
    *tc-backup-end*) echo tc-backup-end ;;
    '\\q') exit 0 ;;
  esac
done
exit 0
STUB

cat >"$stubs/pg_dump" <<STUB
#!/usr/bin/env bash
$log_cmd
printf 'PGDMP fake custom-format archive %s\n' "\${FAKE_DUMP_TAG:-one}"
exit "\${FAKE_PG_DUMP_EXIT:-0}"
STUB

cat >"$stubs/pg_restore" <<STUB
#!/usr/bin/env bash
$log_cmd
case " \$* " in
  *" --list "*)
    head -c 5 | grep -q PGDMP || exit 1
    exit 0 ;;
esac
cat >/dev/null
exit "\${FAKE_PG_RESTORE_EXIT:-0}"
STUB

for tool in createdb dropdb; do
  cat >"$stubs/$tool" <<STUB
#!/usr/bin/env bash
$log_cmd
exit 0
STUB
done

cat >"$stubs/date" <<'STUB'
#!/usr/bin/env bash
case "$*" in
  *%Y%m%d%H%M%S*) echo 20260102030405 ;;
  *) echo "${FAKE_TS:-20260101T000000Z}" ;;
esac
STUB

# gcloud storage cp/ls over a local directory standing in for the bucket.
# An upload records the source file's mode, and refuses an existing object
# under --if-generation-match=0, as GCS does.
cat >"$stubs/gcloud" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "gcloud $*" >>"$TRACE"
[ "$1" = storage ] || exit 2
shift
op="$1"; shift
local_path() { printf '%s/%s' "$FAKE_GCS" "${1#gs://}"; }
case "$op" in
  cp)
    precondition=0; args=()
    for a; do
      case "$a" in
        --if-generation-match=0) precondition=1 ;;
        --*) ;;
        *) args+=("$a") ;;
      esac
    done
    src="${args[0]}"; dst="${args[1]}"
    case "$dst" in
      gs://*)
        if [ -n "${FAKE_GCLOUD_FAIL_ON:-}" ]; then
          case "$dst" in *"$FAKE_GCLOUD_FAIL_ON") echo "ERROR: upload failed" >&2; exit 1 ;; esac
        fi
        target="$(local_path "$dst")"
        if [ "$precondition" = 1 ] && [ -e "$target" ]; then
          echo "ERROR: HTTPError 412: precondition failed" >&2; exit 1
        fi
        printf 'mode %s %s\n' "$(basename "$dst")" "$(ls -l "$src" | cut -c1-10)" >>"$TRACE"
        mkdir -p "$(dirname "$target")"
        cp "$src" "$target" ;;
      *)
        source="$(local_path "$src")"
        [ -f "$source" ] || { echo "ERROR: not found" >&2; exit 1; }
        if [ -d "$dst" ]; then cp "$source" "$dst/"; else cp "$source" "$dst"; fi ;;
    esac ;;
  ls)
    url="${1%/}"
    dir="$(local_path "$url")"
    [ -d "$dir" ] || exit 1
    for f in "$dir"/*; do
      [ -f "$f" ] && printf '%s/%s\n' "$url" "$(basename "$f")"
    done
    exit 0 ;;
  *) exit 2 ;;
esac
STUB

# macOS has shasum, not sha256sum. The scripts run on PATH=stubs:/usr/bin:/bin.
if ! PATH=/usr/bin:/bin command -v sha256sum >/dev/null 2>&1; then
  cat >"$stubs/sha256sum" <<'STUB'
#!/usr/bin/env bash
exec shasum -a 256 "$@"
STUB
fi
chmod +x "$stubs"/*

# --- fixtures ----------------------------------------------------------------
printf 'public.trace_submissions 42\npublic.trace_tenants 3\n' >"$scratch/counts-live"
cp "$scratch/counts-live" "$scratch/counts-good"
printf 'public.trace_submissions 0\npublic.trace_tenants 3\n' >"$scratch/counts-short"
printf 'public.trace_submissions 45\npublic.trace_tenants 3\n' >"$scratch/counts-drifted"

run() {
  # $1 script; $2 name (trace, out, err); then NAME=value overrides.
  local script="$1" name="$2"
  shift 2
  : >"$scratch/$name.trace"
  env -i PATH="$stubs:/usr/bin:/bin" HOME="$scratch/home" \
    TRACE="$scratch/$name.trace" FAKE_GCS="$scratch/gcs" \
    FAKE_LIVE_COUNTS="$scratch/counts-live" \
    FAKE_RESTORED_COUNTS="$scratch/counts-good" \
    TC_DB_BACKUP_SKIP_ROOT_CHECK=1 TC_DB_BACKUP_STATE_DIR="$scratch/state" \
    "$@" bash "$script" >"$scratch/$name.out" 2>"$scratch/$name.err"
}

show() {
  for f in trace out err; do
    echo "--- $1.$f" >&2
    cat "$scratch/$1.$f" >&2 || true
  done
}

# Every database command in a trace runs as postgres. `pg_restore --list`
# reads the archive on stdin and connects to nothing, so it may run as root.
all_as_postgres() {
  local bad
  bad="$(grep -E '^(psql|pg_dump|pg_restore|createdb|dropdb) ' "$1" \
    | grep -v ' as_postgres=1 ' | grep -v -E '^pg_restore as_postgres=0 --list' || true)"
  [ -z "$bad" ] && ! grep -q '^runuser BAD' "$1"
}

line_of() { { grep -n -E "$2" "$1" || true; } | head -n 1 | cut -d: -f1; }

# --- 1. a backup -------------------------------------------------------------
if run "$backup" b1; then
  pass "backup-db.sh succeeds"
else
  fail "backup-db.sh failed"; show b1
fi
t="$scratch/b1.trace"
if all_as_postgres "$t" && grep -q '^pg_dump as_postgres=1 ' "$t" && grep -q '^psql as_postgres=1 ' "$t"; then
  pass "backup: psql and pg_dump run as postgres"
else
  fail "backup: a database command did not run as postgres"; show b1
fi
if grep -q '^pg_dump .*-Fc' "$t" && grep -q '^pg_dump .*--snapshot=00000003-0000001B-1' "$t" \
  && grep -q '^pg_dump .*tracecommons' "$t"; then
  pass "backup: pg_dump -Fc of tracecommons on the counted snapshot"
else
  fail "backup: pg_dump is not -Fc on the exported snapshot"; show b1
fi
if grep -q -E '^pg_dump .*(--enable-row-security| -h |--host)' "$t"; then
  fail "backup: pg_dump enables row security or leaves the local socket"
else
  pass "backup: pg_dump keeps row security off and uses the local socket"
fi
d=$(line_of "$t" "^gcloud storage cp .*$bucket/$prefix/20260101T000000Z\.dump\$")
c=$(line_of "$t" "^gcloud storage cp .*$bucket/$prefix/20260101T000000Z\.counts\$")
s=$(line_of "$t" "^gcloud storage cp .*$bucket/$prefix/20260101T000000Z\.dump\.sha256\$")
last_cp=$(grep -n '^gcloud storage cp ' "$t" | tail -n 1 | cut -d: -f1)
if [ -n "$d" ] && [ -n "$c" ] && [ -n "$s" ] && [ "$d" -lt "$c" ] && [ "$c" -lt "$s" ] \
  && [ "$s" = "$last_cp" ]; then
  pass "backup: dump, then counts, then the sha256 sidecar last"
else
  fail "backup: upload order is not dump, counts, sidecar (d=$d c=$c s=$s last=$last_cp)"; show b1
fi
pgd=$(line_of "$t" '^pg_dump ')
if [ -n "$pgd" ] && [ -n "$d" ] && [ "$pgd" -lt "$d" ]; then
  pass "backup: nothing uploaded before pg_dump finished"
else
  fail "backup: an upload precedes pg_dump"
fi
if [ "$(grep -c '^gcloud storage cp ' "$t")" = 3 ] \
  && [ "$(grep '^gcloud storage cp ' "$t" | grep -c -- '--if-generation-match=0')" = 3 ]; then
  pass "backup: every upload refuses to overwrite (--if-generation-match=0)"
else
  fail "backup: an upload may overwrite an existing object"; show b1
fi
if [ "$(grep -c -E '^mode [^ ]+ -rw-------$' "$t")" = 3 ]; then
  pass "backup: the dump, counts and sidecar files are mode 0600"
else
  fail "backup: a local file is not mode 0600"; grep '^mode' "$t" >&2 || true
fi
obj="$scratch/gcs/${bucket#gs://}/$prefix"
if [ -f "$obj/20260101T000000Z.dump" ] && [ -f "$obj/20260101T000000Z.counts" ] \
  && (cd "$obj" && sha256sum -c 20260101T000000Z.dump.sha256 >/dev/null 2>&1) \
  && [ "$(wc -l <"$obj/20260101T000000Z.dump.sha256" | tr -d ' ')" = 2 ] \
  && cmp -s "$obj/20260101T000000Z.counts" "$scratch/counts-live"; then
  pass "backup: the sidecar verifies the dump and the counts, which match the database"
else
  fail "backup: the uploaded sidecar does not verify both objects"
fi
if [ -z "$(find "$scratch/state" -type f -not -path '*/gcloud/*' 2>/dev/null)" ]; then
  pass "backup: no local copy is left behind"
else
  fail "backup: files left in the state directory"; find "$scratch/state" >&2
fi
if grep -q -E 'trace_submissions|trace_tenants|42' "$scratch/b1.out" "$scratch/b1.err"; then
  fail "backup: printed a table name or a count"; show b1
else
  pass "backup: prints labels, sizes and hashes only"
fi
if grep -q -E '^backup-db: uploaded .*sha256=[0-9a-f]{64}' "$scratch/b1.out"; then
  pass "backup: reports the object, size and sha256"
else
  fail "backup: no closing line with size and sha256"; show b1
fi

# A second run in the same second must refuse, not overwrite.
if run "$backup" b1again; then
  fail "backup: a second backup under the same name succeeded"
else
  pass "backup: refuses to overwrite an existing backup"
fi

# --- 2. a backup that is not a superuser -------------------------------------
if run "$backup" b2 FAKE_SUPERUSER=off FAKE_TS=20260101T010000Z; then
  fail "backup: ran as a session that is not a superuser"; show b2
elif grep -q '^gcloud storage cp' "$scratch/b2.trace" || grep -q '^pg_dump' "$scratch/b2.trace"; then
  fail "backup: dumped or uploaded without a superuser session"; show b2
elif grep -q superuser "$scratch/b2.err"; then
  pass "backup: refuses a session that is not a superuser, before dumping"
else
  fail "backup: refused without naming the superuser check"; show b2
fi

# --- 3. a failed pg_dump ------------------------------------------------------
if run "$backup" b3 FAKE_PG_DUMP_EXIT=1 FAKE_TS=20260101T020000Z; then
  fail "backup: a failed pg_dump succeeded"
elif grep -q '^gcloud storage cp' "$scratch/b3.trace"; then
  fail "backup: uploaded after a failed pg_dump"
else
  pass "backup: a failed pg_dump uploads nothing"
fi

# --- 4. a failed sidecar upload leaves an incomplete backup ------------------
if run "$backup" b4 FAKE_GCLOUD_FAIL_ON=.sha256 FAKE_TS=20260201T000000Z FAKE_DUMP_TAG=newer; then
  fail "backup: a failed sidecar upload succeeded"
else
  pass "backup: a failed sidecar upload fails the backup"
fi

# --- 5. the restore check -----------------------------------------------------
# Newest complete backup is 20260101T000000Z; 20260201T000000Z has no sidecar.
if run "$restore" r1; then
  pass "restore-check.sh succeeds on a good backup"
else
  fail "restore-check.sh failed on a good backup"; show r1
fi
t="$scratch/r1.trace"
if grep -q "^gcloud storage cp $bucket/$prefix/20260101T000000Z.dump " "$t" \
  && ! grep -q '20260201T000000Z.dump ' "$t"; then
  pass "restore: takes the newest backup with a sidecar, not the one without"
else
  fail "restore: picked the wrong backup"; show r1
fi
if all_as_postgres "$t"; then
  pass "restore: every database command runs as postgres"
else
  fail "restore: a database command did not run as postgres"; show r1
fi
if grep -q -E '^createdb as_postgres=1 .*-T template0 .*tc_restore_check_20260102030405' "$t" \
  && grep -q -E '^pg_restore as_postgres=1 .*--exit-on-error.*tc_restore_check_20260102030405' "$t" \
  && grep -q -E '^dropdb as_postgres=1 .*tc_restore_check_20260102030405' "$t"; then
  pass "restore: creates, restores into and drops a throwaway database"
else
  fail "restore: the throwaway database sequence is wrong"; show r1
fi
if grep -q -E '^(createdb|dropdb|pg_restore) .* tracecommons( |$)' "$t"; then
  fail "restore: touched the live database with createdb, dropdb or pg_restore"
else
  pass "restore: never creates, drops or restores into the live database"
fi
if grep -q '^restore-check: OK' "$scratch/r1.out" \
  && ! grep -q -E 'trace_submissions|trace_tenants' "$scratch/r1.out" "$scratch/r1.err"; then
  pass "restore: reports OK with no table name"
else
  fail "restore: no OK line, or a table name printed"; show r1
fi

# Drift on the live database since the dump is reported, not failed.
if run "$restore" r2 FAKE_LIVE_COUNTS="$scratch/counts-drifted"; then
  if grep -q 'drift' "$scratch/r2.out"; then
    pass "restore: live drift since the dump is reported and does not fail"
  else
    fail "restore: live drift not reported"; show r2
  fi
else
  fail "restore: failed on live drift alone"; show r2
fi

# A restored count that differs from the dump-time count fails.
if run "$restore" r3 FAKE_RESTORED_COUNTS="$scratch/counts-short"; then
  fail "restore: a row-count mismatch passed"; show r3
elif grep -q -i 'mismatch' "$scratch/r3.err" \
  && grep -q '^dropdb as_postgres=1 ' "$scratch/r3.trace"; then
  pass "restore: a row-count mismatch fails, and the throwaway database is dropped"
else
  fail "restore: mismatch not named, or the throwaway database kept"; show r3
fi

# A failed pg_restore fails, and still drops.
if run "$restore" r4 FAKE_PG_RESTORE_EXIT=1; then
  fail "restore: a failed pg_restore passed"
elif grep -q '^dropdb as_postgres=1 ' "$scratch/r4.trace"; then
  pass "restore: a failed pg_restore fails, and the throwaway database is dropped"
else
  fail "restore: a failed pg_restore left the throwaway database"; show r4
fi

# A leftover throwaway database from a killed run is dropped first.
if run "$restore" r5 FAKE_LEFTOVER_DBS=tc_restore_check_20251231000000 \
  && grep -q '^dropdb as_postgres=1 .*tc_restore_check_20251231000000' "$scratch/r5.trace"; then
  pass "restore: drops a throwaway database left by an earlier run"
else
  fail "restore: a leftover throwaway database is kept"; show r5
fi

# A dump that does not match its sidecar fails before any database exists.
echo tampered >>"$obj/20260101T000000Z.dump"
if run "$restore" r6; then
  fail "restore: a dump that fails its sha256 passed"
elif grep -q -E '^(createdb|pg_restore as_postgres=1)' "$scratch/r6.trace"; then
  fail "restore: created or restored a database from a dump that failed its sha256"
elif grep -q sha256 "$scratch/r6.err"; then
  pass "restore: a sha256 mismatch fails before any database is created"
else
  fail "restore: a sha256 mismatch failed without naming it"; show r6
fi

# No backup at all.
rm -rf "$scratch/gcs"
mkdir -p "$obj"
if run "$restore" r7; then
  fail "restore: passed with no backup in the bucket"
else
  pass "restore: fails when there is no complete backup"
fi

# --- 6. shared SQL, units, shellcheck ----------------------------------------
sql_of() { sed -n "/^COUNT_SQL=\$(cat <<'SQL'\$/,/^SQL\$/p" "$1"; }
if [ -n "$(sql_of "$backup")" ] && [ "$(sql_of "$backup")" = "$(sql_of "$restore")" ]; then
  pass "both scripts count rows with the same SQL"
else
  fail "the two scripts' COUNT_SQL blocks differ or are missing"
fi

for unit in tc-staging-db-backup.service tc-staging-db-restore-check.service; do
  f="$units/$unit"
  if [ -f "$f" ] && grep -qx 'Type=oneshot' "$f" && grep -qx 'NoNewPrivileges=true' "$f" \
    && grep -qx 'ProtectSystem=strict' "$f" && grep -qx 'ProtectHome=true' "$f" \
    && grep -qx 'PrivateTmp=true' "$f" && grep -qx 'StateDirectoryMode=0700' "$f" \
    && grep -qx 'UMask=0077' "$f" && grep -q '^ExecStart=/usr/local/sbin/' "$f"; then
    pass "$unit: oneshot, hardened, private state directory"
  else
    fail "$unit: missing or not hardened like the other units"
  fi
done
for timer in tc-staging-db-backup.timer tc-staging-db-restore-check.timer; do
  f="$units/$timer"
  if [ -f "$f" ] && grep -q '^OnCalendar=' "$f" && grep -qx 'Persistent=true' "$f"; then
    pass "$timer: scheduled and persistent"
  else
    fail "$timer: missing, unscheduled or not persistent"
  fi
done
if command -v systemd-analyze >/dev/null 2>&1; then
  # Syntax only: the ExecStart paths and postgresql.service do not exist on
  # a CI runner, so only a parse error counts.
  systemd-analyze verify --man=no "$units"/*.service "$units"/*.timer \
    >"$scratch/verify.err" 2>&1 || true
  if grep -E 'Unknown (key|section)|[Ff]ailed to parse|[Ii]nvalid|[Uu]nknown lvalue' \
    "$scratch/verify.err" >&2; then
    fail "systemd-analyze verify finds a parse error in the units"
  else
    pass "systemd-analyze verify finds no parse error in the units"
  fi
else
  echo "SKIP: systemd-analyze is not installed; units not verified"
fi

if command -v shellcheck >/dev/null 2>&1; then
  if shellcheck "$backup" "$restore" "$0"; then
    pass "shellcheck finds nothing"
  else
    fail "shellcheck"
  fi
else
  echo "SKIP: shellcheck is not installed"
fi

if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "all staging database backup checks passed"
