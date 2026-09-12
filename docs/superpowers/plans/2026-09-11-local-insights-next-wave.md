# Local Insights next wave: service, assessments, usage, and mission drafts

Date: 2026-09-11

This packet extends the [local implementation plan](2026-09-11-local-insights-implementation.md) and the [unified Insights program](2026-09-11-trace-insights-program.md). It supplies common service contracts, user-reported evidence, source-native usage inspection, and a local mission proposal check. Desktop screens and mission discovery remain follow-on work.

The intended product remains one Trace Commons experience. Native shells consume shared results and preserve the same evidence labels, missingness, and lifecycle. Multiple evaluator or scout implementations can eventually supply results without requiring separate dashboards.

## Implemented scope

| Slice | Entry point | Current behavior |
| --- | --- | --- |
| Local service | `insights::service::execute` and `dispatch_json` | Analyze, list, explain, delete, annotate, clear annotation, and inspect usage independently of enrollment or daemon startup |
| Native bridge | `tc_insights_call` | Handle-free synchronous JSON request/response API with owned strings and fixed error labels |
| Manual assessment | `LocalInsightStore::annotate` and `clear_annotation` | Typed category/outcome on an already saved snapshot, with timestamp and source-digest provenance |
| Native usage | `insights usage` | Inspect explicitly selected Codex or Claude Code JSONL without persistence, pricing, or per-model allocation |
| Mission proposal | `mission-draft --file` | Validate bounded local draft structure and return a proposal digest requiring curator review |

No new third-party dependency is required by this packet. Shared mission contracts live in the permissive protocol crate; local execution and persistence remain in the permissive contributor crate.

## Local service and FFI foundation

The service opens only the dedicated Insights directory. An explicit `store_dir` overrides the platform local-data default `trace-commons/insights`. Unsaved analysis and native usage do not open this store. CLI and native calls operate on the same saved reports when given the same directory.

The request has an optional `store_dir` and a tagged `operation`. Examples:

```json
{"operation":{"type":"analyze","source":"codex","file":"/chosen/session.jsonl","save":false}}
```

```json
{"store_dir":"/chosen/insights","operation":{"type":"annotate","id":"SNAPSHOT_ID","category":"refactor","outcome":"partial"}}
```

Operation tags are `analyze`, `list`, `explain`, `delete`, `annotate`, `clear_annotation`, and `usage`. JSON rejects unknown request fields and operation tags. `analyze` accepts `codex` or `trajectory`; `usage` accepts `codex` or `claude_code` in JSON. Responses use the corresponding `type` tag.

`tc_insights_call(request, request_len, err)` accepts UTF-8 JSON without a trailing NUL and bounds request size to 64 KiB before reading caller memory. Success returns an owned JSON string; failure returns NULL with an owned fixed error label when `err` is supplied. Callers free returned strings using `tc_string_free` and keep input buffers alive through return.

Operations perform synchronous local I/O. Desktop integrations must call them off the UI thread. Closing a window or dropping a UI task does not cancel a mutation that has started. This packet does not add SwiftUI, GTK, or Windows views, a daemon RPC method, or a remote analytics service.

## Manual assessments and snapshot lifecycle

A saved `LocalInsight` has an optional `manual_annotation` containing:

- `category`: `refactor`, `tests`, `docs`, `debugging`, `other`, or `unknown`.
- `outcome`: `accepted`, `partial`, `rejected`, or `unknown`.
- `provenance`: always `user_reported`.
- `recorded_at`: the assessment timestamp.
- `source_digest`: the source bytes to which the assessment applies.

Both labels are supplied together. No raw free text or user identity is retained. An explicit `unknown` assessment differs from no annotation. These labels describe the selected session proxy; they do not establish verified task boundaries, independent success, or per-model attribution. Existing derived `KnownOutcomes` metrics remain unchanged, and the legacy inferred `task_category` field remains absent/null.

Annotations require a saved report. Identical-content reimports and imported copies retain the shared report's annotation and its original assessment timestamp. A changed source digest does not inherit the old assessment. If another imported copy still refers to the old digest, its old report and assessment survive. Clearing removes only the assessment; deleting a report removes its annotation and all imported aliases while preserving original files.

Store schema v2 reads legacy v1 snapshots without inventing annotations and persists v2 on the next mutation. Invalid annotation fields, unsupported labels, and mismatched source digests are rejected on cache reads. Saved snapshots are refreshed only by explicit reimport; source edits or deletion are not monitored.

## Native usage inspection

Usage is a separate ephemeral result, not a new saved snapshot metric. Saved analysis still supports Codex and trajectory formats; Claude Code support in this packet is limited to the standalone usage path.

Codex reads cumulative `total_token_usage` records, preserving input, cached input, output, reasoning output, and total counters. It does not sum cumulative snapshots or add `last_token_usage`. Cached input and reasoning output are subsets, so they must not be added again. Regressing cumulative counters make totals unavailable.

Claude Code preserves input, cache-read input, cache-creation input, and output as separate categories. Stable message IDs deduplicate repeated assistant message snapshots. Conflicting or regressing duplicate records, missing required fields, and arithmetic overflow make totals unavailable. Missing counters are not zero-filled.

Results identify the explicitly selected file scope, candidate and complete record counts, bounded observed model labels, and a reason when totals are unavailable. Record counts include duplicates and do not establish complete session or per-request coverage. Model labels are declarations, not verified serving identities. Cumulative totals are not allocated across model changes.

The shared source reader limits files to 16 MiB. Usage does not read linked subagent files, discover sessions, save source bodies, estimate cost, or calculate billed spend. It does not provide task comparisons or model recommendations.

## Local mission draft review

`MissionDraft` records author and evaluator labels, title, source URLs, claim, task, starting artifact URL and SHA-256, rubric version, success criteria, required evidence, allowed models/tools, and duration/input/output token budgets.

Parsing is bounded to 64 KiB and rejects unknown fields, unsupported schema versions, empty required fields, malformed digest labels, non-HTTPS or embedded-userinfo URLs, and out-of-range budgets. The current duration limit is one day; each token budget is positive and at most ten million tokens. Budget validation checks structure and declared bounds; it neither enforces execution nor reserves funds.

Typed serialization binds the complete proposal to a SHA-256 digest. A change to its task, rubric, budget, or other content produces a different digest. The review output always reports `needs_curator_review`, `publication_authorized: false`, and `external_sources_verified: false`. Required reviews cover source claims/artifacts, reproducibility/rights, evaluator conflicts, and execution/budget.

No URL is fetched, artifact hash verified against downloaded bytes, evaluator identity authenticated, code executed, draft published, or reward authorized. Review JSON is a structural-check result, not an approval credential. The fixture contains illustrative placeholders and is not a qualified mission. This packet does not persist a mission catalog or implement a scout/feed.

## Commands

Run from the repository root. Build the contributor CLI and native bridge:

```bash
RUSTFLAGS='-D warnings' cargo build -p trace-commons-contributor --bin trace-commons-contributor
RUSTFLAGS='-D warnings' cargo build -p trace-commons-contributor-ffi
```

Analyze a selected file without persistence, then explicitly save it:

```bash
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- --json insights analyze --source codex --file /path/to/session.jsonl
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- --json insights --store-dir /path/to/insights analyze --source codex --file /path/to/session.jsonl --save
```

Use the returned snapshot ID for assessment and lifecycle commands. Replace `SNAPSHOT_ID` and paths with actual values:

```bash
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- insights --store-dir /path/to/insights annotate SNAPSHOT_ID --category refactor --outcome partial
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- --json insights --store-dir /path/to/insights explain SNAPSHOT_ID
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- insights --store-dir /path/to/insights clear-annotation SNAPSHOT_ID
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- --json insights --store-dir /path/to/insights list
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- insights --store-dir /path/to/insights delete SNAPSHOT_ID
```

Inspect source-native usage; the CLI spelling is `claude-code`:

```bash
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- --json insights usage --source codex --file /path/to/codex.jsonl
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- --json insights usage --source claude-code --file /path/to/claude.jsonl
```

Check the illustrative mission fixture locally:

```bash
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- --json mission-draft --file crates/trace-commons-protocol/tests/fixtures/mission-draft.json
```

## Verification and subsequent packets

Integration verification must include workspace tests for the FFI lifecycle change, warning-denied builds, formatting, applicable Clippy checks, the license-boundary test, and standalone permissive-crate builds. Synthetic fixtures cover annotation persistence/migration/invalidation, service dispatch and string ownership, usage missingness/deduplication/overflow, and mission draft bounds/digest/authority. This document does not assert a completed verification run or live source qualification.

Next implementation packets:

1. Build native Insights screens using this service: selected-file analysis, saved history, evidence views, and clearly labeled assessment controls before enrollment.
2. Qualify real adapter fixtures and usage semantics, then design versioned persisted usage with model/record coverage and cache validation before deriving costs.
3. Add task episode boundaries and Git/test/review evidence, preserving explicit attribution for mixed-model work before generating personal comparisons.
4. Add a local mission draft editor/history and curator workflow, followed by one reviewed scout source adapter and a unified discovery feed.
5. Qualify a second evaluator against shared evidence and presentation contracts before exposing provider discovery or wider community comparisons.

Hosted analytics, public rankings, mission execution, automatic publication, provider billing, and rewards remain separate authorization and qualification work.
