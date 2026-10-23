#!/usr/bin/env bash
set +x
# The staging database backup's restore check (staging design 3.2). Run on
# the staging VM as root, weekly, by tc-staging-db-restore-check.service:
#
#   /usr/local/sbin/tc-staging-db-restore-check
#
# Takes the newest complete backup written by backup-db.sh (the newest
# <timestamp>.dump.sha256 under backups/postgres/; a dump without its
# sidecar is incomplete and never read), and:
#
#   1. checks the sha256 of the dump and the counts against the sidecar,
#      before any database exists;
#   2. restores the dump into a throwaway database, tc_restore_check_<UTC
#      time>, created from template0 in the same cluster (so the roles the
#      dump names exist);
#   3. counts every table's rows there, as the postgres superuser, and
#      requires each count to equal the count taken in the dump's own
#      snapshot (the .counts object). Any difference, a missing table or an
#      extra one, fails;
#   4. reports, without failing, how many tables' counts on the live database
#      now differ from the restore: the live database keeps taking writes
#      after the dump, so that is drift, not a fault;
#   5. drops the throwaway database, whatever happened.
#
# Everything that reaches PostgreSQL runs as `postgres` through
# `runuser -u postgres --` (backup-db.sh says why), and the live session must
# be a superuser. Only labels, sizes and hashes are printed on success; a
# count mismatch names the tables that differ.
#
# Settings (environment): TC_DB_BACKUP_BUCKET, TC_DB_BACKUP_PREFIX,
# TC_DB_BACKUP_DATABASE (the live database), TC_DB_BACKUP_STATE_DIR and
# CLOUDSDK_CONFIG, with backup-db.sh's defaults.
#
# TC_DB_BACKUP_SKIP_ROOT_CHECK=1 exists for
# scripts/ci/test-staging-db-backup.sh, which runs this against stubs.
set -euo pipefail
umask 077

bucket="${TC_DB_BACKUP_BUCKET:-gs://tc-staging-artifacts-20261021}"
prefix="${TC_DB_BACKUP_PREFIX:-backups/postgres}"
db="${TC_DB_BACKUP_DATABASE:-tracecommons}"
state="${TC_DB_BACKUP_STATE_DIR:-/var/lib/tracecommons-db-backup}"
export CLOUDSDK_CONFIG="${CLOUDSDK_CONFIG:-$state/gcloud}"

refuse() {
  echo "restore-check: refusing: $*" >&2
  exit 1
}

if [ "${TC_DB_BACKUP_SKIP_ROOT_CHECK:-0}" != 1 ] && [ "$(id -u)" != 0 ]; then
  refuse "run as root; the dump is fetched to a root-only directory"
fi
[[ "$bucket" =~ ^gs://[a-z0-9._-]+$ ]] || refuse "TC_DB_BACKUP_BUCKET is not gs://<bucket>"
[[ "$prefix" =~ ^[a-z0-9/_-]+$ ]] || refuse "TC_DB_BACKUP_PREFIX is not a plain prefix"
[[ "$db" =~ ^[a-z0-9_]+$ ]] || refuse "TC_DB_BACKUP_DATABASE is not a plain name"
for tool in runuser psql pg_restore createdb dropdb gcloud sha256sum; do
  command -v "$tool" >/dev/null || refuse "$tool is not installed"
done

as_postgres() {
  runuser -u postgres -- "$@"
}

# The same text as in backup-db.sh; the CI test requires the two to match.
COUNT_SQL=$(cat <<'SQL'
SELECT format('SELECT %L || '' '' || count(*) FROM %I.%I',
              n.nspname || '.' || c.relname, n.nspname, c.relname)
  FROM pg_catalog.pg_class c
  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
 WHERE c.relkind = 'r'
   AND n.nspname <> 'information_schema'
   AND n.nspname !~ '^pg_'
   AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d
                    WHERE d.classid = 'pg_catalog.pg_class'::regclass
                      AND d.objid = c.oid AND d.deptype = 'e')
 ORDER BY n.nspname, c.relname
SQL
)

# $1: database. Every table's `<schema>.<table> <count>` line.
count_rows() {
  { printf '%s\n' "$COUNT_SQL"; printf '%s\n' '\gexec'; } \
    | as_postgres psql -X -q -A -t -v ON_ERROR_STOP=1 -d "$1"
}

# $1: database; $2: SQL whose answer is one line.
one_value() {
  printf '%s\n' "$2" | as_postgres psql -X -q -A -t -v ON_ERROR_STOP=1 -d "$1"
}

throwaway_pattern='^tc_restore_check_[0-9]{14}$'
target=""
install -d -m 0700 "$state"
chmod 0700 "$state"
work="$(mktemp -d "$state/restore.XXXXXX")"
cleanup() {
  local status=$?
  if [ -n "$target" ]; then
    if ! as_postgres dropdb --if-exists "$target"; then
      echo "restore-check: could not drop $target; drop it by hand" >&2
      status=1
    fi
  fi
  rm -rf "$work"
  exit "$status"
}
trap cleanup EXIT

# The newest complete backup.
dest="$bucket/$prefix"
# An empty prefix is "matched no objects"; any other gcloud failure (auth,
# permission, a broken gcloud) is named as such, not as a missing backup.
if ! listing="$(gcloud storage ls "$dest/" 2>"$work/ls.err")"; then
  if ! grep -q 'matched no objects' "$work/ls.err"; then
    cat "$work/ls.err" >&2
    refuse "gcloud storage ls of $dest/ failed"
  fi
  listing=""
fi
latest="$(printf '%s\n' "$listing" \
  | grep -E "^$dest/[0-9]{8}T[0-9]{6}Z\.dump\.sha256\$" | sort | tail -n 1 || true)"
[ -n "$latest" ] || refuse "no complete backup (no .dump.sha256) under $dest/"
ts="$(basename "$latest" .dump.sha256)"
for name in "$ts.dump.sha256" "$ts.dump" "$ts.counts"; do
  gcloud storage cp "$dest/$name" "$work/$name" || refuse "could not fetch $name"
done

# 1. The hashes, before any database exists. The sidecar must list exactly
# the dump and the counts.
listed="$(awk '{ print $2 }' "$work/$ts.dump.sha256" | tr '\n' ' ')"
[ "$listed" = "$ts.dump $ts.counts " ] || refuse "the sidecar for $ts does not list the dump and the counts"
(cd "$work" && sha256sum -c "$ts.dump.sha256" >/dev/null 2>&1) \
  || refuse "the backup $ts fails its sha256"
dump_sha="$(awk 'NR == 1 { print $1 }' "$work/$ts.dump.sha256")"
dump_bytes="$(wc -c <"$work/$ts.dump" | tr -d ' ')"
tables="$(wc -l <"$work/$ts.counts" | tr -d ' ')"
[ "$tables" -gt 0 ] || refuse "the backup $ts counted no tables"
echo "restore-check: backup $ts bytes=$dump_bytes tables=$tables sha256=$dump_sha verified"

[ "$(one_value "$db" "SELECT current_setting('is_superuser');")" = on ] \
  || refuse "the session is not a superuser; FORCE ROW LEVEL SECURITY would hide rows"

# A throwaway database left by a run that was killed before its cleanup.
leftovers="$(one_value postgres \
  "SELECT datname FROM pg_database WHERE datname ~ '^tc_restore_check_[0-9]{14}\$' ORDER BY 1;")"
for old in $leftovers; do
  [[ "$old" =~ $throwaway_pattern ]] || continue
  echo "restore-check: dropping $old, left by an earlier run"
  as_postgres dropdb --if-exists "$old"
done

# 2. Restore into a throwaway database. Never the live one.
candidate="tc_restore_check_$(date -u +%Y%m%d%H%M%S)"
[[ "$candidate" =~ $throwaway_pattern ]] || refuse "the clock gave an unusable database name"
[ "$candidate" != "$db" ] || refuse "the throwaway database would be the live one"
as_postgres createdb -T template0 "$candidate"
target="$candidate"
as_postgres pg_restore --exit-on-error --no-password -d "$target" <"$work/$ts.dump" \
  || refuse "pg_restore of $ts failed"

# 3. Every table's rows, against the counts taken in the dump's snapshot.
count_rows "$target" >"$work/restored.counts"
if ! cmp -s "$work/restored.counts" "$work/$ts.counts"; then
  echo "restore-check: row-count mismatch between the restore and the counts at dump time:" >&2
  diff "$work/$ts.counts" "$work/restored.counts" | grep -E '^[<>]' >&2 || true
  refuse "row-count mismatch in the backup $ts"
fi
rows="$(awk '{ s += $NF } END { print s + 0 }' "$work/restored.counts")"

# 4. Drift on the live database since the dump: reported, never failed.
count_rows "$db" >"$work/live.counts"
sort "$work/restored.counts" >"$work/restored.sorted"
sort "$work/live.counts" >"$work/live.sorted"
drift="$(comm -13 "$work/restored.sorted" "$work/live.sorted" | wc -l | tr -d ' ')"
echo "restore-check: drift since the dump: $drift table(s) on the live database differ from the restore (informational)"

echo "restore-check: OK $ts tables=$tables rows=$rows sha256=$dump_sha"
