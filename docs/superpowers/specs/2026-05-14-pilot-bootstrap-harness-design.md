# Pilot Bootstrap Replay Harness — Design (Phase A.6)

Date: 2026-05-14
Status: Draft (pre-implementation)
Owner: Trace Commons / Pilot launch
Predecessors:
- `2026-05-14-gate-floor-recalibration-design.md` (A2.5 — pilot launch readiness)
- `2026-05-11-private-vector-system-design.md` (system design from which the submission API contract derives)
- `README.md` § "What Blocks First Real Use" item 2 (Ironclaw extraction)
Driver: Pilot launch requires real contributor traffic to (a) calibrate the tail-fraction floor per A2.5's pending work, (b) collect labeled novel/duplicate exemplars for Phase A.5 metric design, and (c) prove the audit chain + credit pipeline end-to-end. The Ironclaw client wiring isn't done. HuggingFace agent-traces datasets are a usable bootstrap source.

## Motivation

Phase A is code-complete and bake-off-validated, but the server has no clients yet. The Ironclaw extraction work (item 2 of the README's "What Blocks First Real Use") is a separate work stream on Ironclaw's side; its timeline is not under our control.

Three pilot-launch goals depend on real traffic flowing through the server:

1. **A2.5 tail-fraction calibration** — needs ~1000 real submissions to calibrate `TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS` against actual contributor distribution.
2. **Phase A.5 perplexity-replacement metric design** — needs labeled novel/duplicate exemplars from real contributor work to design a metric that actually discriminates.
3. **End-to-end pipeline validation** — the audit chain, RLS, credit accumulation, revocation propagation, and operator-summary surfaces have all been validated in isolation. None have been exercised at scale with real-shaped inputs.

Waiting for Ironclaw blocks all three for an unbounded period. A thin replay harness that pulls from HuggingFace agent-traces datasets and POSTs through the existing API gets us moving on goals 1 and 3 immediately, and substantially on goal 2 (the "labeled" part comes from the source-dataset metadata).

The harness is **not a substitute for real users**. Real contributor work has properties (privacy, distribution, attribution semantics) that HF datasets don't fully replicate. But it's enough to:

- Stress-test the gate at scale
- Generate real audit-chain traffic
- Measure gate behavior across diverse domains (security audits, coding sessions, agentic reasoning)
- Populate the vector index with realistic embeddings so the novelty-floor logic exercises non-degenerate state

When Ironclaw integration lands, the harness gets retired (or kept as a load-generation tool for ops).

## Goal

A CLI tool that:

1. Pulls a configurable HuggingFace agent-traces dataset
2. Translates each row to a submission envelope per `tracedao-protocol`
3. Authenticates as a configured "bootstrap-contributor" tenant
4. POSTs through `/v1/submissions` at a configurable rate
5. Records the submission ID + source-dataset + domain label in a sidecar JSONL so calibration tooling can filter by source

End result: 30k+ realistic submissions flowing through the production gate within ~24 hours of operator launch.

## Non-goals

- **Replacing Ironclaw.** This is a load-generation tool, not a contributor UX.
- **Multi-tenant simulation.** Single bootstrap-contributor tenant is enough for the calibration goals; multi-tenant testing is a separate concern.
- **Adversarial / fuzz testing.** Send well-formed envelopes only; the gate's adversarial-input behavior is its own work item.
- **Long-running daemon.** This is a CLI that runs to completion (configurable trace count). Restart semantics + checkpointing are not needed for the pilot-bootstrap use case.
- **Real credit issuance.** Run with the existing zero-credit calibration semantics (`TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=0`). Bootstrap traces accumulate hash-only audit entries but no real credit transfer.
- **Embedder backfill / vector index seeding outside the gate.** The harness submits through the standard ingest path; the vector index gets populated naturally as gate decisions are made.

## Decisions baked in

| Decision | Value |
|----------|-------|
| Tool location | `crates/tracedao-server/src/bin/tracedao-pilot-bootstrap.rs` (new binary, alongside `tracedao-ingest` and `tracedao-gate-calibrate`) |
| Auth path | Use existing tenant-token flow (`TRACE_COMMONS_TENANT_TOKENS=bootstrap-tenant:contributor:<token>`). No new auth mechanism. |
| Source data | HF datasets via `huggingface_hub`. Default: `jedisct1/agent-traces-swival`. Configurable via CLI. Pluggable per-dataset row-to-envelope translators. |
| Rate control | Token-bucket or simple sleep-between-posts. Default ~1 request/sec to avoid swamping the server during initial debugging. |
| Idempotency | Per-row deterministic content hash → `submission_id`. Re-running against the same dataset doesn't double-submit. |
| Labeling sidecar | JSONL: `{submission_id, source_dataset, source_row_id, source_domain_tag}` per row. Written alongside output. |
| Output destination | Local server (`--target=http://localhost:3907` default; configurable for staging/prod). |
| Hardware | None (CPU-only client; runs anywhere). The target server has its own hardware needs. |

## Architecture

```
                            +----------------------+
                            |  HF dataset stream   |
                            |  (parquet shards)    |
                            +----------+-----------+
                                       v
                            +----------+-----------+
                            |  row→envelope        |
                            |  translator (per     |
                            |  source format)      |
                            +----------+-----------+
                                       v
                            +----------+-----------+
                            |  rate-limited        |
                            |  HTTP poster         |
                            +----------+-----------+
                                       v
                            +----------+-----------+
                            |  tracedao-ingest     |
                            |  POST /v1/submissions|
                            +----------+-----------+
                                       v
                            +----------+-----------+
                            |  gate decision       |
                            |  + audit chain       |
                            |  + vector index      |
                            +----------+-----------+
                                       v
                            +----------+-----------+
                            |  labeling sidecar    |
                            |  (JSONL local file)  |
                            +----------------------+
```

Per-dataset translators are small modules that map a source row to:

- `trace_body` — the text content (typically `proof + fix_outline + source_code` for swival; `messages` joined for pi-mono; etc.)
- `metadata` — domain tag, severity, etc., carried through as hash-only fields where possible
- `submission_id` — deterministic content hash so reruns are idempotent

Currently planned translators (v1):

| Dataset | Translator focus |
|---------|------------------|
| `jedisct1/agent-traces-swival` | Security audit traces — combine `proof`, `fix_outline`, first ~2000 chars of `source_code` |
| `badlogicgames/pi-mono` | Coding-agent sessions — concatenate top-level message text up to N tokens |
| `TeichAI/DeepSeek-v4-Pro-Agent` | DeepSeek agent traces — message-stream join with tool-call annotations |

v1 ships swival only; pi-mono and DeepSeek are added in subsequent slices as the v1 stabilizes.

## Open questions

1. **Submission rate.** 1/sec is conservative; the server can handle higher. **Recommendation:** start at 1/sec, monitor server CPU + DB load, adjust upward to ~10/sec if the server stays under 50% utilized.

2. **Sidecar storage durability.** Local JSONL is fine for v1, but if the harness runs for 24+ hours and the process dies mid-way, we lose state for in-flight submissions. **Recommendation:** flush per-submission; idempotency means a restart just skips already-submitted rows.

3. **What submission shape do real traces use?** `tracedao-protocol` defines an envelope. v1 of the harness submits text-only `trace_body` content; multi-part content (multiple turns, tool calls) is a v2 concern. **Recommendation:** confirm the v1 envelope contract with the protocol-crate maintainer before writing the translator; if multi-part is required at v1, the translators get more involved.

4. **Embedder shape.** The bootstrap-contributor's traces go through the same embedder + vector-index path as real users. If the harness traces are too uniform (all security-audit-shape), the vector index becomes degenerate. **Recommendation:** mix at least three datasets in v1.5 to give the index distributional diversity.

5. **Real vs synthetic principal identities.** Audit chain links submissions to the auth'd principal. The bootstrap-contributor is a single principal; all 30k traces accumulate to one ledger row. **Recommendation:** acceptable for v1 (zero-credit calibration semantics mean no real credit accumulates). For a multi-principal load test, generate N synthetic contributor tokens and round-robin; that's a v2 concern.

6. **Where does the harness live in the binary layout?** Options:
   - `crates/tracedao-server/src/bin/tracedao-pilot-bootstrap.rs` (same crate as other binaries) — chosen.
   - Separate crate `tracedao-pilot-client` — too much ceremony for what's effectively an internal load tool.

## Deliverables

1. **A.6a — binary scaffolding.** `tracedao-pilot-bootstrap` binary with CLI args, no actual submissions yet. ~1 day.
2. **A.6b — swival translator + idempotent submission.** Single dataset, end-to-end. ~2 days.
3. **A.6c — labeling sidecar + rate control + observability.** ~1 day.
4. **A.6d — operator runbook.** ~half day.
5. **A.6e — multi-dataset support** (pi-mono + DeepSeek-v4-Pro-Agent translators). ~1 day.

Total: ~1 week of focused engineering.

## What this depends on

- `tracedao-protocol` crate — exists; the harness needs to depend on it.
- `tracedao-ingest` binary — exists; the harness submits to it.
- A running ingest binary (locally or on a staging Lambda host). Cost: same as the gate-service deployment.
- HuggingFace dataset access for swival (public, MIT) + others (mostly public).

## What success looks like

Operator runs the harness against a staging deployment. Within 24 hours, 30k swival traces are in the audit chain. The operator pulls a sample of `trace_gate_decisions` rows and confirms:

- All submissions returned a gate decision (accept / reject)
- Audit chain has 30k+ rows with valid prev-hash linkage
- Vector index has 30k+ embeddings populated
- Tail-fraction values for 30k submissions are available for A2.5's pending calibration
- Per-domain (security-audit vs coding-session vs reasoning) gate-pass rate is observable from the labeling sidecar joined against `trace_gate_decisions`

The harness emits a final summary report: total submissions, gate-pass rate by domain, audit-chain row count, vector-index size, time elapsed.

## What success doesn't look like

- Real contributor data going through the harness (it's HF datasets, not user data)
- Real credit accumulating to real identities (zero-credit calibration mode)
- A production-grade load tester (this is single-process, single-tenant)

When Ironclaw lands, this binary either gets retired or kept as an ops load-generation tool for future calibration runs.

## Trade-offs explicitly accepted

- **Single-tenant simulation.** Multi-tenant load test is v2.
- **Single-process submitter.** Parallel submitters are v2.
- **HF agent-traces datasets are not real users.** The gate behavior on this data is informative-but-not-authoritative; real-user behavior may differ in distribution shape.
- **One more deployment surface.** Another binary to operate alongside `tracedao-ingest`, `tracedao-upload-claim-issuer`, and `tracedao-gate-calibrate`. Acceptable cost.

## Out of scope (recorded so we don't accidentally re-open it)

- Adversarial / fuzz testing
- Multi-tenant load
- Daemon-mode operation
- Real credit issuance via the harness
- Bypassing the gate (the harness goes through the standard ingest path; no shortcuts)
- Phase B / dstack (the harness operates one layer up from the gate-service binary)
