# TraceCommons

**A user-owned register of AI agent work.**

When an AI agent does work for someone, it leaves a record of what actually
happened: the tools it called, the places it failed, the result it gave back.
That record is becoming valuable. The companies building the next generation
of agents need millions of those records to train against, and most of them
live inside private user sessions today — collected unilaterally by whoever
runs the model, on terms the user never specifically agreed to.

TraceCommons keeps the record under the contributor's control. Capture and
scrubbing both happen on the user's machine; only the scrubbed version moves
to a shared server, where two checks decide whether the record is worth
keeping. One asks whether the record is genuinely different from everything
already filed. The other asks whether it is substantive work rather than
template-shaped filler. Both must pass. Accepted records are signed, dated,
and filed into a register. Frontier labs, auditors, and regulators can query
the register under selective disclosure; they see what they need, and the
rest stays encrypted.

A **Trace Credit** is the signed, on-chain record that one of a contributor's
submissions was accepted into the register. Credits are how recognition flows
back when buyers later pay to query the evidence. They are non-transferable.

The contract is "local-first, opt-in, scrub before upload":

- Trace contribution is **off by default**. Raw traces stay on the user's
  device unless they explicitly opt in.
- Uploads carry only `ironclaw.trace_contribution.v1` envelopes — text and
  tool payloads are stripped or replaced with stable placeholders during
  local deterministic scrubbing.
- The server gates incoming envelopes on **two** axes — novelty against the
  existing register and substantive-work signal against a frontier model.
  In Phase A this runs on regular GPU hardware in NEAR AI's TEE-hosted vLLM;
  the Phase B milestone moves scoring inside attested hardware that even the
  operators of the server cannot read.
- Accepted, settled records mint **Trace Credits** through a hash-only
  utility-attestation pipeline. Credits are non-transferable, bound to
  reviewed evidence, and settle on-chain via NEAR; uploads alone don't pay.

This repository — `trace-commons-server` — is the hosted control plane:
ingest, review, retention, revocation, encrypted artifact storage,
upload-claim issuing, audit chain, and credit settlement. The contributor
client lives in a separate repo (Ironclaw); the shared protocol DTOs live
in `crates/trace-commons-protocol`.

## Status: Pilot Deployment

TraceCommons is **in pilot deployment as of May 2026**. What that means
concretely:

| Component | State |
|---|---|
| Hosted server (this repo) | Phase A code-complete, smoke-validated, deployable. |
| Scoring backend | **NEAR AI Cloud** (TEE-hosted vLLM, Intel TDX + NVIDIA GPU TEE) — chosen so a pilot host needs no local CUDA stack. Smoke-validated against `Qwen3.6-35B-A3B-FP8`. |
| Gate floors | Recalibration against the hosted model is required before first contributor traffic — see [`docs/operator/a27-perplexity-floor-calibration.md`](docs/operator/a27-perplexity-floor-calibration.md). |
| Contributor gate | Invite-code allowlist on the upload-claim issuer; off by default, enabled for the pilot — see [`docs/operator/pilot-allowlist.md`](docs/operator/pilot-allowlist.md). |
| KMS / KEK | Cloud KMS (GCP first) with envelope-encrypted per-object DEKs. Phase A trust boundary. |
| TEE trust upgrade | Phase B — move the gate service into an attested dstack enclave once dstack-GPU primitives stabilize. The current KEK boundary is honestly weaker than the Phase B target; this is documented, not papered over. |
| Contributor client | Ironclaw integration is the remaining gate before live contributor traffic. The `trace-commons-pilot-bootstrap` binary stands in as a load-generation harness against real HF agent-traces sessions so calibration and end-to-end validation can proceed without it. |
| Credits | Settlement, hash-only attestation pipeline, central-issuer ABAC, NEAR receipt outbox — all in. Credit-bearing routes are gated by a central-issuer principal allowlist. |

Pilot intentionally **scopes down** from the original design: regular GPU
hardware (in NEAR AI's TEE) instead of an attested local enclave; cloud
KMS as the KEK; a single calibrated model rather than per-tenant selection.
Phase B narrows the trust gap; Phase A proves the path with operators.

The full phasing and open work queue lives in
[`docs/trace-commons-roadmap.md`](docs/trace-commons-roadmap.md). Per-slice
design specs and implementation plans live under
[`docs/superpowers/`](docs/superpowers/).

## Architecture

```
┌────────────────────┐    ┌────────────────────┐    ┌──────────────────┐
│  Ironclaw client   │    │  trace-commons-    │    │  NEAR AI Cloud   │
│  (separate repo)   │───▶│  ingest (this repo)│───▶│  (TEE-hosted vLLM│
│  local redaction   │    │                    │    │   scoring)       │
└────────────────────┘    │  ┌──────────────┐  │    └──────────────────┘
                          │  │ gate-enclave │  │
                          │  │  orchestrator│  │
                          │  └──────────────┘  │
                          │  ┌──────────────┐  │    ┌──────────────────┐
                          │  │  PostgreSQL  │◀─┼───▶│  Object storage  │
                          │  │  with RLS    │  │    │  (GCS, FS, local)│
                          │  └──────────────┘  │    └──────────────────┘
                          │  ┌──────────────┐  │    ┌──────────────────┐
                          │  │  credit      │──┼───▶│  NEAR receipt    │
                          │  │  settlement  │  │    │  outbox          │
                          │  └──────────────┘  │    └──────────────────┘
                          └────────────────────┘
```

Authoritative contracts to read before changing anything substantive:

- [`docs/trace-commons.md`](docs/trace-commons.md) — envelope contract and
  threat model
- [`docs/trace-commons-storage.md`](docs/trace-commons-storage.md) — storage
  contract
- [`docs/trace-commons-roadmap.md`](docs/trace-commons-roadmap.md) — phased
  open work and "Production Gap Queue"

## Repository Layout

```
crates/
├── trace-commons-protocol/      DTOs + redaction helpers shared with the client.
├── trace-commons-gate-enclave/  Scoring orchestrator (perplexity, embedder, vector index).
│                                Two real perplexity backends: mistralrs (local CUDA,
│                                feature `local-gpu-models`) and NEAR AI Cloud HTTP
│                                (feature `near-ai-scorer`).
└── trace-commons-server/        All hosted binaries.
    └── src/bin/
        ├── trace-commons-ingest                 Hosted ingest / review / admin / worker API.
        ├── trace-commons-upload-claim-issuer    EdDSA/Ed25519 upload-claim issuer.
        ├── trace-commons-gate-calibrate         Offline calibration + model bake-off.
        ├── trace-commons-pilot-bootstrap        HF agent-traces load generator for pilot.
        └── trace-commons-vector-replay          Vector-index replay tool.

migrations/                      PostgreSQL schema.
docs/
├── trace-commons.md, trace-commons-storage.md, trace-commons-roadmap.md  Authoritative contracts.
├── operator/                    Operator runbooks (per slice).
└── superpowers/                 Per-slice design specs + implementation plans.
.github/workflows/ci.yml         CI gates.
```

## Getting Started

### Build + Test

```bash
# Minimum: default-features build + tests (no GPU, no external scoring)
cargo check -p trace-commons-server --bins
cargo test  -p trace-commons-server

# With the NEAR AI scoring backend (pilot configuration)
cargo check -p trace-commons-server --bins --features near-ai-scorer

# With local CUDA scoring (mistralrs; needs CUDA toolchain for the cuda subfeature)
cargo check -p trace-commons-server --bins --features local-gpu-models
```

CI applies `RUSTFLAGS=-D warnings` to every cargo invocation, so warnings
fail the build. To catch what CI catches before pushing:

```bash
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins
RUSTFLAGS='-D warnings' cargo test  -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -D warnings \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```

PostgreSQL integration tests require a live database; export
`TRACE_COMMONS_PG_TEST_DATABASE_URL` and run:

```bash
cargo test -p trace-commons-server --test trace_corpus_pg_store
```

### Run a Local Ingest Server

```bash
TRACE_COMMONS_TENANT_TOKENS='tenant-a:dev-token-a;expires_at=2027-01-01T00:00:00Z' \
TRACE_COMMONS_BIND='127.0.0.1:3907' \
cargo run --bin trace-commons-ingest
```

This runs the dev profile against the local-encrypted artifact store and
the in-process mock gate. Production-grade configuration (cloud KMS, real
gate scorer, central-issuer credit profile, RLS-pinned PostgreSQL role,
fresh rollout-smoke) is documented in
[`docs/operator/`](docs/operator/) — start there before touching any
deployment.

## Contributing

Branch protection on `main` requires:

- All CI checks green (`cargo fmt --check`, three `cargo check` variants,
  `cargo clippy`, `cargo test`, `pilot-bootstrap-smoke`).
- A pull request (no direct pushes).
- Linear history (squash or rebase, no merge commits).
- Any review conversations resolved before merge.

Self-merge is permitted; reviewer approval is not currently required (the
project is still small). When this changes, the requirement will land here
and in the GitHub branch protection settings simultaneously.

### Conventions worth knowing

- **Hash-only audit and logging.** Audit rows, error logs, and operational
  surfaces are hash-only or label-only. Raw URLs, bearer tokens, ARNs,
  account references, transaction hashes, contributor identity, and trace
  bodies must never appear in stored rows or log strings.
- **Fail-closed by default.** When a required gate is configured but its
  dependency is missing, refuse the path with a safe missing-control name.
  Never silently fall back to plaintext or a less-restricted backend.
- **Tenant scoping.** Every read/write is driven by auth-derived tenant +
  actor context. Envelope tenant fields are attribution only.
- **PostgreSQL RLS is forced** on every Trace Commons table; tenant
  predicates go through `trace_current_tenant_id()`.
- **Commit style.** Short imperative subjects (no `feat:` / `fix:` prefixes
  — match the existing log). No emojis in commits, PRs, code, or reports.

A more complete style + workflow note for AI-assisted development is in
[`CLAUDE.md`](CLAUDE.md); the conventions there are the same ones humans
follow.

### Where to look for what

- Roadmap and pilot blockers: [`docs/trace-commons-roadmap.md`](docs/trace-commons-roadmap.md)
- Envelope + threat model: [`docs/trace-commons.md`](docs/trace-commons.md)
- Storage contract: [`docs/trace-commons-storage.md`](docs/trace-commons-storage.md)
- Per-slice design specs: [`docs/superpowers/specs/`](docs/superpowers/specs/)
- Per-slice implementation plans: [`docs/superpowers/plans/`](docs/superpowers/plans/)
- Operator runbooks: [`docs/operator/`](docs/operator/)

## Public Reference Notes

The Trace Credits, ranking-evidence, and external-adapter surface areas are
each large enough to warrant their own documents. Until they're broken out,
they live inline in `docs/`:

- **Trace Credits** — settlement model, central-issuer profile, NEAR outbox.
  See `docs/operator/calibration.md` and the credit-settlement specs under
  `docs/superpowers/specs/`.
- **Ranking evidence** — calibration registry, label-source authority, model
  promotion. See ranking-related specs under `docs/superpowers/specs/`.
- **External adapters** — benchmark/process evaluators, NEAR credit
  submit/confirm. All adapters are operator-owned; the server only records
  `configured` / `not_configured` readiness fields in
  `/v1/admin/config-status`.
