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

## The redaction-witness bypass

**Shipped off. `TRACE_COMMONS_WITNESS_BYPASS_ENABLED` defaults to `false`,
and with it off an arriving certificate is ignored entirely.**

A contributor whose redaction ran inside an attested redaction-witness
enclave can send the resulting certificate with their submission, in two
headers:

| Header | Carries |
|---|---|
| `x-trace-witness-certificate` | the certificate as compact JSON, forwarded byte for byte from the witness's own response header |
| `x-trace-witness-signature` | the EIP-191 signature over it, `0x`-hex |

When the bypass is configured and that certificate verifies, the submission
is left `accepted` instead of being held on `awaiting_pii_backstop`.

### 1. What it changes, and what it does not

It changes **one thing**: whether a submission enters the hold. It changes no
risk tier, lifts no quarantine, releases nothing already held, and **never
means the trace is clean**.

What a verified certificate says is that a known program, in an enclave whose
measurement you pinned, reached a `Low` residual-PII verdict over the
*originating* redaction pass. That is a real statement and a narrow one. In
particular a credential the prose classifier itself wrote back into a field it
was handed survives that verdict — which is why the bypass is not, and cannot
be, a wholesale skip of the backstop.

**The deterministic sweep still runs, on every submission, witnessed or not.**
`rescrub_trace_envelope` — the deterministic pass over `redacted_content` and
`structured_payload`, plus the residual scan — runs synchronously in the
submit handler *before* the hold is decided. If it raises the risk, the
submission is quarantined and the certificate cannot lift it. The only thing
the bypass skips is the queued classifier re-check that the driver would
otherwise perform. Four further conditions all have to hold: the bypass is
configured, the certificate verified against the pin, its verdict is `Low`,
and its policy alias is allowlisted.

A witnessed trace's receipt says which pass admitted it, so a contributor can
tell the two bases apart.

### 2. It will not drain the queue

If you are turning this on to move the held backlog, it will not, and you will
conclude the feature is broken. Three independent reasons:

- No contributor client emits a certificate yet.
- The held traces are **already past** the decision point this changes. It is
  a submit-path decision; nothing re-evaluates an existing hold.
- What actually stops the queue is the driver's per-tick classifier canary,
  which aborts the whole tick before enumeration — not per-trace classifier
  cost.

Draining the backlog is separate work. See the drain-rate and quarantine
disposition notes above.

### 3. Only `full-pipeline` aliases belong in the allowlist

`TRACE_COMMONS_WITNESS_ALLOWED_POLICY_VERSIONS` is a separate control from the
measurement pin because it is a separate hole.

A witness reporting a `deterministic-only` policy alias **never ran a prose
classifier at all**. Admitting that alias means no classifier ever reads that
trace's prose — not the witness's, because it had none, and not the server's,
because you just skipped it. Nothing in the code can distinguish that from a
deliberate operator choice, so the only refusal available is on an *empty*
allowlist. Allowlist only aliases that name a full pipeline: a deterministic
pass **and** a classifier.

### 4. Pin before you enable

Enabling the bypass without a pinned signing address, without a measurement
set, or with an empty policy allowlist is a **boot refusal** naming the
missing control. That is by design, and it is the same shape
`near_ai_expected_measurements` uses: a control an operator believes is in
place but is not is worse than a server that will not start.

The recommended sequence:

1. Set the three pin variables (and, optionally, the age window) with the
   switch still `false` or absent. The
   binary boots normally and ignores certificates; nothing changes.
2. Confirm the measurement matches a real witness deployment's
   `/v1/attestation` output.
3. Set `TRACE_COMMONS_WITNESS_BYPASS_ENABLED=true`.

| Variable | Missing-control name on refusal |
|---|---|
| `TRACE_COMMONS_WITNESS_BYPASS_ENABLED` | (the switch itself; absent means off, not an error) |
| `TRACE_COMMONS_WITNESS_SIGNING_ADDRESS` | `witness_signing_address` |
| `TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` | `witness_expected_measurement` |
| `TRACE_COMMONS_WITNESS_ALLOWED_POLICY_VERSIONS` | `witness_allowed_policy_versions` |
| `TRACE_COMMONS_WITNESS_CERTIFICATE_MAX_AGE_SECONDS` | (optional; defaults to 86400. A value that is not a positive integer is a boot refusal, never a silent fallback) |

### How long a certificate stays usable

A certificate names no submitter and carries no nonce, so the pair of
(envelope bytes, certificate) is a **bearer token**: whoever holds one can
submit those bytes under any account and get the bypass. The only thing that
bounds this is the age window.

`TRACE_COMMONS_WITNESS_CERTIFICATE_MAX_AGE_SECONDS` sets it, defaulting to
24 hours. Certificates stamped more than five minutes ahead of this host's
clock are refused as future-dated — a separate refusal from expiry, because
it points at the witness's clock rather than at the submission. Keep both
hosts on NTP.

Narrowing the window narrows the replay exposure proportionally. The honest
path takes seconds, so a much smaller value is usually safe; the default is
generous only so that a contributor who loses connectivity after witnessing
does not have to send the raw session a second time.

**This does not bind a certificate to a submitter.** Inside the window it is
still replayable by anyone holding it. Making one single-use requires a nonce
or a submission identifier inside the signed preimage, which is a protocol
change on both the witness and the client and is not implemented.

### A malformed certificate never rejects a submission

Every failure on this path — a half-present header pair, an unparseable
certificate, a signature that does not recover, a measurement that is not
pinned, a digest over other bytes — refuses **the bypass** and holds the trace
exactly as an unwitnessed one. A witness outage must not become a submission
outage. Refusals are logged at `debug`, by name only; no header value, digest
or signature reaches a log line.
