# Deployment Topology

One-page picture of every box a `trace-commons-server` pilot needs and how they
fit together. Audience: deployment operator orienting before the first
deploy. Read [`deployment.md`](deployment.md) next.

## v1 scope

GCP-only. A second cloud's KMS adapter would justify a cloud-agnostic
refactor — until then this doc names GCP resources concretely.

The pilot is single-host, single-GPU, single-binary. Horizontal scaling and
multi-region failover are explicitly out of scope.

## Boxes and wires

```
                  +----------------------------------------+
                  |          Contributor (Ironclaw)        |
                  |  - holds upload claim                  |
                  |  - encrypts envelope client-side       |
                  +-------------------+--------------------+
                                      |  HTTPS
                                      v
+-----------------------+   HTTPS   +------------------------------+
| trace-commons-upload-      |<--------> | trace-commons-ingest              |
| claim-issuer (Ed25519)|           |  (single binary, this repo)  |
+-----------------------+           |                              |
                                    |  - REST + admin + worker API |
                                    |  - in-process: KEK wrapper,  |
                                    |    audit chain, ABAC, RLS    |
                                    |    pool, schedulers          |
                                    |  - in-process when feature   |
                                    |    local-gpu-models is on:   |
                                    |    candle perplexity scorer, |
                                    |    fastembed embedder,       |
                                    |    usearch vector index      |
                                    +---+--------+--------+--------+
                                        |        |        |
                  encrypted bytes       |        |        |  metadata + audit + credit
                                        v        |        v
                              +----------------+ |    +--------------------+
                              | GCS bucket     | |    | PostgreSQL         |
                              | (object        | |    |  - RLS enforced    |
                              |  versioning,   | |    |  - audit chain     |
                              |  CMEK to KMS)  | |    |  - credit ledger   |
                              +----------------+ |    |  - vector_entry_id |
                                                 |    +--------------------+
                                                 v
                                       +-------------------+
                                       | Cloud KMS         |
                                       | (CryptoKey, KEK)  |
                                       +-------------------+
```

### What lives where

| Component | In-process with `trace-commons-ingest`? | Notes |
|---|---|---|
| HTTP API (REST + admin + worker) | yes | Axum |
| KEK wrapper (KEK adapter) | yes | Calls Cloud KMS encrypt/decrypt |
| Audit chain writer | yes | Hash chain on Postgres |
| ABAC + RLS enforcement | yes | RLS forced on every Trace Commons table |
| Schedulers (credit, vector, retention, etc.) | yes | Each guarded by its own bearer token |
| Perplexity scorer (Llama-3.1-8B) | yes (with `local-gpu-models`) | Uses CUDA via candle |
| Embedder (BGE-large) | yes (with `local-gpu-models`) | fastembed (ONNX Runtime) |
| Vector index | yes | usearch, disk-backed under `TRACE_COMMONS_VECTOR_INDEX_ROOT` |
| GCS object store | no | `google-cloud-storage` client; CMEK |
| Cloud KMS | no | `google-cloud-kms` client; provides the KEK |
| PostgreSQL | no | Managed (Cloud SQL recommended); RLS-forced |
| Upload-claim issuer | no, separate binary | EdDSA signer; same repo |
| Ironclaw client | no, separate deployment | Out of scope for this repo |

### Why "single binary with in-process GPU"

Phase A's design keeps the perplexity scorer, embedder, and vector index in
the same process as the gate worker route. The seam is the
`trace-commons-gate-enclave` crate's `EnclaveGateOrchestrator` trait surface —
when Phase B's dstack migration lands, the same trait gets a remote
implementation and these components move out-of-process into the enclave.
The runbook does not need to change for that swap.

### Trust boundary in v1

The KEK lives in Cloud KMS. Plaintext envelopes only exist inside
`trace-commons-ingest` and in the GPU box (which is the same process). Wrapped
DEKs travel to GCS attached to ciphertext; raw DEKs never leave the
process. `tenant_ctx` is the canonical authorization input — see
`docs/superpowers/specs/2026-05-12-trace-kek-strategy-design.md` for the
full threat model.

The operator and the central credit issuer are the same actor in v1. The
operator-constrained trust model (separating the two) is Phase B.

### What is NOT in this picture

- Infrastructure-as-code. The runbook references resources by purpose, not
  Terraform module.
- Backup/restore wiring. See [`backup-restore.md`](backup-restore.md).
- Multi-region or multi-AZ replication. Single-region pilot.
- Phase B dstack attestation flow. Placeholder only.
