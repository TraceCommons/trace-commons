# Switching on invite-free contribution

Taking the admission ledger from off to enforcing: what must already be true,
the six limits and how to choose them, the order to set them in, what proves
each step worked, how to switch it back off, and what an operator will
actually hit when it goes wrong.

Invite-free contribution means a NEAR account that bootstrapped its own
identity — no invite grant, no allowlist entry — may upload, provided every
new submission carries receipt-bound admission evidence. Identity is not
authority here: `admission::evidence_binding` refuses a new submission from a
provisioned account that presents no evidence headers, and there is no second
path. What buys the upload is the receipt, not the account.

Two properties to understand before you start, because both are deliberate
and neither is going to change:

**The admitted claim is about the enclave, not the model.** A verified
admission receipt says: these request and response bytes were produced inside
an attested NEAR AI enclave whose signing key we pinned. Which *model* served
them is NEAR AI's statement — the three-part `provider_tee` receipt carries
the model name inside the signed text, and we compare it against
`TRACE_COMMONS_ADMISSION_ACCEPTED_MODELS`, but the hardware attests the
enclave and the key, not the routing decision behind them. The two-part
`gateway` receipt is weaker still: it names no model at all, which is why
`AdmissionProviderTrust` keeps the two signer sets separate and refuses every
gateway receipt unless an operator sets `TRACE_COMMONS_ADMISSION_GATEWAY_SIGNERS`
on purpose. See `deploy/witness/README.md`, "A gateway receipt attests the
bytes, not the model". This is a documented property of the feature, ruled on
deliberately. Do not describe an admitted trace as model-attested.

**Every gate on this path fails closed.** A missing limit, an absent provider
signer set, a database whose grants are not as expected, a witness that is not
configured — each one refuses. What a misconfiguration produces is *refused
contributions*, never admitted-but-unchecked ones. Concretely: the binary
refuses to boot on bad configuration, and at request time an unverifiable
receipt is `403 admission_refused` with no ledger row. The failure mode of
this feature is that nobody can contribute, which is the correct direction to
fail in and the reason the verification steps below matter — a green service
proves nothing about whether contributions are getting through.

---

## 1. Preconditions

### 1.0 Blocking: the anchor check has not caught up with V61/V62

**Do not run this procedure against a build at or after V61 until this is
fixed.** It is a code defect, not a configuration one, and it makes every step
below verify green and every contribution fail.

`admission::anchor` decides an account's anchor by taking the authenticated
tenant id, stripping `near-`, and requiring the remaining 64 hex characters to
equal the stored provisioned anchor:

```rust
// crates/trace-commons-server/src/bin/trace_commons_ingest_internal/admission.rs
let stored = stored.strip_prefix("sha256:").ok_or_else(denied)?;
if stored != candidate {
    return Err(denied());
}
```

That equality held under V58, where the schema constrained
`tenant_id = 'near-' || substring(anchor_hash from 8)`. V61 (#716) removed that
derivation: the anchor became a keyed blind index and the tenant id became 32
random bytes, and V62 dropped the constraint V61 had only appeared to drop.
`near_account_identity::tenant_id_is_independent_of_every_public_input` now
asserts that a tenant id never contains the blind index — which is exactly the
equality the line above requires. V61 also refuses outright if pre-salting rows
exist, so no deployment can hold a mix.

Consequences on such a build: every submission from a `near-…` tenant is
`403 admission_refused` — with admission enabled *and* with it disabled, since
`admission::reserve` calls `anchor()` before it consults
`state.admission`. A fix belongs in that function, comparing against the stored
anchor rather than against the tenant suffix.

This is a reading of the code, not a reproduction. The end-to-end matrix that
would go red for it (`admission_pg_tests::actual_postgres_challenge_witness_ingest_and_terminal_retry`)
needs an isolated PostgreSQL and is `#[ignore]`d, and the pure-function unit
tests that do run in CI cover `evidence_binding`, not `anchor`. Confirm against
your deployment before acting on it.

### 1.1 The ingest is already a durable, RLS-forced, witness-trusting deployment

`admission::config_from_env` refuses to build a configuration
(`admission_requires_witness_and_durable_database`) unless all of these hold:

| Requirement | How it is expressed |
|---|---|
| A PostgreSQL mirror is configured | `DATABASE_URL` resolves and the mirror is constructed |
| Mirror writes are required, not best-effort | `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` |
| Tenant RLS readiness is required | `TRACE_COMMONS_REQUIRE_POSTGRES_TRACE_RLS_READY=true` |
| The redaction witness is pinned and enabled | the four `TRACE_COMMONS_WITNESS_*` controls, per [`./attested-inference.md`](./attested-inference.md) §2 |

The witness requirement is not decorative. Admission evidence is verified
against a witness certificate: `admission::reserve` refuses unless
`state.witness_bypass` is present and the certificate's redaction policy
version is in the allowlist. Bring the witness up and prove it first.

### 1.2 Native NEAR provisioning is configured

Only a tenant whose id is `near-<64 hex>` and whose stored provisioned anchor
matches can reach the admission path at all (`admission::anchor`; see §1.0 for
why that match currently cannot succeed). That
namespace is allocated by [`./near-native-provisioning.md`](./near-native-provisioning.md),
which needs `TRACE_COMMONS_NEAR_PROVISIONING_ENABLED=true` (compared against
the literal string `true` — unlike the witness switch, `1` and `yes` do not
work here), plus `..._PUBLIC_ORIGIN`, `..._ISSUER_URL`, `..._AUDIENCE` and
`..._WITNESS_JSON`.

The dependency runs both ways: provisioning publishes its capabilities only
once admission configuration has validated, which is what makes
`/v1/account/near/provision/capabilities` a usable readiness probe in §4.

### 1.3 V59 and V60 are applied, and the runtime role does not own their tables

Both migrations are embedded in the binary and applied at boot by
`run_migrations` — there is no separate migrate binary, despite what
[`./deployment.md`](./deployment.md) §Prerequisites says. Whatever role the
process connects as is the role that runs the DDL, **and therefore owns the
tables**.

That matters because the ingest's own startup check
(`PgBackend::check_admission_runtime`) requires the runtime role to be a
stranger to those tables: not superuser, not `BYPASSRLS`, not a member of
`trace_admission_guard` or `trace_onboarding_retention_guard`, holding no
direct privilege on `trace_admission_receipts` or
`trace_admission_global_budget`, and — the one that catches single-role
deployments — **not the owner of any of the five admission tables**. A
deployment where one role both migrates and serves will boot fine today and
refuse with `admission_runtime_permissions_not_ready` the moment admission is
switched on.

So: apply V59/V60 as a dedicated schema owner (point `DATABASE_URL` at the
migrator role for one boot, or apply `migrations/V59__trace_admission_ledger.sql`
and `migrations/V60__onboarding_retention.sql` with `psql` and record them in
`_trace_commons_migrations` yourself), then grant the runtime role what it
needs. The exact grant block is in
[`./native-admission-session.md`](./native-admission-session.md), "Migration,
ingest and retention roles"; do not re-derive it here. If V59/V60 were already
applied by the runtime role, the repair is an ownership transfer to the
migrator, not a re-run.

### 1.4 Provider signer keys, derived out of band

`TRACE_COMMONS_ADMISSION_PROVIDER_SIGNERS` is a comma-separated set of
lowercase 64-hex ed25519 signing addresses, no `0x`. These are the same
per-model `provider_tee` keys the witness pins, and they are derived the same
way: one nonce-bound `signing_algo=ed25519` attestation report, with the
`report_data` binding read out of the TDX quote at hex `[1136:1264]` behind a
v4/TEE-type header gate. The authoritative derivation — including why
`signing_algo=ed25519` is load-bearing and why filtering on a `.report_data`
field yields an empty list — is in `deploy/witness/README.md`:

```bash
sed -n '/^### Pinning the receipt signing keys/,/^#### Upgrade order/p' \
  deploy/witness/README.md
```

Take the `signing_address` values that block prints and use them for both
`TRACE_COMMONS_ADMISSION_PROVIDER_SIGNERS` (ingest) and
`TRACE_COMMONS_WITNESS_ADMISSION_PROVIDER_SIGNERS` (witness). The two sets are
read by the same `AdmissionProviderTrust::from_env`, under different prefixes,
and are checked independently — the witness at certification time, ingest as
defence in depth over the witness signature.

> **Pending: live receipt-signer verification.** What ships today is a *pinned
> key set*, refreshed by an operator. It is not live quote verification at
> verification time, and a syntactically valid self-reported key is not an
> attested one. Server-side receipt-signer verification is being built
> separately; when it lands, this step gains a check that the pinned key is
> the one a fresh nonce-bound report attests, and this note comes out. Until
> then, re-derive the pins on the same cadence you re-derive the witness
> measurement, and treat a key you copied out of a receipt rather than out of
> a nonce-bound report as unpinned.

### 1.5 The retention worker is scheduled for each admission tenant

Expired challenges and pre-account ceremonies are pruned by
`POST /v1/workers/retention-maintenance`, bounded to 1000 records per call and
scoped to the authenticated worker's tenant. Account counters, submission
identities, terminal receipts, the global receipt dedup set and the global
budget are **retained** — cleanup never returns a spent budget unit. Schedule
it before you switch on, not after.

---

## 2. The six limits

`AdmissionLimits::from_env` requires all six to be present and to parse, and
refuses startup otherwise. There are no defaults, deliberately: every one of
them names something only an operator can know.

```
TRACE_COMMONS_ADMISSION_WINDOW_ATTEMPTS         >= 0
TRACE_COMMONS_ADMISSION_ACCOUNT_COST_LIMIT      >  0
TRACE_COMMONS_ADMISSION_GLOBAL_COST_LIMIT       >  0
TRACE_COMMONS_ADMISSION_PROCESSING_COST_BOUND   >  0
TRACE_COMMONS_ADMISSION_LEASE_SECONDS           1..=86400
TRACE_COMMONS_ADMISSION_CHALLENGE_TTL_SECONDS   1..=86400
```

A cost bound is **an operator-configured unit, never a fabricated USD
conversion** — the module says so in its first three lines, and nothing in
this file assigns it a currency, a token count or a per-call price.

### 2.0 Two structural facts that decide most of the values

**The budgets are lifetime and monotone.** `trace_reserve_admission` adds
`p_cost` to the account row and to the singleton global row on every
reservation. The only subtraction is `trace_transition_admission(... ,
'released')`, which fires exclusively for a submission that failed *before*
processing began. Nothing else decrements, no window rolls, and retention
explicitly retains the counters. These are stop-valves with a finite lifetime
supply, not rate limits. Rate limiting is done elsewhere and is already tight:
30 submissions per principal per minute with concurrency 2
(`SUBMIT_PER_PRINCIPAL_LIMIT` / `SUBMIT_PER_PRINCIPAL_CONCURRENCY`), and 10
challenge mints per minute.

**Three of the six are frozen at first use.** `trace_reserve_admission`
compares the passed `p_global_limit` against the stored singleton row, and
`p_attempt_limit` / `p_account_limit` against the stored account row. A
mismatch returns `configuration_changed`, which the Rust maps to
`AdmissionDecision::Refused` — every reservation, for every account, becomes
`403 admission_refused`. So `WINDOW_ATTEMPTS`, `ACCOUNT_COST_LIMIT` and
`GLOBAL_COST_LIMIT` cannot be changed by editing the environment alone once
rows exist; see §5.3. `PROCESSING_COST_BOUND`, `LEASE_SECONDS` and
`CHALLENGE_TTL_SECONDS` are compared to nothing and are freely changeable.

Choose the first three as though they were schema.

### 2.1 `TRACE_COMMONS_ADMISSION_WINDOW_ATTEMPTS` = **0**

*Derived from code, not measured.*

**What it bounds.** The `window` reservation kind: a submission with no
receipt. `charge_attempt` in the SQL is true only when `p_receipt IS NULL`, and
`admission::evidence_binding` never produces that case — a request without
evidence headers is refused before a reservation is built, and
`an_unverified_request_never_binds_a_reservation` fails if that is reverted.
The ingest has no producer for a `window` row.

**Why 0.** It is the fail-closed value for a path with no legitimate traffic.
If a future change reintroduces the evidence-less window, reservations refuse
(`window_exhausted` → `429 admission_limit_reached`) rather than quietly
spending an allowance nobody sized.

**Too low:** not reachable — 0 is the floor, and today it refuses nothing that
is otherwise admissible. **Too high:** a free per-anchor upload allowance the
moment the window path returns, and a window is per *verified account anchor*,
not per human — another controlled NEAR account is another window, possibly a
dust-funded implicit one.

**What would tell you to change it:** only a deliberate decision to reopen the
evidence-less window. `429 admission_limit_reached` on an account with no rows
in `trace_admission_submissions` would mean the window path is live again and
you did not intend it.

### 2.2 `TRACE_COMMONS_ADMISSION_PROCESSING_COST_BOUND` = **1**

*Derived from code, not measured.*

**What it bounds.** The amount charged per reservation against both budgets.
`admission::reserve` passes the configured constant for every submission,
whatever its size — the charge is completely insensitive to bytes, chunks or
scoring time. So the unit has exactly one honest meaning: one reserved
processing attempt. Setting it to 1 makes the two budgets read directly as
counts of attempts; any other value only rescales them.

**Too low:** cannot be — the validator requires `> 0`, and at 1 the budgets are
integers of attempts. **Too high** relative to the budgets: fewer attempts fit
before a legitimate contributor gets `429 admission_limit_reached`.

**What would tell you to change it:** a change making the charge proportional
to something real (bytes, chunk count, measured scoring seconds). At that point
this becomes a unit with content and every budget below needs re-deriving.
Note this is the one cost knob you *can* re-scale without the §5.3 dance —
previously-consumed amounts stay in the old units.

### 2.3 `TRACE_COMMONS_ADMISSION_ACCOUNT_COST_LIMIT` = **500**

*Partly measured; the multiplier is judgement.*

**What it bounds.** With the bound above, the number of submission-processing
attempts one NEAR account anchor may ever reserve. It protects against a single
anchor consuming the whole global budget — that is its only job, since the
per-minute rate limits already handle bursts.

**Where 500 comes from.** The measured pilot corpus is 1,055 submissions across
its entire history to 2026-08-27
(`docs/superpowers/plans/2026-08-27-gate-scoring-throughput.md`), and the
busiest week on record is 151 gate decisions, "more than the prior three months
combined" (`docs/superpowers/specs/2026-08-27-novelty-signal-scope.md`). 500 is
a bit over three such weeks and about half the entire pilot corpus to date, for
one account. The 3.3x headroom over the busiest observed week is a judgement
call, made large because §5.3 makes raising this expensive.

**Too low:** a productive contributor is refused with `429
admission_limit_reached`, permanently — the counter never resets — and only an
operator can lift it. **Too high:** the per-account limit stops being a
meaningful sub-division of the global budget, and one anchor can drain it.

**What would tell you to change it:** `SELECT anchor_hash, cost_bound_used FROM
trace_admission_accounts` showing any anchor above about half the limit while
you still consider its traffic legitimate. Raise it *before* it binds, in a
maintenance window, per §5.3.

### 2.4 `TRACE_COMMONS_ADMISSION_GLOBAL_COST_LIMIT` = **5000**

*Partly measured; the multiplier is judgement.*

**What it bounds.** Total reserved attempts across all anchors, for the
lifetime of the deployment. It is the stop-valve for the whole feature: the
number that decides how much work invite-free contribution can queue before an
operator has to look at it again.

**Where 5000 comes from.** The pilot host scores at a measured p50 of 287 s per
trace, ~12.5 traces/hour, one at a time
(`docs/superpowers/plans/2026-08-27-gate-scoring-throughput.md`). 5000 admitted
submissions is therefore roughly **17 days of continuously saturated
scoring** on the box as it is configured today, and about 5x the pilot's entire
three-month corpus. That is the useful reading of the number: not money, but
how long the queue can grow before the operator is forced back into the loop.

**Too low:** the whole feature stops. Every uninvited contributor gets `429
admission_limit_reached`, including one mid-session, and there is no automatic
recovery. **Too high:** the protection is nominal — you have accepted an
unbounded backlog on a host measured to clear 12.5 traces/hour, and the pilot
has already shown what a backlog does (`decided_at - received_at` p90 of 32.5 h
on that same measurement, and one submission that consumed 50 hours of CPU
while 210 others were never enumerated).

**What would tell you to change it:** `cost_bound_used` on
`trace_admission_global_budget` passing about 70% of the limit, or a scoring
backlog that is growing rather than draining. Raising it is §5.3, and it is a
fleet-wide operation — plan it, do not do it under pressure at 100%.

### 2.5 `TRACE_COMMONS_ADMISSION_LEASE_SECONDS` = **900**

*Not measured for this path; matched to a measured sibling bound.*

**What it bounds.** How long one submission id stays `busy` after a
reservation. Within the lease, a second reservation for the same id returns
`busy` → `409 admission_in_progress`. The lease covers only the *synchronous*
submit handler — reserve, mark processing, deterministic re-scrub, storage
write, record insert, finish — because gate scoring and the PII backstop run in
separate drivers. While the process lives, a session advisory lock already
serialises attempts; the lease is what remains after a crash or restart, when
that lock is gone.

**Where 900 comes from.** There is **no measured POST-to-completion figure in
the tree** — say so rather than dressing one up. 900 s is the value the same
process already uses, and has lived with, for a comparable per-submission
bound: `TRACE_COMMONS_PII_BACKSTOP_PER_SUBMISSION_TIMEOUT_SECONDS` defaults to
900 with the identical `[1, 86400]` clamp. It also sits comfortably above the
p90 of 385 s measured for the heaviest per-trace work on this host, on a box
that is CPU-saturated and where a 16 MB envelope is a plausible submission.

**Too low:** a lease that expires under a live request. The SQL is explicit
that expiry never releases a cost bound — a repeated attempt reserves *another*
bound while keeping the original charge. Short leases turn slow requests into
double charges.

**Too high:** after a crash mid-submit, that submission id is unusable until
the lease expires; the contributor's retry gets `409 admission_in_progress` for
up to 15 minutes. This is recoverable by waiting, and no data is lost.

**What would tell you to change it:** `409 admission_in_progress` on retries
where the original request is known to have died (lower it), or accounts whose
`cost_bound_used` exceeds their completed submission count (raise it — that gap
is double-charged expiries).

### 2.6 `TRACE_COMMONS_ADMISSION_CHALLENGE_TTL_SECONDS` = **900**

*Determined by code, not a judgement call.*

**What it bounds.** How long a minted challenge stays consumable. The
contributor mints a challenge, runs one bound inference, and submits; the
receipt is refused (`evidence_refused`) if the challenge has expired or has
already been consumed.

**Why exactly 900.** `challenge_handler` computes the expiry as
`challenge_ttl_seconds.min(900)`, and the native client independently refuses a
binding whose lifetime exceeds 15 minutes. Anything above 900 is *silently* the
same as 900. Set 900 so the configuration says what the system does; setting
3600 here is a configuration that lies to the next operator.

**Too low:** a contributor who mints, runs an inference and then takes longer
than the TTL to upload is refused, and must mint again. **Too high:**
impossible above 900; below it, the only effect of a longer window is that an
unconsumed challenge stays consumable longer — and since `consumed_by` makes a
challenge single-use and bound to one submission, the exposure is one
submission either way.

**What would tell you to change it:** `403 admission_refused` clustering on
contributors whose inference-to-upload gap is long — lower is worse here, and
900 is already the ceiling, so the real remedy is a client-side one.

### 2.7 The block to set

```sh
TRACE_COMMONS_ADMISSION_ENABLED=true
TRACE_COMMONS_ADMISSION_WINDOW_ATTEMPTS=0
TRACE_COMMONS_ADMISSION_PROCESSING_COST_BOUND=1
TRACE_COMMONS_ADMISSION_ACCOUNT_COST_LIMIT=500
TRACE_COMMONS_ADMISSION_GLOBAL_COST_LIMIT=5000
TRACE_COMMONS_ADMISSION_LEASE_SECONDS=900
TRACE_COMMONS_ADMISSION_CHALLENGE_TTL_SECONDS=900
TRACE_COMMONS_ADMISSION_PROVIDER_SIGNERS=<64-hex,64-hex>
TRACE_COMMONS_ADMISSION_ACCEPTED_MODELS=<exact model id[,...]>
TRACE_COMMONS_ADMISSION_MIN_REQUEST_BYTES=<positive integer>
```

`TRACE_COMMONS_ADMISSION_ENABLED` accepts exactly `true`/`1` (on) or
`false`/`0` (off); absent is off. Any other value is
`admission_configuration_invalid` at boot — a typo does not read as off.

`ACCEPTED_MODELS` and `MIN_REQUEST_BYTES` are eligibility controls, not proof
of spend: a byte floor is paddable and no per-call cost is inferred anywhere in
this implementation. Pick the floor from measured provider behaviour for the
models you accept; there is no defensible default and this file does not
invent one. Leave `TRACE_COMMONS_ADMISSION_GATEWAY_SIGNERS` **unset** unless
you have read §0 and accepted body-asserted model labels.

---

## 3. Order of operations

Each step leaves the deployment fail-closed. Do them in this order, and do the
verification in §4 between each one.

1. **Migrations and roles** (§1.3). Apply V59/V60 as the migrator, grant the
   runtime role, confirm ownership. Nothing is enabled yet; admission is off
   and the tables are inert.
2. **Witness side first.** Set `TRACE_COMMONS_WITNESS_ADMISSION_PROVIDER_SIGNERS`,
   `..._ACCEPTED_MODELS` and `..._MIN_REQUEST_BYTES` on the witness and
   redeploy it. *Before ingest*, because the witness is what certifies evidence:
   an ingest that trusts admission evidence while the witness cannot produce any
   admits nothing but does generate contributor-visible failures. Remember that
   a witness redeploy moves its measurement — every pin, including the ingest's
   `TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS`, moves with it
   ([`./attested-inference.md`](./attested-inference.md) §3).
3. **Ingest configuration, admission still off.** Write the §2.7 block into the
   drop-in with `TRACE_COMMONS_ADMISSION_ENABLED=false`, restart, and confirm
   the service still boots. This separates "did I typo a signer set" from "did
   the ledger permissions check fail", which otherwise arrive as one boot
   refusal.
4. **Flip `TRACE_COMMONS_ADMISSION_ENABLED=true`** and restart. This is the
   step that can refuse to boot; §6 lists what each refusal means.
5. **Schedule the retention worker** for every admission tenant, if it is not
   already scheduled.
6. **One real end-to-end contribution** from a provisioned account, before you
   tell anyone the door is open.

**Why this order, and what is recoverable.** Steps 1–3 are reversible with no
residue: no reservation has been made, so no account row and no global budget
row exist, and the three frozen limits are not yet frozen. Step 4 is the point
of no return for those three values — the first successful reservation writes
`cost_limit` and `attempt_limit` into `trace_admission_accounts` and
`trace_admission_global_budget`, and from then on changing the environment
alone refuses everything (§5.3). If you get step 4 wrong and no contributor has
yet submitted, the clean recovery is: set `ENABLED=false`, restart, `TRUNCATE`
the five admission tables as the migrator, fix the values, and start again. Once
a real contributor has been admitted, that truncate is destroying the replay
and dedup record — do not.

---

## 4. Verifying each step

### After step 1 — migrations and ownership

```sh
PGPASSWORD=$TC_DB_PASSWORD psql -h 127.0.0.1 -U app -d trace-commons -At <<'SQL'
SELECT version, name FROM _trace_commons_migrations WHERE version IN (59,60) ORDER BY version;
SELECT c.relname, pg_get_userbyid(c.relowner), c.relrowsecurity, c.relforcerowsecurity
  FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
 WHERE n.nspname = 'public'
   AND c.relname IN ('trace_admission_challenges','trace_admission_accounts',
                     'trace_admission_submissions','trace_admission_receipts',
                     'trace_admission_global_budget')
 ORDER BY c.relname;
SQL
```

Expected: exactly two migration rows, `59|trace_admission_ledger` and
`60|onboarding_retention`; then five table rows, every one of them owned by the
**migrator** role (not the ingest runtime role) with both boolean columns `t`.
Any table owned by the runtime role, or either boolean `f`, means step 4 will
refuse with `admission_runtime_permissions_not_ready`.

The full predicate the binary evaluates is the single query in
`PgBackend::check_admission_runtime`
(`crates/trace-commons-server/src/admission_ledger.rs`). To pre-flight it,
connect **as the runtime role** and run that query verbatim; it returns one
boolean and `t` is what boot requires. Reading it out of the source rather than
re-typing it here keeps this file from drifting from the check that actually
gates the boot.

### After step 2 — the witness

The witness runs as a dstack CVM, not a systemd unit, so there is no
`/proc/<pid>/environ` to read and **no witness surface reports whether its
admission trust is configured**. Two things are checkable, and between them
they cover it.

First, that the deployed image has the admission route at all — an older
witness image does not:

```sh
curl -s -o /dev/null -w '%{http_code}\n' -X POST \
  -H 'Content-Type: application/json' -d '{}' \
  https://<witness-host>/v1/witness/admission
```

Expected: `400` (or `403`) — the route exists and refused an empty body. A
`404` means the running image predates `/v1/witness/admission` and nothing you
set in its environment matters yet.

Second, that the variables are in the compose dstack actually stored — which
is also what the measurement covers:

```sh
phala cvms get <cvm-id> --json | jq -r '.compose_file' \
  | grep -o 'TRACE_COMMONS_WITNESS_ADMISSION_[A-Z_]*'
```

Expected: the three names (four with `..._GATEWAY_SIGNERS`). `deploy/witness/README.md`
documents why the stored `compose_file` is the authority here rather than the
manifest this repository generates. Remember that adding these variables
changed the compose and therefore the measurement — re-pin
`TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` on ingest before expecting a
single certificate to verify.

A witness that is *running* with `TRACE_COMMONS_WITNESS_ADMISSION_PROVIDER_SIGNERS`
set has a valid policy: it refuses to start with
`admission_provider_policy_missing_or_invalid` when the companion variables are
missing or malformed. A witness with the variable unset starts perfectly
happily and certifies no admission evidence at all, which is why the compose
check above is not optional.

### After steps 3 and 4 — the ingest

```sh
systemctl is-active trace-commons-ingest
PID=$(systemctl show -p MainPID --value trace-commons-ingest)
sudo cat /proc/$PID/environ | tr '\0' '\n' | grep -c '^TRACE_COMMONS_ADMISSION_'
```

Expected: `active`, then `10` (11 with `..._GATEWAY_SIGNERS`). The env file and
the drop-in say what was *written*; `/proc/<MainPID>/environ` says what the
process read, and on this deployment those have disagreed before.

Then the readiness probe, which is the only surface that reports the admission
configuration having validated:

```sh
curl -s http://127.0.0.1:3907/v1/account/near/provision/capabilities | jq '.ready'
```

Expected after step 4: `true`. Expected after step 3 (admission still off):
`false`. That field is derived from the validated admission configuration —
`published_witness` returns nothing unless `near_provisioning_admission_ready`
is set, which is `admission.is_some()`. It is unauthenticated and safe to curl
on loopback.

There is **no startup log line naming the admission limits**, and application
logs go to `/var/log/tracecommons/ingest.log`, not the journal — a clean
`journalctl -u trace-commons-ingest` proves nothing here.

### After step 6 — one real contribution

The honest evidence is a ledger row, not a 200:

```sh
PGPASSWORD=$TC_DB_PASSWORD psql -h 127.0.0.1 -U app -d trace-commons <<'SQL'
SELECT set_config('trace_commons.trace_tenant_id', 'near-<64hex>', false);
SELECT submission_id, kind, status, attempt_held, ever_processed, last_cost_bound
  FROM trace_admission_submissions ORDER BY lease_expires_at DESC LIMIT 5;
SELECT anchor_hash, attempt_limit, cost_limit, attempts_used, cost_bound_used
  FROM trace_admission_accounts;
SQL
```

Expected: one row with `kind = attested`, `status = completed`,
`attempt_held = f` (attested submissions charge no window attempt) and
`last_cost_bound` equal to your `PROCESSING_COST_BOUND`; and an account row
whose `cost_limit` / `attempt_limit` are exactly the values you configured —
if they are not, the environment and the ledger have already diverged and every
reservation is about to be refused (§5.3).

Note the GUC is `trace_commons.trace_tenant_id`, with an underscore. The
tenant-scoped tables are readable this way; `trace_admission_global_budget` and
`trace_admission_receipts` are **not** — their policies name
`trace_admission_guard`, so read them as the migrator or a guard member, and
never grant that membership to the runtime role.

---

## 5. Rollback

### 5.1 Switching it off

```sh
sudo sed -i 's/^Environment=TRACE_COMMONS_ADMISSION_ENABLED=true$/Environment=TRACE_COMMONS_ADMISSION_ENABLED=false/' \
  /etc/systemd/system/trace-commons-ingest.service.d/admission.conf
sudo systemctl daemon-reload
sudo systemctl restart trace-commons-ingest
curl -s http://127.0.0.1:3907/v1/account/near/provision/capabilities | jq '.ready'
```

Expected: `false`. Confirm the substitution actually changed the line — a no-op
`sed` looks exactly like a successful one. `grep ADMISSION_ENABLED` the file
before and after.

Removing the whole drop-in works too and is cleaner if you are abandoning the
attempt:

```sh
sudo rm /etc/systemd/system/trace-commons-ingest.service.d/admission.conf
sudo systemctl daemon-reload && sudo systemctl restart trace-commons-ingest
PID=$(systemctl show -p MainPID --value trace-commons-ingest)
sudo cat /proc/$PID/environ | tr '\0' '\n' | grep -c '^TRACE_COMMONS_ADMISSION_'
```

Expected: `0`. With the switch off, `AdmissionLimits::from_env` returns
`Ok(None)` and the uninvited-contribution path is closed again. Invited
contributors on ordinary invite grants are unaffected throughout — turning
admission off does not put a receipt requirement on anybody, and turning it on
did not either.

**One asymmetry to expect.** Switching off is not a clean return to "no
admission" for wallet accounts. `admission::reserve` resolves the anchor
*before* it consults `state.admission`, so a tenant in the `near-…` namespace
with a provisioned anchor row hits `state.admission.as_ref().ok_or_else(denied)?`
and is refused `403 admission_refused` — with the feature off. Rolling back does
not restore uploads for accounts that were provisioned through native NEAR
provisioning; it stops new ones being provisioned into a path that then refuses
them. Invite-bearing tenants, whose ids are not in that namespace, are
untouched either way.

### 5.2 What persists after you switch it off

Everything the ledger recorded:

- **A contributor admitted while it was on stays admitted.** Their submissions
  are stored, scored and credited like any other; nothing re-evaluates
  admission after the fact. This is a submit-path decision, made once.
- **Account rows keep their counters**, including `cost_limit` and
  `attempt_limit` frozen at the values that were configured when the row was
  created. Switching back on with different values refuses every reservation
  (§5.3).
- **The global budget row keeps `cost_bound_used`.** Time off does not refill
  it.
- **Consumed receipt hashes stay in `trace_admission_receipts` forever**, which
  is what makes a receipt single-use. Retention cleanup explicitly retains
  them; a receipt spent before the rollback cannot be spent after it.
- **Completed submissions remain readable as terminal retries while the
  feature is on.** An exact byte-identical retry of a completed submission is a
  read of an existing result and needs no evidence headers — short-lived
  evidence may legitimately have expired by then. With the feature switched
  off, that read is refused along with everything else in the `near-…`
  namespace, per the asymmetry in §5.1.

There is no "un-admit". If a contribution must be withdrawn, that is the
revocation path (`DELETE /v1/traces/{submission_id}`), not this switch.

### 5.3 Changing one of the three frozen limits

`WINDOW_ATTEMPTS`, `ACCOUNT_COST_LIMIT` and `GLOBAL_COST_LIMIT` are compared
against stored rows on every reservation. Changing the environment alone turns
every reservation into `configuration_changed` → `403 admission_refused`, for
every account, with no log line naming the cause. Change the environment and
the rows in one maintenance window:

```sh
# 1. stop admitting: ENABLED=false, restart (§5.1)
# 2. as the migrator (guard-visible), align the stored rows:
PGPASSWORD=$TC_MIGRATOR_PASSWORD psql -h 127.0.0.1 -U <migrator> -d trace-commons <<'SQL'
BEGIN;
UPDATE trace_admission_global_budget SET cost_limit = <new global> WHERE singleton;
UPDATE trace_admission_accounts SET cost_limit = <new account>, attempt_limit = <new attempts>;
COMMIT;
SQL
# 3. update the drop-in to the same values, ENABLED=true, restart
# 4. verify per §4: an account row's cost_limit must equal the env value
```

`cost_bound_used` is deliberately not reset by this: raising a limit grants
more headroom, it does not forgive what was already spent. Resetting consumption
is a different decision and should be a deliberate, separately-reasoned one.

---

## 6. Failure modes you will actually hit

**A missing or unparseable limit refuses startup.** Any of the six absent, or
not an `i64` → `admission_configuration_missing`. Out of range (a zero cost
limit, a lease of 0 or 90000) → `admission_configuration_invalid`. Neither
message names *which* variable. Count them first:
`sudo cat /proc/$PID/environ | tr '\0' '\n' | grep -c '^TRACE_COMMONS_ADMISSION_'`
against the expected 10, then check the ranges in §2 by eye. A `TRACE_COMMONS_ADMISSION_ENABLED`
typo — `True`, `yes`, `enabled` — is also `admission_configuration_invalid`,
not "off".

**An unpinned or wrong signer refuses every receipt.** With
`PROVIDER_SIGNERS` empty, malformed (not exactly 64 lowercase hex), or holding
a key in both the provider and gateway sets, the ingest refuses to boot with
`admission_provider_policy_missing_or_invalid`. With a *well-formed but wrong*
key set it boots happily and every contribution is `403 admission_refused` —
the same fixed code an expired challenge and a forged signature produce. There
is no signature error naming the key, by design: these surfaces are hash-only.
The discriminator is the ledger — a refused receipt leaves **no row** in
`trace_admission_submissions`, so an account with a challenge minted and no
submission row is a verification failure, not a budget failure.

**A stale key set refuses everything with a signature error naming nothing
useful.** NEAR AI rotates enclave keys; a pin derived weeks ago can stop
matching without anything on this host changing. Presentation is identical to
the wrong-key case above: universal `403 admission_refused`, no useful label.
Re-derive the pins (§1.4) and compare against what you have set before
concluding anything else is wrong. This is the single most likely cause of "it
worked yesterday".

**Wrong database ownership refuses startup.** `admission_runtime_permissions_not_ready`
means `check_admission_runtime` returned false: usually the runtime role owns
the admission tables because it ran V59 itself (§1.3), sometimes a stray
`GRANT` on `trace_admission_receipts` or `trace_admission_global_budget`,
sometimes leftover `trace_admission_guard` membership from a deployment that
ran the pre-revoke V59. Editing an already-applied migration file does not
re-run it; repair with explicit SQL.

**`admission_requires_witness_and_durable_database` at boot** means admission is
enabled without the witness bypass or without both durability flags. Check
`TRACE_COMMONS_WITNESS_BYPASS_ENABLED`, `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES`
and `TRACE_COMMONS_REQUIRE_POSTGRES_TRACE_RLS_READY` before suspecting anything
about admission itself.

**`429 admission_limit_reached`** is a budget refusal: either the account or the
global budget is exhausted. Distinguish by reading `cost_bound_used` against
`cost_limit` in `trace_admission_accounts` (tenant-scoped) and
`trace_admission_global_budget` (guard-scoped). Neither refills. §5.3 is the
remedy for both, and it is not a fast one — which is the reason §2 argues for
choosing generously up front.

**`409 admission_in_progress`** is either a genuinely concurrent submission of
the same id (the advisory lock) or an unexpired lease from a request that died
(§2.5). It clears by itself within `LEASE_SECONDS`.

**`409 admission_identity_conflict`** means the submission id already exists
with different bytes, a different anchor, or different evidence. A client that
retries a changed body under a used id gets this; it is working as intended and
the fix is client-side.

**A refused contribution is the expected failure.** All of the above deny
uploads. None of them admit anything unchecked. If you are debugging this
feature under time pressure, that is the reassurance worth keeping in mind:
leaving it broken is safe, and leaving it off is safer still.

---

## Related

- [`./native-admission-session.md`](./native-admission-session.md) — the
  contributor-side flow, the provider-trust settings in detail, and the exact
  migration/ingest/retention role grants.
- [`./near-native-provisioning.md`](./near-native-provisioning.md) — allocating
  the `near-…` tenant namespace this path requires.
- [`./attested-inference.md`](./attested-inference.md) — the witness pin these
  preconditions assume, and its rollback.
- `deploy/witness/README.md` — deriving receipt signing keys from a
  nonce-bound attestation report, and why a gateway receipt attests bytes
  rather than a model.
- `crates/trace-commons-server/src/admission_ledger.rs`,
  `migrations/V59__trace_admission_ledger.sql` — the limits, the reservation
  state machine, and the runtime permission check, all authoritative over this
  file.
