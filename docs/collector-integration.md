# Collector integration

For a platform that scores participants on contributed traces — a hackathon
organizer, or an MCP server acting for one. It covers the sequence to drive,
the machine-readable shapes, and the verification you **must** perform.

## The property that matters

A submission id is not proof of authorship. Anyone who learns an id can put
it in a list and claim the score attached to it. Ids have been published in
plain text before now.

Do not score a participant on a list of submission ids they hand you. Score
them on a **signed attestation**, and verify the signature. A collector that
accepts an unverified attestation is exactly as exposed as one accepting a
raw id list — the signature is the whole control.

## Sequence

Every command below accepts `--json`, which is what you should drive. Human
output is not a stable interface.

### 1. Enroll

```
trace-commons-contributor login --invite '<full invite link>'
```

The invite link is the one the participant was given, fragment included:
`https://issuer.example.ai/onboard#CODE`.

This is **not idempotent** — each success spends one use of the invite. Run
it once. It refuses up front if the device is already enrolled, rather than
burning a use.

Installing the CLI: on macOS and Linux, `scripts/install.sh` fetches a
verified release binary into `~/.local/bin`; on macOS there is also a
Homebrew tap (`brew tap TraceCommons/tap && brew install
trace-commons-contributor`); on Windows, `scripts/install.ps1`, and
`winget install TraceCommons.Contributor`. Building from source
(`cargo build --release -p trace-commons-contributor`) works too, but it is
not the only path and should not be the one you tell participants about. See
the repository README for the full instructions and how to verify a download
by hand. There is no npm or PyPI package.

### 2. Confirm enrollment

```
trace-commons-contributor --json whoami
```

```json
{
  "schema_version": "trace_commons.whoami.v1",
  "instance_id": "...", "tenant_id": "...",
  "device_key_id": "sha256:...",
  "user_subject_hash": "...", "config_dir": "..."
}
```

An error here means the config was not written and nothing will submit.

### 3. Submit

Always dry-run first and show the participant what it contains. Session
transcripts routinely carry credentials pasted while working.

```
trace-commons-contributor --json submit --dry-run --since 7d
trace-commons-contributor --json submit --since 7d
```

```json
{
  "schema_version": "trace_commons.submit_result.v1",
  "results": [
    { "outcome": "submitted", "submission_id": "...", "status": "accepted" },
    { "outcome": "already-submitted", "submission_id": "...", "status": "accepted" },
    { "outcome": "refused", "reason": "secret-leak-detected" },
    { "outcome": "failed", "reason": "claim-mint-failed" },
    { "outcome": "skipped", "reason": "parse-failed" }
  ]
}
```

`outcome` is one of `submitted`, `already-submitted`, `refused`, `failed`,
`skipped`. `reason` is always a fixed label, never a message or a path.

**`already-submitted` is not a failure.** It means the session was delivered
on an earlier run, and `status` carries what the server actually decided —
`accepted`, `quarantined`, or `submitted`. Treat it as a success with that
status. Reporting it as an error is a misreading that has already happened.

The process exits nonzero if anything was refused or failed, in both output
modes. Do not treat exit 0 as the only success signal, or a nonzero exit as
meaning nothing landed.

### 4. Attest

```
trace-commons-contributor --json attest --out attestation.jws
```

```json
{
  "schema_version": "trace_commons.attest_result.v1",
  "attestation": "<compact JWS>",
  "written_to": "attestation.jws"
}
```

Collect **this**, not the id list. `--manifest` still exists for
non-adversarial uses; it is not suitable for scoring.

## Verifying an attestation

The attestation is a compact JWS, EdDSA (Ed25519), with `kid` in the header.

1. Fetch the keyset:
   `GET https://<ingest-host>/.well-known/trace-commons-attestation-keyset.json`

   ```json
   { "keys": [ { "kid": "...", "public_key_pem": "..." } ] }
   ```

   Select the key by the `kid` in the attestation header. **Never by
   position** — the array carries more than one key during rotation.

   A `503` means the server has no signing key configured. Refuse the
   attestation; do not fall back to unsigned data.

2. Verify the signature with that key. Reject on failure. This step is the
   control; everything else is bookkeeping.

3. Check `expires_at` against your clock and reject if past. Do not cache an
   attestation beyond its expiry.

4. Read the payload:

   ```json
   {
     "schema_version": "trace_commons.score_attestation.v2",
     "tenant_id": "...",
     "auth_principal_ref": "...",
     "submissions": [
       { "submission_id": "...", "credit_quality_micros": 0,
         "perplexity_micros": 0, "novelty_score_micros": 0,
         "gate_passed": true,
         "coverage": { "coverage_state": "complete",
                       "chunks_scored": 3, "chunks_total": 3 } }
     ],
     "issued_at": "...", "expires_at": "...", "nonce": "..."
   }
   ```

   `schema_version` is **`trace_commons.score_attestation.v2`**. v1 is no
   longer issued; a verifier that pins the v1 string will reject every
   attestation until it is updated. See "Migrating from v1" below.

   **Ignore top-level fields you do not recognise, within a schema version.**
   A verifier must reject an unknown `schema_version`, and must reject an
   unknown `coverage_state` (step 5) — but it must not reject a document for
   carrying a top-level key it has never seen. Additive top-level fields are
   how this contract grows without a version bump; `pending` and `unknown`
   (see "Scoping an attestation") are the first two, and a verifier written
   before they existed keeps working precisely because of this rule. The rule
   is deliberately narrow: it does not extend to new fields inside
   `submissions` entries or inside `coverage`, where a silently ignored field
   could change what an existing field means.

5. Read `coverage` on every entry. **The scores may describe only part of
   the trace.** The gate splits a large trace into chunks and scores at most
   a fixed number of them; anything past that cap is not scored, and
   `gate_passed` is then a judgment on a prefix. This is not rare: on the
   pilot, 15% of decisions overall and 41% of one recent month's were
   capped, and capped traces pass far less often than uncapped ones.

   `coverage_state` has exactly three values:

   | `coverage_state` | Meaning | `chunks_total` |
   | --- | --- | --- |
   | `complete` | Every chunk was scored. | present, equals `chunks_scored` |
   | `partial` | The cap dropped chunks; the pre-cap total is known. Scores cover `chunks_scored` of `chunks_total`. | present, greater than `chunks_scored` |
   | `partial_unknown_total` | The cap dropped chunks, but this decision predates the column that records the total. How much was omitted is **unknown**. | **absent** |

   `chunks_total` is absent — not zero, not `-1`, not null — when the
   denominator is unknown. The server will not estimate it (for example from
   envelope size): an estimate inside a signed statement would be worse than
   an honest unknown. Do not estimate it yourself and then treat the result
   as attested.

   Treat an unknown state as unknown. A reasonable policy is to accept
   `complete` at face value, discount or review `partial` by its ratio, and
   route `partial_unknown_total` to whatever you do with unverifiable
   claims — but that is your policy call, not something the attestation
   makes for you. Reject an entry whose `coverage_state` you do not
   recognize rather than defaulting it to `complete`; new states may be
   added under a future schema version.

6. Bind `auth_principal_ref` to your own participant record, and reject a
   second participant presenting an attestation for the same ref. `nonce`
   lets you detect a replayed document inside its validity window.

## Scoping an attestation

`GET /v1/contributors/me/score-attestation` attests to everything the calling
contributor has that carries a score. It takes no parameters, and it is
capped: past the cap it returns the contributor's most recent scored
submissions and drops the oldest.

`POST` to the same path, with the contributor's own credential, attests to
**exactly** the ids you name:

```json
{ "submission_ids": ["...", "..."] }
```

At most 500 ids per request; more is a `413`, so chunk. The body carries a
submission-id list and nothing else — an unrecognised field is rejected,
because this endpoint must never grow a way to name a principal other than
the caller.

A scoped response is:

```json
{ "attestation": "<compact JWS>", "scored": 2, "pending": 1, "unknown": 0 }
```

The counts are a convenience. The signed payload is the contract, and it
carries two top-level fields the unscoped document does not:

```json
{
  "schema_version": "trace_commons.score_attestation.v2",
  "submissions": [ { "submission_id": "...", "...": "..." } ],
  "pending": ["..."],
  "unknown": ["..."]
}
```

- `submissions` — asked about, owned by this contributor, scored. Same entry
  shape as always.
- `pending` — asked about, owned by this contributor, **not scored yet**.
  Scoring is asynchronous, so this is the normal state for a trace submitted
  minutes ago, and at a deadline it may be most of them. A `pending` entry is
  a signed statement that the trace exists and is queued — not an absence.
- `unknown` — asked about, and not this contributor's. It says nothing about
  whether the id exists at all, deliberately: this route cannot be used to
  probe for someone else's submission ids.

`schema_version` **stays `trace_commons.score_attestation.v2`**. These fields
are additive and appear only in a scoped response. An unscoped request
returns the document it always did, with no `pending` key and no `unknown`
key — not empty arrays, absent. That is what the forward-compatibility rule
above buys: a verifier built against the unscoped document keeps working, and
sees the new fields only once it opts in by asking a scoped question.

The contributor CLI can produce a scoped attestation as part of a submit run:
`trace-commons-contributor submit --attest-out attestation.jws` requests one
over exactly the traces that run uploaded, waits a bounded time for `pending`
to drain, and writes it either way, reporting `N of M traces scored, K
pending` rather than failing. When a run uploads more than 500 traces the
file holds one signed document per line.

## Reading scores for ids you already hold

A participant who submits at the deadline hands you an attestation dominated
by `pending`. Resolving those later is a collector-side read, not something
the participant has to come back for.

```
POST /v1/admin/scores-by-submission
Authorization: Bearer <competition read worker token>
```

```json
{ "submission_ids": ["...", "..."] }
```

```json
[ { "submission_id": "...", "credit_quality_micros": 0,
    "perplexity_micros": 0, "novelty_score_micros": 0,
    "gate_passed": true } ]
```

At most 500 ids per request (`413` past that). Ids with no gate decision are
**omitted from the response** — this route has no `pending` signal, so treat
an absent id as "still unscored, ask again later" rather than as a refusal.
Ask again rather than assuming: a submission can also exhaust its scoring
attempts, in which case it never appears here.

The credential is a token with role `competition_read_worker`, issued by the
operator running the ingest deployment. It is scoped to this read: it cannot
submit, review, export, or reach any other admin route, and it is not the
contributor credential the participant holds. An `admin` token also works;
prefer the narrow one. Anything else gets a `403`.

Two limits worth knowing before you build on it:

- The read is by `submission_id` across every contributor's tenant, so it
  **does not tell you whose submission an id is**. Only the attestation binds
  ids to an `auth_principal_ref`. Read scores here for ids you already know
  belong to a participant, from an attestation you already verified; do not
  use this route to discover ownership.
- The entries here carry no `coverage`. A score read back this way may still
  describe only part of a trace, and this response will not say so — the
  attestation for the same id will.

## Migrating from v1

v2 is a deliberate breaking change to this contract. What changed:

- `schema_version` is now `trace_commons.score_attestation.v2`.
- Every entry in `submissions` carries a new required `coverage` object.
- Nothing was removed or renamed; the five v1 score fields are unchanged.

**v1 attestations are no longer issued.** There is no compatibility mode and
no way to request a v1 document.

To migrate:

1. Accept `trace_commons.score_attestation.v2` where you previously required
   `...v1`. If you accept a set of versions, drop v1 from it once your
   collector is deployed — nothing will ever present a v1 document again, and
   continuing to accept the string only leaves a hole for a forged one.
2. Parse `coverage` per entry and decide what a partially scored trace is
   worth to you. If you do nothing else, at minimum stop treating
   `gate_passed` as a statement about a whole trace.
3. Handle `partial_unknown_total` explicitly. It is the honest report for
   decisions made before the server recorded the denominator, and those rows
   will never gain one.

Everything else — keyset fetch, signature verification, expiry, and the
`auth_principal_ref` binding — is unchanged.

## What this does and does not prove

**Proves:** the listed submissions belong to the contributor identified by
`auth_principal_ref`, and carried these scores when the attestation was
issued.

**Does not prove** that the participant submitted everything they produced.
Attestations cover what is claimed, not completeness — a participant may
withhold traces.

**Does not prove** that a score covers a whole trace. Read `coverage`; a
`partial` or `partial_unknown_total` entry was scored on a prefix.

**Does not bind** the contributor to an external identity. Mapping
`auth_principal_ref` to a platform account is your join. The most robust
place to make it is at enrollment, inside a flow you already authenticate:
if your MCP drives step 1, record the resulting identity there rather than
accepting it from the participant later.

## Failure modes

| Symptom | Meaning |
| --- | --- |
| `login --invite` refuses, already enrolled | The device has config. `logout` first, or reuse the enrollment. |
| `403 InviteAlreadyConsumed` | Invite uses exhausted. Issue a new invite. |
| Every session `already-submitted` | Normal on a re-run. Read `status` per entry. |
| `refused` with `secret-leak-detected` | Redaction found a residual secret and fail-closed. Not retryable as-is. |
| `attest` returns 503 | Server signing key unconfigured. An operator problem; do not proceed with unsigned data. |
| Scoped attestation returns `413` | More than 500 ids in one request. Chunk them. |
| Attestation `pending` never empties | Scoring is asynchronous and can be disabled or backlogged. Take the attestation as it is and resolve the ids later via `POST /v1/admin/scores-by-submission`. |
| `scores-by-submission` omits an id | No gate decision yet, or scoring exhausted its attempts. Not a refusal; retry, then escalate to the operator. |
| `scores-by-submission` returns `403` | Token is neither `admin` nor `competition_read_worker`. |
