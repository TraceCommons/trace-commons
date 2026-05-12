# Trace Commons KEK Strategy — Design Brief

Date: 2026-05-11 (initial), 2026-05-12 (path chosen)
Status: **Chosen path: A (cloud KMS) for pilot; B1 or C (dstack-based) for the trust upgrade after pilot.**
Owner: Trace Commons / Auth + Keying lane
Predecessor: `2026-05-11-cloud-trace-artifact-provider-design.md` (shipped the
`KmsKeyWrapper` trait surface + `LocalMasterKeyWrapper` dev impl + production
refusal gate)

> **Update 2026-05-12:** After scoping the dstack-resident gate work
> against the available hardware timeline, the pragmatic decision is to
> ship a real pilot on path **A (cloud KMS, GCP first)** with real models
> on regular GPUs, and migrate to a dstack-based KEK + in-enclave gate
> service as a follow-up (paths B1 or C from the framework below). This
> regresses the trust model versus the original "operator-constrained
> from day one" goal: under cloud KMS the operator (and the cloud
> provider) can read every contributor's trace via KMS Decrypt. The
> pilot must be operated by an actor whose visibility into trace
> content is acceptable, and contributor-facing language must be honest
> that TEE-rooted privacy is a planned upgrade, not a current property.
> Migration cost when dstack is ready: one re-wrap pass over every DEK
> (the wrap format already includes `wrapper_kind` so v2 envelopes are
> forward-compatible) plus a swap of the binary's hosting location.
> No schema, envelope-format, or trait changes. The framework and
> options below stay as-is for the eventual upgrade; the chosen-path
> note above governs near-term decisions.

## What this document is

The cloud trace artifact provider work shipped the trait, the wrapping format,
the v1/v2 schema gating, the production-refusal startup gate, and the GCS
backend. It deliberately did not pick a KEK vendor. This brief is the
framing document for that choice. It is **not** an implementation spec — that
arrives once the platform is selected.

## Why the KEK choice is load-bearing

The KEK decides who can decrypt a contributor's trace bytes. Everything else
in the Trace Commons threat model — hash-only audit, central-issuer fail-
closed, no plaintext fallback, RLS on every tenant table — operates under
"the operator is constrained, not trusted." The KEK is where that posture
either succeeds or collapses.

- A cloud-KMS KEK trusts the operator (and the cloud provider) to honor
  `Decrypt` calls only when their IAM policy says so. Operators with KMS
  decrypt permission can read every artifact at will.
- A TEE-rooted KEK trusts only attested code running in a TEE. Operators
  without the attested binary cannot decrypt, regardless of IAM. Compromise
  surface narrows to the TEE platform vendor and the attestation chain.

Trace Commons already operates as if the operator is a constrained actor. A
cloud-KMS-only KEK is a small step up from the current `LocalMasterKeyWrapper`
trust-wise (same model, just managed elsewhere). A TEE-rooted KEK is the
qualitatively different option.

## What the trait already locks in

`KmsKeyWrapper` is implementation-agnostic. It takes a 32-byte DEK plus a
`KekContext { tenant_storage_ref, artifact_kind }`, returns an opaque
`WrappedDek { wrapper_kind, key_ref_hash, ciphertext_base64, context_hash }`.
Both AWS KMS and a TEE wrapping key fit cleanly. Switching from one impl to
another only requires writing a new struct and flipping the constructor in
`trace-commons-ingest.rs`; the on-disk envelope format does not change.

The trait already provides:
- `is_production_trust_boundary() -> bool` — the impl declares whether it can
  back production. Local impl returns `false`; production impls return `true`.
- `safe_status()` — the impl exposes a `kind` + `key_ref_hash` for
  config-status without leaking key material.
- Context binding — both the outer `context_hash` field and an inner
  context-prefixed plaintext tag, so cross-object DEK substitution fails
  closed regardless of which impl decrypts.

This brief therefore only argues about which impl(s) to build.

## Candidate platforms

### A. Cloud KMS (operator-trusted)

**Two flavors:** AWS KMS or GCP Cloud KMS. Mechanically identical from the
KEK perspective. Choose by deployment fit.

- The wrapping key lives in the cloud provider's HSM. Wrap/unwrap is an
  authenticated API call with optional `EncryptionContext` that the trait's
  `KekContext` maps directly onto.
- Audit logging of every wrap/unwrap is the cloud provider's responsibility
  (CloudTrail / Cloud Audit Logs).
- Operator with the right IAM role decrypts at will. The KEK is bound to
  the cloud account, not to specific running code.
- Key rotation is built in (annual automatic for AWS KMS; manual for GCP
  via key-version promotion).

**Strengths:** small implementation effort (~1 week including tests); mature;
well-understood operations; cheap.

**Weaknesses:** operator (and provider) can read user content. Does not
materially change the trust model from `LocalMasterKeyWrapper` — just moves
the master key off-host.

**When this is the right answer:** if the deployment treats the operator as
trusted and only wants the master key out of process memory. Suitable for
internal-team-only pilots where the threat model is "stop accidental
exposure, not malicious operator."

### B. TEE-rooted (operator-constrained)

The wrapping key only exists inside an attested enclave. The KEK trait impl
runs inside the enclave (or RPCs to a sibling service that does); operators
without the attested binary cannot unwrap, even with full host root.

**Three platform candidates, each with distinct trade-offs:**

#### B1. Phala dstack

- Open-source TEE platform built on Intel TDX with attestation primitives
  designed for verifiable services. Mature attestation tooling, growing
  ecosystem in the privacy / agent space.
- Sealing keys derived from the enclave measurement (MRENCLAVE / MRTDX
  analog). Unsealing requires the matching attested binary.
- Aligns with the project's existing exposure to dstack-style ecosystems
  (cf. user's TEE background).
- **Strengths:** open-source platform, transparent attestation chain,
  community-driven, project-philosophy match.
- **Weaknesses:** smaller deployment footprint than AWS/GCP TEEs; ops story
  thinner; vendor concentration risk (single platform).

#### B2. AWS Nitro Enclaves

- Mature, widely deployed. Vsock-based comm with parent EC2 instance.
- Attestation via PCR values signed by AWS; KMS integration via Condition
  keys on `kms:RecipientAttestation:*` so a KMS key can refuse Decrypt
  unless the requester is the right Nitro enclave. **This composes the
  cloud-KMS path into a TEE-gated unseal** — a meaningful hybrid (see C).
- **Strengths:** mature ops, AWS-side attestation guarantees, KMS hybrid is
  textbook supported.
- **Weaknesses:** AWS-vendor lock-in for both compute and key; attestation
  chain rooted in AWS.

#### B3. GCP Confidential Space

- TEE-of-TEEs platform: AMD SEV-SNP or Intel TDX VM hosting workloads with
  attestation tokens signed by Google. Integrates with Cloud KMS via
  workload identity assertions for the same TEE-gated-unseal pattern as
  Nitro+KMS.
- **Strengths:** modern AMD SEV-SNP path (no SGX dependence), good Cloud
  KMS integration, well-documented workload-identity attestation.
- **Weaknesses:** Google vendor lock-in; relatively newer than Nitro.

**Cross-cutting weaknesses for all of B:** higher implementation effort
(~3-6 weeks including attestation, sealing, recovery, key rotation
rehearsal); operational complexity (enclave lifecycle, attestation refresh,
recovery from re-attestation failure); harder local dev story (must mock
attestation in tests, careful not to mock-bypass in production).

### C. Hybrid: cloud KMS gated by TEE attestation

A specific combination of A + B: the operator owns a Cloud KMS (AWS or GCP)
key whose policy refuses `Decrypt` unless the requester proves it is the
attested Trace Commons binary. The TEE wraps a sealing key derived inside
the enclave; the sealing key is encrypted under KMS for durability; KMS
only releases it to the attested enclave.

- **Strengths:** best-of-both. Persistence story (KMS) survives enclave
  re-deploys. Attestation story (TEE) prevents operator-with-IAM unseal.
  Operator-cannot-read holds even if the enclave restarts.
- **Weaknesses:** largest implementation surface. Adds a runtime dependency
  on KMS liveness. Two failure modes to debug instead of one.

This is the architecturally correct answer if TEE is on the table at all,
and it is what production-grade systems in this space tend to converge to.

## Comparison table

| | Local (today) | A. Cloud KMS | B1. dstack | B2. Nitro | B3. Conf. Space | C. Hybrid (Nitro+KMS) |
|---|---|---|---|---|---|---|
| Operator can decrypt | yes | yes | no | no | no | no |
| Vendor lock-in | none | one cloud | one platform | AWS | GCP | AWS or GCP |
| Implementation effort | 0 | low | medium | medium-high | medium-high | high |
| Ops complexity | low | low | medium | medium | medium | high |
| Attestation chain | n/a | n/a | dstack | AWS | Google | AWS or Google |
| `is_production_trust_boundary` returns | false | true* | true | true | true | true |
| Project-philosophy match | n/a | medium | high | medium | medium | high |
| Risk surface | host RAM | cloud IAM | TEE platform | TEE + AWS | TEE + GCP | TEE + cloud KMS |

*Cloud KMS earns `true` only by convention. Strictly speaking, it does not
provide an operator-constrained trust boundary — the convention is "the
operator has narrowed the surface enough to call it production-grade." If
the trust model assumes constrained operators, **only B and C should set
the flag to `true`**.

## Decision factors

Questions to answer before choosing:

1. **Does the threat model treat the operator as trusted?** If yes, A is
   fine. If no, A is structurally wrong even if it's easy.
2. **What is the deployment target?** Self-hosted on bare metal favors B1
   (dstack). AWS-centric ops favors B2 or C with AWS KMS. GCP-centric
   favors B3 or C with GCP KMS.
3. **How tolerant is the deployment of operational complexity?** B and C
   each demand TEE lifecycle management, attestation rehearsals, recovery
   playbooks. If the deployment team is one person, A may be the only
   feasible choice in practice.
4. **Is there a horizon for ZK / verifiable computation?** TEE-rooted KEKs
   sit naturally on the path toward verifiable Trace Commons execution.
   Cloud KMS is a dead end in that direction.
5. **Are there regulatory drivers** (data-residency, customer-managed
   keys, compliance frameworks) that bias toward specific clouds?

## Recommendation framework

The brief deliberately does not pick. The right answer depends on the
specific first deployment, and there are no current deployments. But the
framework for picking, in priority order:

1. **If a TEE platform is acceptable and a real deployment is imminent →
   choose C (TEE + cloud KMS hybrid).** Pick the TEE platform by deployment
   fit: dstack for self-hosted / privacy-ecosystem alignment; Nitro for
   AWS-centric ops; Confidential Space for GCP-centric ops. Spend the
   implementation effort up front; it is the only path that scales beyond
   one operator.

2. **If TEE is acceptable but the deployment is small / early → choose B1
   (dstack alone)** without the KMS hybrid. Less ops surface; sealing keys
   live entirely in the enclave; re-deploys require re-attestation but
   that is acceptable at small scale. Migrate to C when persistence cost
   becomes painful.

3. **If TEE is not in scope for the foreseeable deployment → choose A
   (cloud KMS, AWS or GCP per deployment).** Acknowledge in the spec that
   `is_production_trust_boundary()` returning `true` is a convention, and
   add a "no TEE" disclaimer to the threat-model docs.

4. **Do not choose `LocalMasterKeyWrapper` for production.** That is the
   role of the existing `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY`
   gate.

## Implementation cost estimates

| Path | Estimated effort | First useful checkpoint |
|------|------------------|-------------------------|
| A. Cloud KMS (AWS or GCP) | 1 week | `CloudKmsKeyWrapper` impl + integration test against the cloud, key rotation rehearsal documented |
| B1. dstack | 3 weeks | Enclave bootstrap with attestation + sealing key + `DstackKeyWrapper` impl + rehearsal of binary-replacement re-attestation |
| B2. Nitro alone | 3 weeks | Same shape, AWS-specific |
| B3. Conf. Space alone | 3 weeks | Same shape, GCP-specific |
| C. Hybrid | 5 weeks | TEE path + KMS-attested-unseal + recovery playbook for both halves |

These are honest estimates including testing, documentation, and one
rehearsal of key rotation. They assume the implementer is familiar with the
chosen platform. Add 1-2 weeks if not.

## What this brief does not commit to

- A platform.
- A timeline.
- A vendor.
- An implementation effort.

## What needs to happen next

1. Pick a path (A, B1/B2/B3, or C).
2. Write the implementation spec at
   `docs/superpowers/specs/<date>-trace-kek-<platform>-design.md`.
3. Add the chosen platform to `~/.claude/approved-dependencies.md` if it
   introduces new crates.
4. Implement following the same TDD-with-review cadence as the cloud
   provider work.

## Notes carried forward

- The `KmsKeyWrapper::unwrap_dek` return type should change from `[u8; 32]`
  to `Zeroizing<[u8; 32]>` when the chosen impl lands. That trait-signature
  change is small and clean if done at the same time as adding the new
  impl; doing it later forces a synchronized change across multiple impls.
- Whatever the choice, the implementation should set
  `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY=true` in its
  reference deployment config so the gate is visibly load-bearing.
- For cloud KMS impls, the trait's `KekContext` maps cleanly onto the
  provider's `EncryptionContext` field — use it. Cross-object DEK
  substitution gets a second layer of defense from the cloud KMS side.
- For TEE impls, sealing keys derived from the enclave measurement must be
  decoupled from any operator-supplied input — otherwise an attacker who
  controls inputs can grind toward a target key. Use a fixed enclave-side
  derivation context.
