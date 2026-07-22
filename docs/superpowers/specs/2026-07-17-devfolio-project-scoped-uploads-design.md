# Devfolio project-scoped uploads + score read-back — design

Date: 2026-07-17 (revised after the neutral-schema pivot)
Status: Approved; implemented (contributor scope + manifest + score read-back)

## Problem

The contributor CLI (`crates/trace-commons-contributor`) treats a contributor's
**entire local corpus** as the candidate set for upload: it scans both hardcoded
roots (`~/.claude/projects/**` and `~/.codex/sessions/**`), one `.jsonl` = one
session = one submission, and subsets at submit time via an interactive picker
or the `--project` / `--source` / `--since` filters.

The devfolio workflow (devfolio is a hackathon platform, the first vouching
instance) needs three things:

1. **User-controlled, project-scoped upload** — a participant uploads only the
   traces from *this hackathon project*, not their whole machine.
2. **A link from uploaded traces to a devfolio submission** — so devfolio can
   associate a participant's traces with their hackathon entry.
3. **Scores per trace, reported back to devfolio** — so devfolio can rank the
   competition.

## Design decisions

### The trace envelope stays neutral

The linkage is **NOT** carried in the trace schema. An earlier iteration stamped
a `feature_flags["devfolio_submission_id"]` on the envelope; that was reverted.
Rationale: the Trace Commons envelope should not carry consumer-specific
(devfolio) fields. Instead:

- The upload **emits the envelope ids** (`submission_id`s) it produced as a
  machine-readable manifest.
- **Devfolio collects** those ids on its side (participant hands them over /
  pastes them into devfolio). Devfolio owns the submission↔ids mapping.
- Trace Commons **reports scores keyed by `submission_id`** back to devfolio.

`submission_id` is the join key across all three steps — it is already the id
the server keys gate decisions on (`trace_gate_decisions` is per-`(tenant_id,
submission_id)`), so it threads cleanly upload → devfolio → score read-back.

### Part 1 — Scope control (`--project`) — DONE

`--project <path>` matches the session's *true decoded working directory*
(hyphen-safe, component-wise path prefix), falling back to the legacy
basename/path heuristic only when the true cwd is unavailable. The decoded cwd
is populated onto `SessionRef` at discovery via a cheap, UTF-8-tolerant
early-stop peek (mirrors `load_session`), so filtering does not full-load the
corpus. The interactive picker remains the final control.

### Part 2 — Upload manifest (`submit --manifest`) — DONE

`submit --project X --manifest ids.json` writes a JSON array of
`{ submission_id, status }` for the batch's uploaded traces (the
`Submitted` and `AlreadySubmitted` outcomes; refused/failed/skipped are
excluded). The participant hands that file's ids to devfolio. Logging is
count-only. No trace-schema change.

### Part 3 — Score read-back by envelope id — DONE

A new server-side worker route lets devfolio pull scores for a set of
`submission_id`s.

- **Route:** `POST /v1/admin/scores-by-submission`, body
  `{ submission_ids: [uuid] }` (batch capped at 500, mirroring the status
  read-back cap). Registered in the admin router chain beside the recompute
  routes (`bin/trace-commons-ingest.rs` ~6841-6847).
- **Auth — new scoped credential.** A new `TokenRole::CompetitionReadWorker`
  plus a `require_competition_operator(auth)` gate (`can_admin() || role ==
  CompetitionReadWorker`). Devfolio gets a narrow token that can ONLY do the
  score read-back — not full admin, and not mixed with any other worker gate
  (per the repo's scoped-credential convention). Wiring: `TokenRole` enum +
  `parse` + `storage_name`, the `TraceTenantAccessGrantRole` variant +
  `trace_tenant_access_grant_role_for_token` map arm, and the gate. Provisioned
  like any token (`TRACE_COMMONS_TENANT_TOKENS=tenant:competition_read_worker:<secret>`
  or a signed JWT `role=competition_read_worker`).
- **Cross-tenant read.** Devfolio traces live under many per-user tenants
  (`tenant-<hash(instance‖user)>`), so the query spans tenants. It reads through
  the existing `gate_driver_pool` (the `trace_gate_driver` Postgres role:
  `NOBYPASSRLS` + permissive cross-tenant `SELECT` policies from V36), the same
  mechanism the recompute/dedup passes use. No tenant GUC is set; the read is
  `SELECT`-only. **No migration** — the score columns (V23/V37/V39) and the
  driver grants (V36) already exist.
- **Store method.** `list_scores_by_submission_ids(&[Uuid])` on the gate-driver
  path (beside `list_gate_decisions_for_credit_scoring`), `WHERE submission_id =
  ANY($1)`, returning the **latest** decision per submission
  (`DISTINCT ON (submission_id) ... ORDER BY submission_id, decided_at DESC,
  decision_id DESC` (the `decision_id` tiebreaker keeps ties deterministic)).
- **Score bundle returned per id:** `credit_quality_micros` (the graded
  credit-quality `q`, headline) — nullable, since it is populated only after the
  shadow-mode `score-credit-quality` pass runs — plus `perplexity_micros`,
  `novelty_score_micros`, and `gate_passed` (= `perplexity_passed &&
  novelty_passed`). An id with a decision row but no `q` yet is returned with
  `credit_quality_micros: null` (distinguishable "gated but not credit-scored");
  an id with no decision row is omitted (distinguishable "not yet gated").
- **Guard.** If the gate-driver pool / `db_mirror` is not configured, the route
  fails closed with `503 SERVICE_UNAVAILABLE` (mirrors the recompute handlers).
- **Hash-only audit.** Emit `append_control_plane_read_audit(state, &auth,
  "scores_by_submission", results.len())` before returning — surface label +
  count only, never the score values or submission ids.

## Out of scope

- No devfolio-signed submission attestation; devfolio owns its
  submission↔ids mapping out of band.
- No devfolio field on the trace envelope / protocol (reverted).
- No new migration (score columns + gate-driver grants already exist).
- The read-back returns scores only — never trace content, contributor
  identity, or raw material.

## Conventions honored

- **Scoped credential** — a dedicated `CompetitionReadWorker` gate; not mixed
  with export/utility/etc. (CLAUDE.md).
- **Fail-closed** — 503 when the cross-tenant read backend is unconfigured;
  FORBIDDEN when the token role is wrong.
- **Hash-only / label-only audit** — surface + count, no scores/ids/content in
  audit rows or logs.
- **RLS intact** — cross-tenant read goes only through the purpose-built
  `trace_gate_driver` role's permissive SELECT policies; the runtime tenant pool
  is untouched.

## Testing

- Contributor side (done): `--project` cwd matching incl. hyphenated/sibling/
  prefix-collision + invalid-UTF-8 peek; `build_manifest` inclusion/exclusion +
  round-trip.
- Read-back: `TokenRole::parse`/`storage_name` round-trip for the new role; the
  gate admits admin + `CompetitionReadWorker` and rejects other roles with
  FORBIDDEN; the store method returns the latest decision per submission with
  `q` nullable and omits unknown ids; the handler emits the hash-only audit and
  503s when the backend is unconfigured.
