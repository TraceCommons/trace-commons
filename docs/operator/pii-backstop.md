# Server-side NEAR AI PII backstop

At ingest, a Low-risk trace whose envelope has `message_text_included` is
held in corpus status `awaiting_pii_backstop` instead of `Accepted` when the
backstop is enabled — the submit-time deterministic/near-ai privacy filter
already ran, but message-text traces get one more async pass before they
join the corpus. A background driver, folded into the perplexity-scoring
driver task family, runs the chunked NEAR AI prose-PII classifier over the
held trace's message text, re-redacts any residual PII, re-stores a
rescrubbed envelope (appends `+near-ai-pii-backstop-v1` to the pipeline
version, writes a `RescrubbedEnvelope` object ref, and invalidates the old
`submitted_envelope` ref), and transitions the submission to
`Accepted`/`Quarantined`, releasing the hold. On failure it bumps the
`trace_pii_backstop` attempt counter and leaves the trace held — fail-closed,
never released un-rescrubbed. It ships **disabled**.

## Enable prerequisites

Work through these in order before setting `TRACE_COMMONS_PII_BACKSTOP_ENABLED=1`:

1. **Migration V38 applied.** `V38__trace_pii_backstop.sql` creates the
   `trace_pii_backstop` attempt-bookkeeping table (RLS-forced, tenant-scoped)
   and the `trace_pii_backstop_driver` role. Confirm it ran:

   ```sql
   SELECT to_regclass('trace_pii_backstop');
   SELECT rolname FROM pg_roles WHERE rolname = 'trace_pii_backstop_driver';
   ```

2. **`trace_pii_backstop_driver` role has a LOGIN path, and
   `TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL` points at it.** This
   role reads cross-tenant with no tenant context — same mechanism as
   `trace_login_resolver` and `trace_gate_driver`; see
   [`login-resolver-role.md`](login-resolver-role.md) and
   [`perplexity-scoring-driver.md`](perplexity-scoring-driver.md) for the
   full writeup. It ships `NOLOGIN NOBYPASSRLS`; migration V38 grants it
   column-scoped `SELECT` on `trace_submissions`, `trace_object_refs`, and
   `trace_pii_backstop`, each gated by a role-scoped permissive
   `trace_pii_backstop_driver_cross_tenant_read` policy — never a
   `BYPASSRLS` grant. (`trace_gate_driver` follows the same column-scoped
   convention after V42; see [`perplexity-scoring-driver.md`](perplexity-scoring-driver.md).)
   Recommended: a dedicated LOGIN role granted membership
   in the base role:

   ```sql
   CREATE ROLE tc_pii_backstop_driver_login LOGIN PASSWORD '<secret>' NOBYPASSRLS;
   GRANT trace_pii_backstop_driver TO tc_pii_backstop_driver_login;
   ```

   ```sh
   export TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL="postgres://tc_pii_backstop_driver_login@/trace-commons?host=/cloudsql/.../trace-commons"
   ```

   Do **not** grant the login role `BYPASSRLS`. Fail-closed: if
   `TRACE_COMMONS_PII_BACKSTOP_ENABLED=true` and this URL is unset or blank,
   the binary refuses to start.

3. **`TRACE_NEAR_AI_PRIVACY_API_KEY` provisioned server-side.** The driver
   reuses the same NEAR AI Cloud privacy-filter credentials as the
   submit-time filter (`TRACE_NEAR_AI_PRIVACY_API_KEY`, plus optional
   `TRACE_NEAR_AI_PRIVACY_BASE_URL` / `_MODEL` / `_TIMEOUT_MS`). Fail-closed:
   if the backstop is enabled and the key is unset or blank, the binary
   refuses to start.

4. **Flip the toggle** and restart:

   ```sh
   export TRACE_COMMONS_PII_BACKSTOP_ENABLED=1
   ```

   See [`env-reference.md`](env-reference.md) § "Server-side NEAR AI PII
   backstop driver" for the full var table (tick interval, batch size, max
   attempts, backoff base — all optional, conservative defaults).

## Drill: seed, hold, release

1. Submit (or seed) a Low-risk trace whose envelope carries
   `message_text_included` — e.g. run the normal contributor submit path
   with a message-text-bearing trace against a tenant that has the backstop
   enabled.
2. Confirm it lands held rather than accepted:

   ```sql
   SELECT status FROM trace_submissions WHERE submission_id = '<id>';
   -- expect: awaiting_pii_backstop
   ```

3. Wait for at least one driver tick (`TRACE_COMMONS_PII_BACKSTOP_TICK_INTERVAL_SECONDS`,
   default 45s), then re-check:

   ```sql
   SELECT status FROM trace_submissions WHERE submission_id = '<id>';
   -- expect: accepted (or quarantined if residual PII pushed risk up)
   ```

4. Confirm the release rewrote the envelope pipeline version and object
   refs — hash-only, no raw envelope contents in the query:

   ```sql
   SELECT pipeline_version FROM trace_submissions WHERE submission_id = '<id>';
   -- expect a trailing "+near-ai-pii-backstop-v1" suffix
   SELECT ref_kind FROM trace_object_refs WHERE submission_id = '<id>';
   -- expect a RescrubbedEnvelope ref; the prior submitted_envelope ref is invalidated
   ```

## Inspecting held traces

List everything currently held:

```sql
SELECT tenant_id, submission_id, created_at
  FROM trace_submissions
 WHERE status = 'awaiting_pii_backstop'
 ORDER BY created_at;
```

Check attempt bookkeeping for a specific held submission:

```sql
SELECT attempts, last_attempt_at, last_error_label
  FROM trace_pii_backstop
 WHERE tenant_id = '<tenant>' AND submission_id = '<id>';
```

`last_error_label` is a safe missing-control/error label, never raw error
text or trace content — hash-only/label-only conventions apply throughout
this table.

## Re-driving stuck rows

After `TRACE_COMMONS_PII_BACKSTOP_MAX_ATTEMPTS` failures the driver stops
retrying a submission automatically; it stays held on
`awaiting_pii_backstop` indefinitely rather than releasing un-rescrubbed. If
you deploy a fix (e.g. a NEAR AI outage clears, or a filter-config bug is
resolved) and want previously-failed submissions retried — rather than
raising `MAX_ATTEMPTS` to sidestep the cap — reset their attempt counters:

```sql
UPDATE trace_pii_backstop
   SET attempts = 0, last_error_label = NULL
 WHERE tenant_id = '<tenant>' AND submission_id = '<id>';

-- or, to re-drive every stuck row for a tenant:
UPDATE trace_pii_backstop
   SET attempts = 0, last_error_label = NULL
 WHERE tenant_id = '<tenant>';
```

Run this **after** deploying the fix — otherwise the driver re-attempts the
same submissions against the unfixed binary and the rows fail again on the
next tick.

## Fail-closed guarantees

- **Held until success.** A trace never reaches `Accepted`/`Quarantined`
  without the backstop's re-redaction pass completing. Failures bump the
  attempt counter and leave the submission on `awaiting_pii_backstop`; there
  is no timeout-based auto-release.
- **Boot refuses on misconfiguration.** Enabling the backstop
  (`TRACE_COMMONS_PII_BACKSTOP_ENABLED=true`) without
  `TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL` or without
  `TRACE_NEAR_AI_PRIVACY_API_KEY` refuses to start the binary rather than
  degrading to a weaker path.
- **Per-tick canary.** Before processing a batch, the driver runs a
  liveness/health canary against the NEAR AI privacy filter; if the filter
  is unhealthy, the entire tick aborts rather than partially processing (or
  releasing unrescrubbed) held submissions.

## Known caveat: `require_object_refs` tenants and the read path

A tenant configured with an object-primary read requirement (e.g.
`TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS` /
`TRACE_COMMONS_DB_CONTRIBUTOR_REQUIRE_OBJECT_REFS`) relies on the
`RescrubbedEnvelope`-preferring read path added alongside the ingest hold
(Task 7 of this feature) to always resolve the rescrubbed object over the
original `submitted_envelope` once released. Operators do not need to
configure anything extra for this — it is automatic — but be aware that any
custom tooling reading `trace_object_refs` directly must prefer
`RescrubbedEnvelope` over `submitted_envelope` when both are present, the
same way the built-in read path does, or it will surface the pre-backstop
envelope.
