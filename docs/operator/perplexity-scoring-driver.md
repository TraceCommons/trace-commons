# Perplexity scoring driver

The pilot's perplexity gate (NEAR AI Qwen3.6-35B, `GATE_SERVICE=enclave_near_ai`)
is fully configured but historically never ran: the enclave gate is exposed
only as an async worker endpoint (`POST /v1/workers/gate/evaluate`, one
submission at a time), and nothing on the pilot triggered it. `trace_gate_decisions`
stayed empty — no submitted trace was ever perplexity-scored, even though the
lightweight embedding "duplicate_precheck" ran at submit time.

This slice adds an **in-process background loop** in `trace-commons-ingest`
that enumerates ungated submissions cross-tenant, scores each via the NEAR AI
enclave gate (perplexity + novelty), applies skip-duplicate and cache-cost
controls, records the gate decision, and bumps a bounded attempt counter on
failure. It is **off by default** and, when on, **only records** — the floor
stays 0 until calibration (see the last section).

## Pool/role setup: `trace_gate_driver`

The driver enumerates submissions **cross-tenant with no tenant context**,
which is the same unauthenticated/no-RLS-context problem the login-resolver
role solves for account bootstrap. It uses the identical mechanism, on its
own separate connection pool — see
[`login-resolver-role.md`](login-resolver-role.md) for the full mechanism
writeup; this section states only what differs.

The `V36__trace_gate_driver.sql` migration creates:

```sql
CREATE ROLE trace_gate_driver NOLOGIN NOBYPASSRLS;
-- plus role-scoped permissive SELECT policies on the tables the driver reads
-- (submissions lacking a gate decision) and the driver's own attempt-counter
-- table, trace_gate_evaluation_attempts
```

`V42__trace_gate_driver_column_grants.sql` then narrows those grants to the
exact columns the driver's queries select, join on, filter by, or order by —
the same convention `V38__trace_pii_backstop.sql` established for
`trace_pii_backstop_driver`. Cross-tenant enumeration stays deliberate
(`USING (true)` policies unchanged); a compromised driver credential must not
read object keys, encryption key refs, redaction payloads, or other columns
outside that surface.

Two properties are load-bearing, exactly as for `trace_login_resolver`:

- **`NOLOGIN`** — the role cannot be connected to directly as shipped. An
  operator must provision a way to connect before the driver pool can do
  anything.
- **`NOBYPASSRLS`** — the role MUST remain NOBYPASSRLS. Cross-tenant
  visibility is authorized **only** by the role-scoped permissive policies
  added in V36, never by bypassing RLS outright. Column width is constrained
  by V42.

### What the pool code actually does

Read `crates/trace-commons-server/src/db/postgres.rs`: the gate-driver pool
is built the same way as the login-resolver pool (same function, a few lines
below it) — a **separate, small `deadpool_postgres::Pool`** (`max_size(2)`)
built directly from `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL`, parsed as a
plain `tokio_postgres::Config`. The pool issues **no `SET ROLE`** on
connection and does nothing to assume `trace_gate_driver` at runtime — it
simply connects as whatever role is encoded in the URL. Consequently,
whichever role name is in `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` is the
role Postgres sees on every connection from this pool: authorization comes
either from that role directly being (or inheriting membership in)
`trace_gate_driver`, not from any in-process privilege-elevation step.

### Recommended: dedicated LOGIN role with role membership

Create a separate LOGIN role and grant it membership in `trace_gate_driver`,
mirroring the recommended login-resolver setup exactly:

```sql
CREATE ROLE tc_gate_driver_login LOGIN PASSWORD '<secret>' NOBYPASSRLS;
GRANT trace_gate_driver TO tc_gate_driver_login;
```

Then point the gate-driver pool at the login role:

```sh
export TRACE_COMMONS_GATE_DRIVER_DATABASE_URL="postgres://tc_gate_driver_login@/trace-commons?host=/cloudsql/.../trace-commons"
```

Because the pool does not issue `SET ROLE`, the login role must **inherit**
the base role's grants (the default `INHERIT` behavior) for the driver to see
anything. Do not grant the login role `BYPASSRLS`.

### Alternative: make the base role directly connectable

```sql
ALTER ROLE trace_gate_driver LOGIN PASSWORD '<secret>';
```

```sh
export TRACE_COMMONS_GATE_DRIVER_DATABASE_URL="postgres://trace_gate_driver@/trace-commons?host=/cloudsql/.../trace-commons"
```

Simpler, but couples the credential to the privilege-bearing role. Do **not**
add `BYPASSRLS`.

## Env vars

All are read by `trace-commons-ingest` at boot. Defaults are conservative;
the driver is off unless explicitly enabled. See
[`env-reference.md`](env-reference.md) § "Perplexity scoring driver" for the
canonical table. Summary:

| Var | Default | Notes |
|---|---|---|
| `TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` | `false` | Master toggle. No bearer-token gate — unlike other schedulers, this alone turns the loop on. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_INTERVAL_SECONDS` | `45` | Cadence between enumeration batches. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_BATCH_SIZE` | `5` | Submissions enumerated per tick. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_MAX_ATTEMPTS` | `5` | Bounded attempt counter per submission before the driver stops retrying it. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATES` | `true` | Cache-cost control; falsy values (`0`/`false`/`no`/`off`) disable it. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATE_THRESHOLD_MICROS` | `900000` | Novelty threshold above which a submission is treated as a duplicate and skipped. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_BACKOFF_BASE_SECONDS` | `30` | Base backoff after a scoring failure. |
| `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` | (none) | The `trace_gate_driver`-role pool connection string, per above. |

## Enabling on the pilot

1. Provision the role/pool per the "Pool/role setup" section above.
2. Set:

   ```sh
   export TRACE_COMMONS_GATE_DRIVER_DATABASE_URL="postgres://tc_gate_driver_login@/trace-commons?host=/cloudsql/.../trace-commons"
   export TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED=1
   ```

3. Restart `trace-commons-ingest`. The driver loop starts automatically; no
   separate binary or worker token is needed.

## Verification

Confirm scoring is actually happening by checking that `trace_gate_decisions`
is no longer empty:

```sql
SELECT count(*) FROM trace_gate_decisions;
```

A growing count over successive checks confirms the driver is enumerating
and scoring submissions. Hash-only audit conventions apply — do not paste
real row contents (trace bodies, contributor identity, raw scores tied to a
specific submission) into tickets or logs; the count above is safe because
it carries no per-row content.

## Resetting retry bookkeeping after a fix

When a submission fails scoring, the driver bumps
`trace_gate_evaluation_attempts` and backs off; after
`TRACE_COMMONS_PERPLEXITY_DRIVER_MAX_ATTEMPTS` failures it drops out of the
work set entirely. If you deploy a fix and want the previously-failed
submissions re-scored from scratch — rather than raising `MAX_ATTEMPTS` to
sidestep the cap — clear their attempt rows with:

```bash
sudo deploy/pilot-gcp/reset-gate-attempts.sh                 # all stuck rows
sudo deploy/pilot-gcp/reset-gate-attempts.sh <submission_id> # one submission
```

It only deletes attempt rows for submissions that still have **no** gate
decision (the stuck set); successfully-scored submissions are never touched.
It enumerates cross-tenant via the read-only `trace_gate_driver` role and
deletes per-tenant via the `app` role under RLS, so it needs no extra
grants. Run it **after** deploying the fix — otherwise the driver re-attempts
the same submissions against the unfixed binary and the rows repopulate.

## Floor stays 0 until calibration

Enabling this driver makes perplexity scoring **run and record decisions**.
It does **not** change gating behavior: `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`,
`TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS`, and any perplexity-based
component of the decision stay at their pilot-launch value of `0` (disabled)
until the A2.7 floor calibration completes. See
[`a27-perplexity-floor-calibration.md`](a27-perplexity-floor-calibration.md)
for the calibration procedure and the floor-raising checklist. Until that
calibration runs, this driver's only effect is to populate
`trace_gate_decisions` with real perplexity/novelty measurements for
calibration to consume — it does not reject or quarantine any submission on
its own.
