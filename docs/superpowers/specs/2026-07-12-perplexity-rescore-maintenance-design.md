# Perplexity re-score maintenance — design

## Goal
An operator-triggered, **perplexity-only** re-score of existing `trace_gate_decisions`
using the currently-configured gate scorer (now Qwen3.6-27B dense). It recomputes each
trace's perplexity through the exact production scoring path and updates **only** the
perplexity columns, leaving novelty, tail-fraction, the vector/embedding index, gate
status, and credit completely untouched.

## Why
The pilot scorer was switched from Qwen3.6-35B-A3B (MoE) to Qwen3.6-27B-FP8 (dense) and a
perplexity floor (6,000,000 micros) enabled. The ~349 historical decisions still carry
35B-scored perplexity. A full gate re-drive would recompute novelty against the vector
index (risky — the index is not safely operator-manipulable), so re-scoring must be
surgical: perplexity fields only. Credit is off, so there is no live impact; this is
consistency work ahead of graded credit.

## Non-goals
- Do NOT recompute or touch novelty, `novelty_score_micros`, `novelty_passed`,
  `nearest_neighbor_hash`, `vector_entry_id`, or any embedding/vector state.
- Do NOT change `trace_submissions.status` (PII-risk-based, set at ingest) or credit.
- Do NOT re-run the full gate evaluation (`evaluate_and_record_gate`) — that inserts
  vector entries and records a fresh holistic decision. Reuse only the perplexity path.
- No new migration if avoidable (idempotent full-pass; see below).

## Design
An **admin HTTP route** (mirroring the existing admin/maintenance route pattern and their
bearer-credential gate; find an existing `/v1/admin/*` handler and copy its auth shape)
that triggers a **background re-score task** and returns immediately with a hash-only
acknowledgement. The task:

1. Enumerates all submissions that have a gate decision, cross-tenant, ordered by
   `received_at ASC` (reuse the gate-driver reader pool / the same no-tenant-GUC pattern
   `list_submissions_needing_gate_decision` uses; add a sibling enumeration that returns
   submissions **with** a decision).
2. For each: load the stored envelope via the existing decrypt path
   (`read_submission_record` + `read_envelope_by_record` — reuses `AppState`'s artifact
   store + KEK), build the **same canonical representation + chunking** the production
   perplexity scorer uses, and compute perplexity through the **same scorer** the driver
   uses (the `enclave_near_ai` orchestrator / `NearAiPerplexityScorer` perplexity path in
   `score_one_submission` branch 3 — call ONLY its perplexity computation, not the novelty
   or vector steps). Produce `perplexity_micros`, `peak_perplexity_micros`, and
   `perplexity_passed = perplexity_micros >= TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`.
3. `UPDATE trace_gate_decisions SET perplexity_micros=$, peak_perplexity_micros=$,
   perplexity_passed=$ WHERE tenant_id=$ AND submission_id=$` — a new tenant-scoped DB
   method mirroring `update_trace_gate_decision_credit_withheld_reason`
   (trace_corpus_pg.rs:5513) exactly (tenant pool + `begin_trace_tenant_transaction`).
   Update ONLY those three columns.
4. On a scorer/load failure for one submission: log hash-only, skip it, continue (leave
   its old value). Never abort the whole pass on one failure.

**Idempotent + resumable:** re-scoring is deterministic per (model, trace), so a full pass
is idempotent — re-running just recomputes the same values. No "done" marker/column
needed; if the task dies, re-trigger. Optionally accept a `?limit=N` query param to bound
a run for testing.

## Config / gating
- Admin bearer credential (reuse the existing admin-route credential; do not add a new
  gate). Refuse if the scorer isn't configured. Requires the `near-ai-scorer` feature
  (already built into the pilot binary).
- Hash-only audit: log counts, submission-ids (as existing gate code does), and error
  hashes — never raw trace text, perplexity-input content, keys, or URLs.

## Testing
- Unit: the new DB update method touches only the three perplexity columns (in-memory
  double + a pg-gated test if a DB is available).
- Integration (in-memory / where feasible without a live scorer): the re-score task, given
  a stubbed perplexity scorer, updates perplexity fields and leaves novelty/status columns
  byte-identical. Assert the novelty/vector columns are unchanged before/after.
- Live pg + scorer path is validated on the pilot after deploy (drive the route with a
  small `?limit=` first, confirm perplexity changes and novelty does not).

## Rollout
Ships behind the admin credential; does nothing until the route is called. Deploy via the
pilot Cloud Build (from main). Validate with `?limit=5` on the pilot (confirm perplexity
columns change, novelty columns unchanged), then run the full pass.
