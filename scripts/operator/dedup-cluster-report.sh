#!/usr/bin/env bash
#
# Read-only report on the cross-trace dedup clustering in
# `trace_gate_decisions`, run as the narrow `trace_gate_driver` role and
# touching only the columns V45/V57 grant it. Prints COUNTS ONLY: no ids,
# no simhash values, no tenant references. Its output is the table in
# docs/superpowers/specs/2026-09-21-dedup-simhash-recluster-design.md, so a
# "before" and an "after" are directly comparable.
#
# Run before the re-derivation pass, after its write mode, and after the
# constant flip (docs/operator/dedup-recluster.md).
#
# Flags:
#   --db-url=<postgres URL for the trace_gate_driver role>
#       Defaults to $TRACE_COMMONS_GATE_DRIVER_DATABASE_URL.
#
# Requires PostgreSQL 14+ for bit_count().

set -euo pipefail

DB_URL="${TRACE_COMMONS_GATE_DRIVER_DATABASE_URL:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --db-url=*) DB_URL="${1#*=}"; shift ;;
    *) echo "DedupClusterReportUnknownArg: $1" >&2; exit 1 ;;
  esac
done

bail() { echo "DedupClusterReportFailure: $1" >&2; exit 1; }

[ -n "$DB_URL" ] || bail "db_url_unset"
command -v psql >/dev/null 2>&1 || bail "psql_missing"

# Belt and braces: the role holds no write grants, and the session is
# read-only regardless.
export PGOPTIONS="-c default_transaction_read_only=on"

run_sql() {
  psql "$DB_URL" -X --no-psqlrc -v ON_ERROR_STOP=1 -q "$@"
}

echo "== role =="
run_sql -c "SELECT current_user AS role, now() AT TIME ZONE 'UTC' AS measured_at_utc;"

echo "== clusters =="
run_sql <<'SQL'
WITH sizes AS (
    SELECT dedup_cluster_id, count(*) AS n
    FROM trace_gate_decisions
    WHERE dedup_cluster_id IS NOT NULL
    GROUP BY dedup_cluster_id
),
clustered AS (
    SELECT count(*) AS rows_with_cluster
    FROM trace_gate_decisions
    WHERE dedup_cluster_id IS NOT NULL
)
SELECT
    (SELECT rows_with_cluster FROM clustered)          AS rows_with_cluster,
    count(*)                                            AS clusters,
    count(*) FILTER (WHERE n = 1)                       AS singletons,
    count(*) FILTER (WHERE n BETWEEN 2 AND 9)           AS size_2_9,
    count(*) FILTER (WHERE n BETWEEN 10 AND 99)         AS size_10_99,
    count(*) FILTER (WHERE n >= 100)                    AS size_100_plus,
    coalesce(max(n), 0)                                 AS largest_cluster_size,
    round(100.0 * coalesce(max(n), 0)
          / nullif((SELECT rows_with_cluster FROM clustered), 0), 1)
                                                        AS largest_cluster_share_percent
FROM sizes;
SQL

echo "== simhash values =="
run_sql <<'SQL'
SELECT
    count(*)                                            AS decision_rows,
    count(dedup_simhash)                                AS rows_with_simhash,
    count(DISTINCT dedup_simhash)                       AS distinct_simhashes,
    count(*) FILTER (WHERE dedup_simhash = 0)           AS zero_simhashes
FROM trace_gate_decisions;
SQL

echo "== rows per dedup_signal_version (NULL shown as the legacy label) =="
run_sql <<'SQL'
SELECT
    coalesce(nullif(dedup_signal_version, ''),
             'events.v1+fnv1a-2shingle.v1 (stored NULL, read as legacy)')
                                                        AS dedup_signal_version,
    count(*)                                            AS rows
FROM trace_gate_decisions
GROUP BY 1
ORDER BY rows DESC, 1;
SQL

echo "== Hamming distance from each member to its cluster's earliest-decided member =="
run_sql <<'SQL'
WITH reps AS (
    SELECT DISTINCT ON (dedup_cluster_id)
           dedup_cluster_id, dedup_simhash AS rep
    FROM trace_gate_decisions
    WHERE dedup_cluster_id IS NOT NULL AND dedup_simhash IS NOT NULL
    ORDER BY dedup_cluster_id, decided_at ASC, decision_id ASC
),
dist AS (
    SELECT bit_count((d.dedup_simhash # r.rep)::bit(64)) AS h
    FROM trace_gate_decisions d
    JOIN reps r USING (dedup_cluster_id)
    WHERE d.dedup_simhash IS NOT NULL
)
SELECT
    count(*)                                            AS members,
    min(h)                                              AS min,
    round((percentile_cont(0.5) WITHIN GROUP (ORDER BY h))::numeric, 1)
                                                        AS median,
    round((percentile_cont(0.95) WITHIN GROUP (ORDER BY h))::numeric, 1)
                                                        AS p95,
    max(h)                                              AS max
FROM dist;
SQL

echo "== nearest-member Hamming distance inside the largest cluster =="
run_sql <<'SQL'
WITH largest AS (
    SELECT dedup_cluster_id
    FROM trace_gate_decisions
    WHERE dedup_cluster_id IS NOT NULL
    GROUP BY dedup_cluster_id
    ORDER BY count(*) DESC, dedup_cluster_id
    LIMIT 1
),
members AS (
    SELECT decision_id, dedup_simhash
    FROM trace_gate_decisions
    WHERE dedup_cluster_id = (SELECT dedup_cluster_id FROM largest)
      AND dedup_simhash IS NOT NULL
),
nearest AS (
    SELECT a.decision_id,
           min(bit_count((a.dedup_simhash # b.dedup_simhash)::bit(64))) AS h
    FROM members a
    JOIN members b ON a.decision_id <> b.decision_id
    GROUP BY a.decision_id
)
SELECT
    (SELECT count(*) FROM members)                      AS largest_cluster_members,
    round((percentile_cont(0.5) WITHIN GROUP (ORDER BY h))::numeric, 1)
                                                        AS median_nearest_member_hamming
FROM nearest;
SQL
