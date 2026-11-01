#!/usr/bin/env bash
set +x
# The staging database's nightly backup (staging design 3.2). Run on the
# staging VM as root, by tc-staging-db-backup.service:
#
#   /usr/local/sbin/tc-staging-db-backup
#
# Writes three objects to the private staging bucket, under backups/postgres/:
#
#   <UTC timestamp>.dump         pg_dump -Fc of the database
#   <UTC timestamp>.counts       every table's row count, taken in the same
#                                snapshot the dump was taken in
#   <UTC timestamp>.dump.sha256  sha256sum lines for the two above, uploaded
#                                last: a backup without it is incomplete,
#                                and restore-check.sh never reads one
#
# Every Trace Commons table has FORCE ROW LEVEL SECURITY, which binds the
# table owner too, so neither `app` nor any role without BYPASSRLS can read
# every row: pg_dump as `app` refuses ("query would be affected by row-level
# security policy"), and with --enable-row-security it would dump only the
# rows that role can see. Everything here that reaches PostgreSQL therefore
# runs as the local `postgres` superuser over the Unix socket (peer auth),
# through `runuser -u postgres --` (no setuid, so it works under the unit's
# NoNewPrivileges), and the script refuses a session that is not a superuser.
#
# The counts and the dump are taken in one snapshot: a REPEATABLE READ
# session exports its snapshot, pg_dump --snapshot dumps exactly that, and
# the same session counts every table. restore-check.sh compares a restore
# with these counts, so a dump missing rows fails the check however busy the
# live database has been since.
#
# No client-side encryption: the bucket is private, reachable only by the
# staging runtime service account, and encrypts at rest under its CMEK
# (tc-staging/kek-v1). A second key would need its own secret and its own
# restore path, and the design asks for neither.
#
# The local files live in a root-only state directory, mode 0600, and are
# removed on exit whatever happened. Each upload refuses to overwrite an
# existing object (--if-generation-match=0). Only labels, sizes and hashes
# are printed: no table name, no count, no row.
#
# Settings (environment):
#   TC_DB_BACKUP_BUCKET     default gs://tc-staging-artifacts-20261021
#   TC_DB_BACKUP_PREFIX     default backups/postgres
#   TC_DB_BACKUP_DATABASE   default tracecommons
#   TC_DB_BACKUP_STATE_DIR  default /var/lib/tracecommons-db-backup (the
#                           unit's StateDirectory)
#   CLOUDSDK_CONFIG         default <state dir>/gcloud
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
  echo "backup-db: refusing: $*" >&2
  exit 1
}

if [ "${TC_DB_BACKUP_SKIP_ROOT_CHECK:-0}" != 1 ] && [ "$(id -u)" != 0 ]; then
  refuse "run as root; the dump is written to a root-only directory"
fi
[[ "$bucket" =~ ^gs://[a-z0-9._-]+$ ]] || refuse "TC_DB_BACKUP_BUCKET is not gs://<bucket>"
[[ "$prefix" =~ ^[a-z0-9/_-]+$ ]] || refuse "TC_DB_BACKUP_PREFIX is not a plain prefix"
[[ "$db" =~ ^[a-z0-9_]+$ ]] || refuse "TC_DB_BACKUP_DATABASE is not a plain name"
for tool in runuser psql pg_dump pg_restore gcloud sha256sum mkfifo; do
  command -v "$tool" >/dev/null || refuse "$tool is not installed"
done

as_postgres() {
  runuser -u postgres -- "$@"
}

# Every ordinary table outside the system schemas, extension members aside
# (pg_dump does not dump their rows), one `<schema>.<table> <count>` line
# each, in a fixed order. restore-check.sh holds the same text; the CI test
# requires the two to match.
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

install -d -m 0700 "$state"
chmod 0700 "$state"
work="$(mktemp -d "$state/backup.XXXXXX")"
psql_pid=""
cleanup() {
  exec 3>&- 4<&-
  if [ -n "$psql_pid" ]; then
    kill "$psql_pid" 2>/dev/null || true
    wait "$psql_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

ts="$(date -u +%Y%m%dT%H%M%SZ)"
[[ "$ts" =~ ^[0-9]{8}T[0-9]{6}Z$ ]] || refuse "the clock gave an unusable timestamp"
dump="$work/$ts.dump"
counts="$work/$ts.counts"
sidecar="$work/$ts.dump.sha256"
dest="$bucket/$prefix"

# One psql session held open over two FIFOs, so the snapshot it exports
# stays valid while pg_dump runs. The redirections are opened by this (root)
# shell, so psql, running as postgres, needs no access to the directory.
mkfifo -m 0600 "$work/q" "$work/a"
as_postgres psql -X -q -A -t -v ON_ERROR_STOP=1 -d "$db" <"$work/q" >"$work/a" &
psql_pid=$!
exec 3>"$work/q" 4<"$work/a"

ask() {
  # $1: SQL whose answer is one line. Empty on a closed session.
  local answer=""
  printf '%s\n' "$1" >&3
  IFS= read -r -t 600 -u 4 answer || refuse "the psql session ended or timed out"
  printf '%s' "$answer"
}

printf '%s\n' "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;" >&3
superuser="$(ask "SELECT current_setting('is_superuser');")"
[ "$superuser" = on ] \
  || refuse "the session is not a superuser; FORCE ROW LEVEL SECURITY would hide rows"
snapshot="$(ask "SELECT pg_export_snapshot();")"
[[ "$snapshot" =~ ^[0-9A-F-]+$ ]] || refuse "no exported snapshot"

echo "backup-db: dumping $db at $ts"
as_postgres pg_dump -Fc --snapshot="$snapshot" -d "$db" >"$dump"

{
  printf '%s\n' "$COUNT_SQL"
  printf '%s\n' '\gexec'
  printf '%s\n' "SELECT 'tc-backup-end';"
} >&3
: >"$counts"
while :; do
  line=""
  IFS= read -r -t 3600 -u 4 line || refuse "the psql session ended while counting"
  [ "$line" = tc-backup-end ] && break
  [[ "$line" =~ ^.+\ [0-9]+$ ]] || refuse "an unreadable row-count line"
  printf '%s\n' "$line" >>"$counts"
done
printf '%s\n' "COMMIT;" '\q' >&3
exec 3>&- 4<&-
wait "$psql_pid" || refuse "the psql session failed"
psql_pid=""

tables="$(wc -l <"$counts" | tr -d ' ')"
[ "$tables" -gt 0 ] || refuse "no tables counted"
# A well-formed archive, read on stdin, connecting to nothing.
pg_restore --list <"$dump" >/dev/null || refuse "pg_dump wrote an unreadable archive"

(cd "$work" && sha256sum "$ts.dump" "$ts.counts") >"$sidecar"
dump_sha="$(awk 'NR == 1 { print $1 }' "$sidecar")"
dump_bytes="$(wc -c <"$dump" | tr -d ' ')"

# The sidecar goes last: until it is there, the backup is incomplete.
for f in "$dump" "$counts" "$sidecar"; do
  gcloud storage cp --if-generation-match=0 "$f" "$dest/$(basename "$f")" \
    || refuse "upload of $(basename "$f") failed"
done
echo "backup-db: uploaded $dest/$ts.dump bytes=$dump_bytes tables=$tables sha256=$dump_sha"
