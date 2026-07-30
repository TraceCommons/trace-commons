# Gate trust evidence — design

## Goal

Replace backend-name string matching as the basis for credit-bearing gate trust
with **explicit trust evidence**: a structured, verifiable, expiring record of
what boundary a gate decision was produced in, which measurement that boundary
reported, and which endpoint and model produced the score. Credit-bearing
decisions require verified evidence. Backend names never stand in for
attestation.

This spec delivers the trust model, its persistence, and the credit gate. It
does **not** implement a TEE quote verifier for any specific provider — that is
sequenced behind the model landing, because two of the three boundaries we care
about cannot produce evidence today.

## Background: what the code actually does

Verified at `origin/main` on 2026-07-29.

`is_production_gate_service_kind`
(`crates/trace-commons-server/src/bin/trace-commons-ingest.rs:47250`) is the sole
credit-trust decision:

```rust
fn is_production_gate_service_kind(kind: &str) -> bool {
    matches!(kind, "enclave_local_gpu" | "dstack")
}
```

It is consulted from `attempt_emit_novelty_utility_credit`
(`trace-commons-ingest.rs:47150`), gated on
`TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE`.

Three things are wrong with it.

**The trusted list names a backend that does not exist.** The kinds actually
produced by `safe_status().kind` are `in_memory`, `legacy_deterministic`,
`dstack_stub`, `enclave_mock`, `enclave_local_gpu`, and `enclave_near_ai`.
Nothing ever reports `dstack`: `DstackGateService` reports `dstack_stub`
(`trace_gate_service.rs:519`) and fails every call. `dstack` is dead
classification.

**It excludes the backend the pilot runs.** `enclave_near_ai` is what
`build_enclave_near_ai_gate_service_from_env` constructs and what the pilot
deploys, and it is not in the list. Documentation describes Phase A as running
inside NEAR AI's TEE-hosted vLLM (`docs/trace-commons.md:66-68`).

**Nothing behind the name is verified.** `attestation_verifier_configured` is
hardcoded `false` at *every* site that constructs a `GateServiceStatus` —
`trace_gate_service.rs:397`, `:461`, and `EnclaveGateService::safe_status()` at
`:718`. No gate service in the tree reports a configured attestation verifier,
so the field cannot currently be true for any backend. The stored
`attestation_chain_hash` is
computed over policy and version strings only —
`hash_attestation_chain(gate_policy_version, gate_version_hash)`
(`crates/trace-commons-gate-enclave/src/orchestrator.rs:379-385`, and the
parallel construction at `trace_gate_service.rs:273`). It contains no quote, no
measurement, no endpoint, no model identity. Two deployments pointed at entirely
different scoring endpoints produce the same attestation hash.

## Why the one-line fix is unsafe

The obvious repair is to add `enclave_near_ai` to the match arm. Do not do this.

`NearAiScorerConfig::validate`
(`crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs:80-101`) accepts
any non-empty `base_url` that does not end in `/`. There is no scheme check, no
host allowlist, no pinning. The scorer then posts trace plaintext with a bearer
token to whatever that URL names
(`perplexity_near_ai.rs:165-180`). Combined with
`attestation_verifier_configured: false` and an attestation hash covering
nothing, adding the match arm would extend credit-bearing trust to an arbitrary
operator-supplied HTTP endpoint that nobody verifies.

The correct fix is to stop deciding trust from the backend name at all.

Note also that the current arm trusts `enclave_local_gpu` on the same basis —
its name. Nothing about the local GPU path is attested either. The model below
therefore **removes** trust from `enclave_local_gpu` as well, until it produces
evidence. That is a real behavioural change and is intended: the present
"production trust" for local GPU is an assertion, not a verification.

## Blast radius today

`NoveltyUtility` was removed from settlement eligibility and its emission delta
defaulted to zero (PR #180), so no credit currently reaches on-chain issuance
through this path regardless of trust classification. This work is therefore not
an emergency patch — it is the precondition for ever turning positive issuance
back on, and for any future credit type that wants to consult a trust boundary.

## Model

### Boundary

What kind of execution environment produced the decision. Descriptive, not
trusted on its own.

```rust
enum GateTrustBoundary {
    /// No isolation claim. In-process deterministic services.
    None,
    /// Local process with model weights on the host. No remote attestation.
    LocalProcess,
    /// Local GPU enclave.
    LocalGpuEnclave,
    /// Third-party TEE-hosted inference, identified by provider label.
    RemoteTee { provider: String },
}
```

### Status

```rust
enum GateTrustStatus {
    /// No evidence was presented. Decisions are usable for gating.
    /// Positive credit is withheld.
    Unverified,
    /// Evidence presented, validated, and within its freshness window.
    Verified,
    /// Evidence validated previously but is now past `expires_at`.
    Expired,
    /// Evidence presented and failed validation. Distinct from Unverified:
    /// something claimed to be attested and was not.
    Rejected,
}
```

`Rejected` is deliberately separate from `Unverified`. "Never claimed" and
"claimed and failed" are different operational events and must be distinguishable
in the decision row without reading logs.

### Evidence

```rust
struct GateTrustEvidence {
    boundary: GateTrustBoundary,
    status: GateTrustStatus,
    /// sha256 over the canonical serialization of the full evidence
    /// document, including the raw quote. The quote itself is never stored
    /// in a decision row.
    evidence_hash: String,
    /// Boundary measurement: MRENCLAVE, RTMR set, or compose hash,
    /// depending on the provider. Compared against an allowlist.
    measurement: Option<String>,
    /// sha256 of the normalized scoring endpoint. Hash-only: the raw URL
    /// is operator-configuration and never enters a stored row or log line.
    endpoint_hash: Option<String>,
    /// sha256 of provider + model identifier + revision when the provider
    /// exposes one. Binds the decision to the weights that produced it.
    model_hash: Option<String>,
    /// Identifier of the verifier that validated this evidence.
    verifier_id: Option<String>,
    verified_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}
```

All identifying material is hashed, per the repo's hash-only audit convention.
`endpoint_hash` exists so an operator can prove a decision came from the endpoint
they pinned, without the endpoint appearing in the database.

### The credit predicate

Replaces `is_production_gate_service_kind` entirely. The backend name is not an
input.

```rust
fn gate_trust_permits_credit(
    evidence: &GateTrustEvidence,
    policy: &GateTrustPolicy,
    now: DateTime<Utc>,
) -> Result<(), GateTrustDenial>
```

All of the following must hold:

1. `status == Verified`
2. `expires_at` is present and `now < expires_at`
3. `now - verified_at <= policy.max_evidence_age`
4. `measurement` is present and in `policy.allowed_measurements`
5. `endpoint_hash` is present and in `policy.allowed_endpoint_hashes`
6. `model_hash` is present and in `policy.allowed_model_hashes`

Any failure returns a label-only `GateTrustDenial` that becomes the decision
row's `credit_withheld_reason`, following the existing label convention:
`gate_trust_unverified`, `gate_trust_expired`, `gate_trust_rejected`,
`gate_trust_measurement_not_allowed`, `gate_trust_endpoint_not_pinned`,
`gate_trust_model_not_pinned`, `gate_trust_evidence_stale`.

An empty allowlist denies. It never means "allow everything".

### Fail-closed on missing verifier

If `TRACE_COMMONS_GATE_TRUST_REQUIRE_VERIFIED_FOR_CREDIT` is on and no verifier
is configured for the active boundary, credit is refused with the missing-control
name `gate_trust_verifier`. The path never silently downgrades to
`Unverified`-but-paid. This matches the central-issuer profile's
missing-control convention.

## Phase A disposition

Phase A NEAR AI scoring stays fully usable. It gates: it decides accept,
quarantine and reject, it writes decision rows, it feeds novelty and perplexity.
Its evidence is `boundary: RemoteTee { provider: "near_ai" }`, `status:
Unverified`, and positive credit is withheld with `gate_trust_unverified`.

This is the same practical outcome as today — `enclave_near_ai` earns no credit
under the current match arm either — but for an accurate reason, recorded on the
row, rather than as a side effect of an incorrect list.

## Provider evidence contract

For a remote TEE boundary to reach `Verified`, the provider must expose, bound
into a single attested document:

1. **A quote** over the TEE measurement, verifiable to a hardware root of trust.
2. **The model identity** — provider model id plus a weights revision or digest.
   Without this, a verified enclave can still swap the model.
3. **A channel binding** — the TLS public key or a session key committed inside
   the quote, so the attested enclave is provably the endpoint we talked to and
   not a relay in front of it.
4. **Freshness** — a nonce we supply, or a signed timestamp with a stated
   validity window.

Items 2 and 3 are the ones commonly missing from hosted-TEE inference offerings.
An attestation that proves "some enclave of this image exists" without binding
the model or the channel does not support credit issuance, and this spec should
not pretend otherwise: such evidence validates to `Unverified`, not `Verified`.

Until NEAR AI exposes this, the honest state is `Unverified`. Sequencing the
verifier behind the provider contract is deliberate.

## Persistence

Add to the gate decision row:

- `gate_trust_status` (text, not null, default `'unverified'`)
- `gate_trust_boundary` (text, not null, default `'none'`)
- `gate_trust_evidence_hash` (text, nullable)
- `gate_trust_measurement` (text, nullable)
- `gate_trust_endpoint_hash` (text, nullable)
- `gate_trust_model_hash` (text, nullable)
- `gate_trust_verified_at` (timestamptz, nullable)
- `gate_trust_expires_at` (timestamptz, nullable)

Existing rows default to `unverified` / `none`. They were produced under a trust
model that did not record evidence and must not be readable as verified.

`run_migrations` is hand-rolled — the new migration has to be wired in
explicitly, not just dropped in `migrations/`.

### Attestation chain hash v2

`hash_attestation_chain` currently covers only `gate_policy_version` and
`gate_version_hash`. Version it to `trace_gate_enclave.attestation_chain.v2` and
include `boundary`, `status`, `measurement`, `endpoint_hash`, `model_hash`,
`verifier_id`, `verified_at` and `expires_at`. The domain-separation prefix
changes so v1 and v2 hashes are never confusable, and a v1 hash on a row is
positive evidence that the row predates evidence recording.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TRACE_COMMONS_GATE_TRUST_REQUIRE_VERIFIED_FOR_CREDIT` | `true` | Fail closed. Off is a deliberate dev-only downgrade. |
| `TRACE_COMMONS_GATE_TRUST_ALLOWED_MEASUREMENTS` | empty | Allowlist. Empty denies. |
| `TRACE_COMMONS_GATE_TRUST_ALLOWED_ENDPOINT_HASHES` | empty | Allowlist. Empty denies. |
| `TRACE_COMMONS_GATE_TRUST_ALLOWED_MODEL_HASHES` | empty | Allowlist. Empty denies. |
| `TRACE_COMMONS_GATE_TRUST_MAX_EVIDENCE_AGE` | `24h` | Freshness bound independent of `expires_at`. |

`TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE` is superseded. Keep it
parsed for one release, warn when set, and have it imply
`REQUIRE_VERIFIED_FOR_CREDIT`.

## Non-goals

- Implementing a dstack client. `dstack_stub` keeps failing closed; the dead
  `dstack` classification is deleted rather than made real.
- Verifying NEAR AI. Blocked on the provider contract above.
- Changing gate floors, novelty, perplexity, or dedup.
- Re-scoring historical decisions. Existing rows stay `unverified`.

## Implementation sequence

1. **Types and plumbing.** `GateTrustBoundary`, `GateTrustStatus`,
   `GateTrustEvidence`, `GateTrustPolicy`, `gate_trust_permits_credit`. Every
   backend reports `Unverified`. Replace `is_production_gate_service_kind` at its
   single call site. Delete the `dstack` arm. Behaviour-preserving for
   `enclave_near_ai` and the stubs; **removes** credit trust from
   `enclave_local_gpu`, which is the intended correction.
2. **Persistence.** Migration, decision-row writes, v2 attestation hash.
3. **Local GPU evidence.** Self-attested boundary: process measurement plus a
   digest over the loaded weights. Reaches `Verified` only against a configured
   measurement allowlist. This is the first boundary that can actually produce
   evidence without a third party.
4. **Provider evidence contract.** Drive items 1–4 above with NEAR AI. Implement
   the verifier only once the provider can satisfy them.
5. **dstack**, if and when it becomes a real deployment target.

Steps 1 and 2 are self-contained and worth landing together. Step 3 is where
`Verified` first becomes reachable.

## Open questions

- **Evidence lifetime.** Per-decision attestation is the strongest binding and
  the most expensive. Per-session with a short window is the likely compromise.
  The `expires_at` plus `max_evidence_age` pair is deliberately built to support
  either, but the operational choice is unmade.
- **Who owns the verifier.** A verifier for a provider's TEE is security-critical
  code. If it is contributed by a party with an interest in that provider being
  trusted, the review bar needs to be set accordingly, and the measurement
  allowlist should stay operator-controlled regardless.
- **Retroactivity.** Whether decisions already made under `enclave_near_ai`
  should be re-evaluated if that backend later reaches `Verified`, or whether
  trust is strictly forward-dated. This spec assumes forward-dated.
