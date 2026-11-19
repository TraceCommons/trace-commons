# Ingest service: a measured simplification plan

Status: draft for owner review, 2026-11-10. Design only. No Rust code, no
workflow and no manifest changes in this PR. Nothing was built and no test
suite was run for it. Every number below is labelled measured or estimated.
Measured numbers come from `wc`, `grep` and small scripts over `main` at
e4882bf9b; section 9 says how each was produced.

## Intent

The owner asked for the `trace-commons-ingest` service to be simplified.
`CLAUDE.md` forbids splitting large files without a spec. This document is
that spec for the production code: it surveys the service, ranks the
changes that would make it smaller or easier to review, and says which of
them can start now.

It is the companion of
`2026-11-10-ingest-test-harness-compile-design.md` (draft PR #2055), which
covers the test harness. Two things follow from that spec and shape this
one:

- **This plan claims no compile-time saving.** #2055 measured that module
  boundaries inside one crate do not change the build (about 90% of it is
  single-threaded whole-crate work), and it withholds authority to split
  `trace-commons-ingest.rs` for compile time. The structural changes here
  are argued on review and ownership grounds only: a reviewer can read one
  area, a change touches one file, and two branches collide less often.
- **Its phases decide what can start.** Phases 2 and 3 of #2055 change the
  visibility and `cfg(test)` attributes of the roughly 1,040 items of `app`
  that the tests reference, and move `tests.rs` and its sibling test files.
  Section 6 sorts every change here by one rule taken from that.

The short version:

- The module is 88,080 lines: 1,838 free functions, 555 structs, 601
  constants, about 280 route registrations, and one `AppState` with 185
  fields.
- There is almost no dead code. The compiler already proves it: every item
  is private to a `pub(crate)` module and CI builds with `-D warnings`. One
  17-line shim is all that grep adds.
- The size is breadth and repetition, not rot. The largest removable
  amounts are four families of near-identical code (scheduler setup, drill
  evidence, the Prometheus body, database-mirror result handling), an
  estimated 1,200 to 1,900 lines together, and all four can be done as
  body-only edits.
- The module divides along real seams. The cheapest is drills and the
  operational summary: 5,500 lines that the rest of the module reaches
  through 26 names, and that the tests reach through 7.
- The largest reductions are not refactors. They are owner decisions about
  subsystems that no in-repo deployment configuration turns on (ranking,
  benchmarks), and about the file-backed storage mode beside the database.

## 1. What must not change

Every change in this plan is behaviour-preserving, and each of these
properties is a review item on every PR that follows from it:

- **Licensing.** The crate is AGPL-3.0-or-later. Every new `.rs` file
  carries the two-line copyright and SPDX header. No change adds a
  dependency edge; nothing moves into a permissive crate.
  `GET /v1/source` stays unauthenticated and outside every gate.
- **Fail closed.** A configured gate whose dependency is missing refuses
  with a safe missing-control name. A helper that replaces repeated code
  must refuse in every case the copies refused.
- **Tenant scoping.** Tenant and actor come from authentication. No helper
  takes a tenant id from a request body or an envelope.
- **Forced RLS.** No change alters which pool or role a query runs on.
- **Hash-only logging.** Log lines and audit rows keep their fields. Where
  this plan merges log call sites, the merged site logs the same hashed
  fields and no new ones.
- **Scoped worker credentials.** Each worker route keeps its own
  `require_*` check. Section 8 recommends against the one change that
  would blur them.
- **Gate contracts as trait objects.** Nothing here touches how scorers,
  embedders or indexes are held.
- **Drills feed smoke evidence.** The 18 drills keep their routes, their
  evidence rows and their evidence hashes, byte for byte.

## 2. Measurements

All counts are of `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs`
(below, "the app file") unless a row says otherwise. Line numbers are at
e4882bf9b.

### 2.1 Size

| | measured |
|---|---|
| app file | 88,080 lines |
| sibling production modules under `trace_commons_ingest_internal/` | 33 files, 21,504 lines |
| test corpus (`tests.rs`, `tests/`, `*_tests.rs`, `*_test.rs`, the fixture file) | 39 files, 166,247 lines |
| top-level items in the app file | 3,243: 1,838 `fn`, 601 `const`, 555 `struct`, 146 `impl`, 47 `enum`, 35 `mod`, 10 `trait`, 7 `type`, 4 `static` |
| lines by kind | `fn` 62,691; `impl` 8,413; `struct` 7,129; `const` 976; the rest under 700 |
| functions, including methods in top-level `impl` blocks | 2,185; median 16 lines |
| functions over 100 / 200 / 300 lines | 109 / 37 / 16 |
| lines inside functions over 100 / over 200 lines | 25,347 / 14,994 |

#2055 counts 3,053 top-level items. Counting only named items (no `impl`
blocks, no `mod` declarations) this survey finds 3,062. The two scripts
are independent and agree to within 9; the difference was not traced.

### 2.2 The twenty largest functions

Measured. End lines were checked for each (the line is the function's
closing brace and the next line starts another item).

| line | lines | function | test corpus mentions |
|---|---|---|---|
| 52746 | 1,284 | `trace_operational_metrics_body` | 17 |
| 78092 | 1,263 | `reconcile_db_mirror` | 15 |
| 4183 | 1,231 | `AppState::from_env_with_pipeline_runtime_assembler` | 0 |
| 8890 | 926 | `app` (the router) | 503 |
| 16408 | 684 | `submit_trace_handler_authenticated` | 3 |
| 76695 | 631 | `backfill_db_mirror_from_files` | 3 |
| 14222 | 557 | `trace_commons_config_status_response` | 1 |
| 86412 | 535 | `TraceOperationalPromotionGateSummary::from_inputs` | 0 |
| 30448 | 462 | `run_credit_settlement_unlocked` | 0 |
| 75681 | 447 | `run_maintenance` | 0 |
| 82399 | 362 | `TraceDbReconciliationReport::compute_blocking_gap_summaries` | 6 |
| 43030 | 332 | `run_process_evaluation_worker` | not counted |
| 77771 | 320 | `index_vector_metadata_from_db` | not counted |
| 64560 | 308 | `trace_commons_audit_event_from_storage` | not counted |
| 55441 | 304 | `run_benchmark_conversion_job` | not counted |
| 57503 | 304 | `evaluate_and_record_gate` | not counted |
| 60186 | 285 | `score_one_submission` | 6 |
| 17116 | 281 | `revoke_submission` | not counted |
| 29183 | 276 | `credit_cycle_scheduler_run_handler` | not counted |
| 49865 | 263 | `run_revocation_effects_drill` | 0 |

The largest `impl` blocks: `impl AppState` (line 4083, 1,332 lines, almost
all of it the constructor above), `impl TraceCommonsAuditEvent` (83198, 831
lines, 31 constructors with a median of 25 lines),
`impl TraceOperationalPromotionGateSummary` (86411, 537),
`impl TraceDbReconciliationReport` (82398, 364),
`impl ConfiguredTraceArtifactStore` (3005, 306), `impl AccountRateLimiter`
(22767, 239), `impl TraceCommonsSignedTokenVerifier` (11046, 221).

The largest structs: `AppState` (1846, 547 lines, 185 fields) and
`TraceCommonsConfigStatusResponse` (13925, 267 lines).

Long `match` arms are not a problem here. Of 449 arms that open a block on
one line, 9 are over 50 lines and one is over 100 (line 46989, 119 lines,
an `Err(error)` arm in the export-job runner). The next four are the
`Failed` and `Confirmed` arms of the two outbox confirm workers (33092,
33691, 33027, 33630; 61 to 87 lines each), which are the same shape twice.
Arms whose pattern spans several lines are not in this count.

### 2.3 Routes

Measured, by how each route is registered:

| how | count |
|---|---|
| `.route("...")` with a literal path, in the app file | 225 paths, 246 method and handler pairs |
| `.route(CONSTANT, ...)` with a path constant from the protocol crate (lines 8990 to 9219) | 7 |
| through the `AccountRoutes` builder, one method each: `account_route_groups` (8745) | 26 |
| through the same builder in 8 sibling modules | 24 |

That is 282 registrations. The number of method and path pairs is at
least 303 (246 + 7 + 50); the seven constant-path routes were not opened
to count their methods.

`scripts/operator/pipeline-deployment-inventory.py` reports 225 routes for
the same file. It matches literal `.route("` calls only, so it does not
see the other 57. That is a gap in the inventory tool, noted here because
section 5 uses the route list as an invariant.

The 225 literal paths by prefix:

| prefix | routes |
|---|---|
| `/v1/admin` | 108 |
| `/v1/workers` | 39 |
| `/v1/review` | 14 |
| `/v1/account` and `/account` | 18 |
| everything else | 46 |

By subject, counting path text: 31 ranking, 24 credit, 20 pipeline, 19
drill, 11 export, 10 benchmark.

No handler is unregistered: an unreferenced private function fails the
build. Going the other way, the static stem of each of the 225 literal
paths was searched for outside the ingest crate (`crates/`, `macos/`,
`windows/`, `tauri-desktop/`, `scripts/`, `deploy/`, `docs/`,
`README.md`). Every path is mentioned somewhere. 136 are mentioned only in
Markdown, with no caller in code or scripts. That is expected for admin
and drill routes an operator calls by hand, so it is a list of candidates
and not of dead routes. The notable group is ranking: all 31 ranking
routes are in it. The 57 other registrations were not cross-checked; the
constant-path ones share their constant with the clients, and the builder
ones are the signed-in account surface the apps use.

### 2.4 Dead code

Measured.

- No blanket `#![allow]` in the app file, `ingest.rs` or `lib.rs` (the
  last two were read in full; every `allow(` in the app file is an outer
  attribute on one item).
- 45 `#[allow(dead_code)]` attributes, 39 of them on top-level items. 33
  of the 39 sit on constants and helpers that are used only under
  `local-gpu-models`, `near-ai-scorer` or `gcp-kms` (lines 406 to 649,
  7231 to 7312, 8555). Those features are built, so none of that is dead.
  Three more (57367, 60095, 60186) are on gate-scoring items that have
  callers. The other three are the next two bullets and one
  `#[cfg(test)]` wrapper.
- One function has no caller at all:
  `append_trace_content_read_audit` (74429, 15 lines plus a 2-line
  comment that calls it a backward-compatible shim).
- `visible_submission_records_for_account` (63832, 9 lines) is called only
  from tests.
- 20 top-level items are `#[cfg(test)]`. Ten of them are functions with
  no caller outside the tests (wrappers and reset hooks, 116 lines
  together). They are test support, not dead.
- Feature-gated code: 41 `cfg(feature)` sites over five features
  (`near-ai-scorer` 18, `local-gpu-models` 15, `gcs-client` 3,
  `near-attestation-collateral` 3, `gcp-kms` 2). `ci.yml` compiles ingest
  with four of them. No `ci.yml` job passes `gcp-kms`; the pilot and
  staging build scripts under `deploy/` do. That is a gap in what CI
  checks, not dead code.

### 2.5 Repetition

Measured on the 76,521 non-blank, non-comment lines of the app file.

| measure | result |
|---|---|
| lines sitting in a second or later exact copy of a 6-line window | 6,936 |
| same, 10-line window | 3,097 |
| same, 20-line window | 741 |
| lines sitting in a second or later copy of a 12-line window after identifiers, strings and numbers are replaced by placeholders | 20,043 |

The last row overstates what can be removed: field lists, route
registrations and constant tables match each other under that rule and are
not duplication in any useful sense. It is here to show where to look. The
exact-copy rows are the floor.

The lines repeated most often:

| count | line |
|---|---|
| 591 | `.map_err(internal_error)` |
| 203 | `State(state): State<Arc<AppState>>,` |
| 113 | `let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;` |
| 110 | `tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),` |
| 59 | `append_audit_event_with_db_mirror(` |
| 55 | `let Some(db) = state.db_mirror.as_ref() else {` |
| 54 | `if let Err(error) = &mirror_result {` |
| 41 | `let tenant = authenticate_ctx_with_tenant_access_grant(state.as_ref(), &headers).await?;` |
| 31 | `if state.require_db_mirror_writes {` |
| 18 | `let evidence = TraceRolloutSmokeEvidenceResponse {` |

Families by name (top-level items whose name matches; a line can be in
more than one family):

| family | items | functions | lines | items the tests mention |
|---|---|---|---|---|
| credit | 446 | 274 | 10,335 | 172 |
| ranking and ranker | 405 | 271 | 9,849 | 133 |
| `*_handler` | 204 | 204 | 9,197 | 143 |
| audit | 197 | 157 | 4,825 | 57 |
| drill | 105 | 67 | 4,736 | 14 |
| benchmark | 223 | 125 | 4,541 | 70 |
| export | 199 | 125 | 4,406 | 56 |
| outbox | 142 | 102 | 3,563 | 56 |
| revocation | 110 | 78 | 3,471 | 35 |
| scheduler | 232 | 83 | 3,014 | 56 |
| `*_from_env` | 93 | 93 | 2,513 | 11 |
| operational summary and metrics | 46 | 19 | 2,131 | 25 |
| `reconcil*` | 19 | 9 | 1,982 | 6 |
| `mirror_*_to_db*` | 36 | 36 | 1,571 | 6 |
| `backfill*` | 11 | 7 | 996 | 3 |
| `*_with_db_mirror` | 21 | 21 | 749 | 5 |

### 2.6 Handlers and authentication

Measured over the 204 `*_handler` functions (median 32.5 lines).

- 164 call an `authenticate*` function, 159 of them as their first
  statement: 109 with `authenticate_with_tenant_access_grant`, 40 with
  `authenticate_ctx_with_tenant_access_grant`, 10 with `authenticate`.
- In the 164, the statement after that call is `require_admin` in 89,
  `require_reviewer` in 18, `require_utility_operator` in 16, one of 14
  other checks in 32 (six or fewer uses each: export worker, benchmark
  worker, exporter, vector, retention, revocation propagation, process
  evaluation and so on), and something else in 9. The narrow checks are
  where the scoped worker credentials are enforced.
- 86 of the 159 also take a `Json` body extractor in their signature, so
  for those routes the body is parsed before the handler authenticates.

### 2.7 Errors and responses

Measured.

- One error shape: `type ApiResult<T> = Result<T, (StatusCode, Json<ApiError>)>`
  (79995), used by 465 functions. 692 more return `anyhow::Result`.
- 781 `.map_err(` calls. 591 are `.map_err(internal_error)`.
  `internal_error` (80161) logs a hash of the error and returns a fixed
  500, or a busy response when the pool is exhausted. The remaining 190
  are spread over about 40 closures, none used more than 25 times.
- 10 `trace_*_scheduler_config_error` functions, 84 lines, differ only in
  the name they print.

So the plumbing is already one type and one main conversion. What repeats
is the call, not the code behind it.

### 2.8 Configuration surface

Measured.

- The ingest production source names 348 distinct `TRACE_COMMONS_*`
  variables, 298 of them in the app file.
- Of the 348, two are prefixes that `admission.rs` (lines 58 to 63)
  completes at run time, and two are `PIPELINE_TEST_*` names. So the count
  of whole variable names is 344, plus whatever the two prefixes expand
  to.
- `docs/operator/env-reference.md` names 352. 71 variables that ingest
  reads are not in it, including the storage-mode switches
  (`TRACE_COMMONS_DB_DUAL_WRITE`, `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES`,
  the `DB_*_READS` family) and 19 `TRACE_COMMONS_PIPELINE_*` names. Each
  of the 71 is named in some other document, script or workflow; the
  reference is incomplete as an index, not the documentation as a whole.
- 79 names in the reference are not literals in the ingest source. 66
  appear in another `.rs` file under `crates/`. Of the other 13, three
  are `ACCOUNT_ADMISSION_EVIDENCE_*` names built from the prefix above,
  and ten (`CLIENT_MAX_*_PER_DAY`, `UNBOUND_REAPER_*`) are in the
  reference as deprecated or ignored (two of the ten rows were read).
  None was found stale.
- The files under `deploy/` assign 52 of the 348. They assign none of the
  36 `CREDIT`, 28 `BENCHMARK`, 22 `PIPELINE`, 15 `VECTOR`, 14 `PROCESS`, 12
  `RANKING`, 12 `PERPLEXITY`, 10 `EXPORT` or 8 `RETENTION` names, and no
  `*_SCHEDULER_ENABLED` name.
- `deploy/pilot-gcp/ingest.env.template` sets every storage-mode switch
  the same way: dual-write on, mirror writes required, all five `DB_*_READS`
  on, all three `OBJECT_PRIMARY_*` on, all three `*_REQUIRE_OBJECT_REFS`
  on.

The live pilot's environment was not inspected. The templates are the only
evidence used here, and the pilot is known to carry settings they do not.
No variable is called unused on this evidence alone.

### 2.9 Seams

The app file is laid out in rough subject order, so contiguous line ranges
approximate the seams. For each range: its items, how many of them the
test corpus mentions, how many distinct items of other ranges it uses
("out"), how many of its items other ranges use ("in"), and how many of
the 185 `AppState` fields it reads through `state.`.

| | seam | lines | items | tests mention | out | in | state fields |
|---|---|---|---|---|---|---|---|
| A | constants, `AppState`, startup, env config | 8,889 | 609 | 211 | 194 | 245 | 22 |
| B | router, body gates, token verifier, scheduler spawns | 4,210 | 144 | 56 | 354 | 66 | 30 |
| C | config status, tenant policy, access grants | 2,100 | 69 | 27 | 101 | 41 | 96 |
| D | submit, upload routing, receipts, revoke, status | 3,800 | 96 | 36 | 109 | 33 | 24 |
| E | account: session, login, traces, withdrawal, limits | 5,500 | 205 | 88 | 69 | 74 | 14 |
| F | review, quarantine, lists | 1,500 | 63 | 24 | 79 | 43 | 7 |
| G | credit: types, handlers, settlement, outboxes | 9,000 | 330 | 132 | 162 | 192 | 32 |
| H | ranking and process evaluation | 9,000 | 283 | 71 | 199 | 137 | 16 |
| I | review decisions, export jobs, drivers | 4,500 | 136 | 63 | 258 | 67 | 10 |
| J | drills, operational summary and metrics | 5,500 | 96 | 7 | 96 | 26 | 30 |
| K | benchmark and ranker export | 3,000 | 110 | 53 | 145 | 84 | 7 |
| L | gate, PII backstop, dedup, scoring | 4,000 | 103 | 38 | 70 | 23 | 8 |
| M | policy and authorization helpers | 3,000 | 137 | 34 | 56 | 86 | 16 |
| N | storage reads, database mirror, revocation propagation | 9,000 | 330 | 118 | 132 | 201 | 8 |
| O | audit repair, maintenance, backfill, reconcile | 7,000 | 150 | 50 | 205 | 67 | 11 |
| P | record, response and audit-event types | 8,078 | 201 | 62 | 107 | 116 | 46 |

1,501 distinct item names cross a range boundary. 22 items are used from
eight or more other ranges; the widest are `AppState`, `TenantAuth`,
`tenant_storage_ref`, `ApiResult`, `TraceCommonsAuditEvent`, `api_error`,
`internal_error` and `sha256_prefixed`.

The ranges are not clean. Credit types sit in G but ranking's gate on
settlement makes G use 51 names of H and H use 78 of G; review code is
split between F and I. The numbers are good enough to rank seams and not
good enough to plan a move line by line. Each move PR starts by producing
its own exact list.

What a seam costs to extract depends on which way it is cut:

- **As a child module of `app`**, the way the 33 existing sibling files
  are. A child sees its parent's private items through `use super::*`, so
  the "out" column costs nothing. Only the "in" names need `pub(super)`,
  and one `use child::*;` in `app` keeps every existing path, including
  the tests' `use super::*`, resolving.
- **As a crate.** Both columns become public API, `AppState` has to be
  split first, and #2055 option (c) lists the rest of the cost. Section 8.

## 3. Findings

Each finding gives its evidence, the size of the change, and the risk.
"Removed" means net lines deleted; "moved" means lines that leave the app
file for another file in the same crate.

### F1. Scheduler setup is written out ten times

Evidence (measured): 10 `parse_trace_*_scheduler_config_from_env`
functions, 619 lines (7616 is a typical one); 11
`spawn_trace_*_scheduler_task` functions, 519 lines (11679); 11
`validate_*_scheduler_config` functions, 344 lines; 10
`trace_*_scheduler_config_error` functions, 84 lines. Each parser reads an
enabled flag, a token, an interval, a limit and a purpose with the same
calls in the same order; each spawner logs the settings and wraps one tick
function in `spawn_driver_loop`. The test corpus mentions one of the 21
parse and spawn functions.

Change: one description per scheduler (its variable names, bounds,
defaults and tick function) and one generic parser and spawner. The
validators differ in substance and stay.

Size: estimated 500 to 700 lines removed of 1,222.
Risk: low. The error text names the variables, and operators read it, so
the generic code must produce the same strings. Variable names do not
change.

### F2. Each drill builds, hashes and records its evidence by hand

Evidence (measured): 18 `run_*_drill` functions, 2,522 lines; 17
`*_drill_evidence_hash` functions, 691 lines; 9 `*_drill_blocking_gaps`
functions, 386 lines; 18 sites that build a
`TraceRolloutSmokeEvidenceResponse`, pick `Passed` or `Failed`, and append
`TraceCommonsAuditEvent::rollout_smoke_evidence`. The test corpus names
none of the `run_*_drill` or evidence-hash functions; it reaches them
through the routes.

Change: one helper that takes a drill's check name, readiness and evidence
value, and does the status, hash and audit append. The evidence-hash
functions become the value each drill passes in.

The hash functions themselves are pure: each takes plain values and
returns `json_evidence_hash` of a JSON object that lists them
(`audit_chain_drill_evidence_hash`, 52020, is typical). They are the
schema of each drill's evidence and mostly stay. What merges is the code
around them.

Size: estimated 150 to 300 lines removed. Low confidence: two of the 18
sites were read.
Risk: medium. Promotion gates compare the evidence hash across runs. The
change is correct only if every drill's hash is byte-identical before and
after for the same inputs. Because the hash functions are pure, a test
that calls each with fixed arguments and pins the result is cheap, and it
comes first (section 5).

### F3. The Prometheus body is a 1,284-line function

Evidence (measured): `trace_operational_metrics_body` (52746). 47 `# HELP`
blocks and 48 `push_prometheus_gauge` calls, each written as two
`push_str` calls and a seven-line call. The tests call it (17 mentions).

Change: a table of metric name, help text, labels and value, and a loop.
The function keeps its name and signature.

Size: estimated 400 to 600 lines removed.
Risk: low. The output is text; a byte-for-byte comparison of the body for a
fixed summary proves it. Metric names and label sets are scraped and must
not change.

### F4. Database-mirror results are handled at 54 sites

Evidence (measured): `if let Err(error) = &mirror_result {` 54 times,
`enforce_db_mirror_write_result(` 57, `if state.require_db_mirror_writes {`
31, `let Some(db) = state.db_mirror.as_ref() else {` 55. A typical site
(16948 to 16999, twice within 50 lines) calls a `mirror_*_to_db` function,
logs a hashed warning, and then enforces.

Change: fold the warn-then-enforce step into one helper for the sites
where it is uniform. Sites that roll something back between the two (the
submit path does) stay as they are.

Size: estimated 150 to 300 lines removed. Low confidence: the 54 sites
were counted, not sorted by shape, and the two that were read are of the
kind that stays. The PR's first step is that sort; if fewer than half are
uniform, drop the change.
Risk: medium. This is a fail-closed path:
`TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES` turns a mirror failure into a
refusal. The helper must refuse wherever a copy refused, and log the same
hashed fields. Do it site by site with the PostgreSQL suites, not with a
search and replace.

### F5. Drills and the operational summary are a self-contained 5,500 lines

Evidence (measured): seam J in 2.9. 96 items; the rest of the module uses
26 of them; the test corpus mentions 7. With the summary types in seam P
(`impl` blocks on `TraceOperational*` types, 19 blocks, 1,507 lines) it is
about 7,000 lines.

Change: move it to two child modules, `drills.rs` and `operational.rs`,
beside the existing siblings. 26 names gain `pub(super)`; `app` gains two
`mod` lines and two glob imports.

Size: about 5,500 to 7,000 lines moved; none removed.
Risk: low for behaviour, since nothing but paths changes. Two things read
the app file as text and must move with it:
`crates/trace-commons-ingest/tests/smoke_gate_script.rs` (lines 109 and
149) looks up the drill request and response structs in
`trace-commons-ingest.rs` by path, and would fail, correctly, the moment
they leave. `scripts/operator/pipeline-deployment-inventory.py` (line 179)
reads the routes from the same file, which is one reason the router stays
where it is in every change here. The other risk is conflict with other
branches, which is why section 6 schedules the move.

### F6. `AppState` is the hub: 185 fields and a 1,231-line constructor

Evidence (measured): struct at 1846 (547 lines); constructor at 4183
(1,231 lines, no test mentions); 93 `*_from_env` functions, 2,513 lines;
72 `env_truthy` and 115 `optional_trimmed_env` calls. Seam C alone reads
96 of the fields to build the config-status response, a 267-line struct
filled by a 557-line function. The tests read 43 distinct fields through
`state.`; the sibling modules mention `AppState` 247 times.

Change: group the fields into one struct per area (storage mode, tenant
rollout, schedulers, credit, ranking, benchmark, account, pipeline), each
with its own `from_env`. The constructor becomes a list of those calls.
Config status can then be built per group.

Size: estimated 300 to 500 lines removed, most of it from config status;
the rest is moved. Estimated low confidence.
Risk: medium to high for conflicts, low for behaviour. Every
`state.field` access for a grouped field changes, in production code, in
the siblings and in the tests. This is the change every crate-level split
would need first, and it is the one most likely to collide with #2055.

### F7. Two subsystems have no in-repo caller and no in-repo configuration

Evidence (measured):

- Ranking: 405 items, 9,849 lines by name, plus 7 `impl` blocks (343
  lines); 31 routes, none with a caller in code or scripts, each named in
  one operator document; 12 `TRACE_COMMONS_RANKING_*` variables, none
  assigned under `deploy/`.
- Benchmarks: 223 items, 4,541 lines; 10 routes; 28
  `TRACE_COMMONS_BENCHMARK_*` variables, none assigned under `deploy/`.
- Process evaluation: 49 items, 1,392 lines; 14 variables, none assigned.

`docs/trace-commons-roadmap.md` lists "Ranking evidence storage with
calibration / promotion / model-risk gates" as current capability, and
says in its opening that the rollout apparatus was sized for tenants who
did not exist when it was written.

This is not a finding that the code is dead. The live pilot was not
inspected, an operator can call any of these routes with curl, and credit
settlement reads a ranking calibration gate (seam G uses 51 names of seam
H). It is a finding that about 16,000 lines serve features this repository
never turns on, and that only the owner knows whether the pilot does.

Change: none without a decision. Section 7.
Size: up to an estimated 14,000 to 16,000 lines if both were retired; zero
otherwise.
Risk: high. Removal is not reversible by a flag.

### F8. The file-backed store and the database run side by side

Evidence (measured): 36 `mirror_*_to_db*` functions (1,571 lines), 21
`*_with_db_mirror` (749), `backfill_db_mirror_from_files` (631 lines),
`reconcile_db_mirror` (1,263 lines), 19 `reconcil*` items (1,982 lines),
143 uses of `state.db_mirror`, which is an `Option`. The pilot template
turns every database switch on (2.8). `docs/trace-commons-storage.md` step
5 ("Switch reads to DB-first") is described as the next major cutover and
keeps file-backed records as a compatibility fallback.

Against removal (measured): `tests.rs` mentions `db_mirror` 560 times, and
the ingest PostgreSQL tests self-skip without a database, so a large part
of the 2,006 tests runs in file-backed mode with no database at all. The
rollback, reconciliation and audit-chain drills compare the two stores and
feed smoke evidence.

Change: none now. Section 7.
Size: an estimated 6,000 to 9,000 lines of production code if file-backed
mode were retired, and a rewrite of the test fixture that most tests use.
Risk: very high.

### F9. Almost nothing named "legacy", "compat" or "deprecated" has expired

Evidence (measured): 28 top-level items have `legacy` in their name, 585
lines; four have `compat`; the word `deprecated` marks one variable. By
what they serve:

| group | items | lines | why it is still live |
|---|---|---|---|
| Legacy upload path (`claim_legacy_receipt`, 16345) | 1 | 28 | `docs/operator/pipeline-activation.md`: where an upload goes when a tenant has no routing row or a `legacy` row, what `deactivate` returns a tenant to, and the permanent owner of every receipt it claimed. `GET /v1/admin/pipeline/legacy-drain` reports what it still owes. |
| Audit legacy segments (659, 64981 to 65341, 73098 to 73654, 83086) | 23 | about 530 | Audit rows written before the hash chain covered them. `docs/operator/audit-trail-forensics.md` documents the repair and its switch, `TRACE_COMMONS_ALLOW_LEGACY_SEGMENT_RESUME`. Needed for as long as such rows exist in a deployment's data. |
| `legacy_principal_ref` (79987) | 1 | 3 | A fixed label, used 16 times, 30 test mentions. |
| Store label `TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE` (371) | 1 | 1 | A stored value, not a switch. |
| Compatibility status for clients (`compatibility_statuses_from_database` 18759, `compatibility_credit_decision` 18720) | 2 | 96 | Maps pipeline results to the status shape existing clients read. |
| Old session cookie (`append_legacy_account_session_clear`, 22276) | 2 | 21 | Expires the cookie name used before the `__Host-` prefix. The old name is never read. |
| `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` warning (594, 6648 to 6658) | 1 | about 15 | Warns when a variable that is no longer read is still set. `local-gpu-models` builds only. |

The legacy-invite routes live in a sibling module
(`legacy_invite_link.rs`, 119 lines) and are covered by
`docs/operator/legacy-invite-migration.md`.

The last two rows are the only true closing windows, about 36 lines
together. Each closes on a date the owner picks: the cookie shim once no
browser can still hold the old cookie (its lifetime was not looked up),
the warning once no deployment sets the variable. The migration history
(150 files under `migrations/`) was not read file by file; nothing in the
app file is keyed to a migration number in a way this survey could test.

Conclusion: no legacy code is removable today. The upload path leaves only
when the owner retires the rollback target, after the drain report is
empty for every tenant.

### F10. The env reference is an incomplete index

Evidence: 2.8. 71 variables that ingest reads are not in
`env-reference.md`. No documented name was found stale.

Change: a documentation PR that adds the 71, each with a pointer to the
document that already describes it.
Size: no Rust lines.
Risk: none to the service.

### F11. One dead shim

Evidence: `append_trace_content_read_audit` (74429): `#[allow(dead_code)]`,
one occurrence of its name in the app file, none in the siblings or the
tests.

Change: delete it and its comment. Size: 17 lines. Risk: none; the
compiler confirms it.

## 4. Ranked changes

Safest and highest value first. Each row is one reviewable PR. Sizes are
estimates unless marked.

| # | change | finding | removed | moved | risk | when |
|---|---|---|---|---|---|---|
| 1 | Complete `env-reference.md` from the source | F10 | 0 | 0 | none | now |
| 2 | Delete the dead audit shim | F11 | 17 (measured) | 0 | none | now |
| 3 | One generic scheduler parser and spawner | F1 | 500 to 700 | 0 | low | now |
| 4 | Table-driven Prometheus body | F3 | 400 to 600 | 0 | low | now |
| 5 | One drill-evidence helper, after pinning each drill's hash | F2 | 150 to 300 | 0 | medium | now |
| 6 | One mirror-result helper for the uniform sites | F4 | 150 to 300 | 0 | medium | now |
| 7 | Drills and operational summary to child modules | F5 | 0 | 5,500 to 7,000 | low | after #2055 phase 2 |
| 8 | Gate, PII backstop, dedup and scoring to a child module (seam L: 23 inbound names) | 2.9 | 0 | about 4,000 | low | after #2055 phase 2 |
| 9 | Submit, routing and receipts to a child module (seam D: 33 inbound names) | 2.9 | 0 | about 3,800 | medium | after #2055 phase 3 |
| 10 | Group `AppState` by area; config status per group | F6 | 300 to 500 | about 3,000 | medium | after #2055 phase 3 |
| 11 | Ranking and process evaluation to a child module (seam H: 137 inbound) | 2.9 | 0 | about 9,000 | medium | after 10 |
| 12 | Credit and outboxes to a child module (seam G: 192 inbound) | 2.9 | 0 | about 9,000 | medium | after 10 |
| 13 | Storage, mirror, audit repair and maintenance to child modules (seams N and O) | 2.9 | 0 | about 16,000 | medium | after 10 |
| 14 | Account to a child module (seam E: 74 inbound) | 2.9 | 0 | about 5,500 | medium | after 10 |

Totals, estimated: changes 2 to 6 remove 1,200 to 1,900 lines. Changes 7
to 14 move about 53,000 to 57,000 lines out of the app file and leave it
at roughly 30,000: startup, the router, authentication, policy helpers and
shared types. Nothing in the table changes behaviour.

Changes 11 to 14 are ordered by nothing stronger than size. Do them in
whatever order the open branches make cheapest, one at a time.

## 5. Verification

For every PR:

```bash
export RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0
cargo fmt --all -- --check
cargo check -p trace-commons-server -p trace-commons-ingest --bins
cargo check -p trace-commons-ingest --features near-ai-scorer
cargo check -p trace-commons-ingest --features local-gpu-models
# The pilot's feature set (deploy/pilot-gcp/deploy.sh). No ci.yml job
# compiles ingest with gcp-kms.
cargo check -p trace-commons-ingest \
  --features gcs-client,gcp-kms,near-ai-scorer,near-attestation-collateral
cargo test -p trace-commons-ingest --no-run
cargo clippy -p trace-commons-server -p trace-commons-ingest --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo test -p trace-commons-ingest --lib
cargo test -p trace-commons-server --test it -- 'license_boundary::'
scripts/dev/pg-suites-local.sh
```

The feature checks matter because 33 of the `allow(dead_code)` items exist
only for those builds; a move that compiles under default features can
fail there. Builds go through the build lock and the suites through the
test lock, as `CLAUDE.md` describes.

A passing count is not enough, because a filter that matches nothing
passes. Each PR also compares, before and after:

- **Test names.** `cargo test -p trace-commons-ingest --lib -- --list`,
  sorted. Every name in the first list must be in the second; the only
  additions are the tests the PR says it adds. The names under
  `ingest::app::tests::` are pinned by `scripts/operator/pipeline.py` and
  `pipeline_tooling/`; no change here renames a test module.
- **Routes.** The sorted list of registrations, taken three ways because
  no one tool sees all of them (2.3): the 225 literal paths and their 246
  method and handler pairs, the 7 constant-path registrations, and the 50
  builder registrations.
- **Variable names.** The sorted list of `TRACE_COMMONS_*` strings in the
  production source. 348 today, two of them prefixes.

Per change:

| # | what proves it |
|---|---|
| 2 | The build. |
| 3 | The existing scheduler-config tests (56 scheduler items are mentioned by tests). Add one test per scheduler that asserts the error text for a missing token, written against the current code first. |
| 4 | Capture the body for a fixed `TraceOperationalSummaryResponse` from the current code; assert the new code returns the same bytes. The 17 existing uses stay. |
| 5 | Before the refactor, a test per evidence-hash function that calls it with fixed arguments and pins the result. Then `scripts/dev/pg-suites-local.sh` and the smoke script test (`cargo test -p trace-commons-ingest --test smoke_gate_script`), which ties `scripts/operator/smoke-gate.sh` to the drill structs. |
| 6 | `scripts/dev/pg-suites-local.sh` in full, and the whole ingest library against PostgreSQL with the login-resolver URL, single-threaded, as `CLAUDE.md` gives it. The file-only tests do not reach this code with a database. |
| 7 to 9, 11 to 14 | A move is correct when nothing but paths and visibility changed. Check with `git diff --color-moved=zebra` and by comparing the sorted set of item bodies before and after. Then everything above, plus `python3 scripts/operator/pipeline.py qualify` for any move that touches pipeline or submit code. For 7, update the two text readers named in F5 in the same PR. About 120 files under `docs/`, `scripts/`, `crates/` and `deploy/` name the app file's path, most of them documents; a move PR greps for the items it moved and corrects the ones that point at them. |
| 10 | Everything above. Config status is an operator contract: capture its JSON for a fixed environment before and assert it is equal after. |

New tests written for changes 3 to 5 go in new files, not into `tests.rs`,
and in whatever location #2055 has reached by then.

## 6. What can start, and what must wait

The rule, from what #2055 changes:

- **A body-only edit can start now.** It changes no signature, no
  attribute and no item's place, so it does not touch the lines phases 2
  and 3 of #2055 edit, and no test stops resolving.
- **Deleting an item no test mentions can start now.**
- **Anything that moves an item, changes a signature, or changes a field
  the tests read waits.** A move conflicts with a visibility edit made
  inside the moved text, and the conflict has to be resolved by hand over
  thousands of lines.

Applied:

| # | start | why |
|---|---|---|
| 1 | now | Documentation only. |
| 2 | now | No test mentions the function. |
| 3 | now | No test mentions the 11 spawners; one parser is mentioned and keeps its name and signature. |
| 4 | now | Body-only; the function keeps its name and signature. |
| 5 | now | No test mentions the 18 run functions or the 17 hash functions. |
| 6 | now, last of the six | Body-only, but inside handlers the tests call, and on a fail-closed path. Let 3 to 5 show the pattern first. |
| 7, 8 | after #2055's phase 2 gate | The tests mention 7 and 38 of these items. Once phase 2 has decided whether phase 3 goes ahead, the visibility edits to these ranges are either done or cancelled. |
| 9 to 14 | after #2055 phase 3 lands, or after that spec is closed at its gate | Between 36 and 211 test-mentioned items per seam. Moving them first would force phase 3 to redo its work. |

Changes 2 to 6 edit the app file, which #2055 also edits. They do not
edit the same lines. If the owner wants no overlap at file level at all,
only change 1 starts now and 2 to 6 start after #2055 phase 1.

Two things the moves must keep for #2055 whenever they land:

- `app` re-exports each child with a glob import, so `use super::*` in
  the tests and the paths in the operator tooling keep resolving.
- A moved item keeps whatever `pub` or
  `cfg(any(test, feature = "test-support"))` #2055 gave it.

## 7. Decisions for the owner

These are the large numbers, and none is a refactor.

1. **Ranking, benchmarks and process evaluation (F7).** Does the pilot run
   any of them? If not, retire them (an estimated 14,000 to 16,000 lines,
   more than 40 routes, 54 variables), or keep them and take change 11. Credit
   settlement's calibration gate has to be handled either way.
2. **File-backed mode (F8).** Is a deployment without the database still
   supported? If not, the service can become database-only in stages. That
   is its own spec: it changes the test fixture most tests use and the
   drills that compare the two stores.
3. **File-level overlap with #2055 (section 6).** Start changes 2 to 6
   now, or hold them until its phase 1.
4. **The two closing windows in F9.** When the old-cookie shim and the
   `MODEL_ARCH` warning may go. About 36 lines.

## 8. Recommended against

- **A crate split now.** #2055 option (c) gives the cost. From this
  survey: 1,501 names cross seam boundaries and every seam reads
  `AppState`. Change 10 is the precondition, and after changes 7 to 14 the
  boundaries will be visible in the module tree. Decide then.
- **An authentication extractor, as a simplification.** 159 handlers
  start with the same call, and an extractor would remove it. It is not
  behaviour-preserving: 86 of them take a `Json` body, which today is
  parsed before the handler authenticates, and an extractor placed ahead
  of the body changes which refusal such a request gets. It also rewrites
  159 signatures that the tests call, and it moves each route's role check
  out of the handler body, where a reviewer sees it today. The saving is
  one line per handler. Whether authentication should run before the body
  is parsed is a separate question with its own answer; if the owner wants
  that order, it is a behaviour change with its own PR and tests, and it
  would bring the extractor with it.
- **A blanket error conversion in place of `.map_err(internal_error)`.**
  591 sites, and an `impl From` would let `?` do it. It would also make
  every error that reaches a handler a logged 500 by default. Today that
  is a choice visible at each call. It changes the `ApiResult` alias used
  by 465 functions. Keep the explicit call.
- **Merging the `require_*` role checks or generating the route table
  from a macro.** The narrow role checks are few and deliberate, and
  the 926-line router is long because it is the one place that states
  every route, its body limit and its layers.
- **Splitting long ordered functions for length alone.**
  `submit_trace_handler_authenticated` (684 lines) and
  `run_credit_settlement_unlocked` (462) are sequences of fail-closed
  checks. Cutting them into pieces spreads the order over several
  functions and removes nothing.
- **Removing routes because no code calls them.** 136 routes have no
  in-repo caller. Operators call them by hand and 19 of them are drills.
- **Removing the legacy upload path or the audit legacy-segment code
  (F9).** One is the rollback target; the other reads data that exists.
- **Any of this for build time.** #2055 measured that it does not help.

## 9. How the numbers were produced

All from a checkout of `main` at e4882bf9b, with no build.

- **Items and sizes.** A script reads the app file, matches item headers
  at column 0 and at four spaces, and takes an item to end at the first
  closing brace at the same indent. The file is rustfmt-formatted, which
  makes that reliable; the 20 largest results were checked by eye.
  Methods are counted only inside top-level `impl` and `trait` blocks.
- **Test mentions.** For each named item, the count of whole-word
  occurrences of its name in the test corpus with line comments removed.
  This is the same name-based method as #2055 section 2.6 and has the same
  weakness: a common name can match something else. It finds 1,070
  test-mentioned top-level items against #2055's 1,041.
- **Routes.** Every `.route(` line in the app file and its production
  siblings, split by whether a string literal follows; builder
  registrations by matching `.get(`, `.post(` and the other builder
  methods followed by a path, in `account_route_groups` and in the
  siblings that use `AccountRoutes`. Callers by `grep -rlF` of a literal
  path up to its first `{`.
- **Repetition.** Windows of consecutive non-blank, non-comment lines with
  whitespace collapsed; a line counts once if it lies in any window that
  occurred earlier in the file.
- **Families.** A regular expression on item names.
- **Seams.** Each item is assigned to the line range it starts in. A
  reference is a whole-word occurrence, in one range, of a name defined in
  exactly one other range. Names defined twice (the `cfg` pairs) are left
  out.
- **Variables.** String literals matching `TRACE_COMMONS_[A-Z0-9_]+` in
  production source; documented names are the backticked ones in
  `env-reference.md`; assignments are uncommented `NAME=` lines under
  `deploy/`.

## 10. Not measured

- The live pilot's environment and which routes it receives.
- How many tests run with a database mirror and how many without.
- Private fields and methods that cross a seam; the seam counts are of
  top-level items.
- The sibling modules' own repetition. They are 21,504 lines and were
  sized, not surveyed.
- Any build or test time. This plan claims none.
