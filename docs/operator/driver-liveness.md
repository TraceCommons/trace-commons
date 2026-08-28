# Checking whether a background driver is alive

`GET /v1/admin/driver-liveness` answers one question per background driver:
**when did this last actually work?**

It exists because logs could not answer it. Each of the twelve background
loops used to hand-roll `loop { sleep; match tick { info!, warn! } }`, so a
driver that failed every tick emitted one identical warning per tick and
nothing that said "this has not succeeded since Tuesday". In the 2026-08-26
NEAR AI outage two drivers were dead for days behind roughly 7,000 identical
warning lines. One value — `last_success_at` — makes that obvious; 6,999
warnings do not.

## Calling it

Admin role required (`require_admin`). The pilot sets
`TRACE_COMMONS_REQUIRE_EDDSA_SIGNED_TOKENS` and refuses static bearer tokens,
so mint an EdDSA-signed JWT carrying the admin role, signed with
`issuer-signing-v1.pem`, `kid` taken from the published keyset, with `iss`,
`aud`, and a `jti`. Same mechanism as every other `/v1/admin/*` call — see
`quarantine-review.md` for the reviewer-role variant of the same procedure.

```
curl -sS https://ingest.tracecommons.ai/v1/admin/driver-liveness \
  -H "Authorization: Bearer $TRACE_COMMONS_ADMIN_JWT" | jq .
```

The response is a JSON array, one object per registered driver, ordered by
driver name.

### Why this is not on `/health`

It was proposed for `/health` and deliberately declined. `/health` is
unauthenticated. Which subsystem is currently dead and for how long is
operational intelligence: it tells an anonymous caller exactly when the PII
backstop is not running. A test in the ingest test module asserts that
`/health` does not name a driver; do not "fix" that.

The handler does no database read, no tenant scoping, and no audit write.
Driver health is process-global and tenant-independent, and the endpoint is
meant to stay cheap enough to poll.

## Response fields

| Field | Meaning |
|---|---|
| `driver` | Driver name, e.g. `pii_backstop_driver`. Stable, greppable. |
| `interval_seconds` | This driver's own tick interval. |
| `started_at` | When the loop was registered, i.e. process start. |
| `last_success_at` | Last tick that returned `Ok`. `null` means it has *never* succeeded in this process — after a restart that is the honest answer, not "healthy". |
| `last_failure_at` | Last tick that returned an error. |
| `consecutive_failures` | Failures since the last success. Resets to 0 on any success. |
| `last_failure_class` | Short triage label, see below. `null` if there has been no failure. |
| `last_error_hash` | Hash-only handle for the last error, for correlating with the log line. Never error text. |
| `dead_seconds` | Seconds since this driver last worked (measured from `last_success_at`, or from `started_at` if it has never succeeded). |
| `stale_after_seconds` | The threshold this driver is measured against, so you can see *why* `stale` is what it is without knowing the constants. |
| `stale` | `dead_seconds > stale_after_seconds`. |

**`stale` is derived at read time, not stored.** That matters: a driver whose
tokio task panicked and died outright still reports `stale: true`, because
nothing has moved `last_success_at` forward. It will report that despite
writing no log line, ever again. That is precisely the failure mode logs
structurally cannot report — a dead task emits nothing, and nothing is
indistinguishable from quiet success in a log file.

## Failure-class labels

The class sits alongside the error hash, never instead of it. It carries no
error text, no URL, no vendor response body, and nothing about a trace.

| Class | Means | First thing to check |
|---|---|---|
| `upstream_unavailable` | A dependency outside this process was unreachable or erroring — transport failure, timeout, or a 5xx that survived the adapter's own retries. | The vendor. NEAR AI credit balance and status first, since that is what bit on 2026-08-26. |
| `config_missing` | Required configuration or credentials were absent, so the tick refused before doing any work. | This deployment. `/etc/tracecommons` env files and the systemd drop-ins. |
| `dependency_unavailable` | An in-infrastructure dependency failed: the database mirror, a connection pool, or object storage. | Cloud SQL, the pool, GCS. |
| `content_rejected` | The upstream was reached and rejected the input. A property of the content, not of the system's health. | The trace, not the service. Usually not an outage. |
| `unclassified` | No typed marker matched. | Treat as unknown. Read `last_error_hash` and grep the log for it. |

## Escalation model

A run of failures counts as "dead" rather than "unlucky" after
`max(3 x interval_seconds, 300 seconds)` without a success. The 300-second
floor keeps the fast drivers (45s and 60s ticks) from escalating on a single
blip.

Once escalated, the driver logs one ERROR per **fifteen minutes** for as long
as it stays dead, carrying the duration and the suppressed count — not one
line per tick. Recovery is logged once, on the first success.

## Worked example: the 2026-08-26 NEAR AI outage

Had this endpoint existed on 2026-08-26, the two affected drivers would have
looked roughly like this. The field values are illustrative — tick intervals
are per-deployment config, not constants:

```json
{
  "driver": "pii_backstop_driver",
  "interval_seconds": 60,
  "started_at": "2026-08-21T04:11:02Z",
  "last_success_at": "2026-08-21T04:12:02Z",
  "last_failure_at": "2026-08-26T00:51:02Z",
  "consecutive_failures": 6999,
  "last_failure_class": "upstream_unavailable",
  "last_error_hash": "3f9a1c...",
  "dead_seconds": 419940,
  "stale_after_seconds": 300,
  "stale": true
}
```

`stale: true`, `failure_class: upstream_unavailable`, `dead_seconds` in the
hundreds of thousands, `consecutive_failures` near 7,000. The triage from
there is one step: `upstream_unavailable` on a NEAR AI-backed driver —
`pii_backstop_driver` and `perplexity_score_driver` — means check the vendor
account before touching anything in this repo. The same class on
`credit_cycle_scheduler`, `near_credit_outbox_scheduler`, or
`benchmark_registry_scheduler` points at NEAR **chain RPC**, not NEAR AI.

Two things that are expected rather than an outage:

- `error_hash` values changed for the ten worker-route-backed drivers when
  they moved onto the shared wrapper. Hashes recorded before that change no
  longer correlate with the ones you see now; compare only within an era.
- A deliberate dry-run settlement rehearsal against a policy version that is
  not in `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS` will show
  `credit_settlement_scheduler` (and `credit_cycle_scheduler`, which runs the
  same settlement step) as stale with `config_missing`. That is the rehearsal
  reporting that it settles nothing, not a fault.

## Nothing alerts on this yet

This endpoint is a **thing to check**, not a thing that pages you.

There is no alerting wired to it. Pilot application logs go to
`/var/log/tracecommons/ingest.log`, outside the journal, so a clean
`journalctl` proves nothing and no log-based alerting is watching them
either. Until something polls this endpoint and raises on `stale: true`, the
gap this closes is *diagnosis time*, not *detection time*: it makes a
multi-day outage obvious in one read once someone thinks to look. Someone
still has to look.

Include it in the periodic operator check alongside
`/v1/admin/operational-summary`.

## Registered drivers

`perplexity_score_driver`, `pii_backstop_driver`, `export_job_scheduler`,
`near_credit_outbox_scheduler`, `retention_maintenance_scheduler`,
`vector_index_scheduler`, `benchmark_registry_scheduler`,
`benchmark_pipeline_scheduler`, `credit_cycle_scheduler`,
`credit_settlement_scheduler`, `process_evaluation_scheduler`,
`revocation_propagation_scheduler`.

A driver appears only if its loop was actually spawned, so a driver missing
from the response was not started in this deployment — check the feature
flags and env gates for that subsystem rather than assuming it is healthy.
