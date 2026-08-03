# Quarantine review workflow

How to work the quarantine queue. Written 2026-07-30, when the queue had sat at
48 with zero reviews for 71 days.

## What quarantine actually means

A submission whose residual privacy risk is assessed HIGH becomes `quarantined`
with credit held at 0.0, pending review. It is **not** a rejection, and the
contributor-facing message says so — but nothing releases it without a human.

Two facts that determine how to review these, and which are easy to get wrong:

**The flagged content is already redacted.** `redact_text_with_state` ends in
`apply_redaction_ranges`, so `secret:openai_api_key: 2` in `redaction_counts`
means two keys were *found and replaced* with `[REDACTED:...]`, not that two
keys sit in the stored trace. Envelopes are stored as encrypted ciphertext of
the already-redacted payload. There is nothing to purge.

**So `privacy_risk = high` is reserved for scrub failure or unredactable
findings** (a residual post-scrub scan still matching, an object key that
cannot be rewritten, or a residual scan that could not complete). A secret the
redactor found and removed is `medium`: the report is an annotation on a
reviewable record, not evidence that live credentials remain. Reviewing a
High still means judging residual possibility, not deleting content that is
still present; reviewing a Medium is the same posture with an operator path
(`TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS`) that High does not have.

**Approval releases a high-risk trace without changing its risk level.**
`review-decision --decision approve` sets `status = accepted` and leaves
`privacy_risk = high`. This matters: clearing the backlog does **not** depend on
the monotonic-risk work in the re-scrub path. The existing tooling is sufficient
today. Re-scrubbing under the scrub-polarity rule (#219) can additionally move
successful-secret cases from High to Medium without an approve decision.

## Prerequisites

Both are currently unmet and both block everything below.

**1. A reviewer-scoped credential.** `GET /v1/review/quarantine` gates on
`require_reviewer`, and the pilot sets
`TRACE_COMMONS_REQUIRE_EDDSA_SIGNED_TOKENS`. Mint an EdDSA-signed JWT carrying
the reviewer role, signed with `issuer-signing-v1.pem`, `kid` from the published
keyset, with `iss`/`aud` and a `jti`. Same mechanism as the admin token.

**2. The `trace-commons-review` binary.** Not installed on
`tc-pilot-host` — only `trace-commons-ingest`,
`trace-commons-upload-claim-issuer`, and `trace-commons-smoke-envelope` are. It
is a *client*, so build and run it anywhere:

```
cargo build --release --bin trace-commons-review
```

Point it at `https://ingest.tracecommons.ai` with the reviewer token in the
env var named by `--bearer-token-env`.

## Step 1 — decide the 2026-07-11 batch as one decision

**40 of the 45 are a single contributor on a single day**
(`tenant-09629cf4…`, 2026-07-11, 73% quarantine rate for that tenant). The
remaining five are singles: `tenant-zaki-pilot` x2 (07-28),
`tenant-ironclaw-qa` x2 (07-17), `tenant-ed240a85…` x1 (07-09).

Reviewing 40 traces individually to reach the same conclusion 40 times is waste.
Before touching them, establish what that import was:

```
trace-commons-review quarantine-list --privacy-risk high --limit 50 --json
```

Then determine, for that tenant and date: was it a bulk load, one long session,
or a harness misconfiguration? A 73% quarantine rate for one contributor is
itself the finding — either their environment was unusually dirty, or the risk
threshold is miscalibrated for their workload.

Take **one** documented decision for the batch, and record the reason on every
member so the decision is auditable per-trace. Do not batch-claim
(`review-lease-claim-batch`) if you want per-trace reasons; single decisions
carry their own reason and a verifiable response.

## Step 2 — the five singles, individually

Triage by composition rather than uniformly. Census the labels with:

```sql
SELECT submission_id, jsonb_object_keys(redaction_counts) AS k
FROM trace_submissions WHERE status = 'quarantined';
```

Two independent detectors write labels and conflating them gives wrong answers:

- `secret:*` — the **deterministic** detector. Includes `contextual_entropy`,
  which is the false-positive-prone heuristic.
- `privacy_filter:*` — the **NEAR AI prose filter** (person names, emails,
  addresses, account numbers).

Do **not** classify with `redaction_counts::text LIKE '%secret%'` — it matches
`privacy_filter:secret` too. Use `jsonb_object_keys(...) LIKE 'secret%'`.

Suggested dispositions:

| group | disposition |
| --- | --- |
| `contextual_entropy` only, no prose-PII | approve; the heuristic is FP-prone and nothing else fired |
| prose-PII only | judgement call — the PII was found and replaced, so the question is residual risk |
| hard credential shapes (PEM, OpenAI, provider token, JWT) | approve or hold on the trace's merits, **and** raise a contributor notification (Step 3) |
| test fixtures, empty `redaction_counts` | reject; no value and nothing to review |

## Step 3 — notify contributors whose credentials were caught

This is the only genuinely urgent item, and it is a **notification task, not a
data task**.

A trace flagged `secret:openai_api_key` means a real key existed in that
contributor's environment. The key is redacted in our store, but it is still
live wherever they were working. They need to rotate it.

Affected tenants as of 2026-07-30: `tenant-09629cf4…` (self-enrolled
contributor), `tenant-ironclaw-qa`, `tenant-zaki-pilot`, `tenant-ed240a85…`.

**Do not delete these traces.** Deleting destroys the evidence the contributor
needs to identify which key leaked and when.

## Step 4 — record and verify

Every decision should be confirmable in the database, not just in CLI output:

```
sudo bash -c 'set -a; . /etc/tracecommons/ingest.env; set +a;
  psql "$TRACE_COMMONS_GATE_DRIVER_DATABASE_URL" -c "SELECT status,
  privacy_risk, count(*), count(reviewed_at) FROM trace_submissions
  GROUP BY 1,2;"'
```

Use the **cross-tenant gate-driver role**. The `app` role is
`NOBYPASSRLS` under forced RLS and returns 0 rows; setting a tenant GUC shows
one tenant only, and `trace_tenants` is itself RLS'd so tenants cannot be
enumerated as `app`. A casual `SELECT count(*) FROM trace_submissions` reports
an empty table on a corpus of 400+.

Reaching the host needs `gcloud auth login` first — session policy forces
reauth for SSH/IAP even when `gcloud auth list` looks fine — plus
`--project tracecommons-pilot-2026 --zone us-central1-a --tunnel-through-iap`.

## Stopping the queue from refilling

Working the backlog is not the fix. Arrival is bursty — one tenant contributed
40 of the current 45 in a single day — so the queue regrows the moment a
similar contributor appears.

Three levers, in rough order of leverage:

1. **Client-side redaction is missing things the server catches.** That the
   *server* re-scrub found real credentials means the *client* scrubber let them
   through. Every such trace is a client-side gap. Closing those reduces
   arrivals at the source.
2. **Threshold calibration.** A 73% quarantine rate for one contributor suggests
   HIGH may be firing too readily for some workloads. Worth measuring before
   assuming the contributor is at fault.
3. **Automated re-scrub release.** The async backstop (#166) only intercepts
   *new* submissions, and the re-scrub path cannot lower a HIGH risk, so neither
   clears a backlog today. Work to make re-scrub drive risk classification would
   change that — but note it must fix the zero-span classifier response in the
   same change, or an unavailable classifier silently releases traces.

## What not to do

- **Do not bulk-approve.** The composition is genuinely dirty: of the original
  48, 40 carried flagged person names, 32 emails, 31 account numbers, and up to
  ~10 carried concrete credential shapes. This is not a false-positive pile.
- **Do not auto-expire to accepted.** Same reason.
- **Do not leave items unreviewed indefinitely while the CLI tells contributors
  a review is pending.** That is the state this runbook exists to end. If the
  queue is not going to be worked, change the message rather than leaving a
  promise unkept.
