# Pilot operator dashboard

A read-only Grafana surface for watching the hosted pilot — submission
volume, gate-decision distribution, credit ledger movement, audit-chain
integrity, error-class rates. Operator-only; pseudonymous identifiers
only; no public-facing surface. The production community leaderboard
is a separate, opt-in surface — see
[`../superpowers/specs/2026-05-19-community-analytics-leaderboard-design.md`](../superpowers/specs/2026-05-19-community-analytics-leaderboard-design.md).

## Scope

- One pilot project (currently `tracecommons-pilot-2026`).
- Single tenant (`tenant-zaki-pilot`) at launch; queries already use the
  RLS tenant predicate so adding tenants later is a Grafana variable
  swap.
- Read-only. The dashboard never writes to the DB or to the ingest API.

## Data sources

All tables live in the Cloud SQL `trace-commons` database and are
already populated by the pilot ingest path:

| Table | What's in it | Used for panels |
|---|---|---|
| `trace_submissions` | One row per accepted/rejected submission, including `status`, `privacy_risk`, `received_at`, `auth_principal_ref` | Volume, accept rate, contributor activity |
| `trace_gate_decisions` | Per-submission gate inputs/outputs (perplexity / tail-fraction / novelty, pass/fail, policy version) | Gate distribution, false-positive watch |
| `trace_credit_ledger` | One row per credit event (`accepted`, `revoked`, `corrected`, …) with `points_delta` and `settlement_state` | Pending credits, settlement lag |
| `trace_audit_events` | Append-only, hash-chained audit log per tenant | Chain integrity, rate of admin actions |
| `trace_revocation_propagation_items` | Propagation work for revoked submissions | Revocation backlog |

The `auth_principal_ref` is a `principal_sha256:<hex>` pseudonym (see
`docs/operator/hash-only-logging.md`) — that's the only "contributor
identity" the dashboard ever shows. No raw display names, no invite
codes, no envelope bodies.

## Setup

### 1. Read-only Grafana role

Create a dedicated low-privilege Postgres role that can `SELECT` against
the tables above and nothing else.

```sql
-- Run as a high-privilege admin (via gcloud sql connect or the Cloud
-- Console SQL editor; the app role does not have CREATEROLE).
CREATE ROLE trace_commons_grafana_ro NOINHERIT LOGIN PASSWORD '...';
GRANT CONNECT ON DATABASE "trace-commons" TO trace_commons_grafana_ro;
GRANT USAGE ON SCHEMA public TO trace_commons_grafana_ro;
GRANT SELECT ON
  trace_submissions,
  trace_gate_decisions,
  trace_credit_ledger,
  trace_audit_events,
  trace_revocation_propagation_items,
  trace_tenants
TO trace_commons_grafana_ro;
```

The role still has to satisfy RLS, so panels MUST set the tenant GUC
before each query (see "Init query" below).

### 2. Run Grafana behind the Cloud SQL Auth Proxy

Easiest layout in pilot: run Grafana on the same `tc-pilot-host` and
have it talk to `127.0.0.1:5432`. The proxy is already up.

```
sudo apt-get install -y grafana
sudo systemctl enable --now grafana-server
```

Bind Grafana to `127.0.0.1:3000` and expose it through Caddy with basic
auth (or IAP), not directly on the public IP. Add a stanza to
`/etc/caddy/Caddyfile`:

```
grafana.${TC_PUBLIC_HOST} {
  encode gzip
  basic_auth {
    operator <bcrypt-hash-here>
  }
  reverse_proxy 127.0.0.1:3000
}
```

For multi-operator access, swap basic_auth for IAP or Cloud
Identity-Aware Proxy header verification.

### 3. Configure the Postgres data source

In Grafana → Data sources → PostgreSQL:

- Host: `127.0.0.1:5432`
- Database: `trace-commons`
- User: `trace_commons_grafana_ro`
- TLS/SSL mode: `disable` (the proxy is the TLS edge)
- **Init query** (critical for RLS): set the tenant GUC for every
  connection. Use a Grafana variable so the dashboard can switch
  tenants as the pilot grows.

  ```sql
  SELECT set_config('trace_commons.trace_tenant_id', '$tenant', false);
  ```

  Define `tenant` as a Grafana variable with values pulled from
  `SELECT tenant_id FROM trace_tenants ORDER BY tenant_id;`.

## Suggested panels

These are SQL skeletons, not finished Grafana JSON; paste into a panel,
adjust the time bucket / threshold to taste.

### Submission volume

```sql
SELECT
  date_trunc('hour', received_at) AS time,
  status,
  count(*) AS submissions
FROM trace_submissions
WHERE received_at >= NOW() - INTERVAL '7 days'
GROUP BY 1, 2
ORDER BY 1;
```

Stacked time series: `accepted` / `quarantined` / `rejected` /
`revoked` per hour.

### Accept rate (rolling 24h)

```sql
SELECT
  100.0 * count(*) FILTER (WHERE status = 'accepted')::float
  / NULLIF(count(*), 0) AS accept_rate_pct
FROM trace_submissions
WHERE received_at >= NOW() - INTERVAL '24 hours';
```

Single-stat panel. Alert if it drops below an operator-set threshold
(e.g. 50%) — usually means the gate is over-rejecting.

### Active contributors

```sql
SELECT
  auth_principal_ref,
  count(*) AS submissions,
  max(received_at) AS last_seen
FROM trace_submissions
WHERE received_at >= NOW() - INTERVAL '7 days'
GROUP BY 1
ORDER BY submissions DESC
LIMIT 50;
```

Table with `principal_sha256:...` rows. Operator can correlate to a
contributor via the workload-token-mint log they keep separately.

### Gate decision distribution

```sql
SELECT
  date_trunc('hour', decided_at) AS time,
  CASE
    WHEN perplexity_passed AND novelty_passed THEN 'both_passed'
    WHEN perplexity_passed AND NOT novelty_passed THEN 'novelty_failed'
    WHEN NOT perplexity_passed AND novelty_passed THEN 'perplexity_failed'
    ELSE 'both_failed'
  END AS gate_outcome,
  count(*) AS decisions
FROM trace_gate_decisions
WHERE decided_at >= NOW() - INTERVAL '7 days'
GROUP BY 1, 2
ORDER BY 1;
```

Stacked time series. Confirms the calibration story: at the pilot
launch floors (`PERPLEXITY=0`, `TAIL_FRACTION=0`, `NOVELTY=500000`)
`perplexity_passed` should be ~100% and the action is in
`novelty_passed`.

### Novelty score distribution

```sql
SELECT
  width_bucket(novelty_score_micros, 0, 1000000, 20) AS bucket,
  count(*) AS decisions
FROM trace_gate_decisions
WHERE decided_at >= NOW() - INTERVAL '7 days'
GROUP BY 1
ORDER BY 1;
```

Histogram. Watch the shape near the configured floor; if traffic
clusters just above the cutoff, calibration headroom is too tight.

### Credit ledger throughput

```sql
SELECT
  date_trunc('hour', created_at) AS time,
  event_type,
  sum(points_delta::numeric) AS delta
FROM trace_credit_ledger
WHERE created_at >= NOW() - INTERVAL '7 days'
GROUP BY 1, 2
ORDER BY 1;
```

Stacked bars by `event_type` (`accepted`, `revoked`, `corrected`,
`worker_utility`, …). Pending-vs-settled split:

```sql
SELECT
  settlement_state,
  sum(points_delta::numeric) AS delta
FROM trace_credit_ledger
GROUP BY 1;
```

### Settlement lag

```sql
SELECT
  date_trunc('hour', created_at) AS time,
  avg(EXTRACT(EPOCH FROM (settled_at - created_at)) / 60.0)
    FILTER (WHERE settled_at IS NOT NULL) AS avg_lag_minutes
FROM trace_credit_ledger
WHERE created_at >= NOW() - INTERVAL '7 days'
GROUP BY 1
ORDER BY 1;
```

Surfaces stuck settlement workers.

### Audit-chain integrity (canary)

```sql
SELECT
  count(*) AS audit_events,
  bool_and(audit_sequence = expected_seq) AS chain_intact
FROM (
  SELECT
    audit_sequence,
    row_number() OVER (ORDER BY audit_sequence) AS expected_seq
  FROM trace_audit_events
  WHERE tenant_id = current_setting('trace_commons.trace_tenant_id')
) t;
```

Single-stat panel; `chain_intact = true` and `audit_events` strictly
increasing over time. Alert if `chain_intact` flips to `false`.

### Recent admin actions

```sql
SELECT
  audit_sequence,
  action,
  actor_role,
  actor_principal_ref,
  reason,
  created_at
FROM trace_audit_events
WHERE created_at >= NOW() - INTERVAL '24 hours'
  AND action IN ('revoke', 'review_decision', 'quarantine_release',
                 'retention_purge', 'key_rotate')
ORDER BY audit_sequence DESC
LIMIT 50;
```

Table panel — operator sanity check that no off-policy admin action
slipped in.

### Error-class rates (hash-only)

The ingest binary logs error-class hashes via `safe_*_error_hash`.
Stand up a log aggregator (Cloud Logging works) and a panel that
groups `Trace Commons ingestion operation failed` events by
`error_hash`. Spikes on a single hash mean a single failure class is
recurring — the operator chases it via
`docs/operator/hash-only-logging.md` and
`docs/operator/troubleshooting.md`.

## What this dashboard does NOT show

Intentional gaps; do not "fix" them without a privacy review:

- **No raw envelope content.** The dashboard never selects from
  `trace_object_refs` or pulls bytes from GCS. Envelope inspection is
  a reviewer workflow, not an operator monitoring one.
- **No raw display names, invite codes, or workload-token contents.**
  Operator joins those externally if needed.
- **No cross-tenant aggregation.** Pilot is single-tenant. Once more
  tenants land, cross-tenant aggregates need the analytics-min-cell
  and noise guards already in the binary
  (`TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT`,
  `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE`); the pilot dashboard
  filters to one tenant via the `$tenant` variable and stays out of
  that surface entirely.
- **No leaderboard.** Ranking contributors by accepted-count for
  public display is a separate, opt-in surface; see the design spec
  linked at the top.

## Maintenance

- New panels: add SQL here first, then in Grafana. Keep the SQL in
  this file as the canonical source.
- Schema drift: when a migration changes column names, update the
  panels in the same PR.
- Operator-secret material in init queries: never. The `$tenant`
  variable is a tenant id, not a credential.
