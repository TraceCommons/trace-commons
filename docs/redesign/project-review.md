# Trace Commons Review

A cleaned-up reading of the system as of 2026-08-24, from the notes of a
walkthrough of this repo. Authoritative contracts remain
[`docs/trace-commons.md`](../../trace-commons.md),
[`docs/trace-spec.md`](../../trace-spec.md),
[`docs/trace-commons-storage.md`](../../trace-commons-storage.md), and
[`docs/trace-commons-roadmap.md`](../../trace-commons-roadmap.md).

## 1. What is Trace Commons

Trace Commons is a **user-owned register of AI agent work**. Contributors
opt in to file a locally scrubbed record of what an agent actually did —
user turns, tool calls, results, reasoning, outcomes. Frontier labs,
auditors, and regulators can later query that register under selective
disclosure. They see a filtered projection of accepted envelopes, not raw
sessions.

The point of the product, relative to “the model vendor already has your
logs,” is:

- contribution is **off by default**
- capture and redaction happen **on the contributor’s machine**
- only a versioned envelope (`ironclaw.trace_contribution.v1`) is uploaded
- two gates decide whether a record is worth keeping (novelty and substance)
- accepted records can later earn **non-transferable Trace Credits**

This repository is the hosted control plane (`trace-commons-ingest`,
upload-claim issuer, gate, storage, credit settlement) plus
`trace-commons-contributor`, a CLI humans run against Claude Code / Codex /
Letta Trajectory sessions. Ironclaw is a **separate** TEE-hosted trace
source. It is not in this tree. Shared DTOs live in
`crates/trace-commons-protocol`.

An **agent trace** is one session’s trajectory: an ordered list of events
(user messages, assistant messages, tool calls/results, reasoning, …). It
is not a second envelope type.

## 2. How it works (summary)

```
 local session (Claude Code / Codex / Trajectory / Ironclaw)
        │  opt-in + deterministic redaction
        ▼
 envelope  ironclaw.trace_contribution.v1
        │  short-lived EdDSA upload claim
        ▼
 ingest  re-scrub → residual-risk class → novelty gate → substance gate
        │
        ├── accepted   encrypted object + Postgres metadata + pending credit estimate
        ├── quarantined  held for review (typically medium/high residual PII risk)
        └── rejected   failed a gate
                │
                ├── delayed utility credit (benchmark / ranking / training / regression)
                ▼
         settlement (central issuer) → off-chain ledger + optional NEAR receipt
                │
                ▼
         customer export  filtered projection, consent ∩ allowed-use
```

**Contributor path.** The person (or Ironclaw workload) is not “the harness
POST-ing JSON.” In this repo, `trace-commons-contributor` discovers local
session files, redacts secrets/paths, optionally runs a NEAR AI PII pass,
mints a short-lived upload claim, and submits the envelope. A background
daemon can queue and flush. Identity is a registered **device key** (or a
workload JWT), not a long-lived ingest password.

**Ingest path.** The server treats every upload as untrusted. It re-scrubs,
binds storage to the **auth-derived tenant** (envelope tenant fields are
attribution only), then runs the two gates. “Safe” is **not** what the gate
LLM is for. Residual PII risk, re-scrub, and quarantine are a separate
privacy path. The gate asks only: is this **novel** vs the register, and is
it **substantive** vs filler.

**Storage.** Accepted (and quarantined) envelopes go to an encrypted
**artifact store** (local-encrypted for dev, filesystem-remote, or GCS).
Metadata, audit, grants, credit, and RLS-scoped rows live in **PostgreSQL**.

**Credit.** Uploads do not pay. At accept, ingest records a **pending
estimate** from a local scorecard. Later, privileged jobs append **delayed
utility** events when a trace is actually used. A **central issuer**
principal later **settles** eligible delayed events into non-transferable
account credit and may mirror a hash-only receipt onto NEAR. There is no
cash payout, token, or exchange rate in this codebase.

**Customers** (the spec’s word; README also says buyers). They are not
onboarded with invite codes. They query through **export / replay** routes
with a token whose `allowed_uses` intersect the contributor’s consent
scopes. **Tenant access grants** `(tenant, principal, role)` are a second
hosted-tenant gate. Contributors can **revoke / withdraw** a submitted
trace; that invalidates derived use. Invite codes are for **contributor
onboarding** to the upload-claim issuer, not for customer query.

## 3. Domain model

### How the pieces fit

```
                    ┌─────────────────────────┐
                    │ upload-claim issuer     │
                    │ onboard / mint JWT      │
                    │ publish Ed25519 keyset  │
                    └───────────┬─────────────┘
                                │ Bearer upload claim
 ┌──────────────────┐           │
 │ contributor CLI  │───────────┤
 │ or Ironclaw      │           ▼
 └──────────────────┘   ┌───────────────────┐     ┌─────────────────┐
                        │ ingest            │────▶│ gate orchestrator│
                        │ review / workers  │     │ novelty+substance│
                        │ export / admin    │     └────────┬────────┘
                        └─────────┬─────────┘              │
                    ┌─────────────┼─────────────┐          │
                    ▼             ▼             ▼          ▼
              PostgreSQL     object store    NEAR outbox  vector index
              (RLS, credit,  (encrypted      (receipt     (per-tenant
               grants, audit) envelopes)      mirror)      chunks)
```

Binaries in this repo: `trace-commons-ingest`,
`trace-commons-upload-claim-issuer`, `trace-commons-gate-calibrate`,
`trace-commons-pilot-bootstrap`, `trace-commons-contributor`.

### Contributor (client)

- **Who:** the human (or Ironclaw workload) who files traces.
- **This repo’s client:** `trace-commons-contributor` (`login`, `list`,
  `submit`, `status`, …). Reads:
  - Claude Code: `~/.claude/projects/<encoded-cwd>/<session>.jsonl`
  - Codex: `~/.codex/sessions`
  - Letta Trajectory v1: only if `--trajectory` is passed (never discovered
    implicitly)
- **Ironclaw:** separate repo, TEE-hosted source. Protocol crate is the
  shared envelope. Wiring Ironclaw onto this server is still open work.
- **Wire format:** only `ironclaw.trace_contribution.v1`. Trajectory / Claude
  / Codex files are **source adapters**. They are converted into that
  envelope. There is no second on-the-wire envelope.
- **Identity:** after `login` / onboard, a device Ed25519 key. Claims are
  minted per submit. Principal on the claim is the device (or
  `instance:{tenant}:{device}:user:{subject}`).
- **Queue:** daemon / `submit` flush. Not a required always-on harness hook.

### Envelope / trace

Stored as `ironclaw.trace_contribution.v1`. The trajectory is `events[]`.

Included when message text is declared present (contributor CLI includes
redacted prose by default):

- user messages, including the opening prompt
- assistant messages
- reasoning (`thinking` / Codex `reasoning` / Trajectory `role: reasoning`),
  unless `--no-reasoning`
- tool calls and results (payloads vs names depend on consent flags)

Not included as text:

- harness **system** records (Claude `type: system`, Codex non-user/assistant
  roles) — mapped to Opaque, record-type marker only, no payload
- raw secrets/paths (placeholders after local redaction)

`redacted_content` is a factual declaration (`message_text_included` /
`tool_payloads_included`). Including message text raises residual PII risk
to at least `medium` on typical deployments, which **quarantines** rather
than auto-accepting.

### Tenant, principal, role

| Term | Meaning |
|---|---|
| **Tenant** | Isolation unit: one corpus, one policy, one credit ledger, one RLS scope (`trace_current_tenant_id()`). Object keys and audit rows are tenant-scoped. |
| **Principal** | Authenticated actor (device, workload, reviewer, admin, worker). Stored as `principal_sha256:…`, never the raw token. |
| **Role** | What that actor may do in that tenant: `contributor`, `reviewer`, `admin`, plus scoped workers (`export_worker`, `utility_worker`, …). |

They join as **access grants** `(tenant_id, principal_ref, role)` plus
optional scope/use ceilings. Envelope `tenant_scope_ref` /
`pseudonymous_contributor_id` are **not** authorization inputs.

An upload claim binds: principal P, role `contributor`, tenant T, consent
scopes and allowed uses, until `exp`.

### Register

The filed corpus, not a single table. Postgres holds metadata, status,
grants, credit, audit, gate decisions. The encrypted object store holds
envelope bodies (GCS when compiled with `gcs-client`, else filesystem or
local-encrypted). Vector index holds per-chunk embeddings for novelty.
Customers never bulk-download the object store.

### Gate

Two independent floors. **Both** must pass. They are not a safety
classifier.

| Axis | Question | Mechanism | Pilot floor |
|---|---|---|---|
| **Novelty** | Have we filed this already? | Embed rendered event text; novelty = `1 − max cosine similarity` vs the tenant’s vector index. Exact canonical-summary hash is the strongest duplicate signal. | `NOVELTY_FLOOR_MICROS=500000` (0.5) — this is the floor that currently gates |
| **Substance** | Is this real work vs boilerplate? | LLM logprobs on the same rendered text → aggregate perplexity and tail-fraction. Pass if each is ≥ its floor. | Both default **0** (off) pending calibration |

Novelty is **not** perplexity. Perplexity is the substance proxy.

On small instruct models, mean perplexity was inverted (AUC ≪ 0.5): those
models find OASST-style reasoning *less* surprising than Wikipedia
boilerplate, so a positive perplexity floor would reject real work. A later
27B bake-off (Qwen 3.6 27B Dense, AUC 0.936) showed the metric can
discriminate at that size. A2.7 is “calibrate and maybe turn the perplexity
floor on for a 27B-class scorer,” not “substance is abandoned.”

Token rarity is a deferred A.5a *candidate* replacement for 8B-class
scoring. It is not the production predicate.

Inference failure **fail-closes** (refuses the evaluation). A floor of 0
still passes a zero score, which is why ingest refuses to start if all
three floors are zero; novelty currently satisfies that.

### Upload-claim issuer

Standalone binary. Holds the Ed25519 signing key. Mints short-lived JWTs
(`POST /v1/trace-upload-claim`), publishes
`/.well-known/trace-commons-ed25519-keyset.json`, and runs onboarding
(`POST /v1/onboard`). Ingest **verifies**; it does not mint.

Auth paths: workload JWT (Ironclaw), device-key signature over the request
body, or device JWT. Tenant on the workload path comes from the token, never
from the request body. Pilot invites / device registry / optional tenant
grants further restrict who gets a claim.

Ingest still applies its own policy (tenant submission policy, access
grants, consent, gates) after the JWT verifies.

### Claim / JWT

A JWT is `header.payload.signature`. The upload claim’s payload is
`UploadClaimClaims`: `iss`, `aud`, `sub` / `principal_ref`, `tenant_id`,
`role: contributor`, `iat` / `exp` / `jti`, scopes, uses. Default TTL 300s.
Ingest caches the issuer public keyset over guarded HTTPS. Anyone who holds
the issuer private key can mint a valid claim; ingest is configured to
trust that keyset.

### Central issuer

Not a binary. An **allowlisted principal**
(`TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS` =
`principal_sha256:…`) that ingest will let write **positive credit** and
finalize **settlement**. Upload-claim issuer = “may this contributor talk to
ingest?” Central issuer = “may this operator turn delayed utility into
settled credits?”

Live settlement also requires (when the profile flag is on) source-list
approval, caps, policy-version allowlist, managed EdDSA, grants, NEAR
adapters, rollout-smoke. Unlisted admins can dry-run. They cannot mint.

That does **not** force them to pay. They can refuse to settle. Friend-minting
is what the allowlist, source-list hash, audit rows, and caps constrain.
Under-crediting is the residual operator-trust assumption.

### Trace Credit

A signed record that a contribution was accepted **and** (for settlement)
that delayed utility was proven. Non-transferable. Server ledger is
authoritative; NEAR is a receipt mirror (`settle_credit_receipt` /
`reverse` / `freeze` / `unfreeze` only — no transfers).

```
 accept
   → pending estimate (scorecard: quality, replayability, capped novelty, …)
   → NOT settled

 delayed ledger (privileged)
   → benchmark_conversion, ranking_utility, training_utility, regression_catch
   → reviewer_bonus / abuse_penalty (penalties are not settlement-eligible)

 settlement (central issuer)
   → eligible delayed events only (not novelty_utility, not the pending estimate)
   → per-account line items + optional NEAR outbox
```

`novelty_utility` still accrues as a signal. It **cannot settle**.

Contributor-facing copy: credit is a **record**, not currency. No payout, no
token, no exchange rate, no date.

### Settlement

A governed batch job (`POST /v1/admin/credit-settlements` or the worker
route):

1. Dry-run / drill → canonical `source_list_hash`
2. Listed principal records approval for that exact list
3. Live run (per-tenant lock) selects unused, positive, eligible delayed
   events on still-accepted submissions, excluding held accounts, applying
   the per-account cap
4. Writes a `finalized` batch; optionally enqueues hash-only NEAR receipts
   to a designated payout NEAR account

“Payout destination” = which NEAR account the **receipt** is attributed to.
It is not a NEAR token transfer and not fiat.

### Customers (consumers / buyers)

Spec name: **customers**. They receive a **filtered envelope projection**
via replay-export, never the raw envelope, and only when:

1. status is `accepted` (and not revoked/expired/purged for live use)
2. residual PII risk is `low`
3. contributor consent scopes grant the requested **allowed-use**
4. the customer token (and hosted grant) also carries that allowed-use

Effective permission is the **intersection**; a grant can only narrow.
`public_attribution` is leaderboard/handle only — it grants no trace-content
uses.

Invite codes are unrelated. Those onboard **contributors**.

### Payments

Settled balances are a non-tradable IOU. A designed but **unbuilt** close
would convert points at settlement using realized buyer query revenue
(`points × revenue / points in bucket`). That is
[`2026-05-29-market-signal-credit-design.md`](../specs/2026-05-29-market-signal-credit-design.md).
Nothing in the running server bills customers or pays contributors.

## 4. Trust model

Authorization is always from the **authenticated request**, never from
envelope fields.

| Boundary | Who is trusted with what |
|---|---|
| Contributor device | Raw session never leaves. Redaction is local. Device key proves submits. |
| Upload-claim issuer | Minting contributor JWTs. Ingest trusts its published keyset. |
| Ingest operators | Storage, policy, review, **whether to settle credit**. Phase A KEK is cloud KMS — weaker than Phase B. |
| Gate scorer (Phase A) | NEAR AI TEE-hosted vLLM. Operators of ingest should not see plaintext; the KEK/TEE story is documented as incomplete vs Phase B. |
| Central issuer principal | Only listed hashes may mint/settle positive credit. Audited, source-list bound. |
| Customers | Filtered export under consent ∩ use. No bulk dump. |
| NEAR | Non-transferable mirror. Does not force payout. |

Fail-closed: missing keys, stale allowlist, required grants, incomplete
central-issuer profile, scorer errors. Hash-only logs and audit. PostgreSQL
**FORCE RLS** on Trace Commons tables.

Phase B: move scoring into an attested enclave operators cannot read. That
does not make credit settlement trustless.

## 5. Roadmap (compressed)

Phase A (this repo): hosted ingest, contributor CLI, NEAR AI scoring, cloud
KMS as KEK, centralized credit issuance. Pilot-shaped, not the end state.

Open / blocking (see the roadmap’s Production Gap Queue):

- Calibrate and possibly enable a 27B-class **perplexity floor** (A2.7)
- Tail-fraction floor after real pilot decision rows exist
- Ironclaw client rewire onto `trace-commons-protocol`
- Production KEK (roadmap: only `LocalMasterKeyWrapper` is implemented;
  production startup can fail closed without a real wrapper)

Deferred: A.5 perplexity-replacement (rarity) unless the pilot retreats to
8B hardware; market-signal / revenue-share conversion of credits to money.

## 6. Major issues

**Novelty and substance are fragile proxies.** Novelty is embedding
distance; substance is model surprise. Both are content-surface metrics.
Surprising text is cheap to fabricate. Requiring both does not save you —
they fail in the same direction. The pilot currently leans on the novelty
floor; substance floors are off except as stored numbers. A 27B scorer may
make perplexity usable; 8B scorers did not.

**Credit amounts are policy constants, not a market.** Delayed deltas are
fixed env weights (`BENCHMARK_CONVERSION_CREDIT_POINTS_DELTA`, etc.). The
online scorecard is local heuristics. There is no buyer-demand table and no
clearing price. Settlement decides *which delayed events become final*, not
what a point is worth in money.

**Monetary reward is undefined.** Credits cannot be transferred, sold, or
withdrawn as cash. The intended loop (buyers pay to query → points convert
to a share) is not implemented. Contributors are asked to file traces for
the commons; the IOU is bookkeeping plus an optional public receipt.

**Settlement is operator-trusted.** The central issuer cannot easily mint
in the dark to arbitrary admins. They can still never settle, hold
accounts, or starve caps. That is a product/governance fact, not a bug in
the JWT layer.

**Privacy vs training value.** Reasoning is in the envelope by default and
is the least sanitized slice (it quotes files the assistant message never
does). Message text raises residual risk into quarantine. Customers only
see `low`-risk accepted projections. Those tensions are structural.

**Two “issuers.”** Easy to conflate. Upload-claim issuer is a service that
authenticates contributors. Central issuer is an allowlisted admin who
settles credits.
