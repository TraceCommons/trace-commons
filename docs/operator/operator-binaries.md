# Operator Binaries — `trace-commons-{review,admin,worker,tenant}`

Four CLI binaries ship with `trace-commons-server` for operator-side workflows
against a deployed hosted ingest. Each binary speaks the same JSON envelope
over the same `trace_commons_operator_client::Client` (host-allowlisted,
never logs bearer tokens or query strings), but is audience-scoped and gated
by a different bearer-token surface. Together they replace the slice of the
old Ironclaw `ironclaw traces …` surface that operators reach for during
review, admin, worker, and tenant-management work.

This runbook is intended to be readable end-to-end in under ten minutes.

## Binaries at a glance

| Binary | Audience | Default bearer env | Subcommands |
|---|---|---|---|
| `trace-commons-review` | Reviewers | `TRACE_COMMONS_REVIEWER_BEARER` | 8 |
| `trace-commons-admin` | Hosted-service admins | `TRACE_COMMONS_ADMIN_BEARER` | 12 |
| `trace-commons-worker` | Per-route worker operators | one per subcommand (see below) | 8 |
| `trace-commons-tenant` | Tenant-policy operators | `TRACE_COMMONS_TENANT_BEARER` | 12 |

Each binary takes:

- `--endpoint <URL>` (or `TRACE_COMMONS_ENDPOINT`) — hosted ingest base URL.
- `--bearer-token-env <NAME>` — env var holding the bearer token (the
  binary reads the value at call time; the env *name* travels on the CLI
  but the *value* never appears in logs).
- `--allowed-hosts <CSV>` (or `TRACE_COMMONS_ALLOWED_HOSTS`) — defense-in-depth
  host allowlist. Empty = permissive; populated = the foundation client
  refuses any URL whose host is not in the list.
- `--json` — emit a sanitized request envelope plus the raw server JSON
  instead of a human-readable summary.

## Install

```bash
cargo build --release \
  --bin trace-commons-review \
  --bin trace-commons-admin \
  --bin trace-commons-worker \
  --bin trace-commons-tenant
```

The resulting binaries live in `target/release/`. There are no required
features; the binaries are buildable in hermetic CI.

## Env-var matrix

Each row lists a single ingest-server route gate. The binary subcommands
that target that gate read the bearer from the listed env var by default;
an operator can override via `--bearer-token-env <CUSTOM_NAME>` if they
keep their tokens under non-default names.

| Bearer env (default) | Gate (server side) | Consumed by |
|---|---|---|
| `TRACE_COMMONS_REVIEWER_BEARER` | reviewer role | `trace-commons-review` (all 8 subcommands) |
| `TRACE_COMMONS_ADMIN_BEARER` | admin role | `trace-commons-admin` (all 12 subcommands) |
| `TRACE_COMMONS_TENANT_BEARER` | admin role (tenant ops use the same gate) | `trace-commons-tenant` (10 server-backed subcommands) |
| `TRACE_COMMONS_UTILITY_CREDIT_WORKER_BEARER` | utility-credit worker | `trace-commons-worker worker-utility-credit` |
| `TRACE_COMMONS_RETENTION_WORKER_BEARER` | retention worker | `trace-commons-worker worker-retention-maintenance` |
| `TRACE_COMMONS_VECTOR_WORKER_BEARER` | vector-index worker | `trace-commons-worker worker-vector-index` |
| `TRACE_COMMONS_BENCHMARK_WORKER_BEARER` | benchmark-conversion worker | `trace-commons-worker worker-benchmark-convert` |
| `TRACE_COMMONS_EXPORT_WORKER_BEARER` | export worker | `trace-commons-worker worker-replay-dataset-export` |
| `TRACE_COMMONS_RANKER_WORKER_BEARER` | export worker (shared with ranker training routes) | `trace-commons-worker worker-ranker-training-candidates`, `worker-ranker-training-pairs` |
| `TRACE_COMMONS_PROCESS_EVALUATION_WORKER_BEARER` | process-evaluation worker | `trace-commons-worker process-evaluation-submit` |

The ingest server enforces these gates through per-role tokens carried in
`TRACE_COMMONS_TENANT_TOKENS`; on the operator side the per-route env names
are intentionally distinct so an operator cannot accidentally reuse a
broader-scoped token where a narrower one is sufficient.

Two `trace-commons-tenant` subcommands do not hit the server and so do not
consume a bearer:

- `tenant-principal-ref` — derives the stored `principal_ref` locally from
  a static token, signed-claim `(tenant_id, actor_ref)` pair, or onboarding
  device-key `(tenant_id, device_key_id)` pair.
- `privacy-filter-canary` — spawns the locally configured privacy-filter
  sidecar, pipes a canary string through it, and verifies no canary token
  survives the redaction. The subcommand reads its configuration from
  environment variables:

  | Canonical | Legacy fallback | Purpose |
  |---|---|---|
  | `TRACE_COMMONS_PRIVACY_FILTER_COMMAND` | `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND` | Sidecar executable path. Required. |
  | `TRACE_COMMONS_PRIVACY_FILTER_ARGS` | `IRONCLAW_TRACE_PRIVACY_FILTER_ARGS` | Whitespace-separated argv. |
  | `TRACE_COMMONS_PRIVACY_FILTER_TIMEOUT_MS` | `IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS` | Wall-clock timeout (ms). |
  | `TRACE_COMMONS_PRIVACY_FILTER_MAX_INPUT_BYTES` | `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES` | Refuse oversized inputs. |
  | `TRACE_COMMONS_PRIVACY_FILTER_MAX_STDOUT_BYTES` | `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES` | Cap captured stdout. |
  | `TRACE_COMMONS_PRIVACY_FILTER_MAX_STDERR_BYTES` | `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES` | Cap captured stderr. |

  When both names are set, the canonical `TRACE_COMMONS_*` value wins; a
  one-shot `tracing::warn!` is emitted whenever the legacy name is used so
  operators know to migrate. The subcommand exits non-zero if no command is
  configured, the sidecar times out, or the redacted output retains a
  canary token (hash-only diagnostics, never the raw token).

## Common workflows

### Reviewer: claim → decide → release

```bash
export TRACE_COMMONS_ENDPOINT=https://trace-commons.example.com
export TRACE_COMMONS_REVIEWER_BEARER=...

# 1. See what's open.
trace-commons-review quarantine-list --lease-filter available

# 2. Claim the next one (or a specific submission_id).
trace-commons-review review-lease-claim-next --privacy-risk medium

# 3. Inspect, then approve or reject.
trace-commons-review review-decision \
  --submission-id <UUID> --decision approve \
  --reason "tools verified, side effects OK"

# 4. If you need to walk away, release the lease.
trace-commons-review review-lease-release --submission-id <UUID>
```

### Retention maintenance: admin vs worker

Two paths target similar effects but enforce different roles:

- `trace-commons-admin maintenance-run` exercises the admin gate at
  `/v1/admin/maintenance` and supports the full set of toggles
  (`--backfill-db-mirror`, `--reconcile-db-mirror`, `--verify-audit-chain`,
  `--index-vectors`, in addition to the retention flags).
- `trace-commons-worker worker-retention-maintenance` exercises the
  retention-worker gate at `/v1/workers/retention-maintenance` and is
  scoped to retention behaviors only.

Use the admin entry point for cross-cutting maintenance runs that mix
retention with mirror reconciliation or audit-chain verification; use the
worker entry point for scheduled retention sweeps from a service account
that should not hold admin scope.

### Tenant rotation: policy + access grants

```bash
export TRACE_COMMONS_ENDPOINT=https://trace-commons.example.com
export TRACE_COMMONS_TENANT_BEARER=...

# Inspect the current policy.
trace-commons-tenant tenant-policy-get --json

# Roll forward to a new policy version.
trace-commons-tenant tenant-policy-set \
  --policy-version 2026-05-19-pilot \
  --allowed-consent-scopes benchmark-only,model-training \
  --allowed-uses benchmark-generation,model-training

# Derive the principal_ref for a new reviewer's static token.
trace-commons-tenant tenant-principal-ref --token-env REVIEWER_BOB_TOKEN

# Derive the principal_ref for an onboarded Ironclaw device key.
trace-commons-tenant tenant-principal-ref \
  --device-tenant-id tenant-zaki-pilot \
  --device-key-id sha256:<64-hex-device-key-id>

# Grant it. The grant_id is server-allocated unless --grant-id is passed.
trace-commons-tenant tenant-access-grant-create \
  --principal-ref principal_sha256:... \
  --role reviewer \
  --allowed-consent-scopes benchmark-only,model-training \
  --allowed-uses benchmark-generation,model-training \
  --reason "rotate reviewer bob into the pilot"

# Revoke an old grant.
trace-commons-tenant tenant-access-grant-revoke \
  --grant-id <UUID> --reason "key compromise rotation"
```

### Benchmark conversion: convert then lifecycle

```bash
export TRACE_COMMONS_ADMIN_BEARER=...

# Kick off a conversion run for approved replay-eligible traces.
trace-commons-admin benchmark-convert \
  --purpose "2026-05 pilot weekly" \
  --consent-scope benchmark-only \
  --status accepted \
  --limit 50

# Once the registry and evaluator have run, record their outcomes.
trace-commons-admin benchmark-lifecycle-update \
  --conversion-id <UUID> \
  --registry-status published \
  --evaluation-status passed \
  --score 0.87 \
  --reason "weekly registry run"
```

## Defense-in-depth: host allowlist and JSON envelopes

`TRACE_COMMONS_ALLOWED_HOSTS` is a CSV of hostnames the foundation client
will accept; any other host raises `Error::HostNotAllowed` before the
request is dispatched. Use it whenever the operator host has any chance of
running against the wrong endpoint (CI, dev, etc.).

`--json` emits a two-part envelope per request:

```json
{
  "request": {"method": "GET", "url": "https://…/v1/audit/events?limit=25"},
  "data":    {"items": [ … ]}
}
```

The `url` field is sanitized — query-string values get stripped to
keys-only before logging. Bearer tokens never appear in the envelope; they
travel only as an `Authorization` header and are never echoed.

## Troubleshooting

The foundation client returns one of the following error variants. The
binary surfaces each as `Error: <variant-name>: <message>`.

| Variant | Typical cause | Fix |
|---|---|---|
| `bearer-missing` | The env var named by `--bearer-token-env` is unset or empty. | Export the env var; double-check you're using the per-route default for `trace-commons-worker`. |
| `host-not-allowed` | The endpoint hostname is not in `TRACE_COMMONS_ALLOWED_HOSTS`. | Add the host to the CSV, or unset the env to go permissive. Never paste tokens into a permissive shell. |
| `server-label:<class>` | The server returned a non-2xx with a hash-only error class. | Cross-reference the class against [`./hash-only-logging.md`](./hash-only-logging.md). |
| `http-failure` | The server returned a non-2xx without a class (most often a 4xx body the operator must read). | Re-run with `--json` and read the `data` field. |
| `malformed-response` | The server returned 2xx but the body did not parse as JSON. | The hosted service is mis-deployed or behind an HTML interstitial. Confirm `--endpoint` is correct and reachable. |
| `transport` | Network failure: DNS, TLS, refused connection. | Standard network triage. Check the endpoint URL, then the host allowlist, then DNS, then TLS roots. |

## When to update this runbook

- New subcommand in any binary: add a row to the binaries-at-a-glance
  table and (if it introduces a new bearer surface) the env-var matrix.
- New foundation `Error` variant: add a row to the troubleshooting table.
- New `TRACE_COMMONS_PRIVACY_FILTER_*` env var added to the sidecar
  contract: add a row to the table under `privacy-filter-canary` above.
