# Status and reference notes

This page holds the deployment-status and reference material that was in
the root [`README.md`](../README.md) until the 1.0 release. The open work
queue is in [`trace-commons-roadmap.md`](trace-commons-roadmap.md), under
"Production Gap Queue"; the operator runbooks are indexed in [`operator/README.md`](operator/README.md).

## Status: Pilot Deployment

TraceCommons is **in pilot deployment as of May 2026**. What that means
concretely:

| Component | State |
|---|---|
| Hosted server (this repo) | Phase A code-complete, smoke-validated, deployable. |
| Scoring backend | **NEAR AI Cloud** (TEE-hosted vLLM, Intel TDX + NVIDIA GPU TEE) — chosen so a pilot host needs no local CUDA stack. Smoke-validated against `Qwen3.6-35B-A3B-FP8`. |
| Gate floors | Recalibration against the hosted model is required before first contributor traffic — see [`docs/operator/a27-perplexity-floor-calibration.md`](operator/a27-perplexity-floor-calibration.md). |
| Contributor gate | Invite-code allowlist on the upload-claim issuer; off by default, enabled for the pilot — see [`docs/operator/pilot-allowlist.md`](operator/pilot-allowlist.md). |
| KMS / KEK | Cloud KMS (GCP first) with envelope-encrypted per-object DEKs. Phase A trust boundary. |
| TEE trust upgrade | Phase B — move the gate service into an attested dstack enclave once dstack-GPU primitives stabilize. The current KEK boundary is honestly weaker than the Phase B target; this is documented, not papered over. |
| Contributor client | `trace-commons-contributor` (this repo) is available for direct human contributors: `login`, `submit`, `status`, `whoami`, `logout`, `mint-grant`. Ironclaw integration (a TEE-hosted trace source) is separate, ongoing work. The `trace-commons-pilot-bootstrap` binary stands in as a load-generation harness against real HF agent-traces sessions so calibration and end-to-end validation can proceed without either. |
| Credits | Settlement, hash-only attestation pipeline, central-issuer ABAC, NEAR receipt outbox — all in. Credit-bearing routes are gated by a central-issuer principal allowlist. |

Pilot intentionally **scopes down** from the original design: regular GPU
hardware (in NEAR AI's TEE) instead of an attested local enclave; cloud
KMS as the KEK; a single calibrated model rather than per-tenant selection.
Phase B narrows the trust gap; Phase A proves the path with operators.

The full phasing and open work queue lives in
[`docs/trace-commons-roadmap.md`](trace-commons-roadmap.md). Per-slice
design specs and implementation plans live under
[`docs/superpowers/`](superpowers/).

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
