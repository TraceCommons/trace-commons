# Crate splitting and build caching: what the measurements say

Status: draft for owner review, 2026-10-17. No code has changed.

## Summary

The question was whether splitting the large crates (`trace-commons-server`,
`trace-commons-contributor`) would give sccache and cargo more units to
reuse, so that an edit rebuilds less. We measured first (Appendix A), at
`a5ebb90bc`. The three commits between that and `61570a3cc`, the base of
this spec, do not touch `crates/trace-commons-server/src/bin`,
`src/lib.rs` or the server `Cargo.toml`.

The answer is narrower than the hypothesis:

1. **A general crate split is not the main lever.** The dominant edit-loop
   cost is one unit, the server library's test harness
   (`cargo test -p trace-commons-server --lib`). It takes about 78 s wall and
   156–250 s user CPU, and every server edit triggers it, including an edit
   to `tests.rs` or to a module that ingest never touches. sccache never
   caches it, because a test harness is a bin crate-type. Splitting the
   *rlib* into more crates does not shrink that unit. Only two things do:
   making the harness smaller, or making it incremental.
2. **Incremental compilation is the largest win we measured.** It cuts the
   same edit from about 78 s to 45.8 s, and to 22.7 s on the next edit. It
   is off locally only because the environment sets `CARGO_INCREMENTAL=0`
   for CI parity. Step 0 is a local-only change and needs no code.
3. **One split does pay: extract the ingest service into its own AGPL
   library crate, `trace-commons-ingest`.** Since #1428 the ingest service
   (`src/bin/trace-commons-ingest.rs` plus `trace_commons_ingest_internal/`)
   is compiled into the server library as `ingest::app`. That makes the
   server harness carry all of it, and makes every ingest edit rebuild the
   server rlib that the 10 bins, 61 integration tests and the contributor
   tests link. Extracting it is the one boundary where the measured costs
   fall on both sides. The projections are in section 4.
4. **Moving `tests.rs` alone into an integration-test crate is not the cheap
   step it looks like.** `tests.rs` does `use super::*` and reaches about 486
   private functions and 255 private types of the app module. Across a crate
   boundary all of those would have to become `pub`.
5. **Protocol fan-out is inherent and stays.** An edit to
   `trace-commons-protocol` rebuilds everything downstream (99 s for server
   `--lib`, 144 s for contributor tests), because the changed rlib hash
   invalidates every downstream cache key. No split changes that.
6. Linker choice and debuginfo level were **not measured**. They stay in the
   "measure before deciding" list (section 6), not in the recommendations.

## 1. What the measurements show

The full table is Appendix A. These are the readings that decide the design.

**Where the time goes on a server edit.** Scenarios 2a, 3 and 4a edit an
ingest handler, `tests.rs`, and an unrelated server module
(`audit_chain.rs`). Each rebuilds exactly one unit, the server `lib (test)`
harness, in 75–80 s wall. The cost does not depend on what was edited.
Reverting the edit and rebuilding costs the same again (`bl_after*_lib`,
about 78 s), because the harness is never in sccache.

**Frontend against codegen** for that unit:

- `cargo check --lib --profile test` takes 57–80 s user CPU.
- `cargo test --lib --no-run` takes 156–250 s user CPU.
- The frontend is therefore roughly a quarter to a third of the CPU, and
  serial: about 45–65 s of a 68–80 s wall on a quiet machine.
- Codegen runs in parallel across codegen units and adds roughly 15–30 s of
  wall.
- The serial frontend is the floor. Only less source (a smaller harness) or
  incremental reuse lowers it.

**What the rlib costs.** The server rlib compiles in 44–53 s. It is
needed by `--bin trace-commons-ingest` (2b: 54.6 s), by the 61 integration
tests and 10 bins (4b), and by contributor tests through the dev-dependency
(4c: server rlib 46 s of 94.4 s). After a revert it comes back from sccache:
`--bin` takes 4.1 s against 54.6 s. Here caching works as intended. The
problem is that an ingest edit invalidates it.

**sccache hit rates.** Cold dependency builds hit 93–97%. Edit scenarios hit
about 0%, because the edited crate misses and the harness units cannot be
cached. Raising the hit rate on edits would need fewer uncacheable units on
the path, not more cacheable ones.

**Contributor** (scenario 5, edit in `managed/copy.rs`):

| Unit | Cost |
|---|---|
| contributor rlib | 25.9 s |
| contributor `lib (test)` | 45.9 s |
| 40 test binaries | about 115 s CPU in total, 1–5 s each |

The server rlib was not rebuilt. The contributor costs are real but about
half the server's, and are dominated by the same harness pattern.

## 2. What is in the server harness today

Line counts at `61570a3cc` (identical to the measured `a5ebb90bc` for these
paths):

| Part | Path | Lines |
|---|---|---|
| Server library modules (not `src/bin/`) | `crates/trace-commons-server/src/**` | 134,094 |
| Ingest app | `src/bin/trace-commons-ingest.rs` | 86,559 |
| Ingest internal modules and tests | `src/bin/trace_commons_ingest_internal/**` | 172,269 |
| of which `tests.rs` | `trace_commons_ingest_internal/tests.rs` | 101,641 |
| of which `tests/` | `trace_commons_ingest_internal/tests/` | 10,306 |
| of which non-test-named | (route and runtime modules) | 16,084 |
| Other bins and their modules | `src/bin/**` excluding ingest | 17,108 |

The harness compiles the first three rows, 392,922 lines. Ingest accounts
for 258,828 of them (65.9%) and the library modules for 134,094 (34.1%). The
library modules hold 1,171 `#[test]` / `#[tokio::test]` functions; ingest
holds about 1,800, 1,321 of them in `tests.rs`.

The rlib compiles the non-test parts: about 127k lines of library (134,094
minus about 7.1k test-named) plus 86,559 + 16,084 lines of ingest, about
229.6k in total. Ingest is about 45% of that.

## 3. Options considered

| Option | Effect on the harness unit | Cost | Verdict |
|---|---|---|---|
| A. Split the server *library modules* into several crates (e.g. account, pipeline, storage) | None. The harness still compiles the whole library plus ingest, and stays uncacheable | Large: 80 files with inline tests, cross-module `pub(crate)` | Rejected. It does not touch the measured bottleneck |
| B. Move `tests.rs` (and `*_pg_tests.rs`) into an integration-test target | Removes about 112k test lines from the harness, but adds an rlib plus test-binary pair on every handler edit | About 740 private items made `pub` (486 fns, 255 types, by a name-match count of `use super::*` reach) | Deferred. Too much API surface for an unmeasured gain |
| C. Extract ingest into its own AGPL lib crate `trace-commons-ingest`, keeping `#[path]` layout and privacy | Splits the harness 66/34. Ingest edits stop invalidating the server rlib | Moderate. About 10 `pub(crate)` items widened, plus CI and docs churn (section 5) | **Recommended (step 2)** |
| D. Incremental for workspace crates, locally | 78 s to 45.8 s, then 22.7 s (measured) | Env change. Cold base build 151.6 s, and workspace crates lose sccache | **Recommended (step 0), local only** |
| E. Faster linker, lower debuginfo for test profile, codegen-units | Unmeasured. Link was estimated at 1–4 s per executable | Profile change | Measure first (section 6) |
| F. Split `trace-commons-contributor` (e.g. `daemon/`, 131k of 277k lines) | Same harness argument as A | Large | Rejected for now. Revisit only with contributor-specific data |

Option C is a reversal of part of #1428, and should be read that way. #1428
moved ingest into the library so that an embedding binary could call
`ingest::run` with its own `ScorePolicyRegistry`. That seam survives
unchanged:

- `ScorePolicyRegistry` stays in the server crate
  (`src/versioned_pipeline_score_registry.rs`).
- `IngestOptions` and `run` move into the new crate along with the app.

What changes is the path. An embedding binary would name
`trace_commons_ingest::run` instead of `trace_commons_server::ingest::run`.

## 4. Projected savings for option C

Method: today's measured unit cost, scaled by the line share of the source
that unit would compile after the split. Lines are a proxy. Generic-heavy
code, and `tests.rs`'s share of monomorphisation, can move the real number
either way. Treat every figure here as a target to check in section 7, not
a promise. Wall baselines are the quieter single-unit runs.

Shares used:

- Ingest harness: 258,828 / 392,922 = 0.66 of the current harness.
- Server harness after the split: 134,094 / 392,922 = 0.34.
- Server rlib after the split: about 127k / 229.6k = 0.55 of the current
  rlib.
- Ingest rlib: about 102.6k / 229.6k = 0.45.

| Scenario | Today (measured) | After C (projected) | Arithmetic |
|---|---|---|---|
| 2a. Ingest handler edit, ingest lib tests | 79.9 s (one unit) | about 51 s | 0.66 × 78 |
| 3. `tests.rs` edit, ingest lib tests | 77.9 s | about 51 s | 0.66 × 78 |
| 2b. Ingest handler edit, `--bin trace-commons-ingest` | 54.6 s (server rlib 53.5 s) | about 23 s | ingest rlib 0.45 × 49 ≈ 22, plus bin < 1. Server rlib is cached |
| 4b analogue. Ingest handler edit, `-p trace-commons-server --no-run` | 64.0 s (rlib plus 73 test/bin units, 165 s CPU) | about 0 s | The server rlib is unchanged. Only `tests/ingest_library_entry.rs` and the ingest bin depend on ingest, and both move |
| 4c analogue. Ingest handler edit, contributor tests | 94.4 s | about 0 s | Contributor's server dev-dependency does not use ingest |
| 4a. Server module edit, server `--lib` | 75.4 s | about 26 s | 0.34 × 75.4 |
| 4a'. Server module edit, then ingest lib tests too | (included in 4a's 75.4 s) | about 78 s | Server rlib 0.55 × 49 ≈ 27, then ingest harness about 51. Roughly break-even; see below |
| 4b. Server module edit, server `--no-run` | 64.0 s | about 42 s | rlib 44 → about 24 (0.55 ×), test units unchanged |
| 4c. Server module edit, contributor tests | 94.4 s | about 73 s | Server rlib 46 → about 25 |
| 6a. Protocol edit, server `--lib` | 99.0 s | about 54 s | 5.6 + 25.3 + 0.34 × 67.7 ≈ 23 |
| 5. Contributor edit, contributor tests | 74.4 s | 74.4 s | Unaffected |

**The case that gets no better.** Scenario 4a' is the one to watch. Today a
server module edit followed by "run all server lib tests" is one 75 s unit.
After the split it is the server rlib (about 27 s) on the critical path
before the ingest harness (about 51 s) can finish. Cargo pipelining lets the
ingest frontend start on the server rlib's metadata, so the true figure
should be below 78 s, but it is not a saving. Every other row improves or
holds.

**Combined with step 0.** The incremental gain (78 → 45.8/22.7 s) was
measured on today's harness only. It should compound with C, because the
smaller harness is also what incremental reuses, but no number is claimed
until it is re-measured.

## 5. Option C in detail

### 5.1 New crate

`crates/trace-commons-ingest`, package name `trace-commons-ingest`, crate
`trace_commons_ingest`.

- **License: `AGPL-3.0-or-later`**, copyright K&Z Partners LLC, the same as
  the server crate it is carved from. Every `.rs` file keeps its existing
  two-line header.
- **Dependency direction:** `trace-commons-ingest` → `trace-commons-server`
  (normal), plus whatever the app already uses (`trace-commons-protocol`,
  `-gate-api`, `-gate-enclave`, `-attestation`, `-build-info`, axum and the
  rest). Nothing in the server library depends on ingest today. The only
  `crate::ingest` references outside the app are doc comments in
  `src/versioned_pipeline_score_registry.rs` and
  `src/versioned_pipeline_production.rs`, so there is no cycle to break.
- **Permissive crates:** no permissive crate may depend on it. Nothing does
  today, and the contributor's dev-dependency on `trace-commons-server` does
  not reach ingest.
- **`license_boundary.rs`.**
  `crates/trace-commons-server/tests/license_boundary.rs`
  (`both_sides_of_the_boundary_are_populated`) pins the AGPL set at
  `{trace-commons-gate-api, trace-commons-gate-enclave,
  trace-commons-server}`. Adding a fourth AGPL crate is the case its failure
  message anticipates ("update LICENSE and this test together"). It is not
  an edit to make a violation pass. It still needs the owner's explicit
  approval (open question Q2), and `LICENSE`, `AGENTS.md`, `README.md`,
  `CONTRIBUTING.md` and `CLAUDE.md` all name the AGPL crates and change in
  the same PR.
- **`deny.toml`** needs no allow-list change. It allows the
  `AGPL-3.0-or-later` license itself, with no per-crate exception. Only its
  header comment, which names the three AGPL crates, is updated.
- **Features:** the ingest crate forwards `near-ai-scorer`,
  `local-gpu-models`, `near-attestation-collateral`, `gcs-client` and
  `gcp-kms` to the server crate, as far as the app uses them.

### 5.2 What moves

The files move under `crates/trace-commons-ingest/src/` **at the same
relative depth**. Today `src/ingest.rs` declares `#[path =
"bin/trace-commons-ingest.rs"] pub(crate) mod app;`. The new crate's
`src/lib.rs` keeps exactly that declaration, with the files at
`src/bin/trace-commons-ingest.rs` and `src/bin/trace_commons_ingest_internal/`.
Every relative `#[path]` and `include_str!` inside the moved tree then
resolves unchanged, and only the references that cross into the server crate
change (section 5.3). The internal module layout and the `#[cfg(test)] #[path
= "trace_commons_ingest_internal/tests.rs"] mod tests;` split also stay as
they are. CLAUDE.md asks that the tests not be inlined back, and this keeps every
private item private. Inside the app, `use super::*` and `crate::` paths keep
resolving, because the app becomes the root of its own crate.

| From (`crates/trace-commons-server/`) | To (`crates/trace-commons-ingest/`) |
|---|---|
| `src/ingest.rs` (`IngestOptions`, `run`, `OPEN_BUILD_BIN_NAME`) | `src/lib.rs` |
| `src/bin/trace-commons-ingest.rs` | `src/bin/trace-commons-ingest.rs` (a library module via `#[path]`, as today) |
| `src/bin/trace_commons_ingest_internal/**` | `src/bin/trace_commons_ingest_internal/**` |
| `src/bin/trace_commons_ingest_main.rs` (the `trace-commons-ingest` `[[bin]]`) | `src/bin/trace_commons_ingest_main.rs` |
| `tests/ingest_library_entry.rs` | `tests/ingest_library_entry.rs` |
| `tests/smoke_gate_script.rs` (reads the app source by path at run time) | `tests/smoke_gate_script.rs`, or kept in the server crate with its path updated |

Cargo's autobins would treat `src/bin/trace-commons-ingest.rs` as a binary
target. The server crate avoids that today because its explicit `[[bin]]`
named `trace-commons-ingest` points at `trace_commons_ingest_main.rs`. The new
crate's manifest must do the same, or set `autobins = false`.

The server's `pub mod ingest;` and the `extern crate self as
trace_commons_server;` line in `src/lib.rs` go. The second was added only so
the app's `trace_commons_server::` paths resolved inside the library. Outside
the app, the only `trace_commons_server::` uses in the server's `src/` are
doc comments and doctests (`src/account_onboarding.rs` and
`src/versioned_pipeline_production.rs`). Doctests compile as an external
crate, so they do not need the line. Those
66 distinct `trace_commons_server::<module>` paths then resolve to the real
dependency unchanged.

### 5.3 Things that break, found by grep

- **`pub(crate)` items the app reaches in the server library.** A
  name-match of the 92 `pub(crate)` item names in the library against
  identifiers in the ingest files finds at most about 11, some of them
  generic false positives (`new`, `from_parts`, `release`, `app`). The real
  candidates are `env_truthy`, `sha256_prefixed`, `residual_risk_verdict`,
  `TRACE_AUDIT_EVENT_GENESIS_HASH`, `PIPELINE_ADMIN_LOCK_TIMEOUT_SQL`,
  `write_routing_in` and `Answer`. They widen to `pub`, possibly
  `#[doc(hidden)]`. The compiler gives the exact list on the first
  `cargo check`.
- **`include_str!` paths that cross the moved tree.**
  - `tests.rs` line ~79056 reads `"../../trace_corpus_storage.rs"`, a server
    library file.
  - `tests.rs` line ~95912 reads `"../trace-commons-ingest.rs"`.
  - The attestation fixtures near `tests.rs` lines 94795–95016 are also
    included by relative path.
  - `admission.rs` line 1474 reads its own source.
  - `tests/step_up_page_tests.rs`, `step_up_page.rs` and
    `near_provisioning.rs` include files that move with them.

  Each relative path is rewritten in the move commit. One that points into
  the server crate becomes `concat!(env!("CARGO_MANIFEST_DIR"),
  "/../trace-commons-server/src/...")`.
- **`tests/ingest_library_entry.rs` line 209** asserts the literal text
  `trace_commons_server::ingest::run(IngestOptions::open_build())` in the
  bin. It is updated to the new path.
- **`build_version` in `GET /v1/source` and `/health`** is
  `env!("CARGO_PKG_VERSION")`, so it starts reporting the new crate's
  version. Set the new crate's version equal to the server's. The source
  offer itself (`source_offer_handler`) names a repository URL and commit
  and lists no crates, so AGPL section 13 compliance is unaffected. The
  route stays unauthenticated.
- **Nothing in the server crate's `tests/` uses
  `CARGO_BIN_EXE_trace-commons-ingest`**, which would break at compile time
  once the bin moves to another package (checked by grep). The integration
  tests that mention ingest use it as text:
  - `tests/smoke_gate_script.rs` reads
    `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` from disk.
  - `tests/pilot_bootstrap_submitter.rs` mocks it with wiremock.

  Only the first needs a path change.
- **Executable consumers of the source path outside the crate**, rewritten in
  the same PR:
  - `scripts/ci/test-step-up-page.cjs`
  - `scripts/operator/test_pipeline_tooling.py`
  - `scripts/operator/pipeline-deployment-inventory.py`
- **Documentation and comments naming the path**, updated in the same PR:
  - `CLAUDE.md`
  - `docs/trace-commons-storage.md`
  - `docs/legal-counsel-review-checklist.md`
  - seven runbooks under `docs/operator/`, including `agpl-source-offer.md`
  - `imports/ironclaw-server/SOURCE.md`
  - comments in `crates/trace-commons-contributor/src/history_copy.rs`,
    `crates/trace-commons-contributor/src/daemon/community.rs` and
    `macos/Sources/TraceCommonsApp/Models.swift`

  The 57 historical plans and specs under `docs/superpowers/` are left as
  written.

### 5.4 CI

`.github/workflows/ci.yml` names the server crate for tests that move:

- **ingest-bin-postgres** (required check; job name `trace-commons-ingest
  tests, whole bin, against PostgreSQL`): `cargo test --locked -p
  trace-commons-server --lib ingest::app:: -- --test-threads=1` becomes
  `cargo test --locked -p trace-commons-ingest --lib -- --test-threads=1`.
  The job **name** must not change, or the required check stops matching.
  The near-ai-scorer twin job changes its `ingest::app::` step the same
  way. Its `--lib versioned_pipeline_production` step tests a server module
  and stays on `-p trace-commons-server`.
- Named `--lib` filters whose tests live in ingest move to `-p
  trace-commons-ingest`:
  - `account_invite_route_requires_session_and_rejects_cross_site_cookie`
    (`tests.rs`)
  - `inference_connection_requires_explicit_account_session_selection`
    (`tests.rs`)
  - `token_bundle_http_journey` (`token_bundle_journey_test.rs`)
  - `real_http_receipt_completes_and_resumes_after_restart`
    (`pipeline_http_pg_tests.rs`, `pipeline_restore_pg_tests.rs`)
  - `compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export`
    (`pipeline_http_pg_tests.rs`)
  - `legacy_and_pipeline_tenants_match_under_equivalent_configuration`
    (`pipeline_compare_pg_tests.rs`, `pipeline_http_pg_tests.rs`)

  `pipeline_upgrade` (`src/db/postgres/pipeline_upgrade_tests.rs`) stays on
  the server crate.
- `cargo build -p trace-commons-server --bin trace-commons-ingest ...` (the
  near-ai-scorer and near-attestation-collateral builds, and the release
  build of the operator binaries) becomes `-p trace-commons-ingest`.
- Clippy, `cargo check` and `cargo test (default features)` already run
  workspace-wide or are extended by one `-p`. The MSRV job derives
  `rust-version` from metadata and needs no literal change.
- Each of these is a **filter** that can silently match zero tests after
  the move. The PR must show each job's test count before and after, and a
  zero is a failure (see the repository memory on absent signals).

Operator tooling (`scripts/operator/*`) builds by package and bin name.
Every `-p trace-commons-server --bin trace-commons-ingest` is listed and
rewritten in the same PR. `docs/operator/local-ingest-verification.md`
references `trace_commons_server::ingest` and is updated too.

### 5.5 CI cache (trim-cargo-cache, 10 GB ceiling)

The new crate adds one rlib/rmeta pair. The workspace's own rlibs total about
the same source as before, so the size of a saved `target` cache is about
unchanged. `.github/actions/trim-cargo-cache` already deletes every linked
test and bin executable, so the extra ingest harness binary is never saved.
No new cache key is introduced, and only pushes to `main` save.

The gain is in fingerprints. Today an ingest-only PR invalidates the server
rlib and everything downstream of it. After the split, a restored `main`
cache keeps the server rlib valid for such a PR. That matters only while
`main`'s caches restore at all; the trim action's notes record that most
did not before the 2026-10-15 changes.

## 6. Measure before deciding (no recommendation yet)

Each of these is a separate, cheap experiment that changes only a profile or
the environment. Run the same commands as section 7, on a quiet machine,
and report user CPU.

1. **Debuginfo for workspace crates in test builds.** `[profile.test] debug
   = "line-tables-only"` (or `[profile.dev] debug = 1`). Today's workspace
   crates build with full debuginfo. Less debuginfo cuts codegen and link
   work, which section 1 puts at roughly 15–30 s of wall. It costs local
   variable inspection in a debugger.
2. **Linker.** macOS uses the default Apple `ld` (ld-prime). Link time was
   never isolated. The estimate is 1–4 s per executable. Measure with
   `cargo build --timings` on `--no-run` all targets, comparing link-only
   time from `-Z time-passes` on nightly, or the difference between `--no-run`
   and a relink after `touch` of one test file. The repository has no
   `.cargo/config.toml` and would need one for a linker override.
3. **Integration-test consolidation.** Contributor: about 115 s CPU over 40
   test binaries. Server: about 160 s CPU over 71 targets. Merging each
   crate's `tests/*.rs` into one harness (`tests/it/main.rs` with `mod`s)
   removes most of the per-binary link and dependency work. It conflicts
   with how CI selects suites by `--test <name>`, and with `#[ignore]`
   being used as a CI selector. Count the `--test` invocations in `ci.yml`
   and `clients.yml` before proposing it.
4. **Splitting the ingest app by route family.** `account_routes`,
   `token_bundles`, `public_run`, `rewards`, `pipeline_*` and
   `near_provisioning` are separate modules under
   `trace_commons_ingest_internal/`. This is the form of the original
   crate-split hypothesis that could pay, because each piece would carry
   its own share of `tests.rs`. Step one is to measure the coupling: how
   many items in the 86k-line app each family uses, and how much of
   `tests.rs` is per-family. Do not start it without that data.

## 7. Migration order

Each step is independently mergeable and measurable, and none depends on a
later one.

**Step 0: local incremental (no PR).**

- Unset `CARGO_INCREMENTAL` for local edit loops. Cargo's dev profile then
  enables incremental for workspace members only, and third-party crates
  stay non-incremental.
- sccache 0.18.0 refuses to run when `CARGO_INCREMENTAL=1` is set
  explicitly. Unset worked in scenario 7.
- With incremental on, sccache marks every workspace-crate compile
  non-cacheable ("incremental"), so those crates are not cached.
- In scenario 7 the base build in a fresh target directory was 151.6 s with
  0 sccache hits. Why dependencies missed there was not established; it may
  be a target-path component in the key. Check before relying on it.
- `-D warnings` is unaffected, and CI keeps `CARGO_INCREMENTAL=0`. The cost
  is that local and CI artifacts differ, which is open question Q1.
- Optionally commit `[profile.dev] incremental = true` with
  `[profile.dev.package."*"] incremental = false`, instead of relying on the
  environment.

**Step 1: profile experiments.** Run items 1 and 2 of section 6. Merge a
profile change only if it beats its noise band.

**Step 2: extract `trace-commons-ingest`.** One PR, as described in
section 5:

- a pure move, with no behaviour change;
- the license set update;
- the CI rewrites.

Within that PR, the commits are ordered so that review can follow them:

1. Create the crate and move the files with `git mv`.
2. Widen the `pub(crate)` items.
3. Fix the `include_str!` paths.
4. Rewrite CI and scripts.
5. Update the docs and license set.

Gate: every moved test still runs, with test counts per CI job matched
before and after.

**Step 3: re-measure** (section 8) and decide whether section 6 items 3 and
4 are worth pursuing.

## 8. How we will know it worked

Re-run the Appendix A scenarios with the same environment:

- `RUSTFLAGS="-D warnings"`;
- `CARGO_INCREMENTAL=0` except in the incremental scenario;
- sccache on;
- a fresh target directory;
- `/usr/bin/time -l`;
- a machine with load average below the core count.

Report user CPU as the gate and wall as context. Today's wall varied ±30%
under load.

```bash
# baseline harness (today) vs ingest harness (after step 2)
cargo test -p trace-commons-server --lib --no-run      # warm
# edit an ingest handler (append a no-op statement), then:
/usr/bin/time -l cargo test -p trace-commons-ingest --lib --no-run
/usr/bin/time -l cargo test -p trace-commons-ingest --bin trace-commons-ingest --no-run
/usr/bin/time -l cargo test -p trace-commons-server --no-run
/usr/bin/time -l cargo test -p trace-commons-contributor --no-run
# edit src/audit_chain.rs, then:
/usr/bin/time -l cargo test -p trace-commons-server --lib --no-run
/usr/bin/time -l cargo test -p trace-commons-ingest --lib --no-run
```

Targets (measured baseline → target):

| Scenario | Baseline | Target |
|---|---|---|
| Ingest handler edit, ingest lib tests | 156–250 s user CPU / 78 s wall | ≤ 0.70 × baseline CPU, about ≤ 55 s wall |
| Ingest handler edit, `--bin trace-commons-ingest` | 54.6 s | ≤ 30 s |
| Ingest handler edit, server `--no-run` | 64.0 s | ≤ 5 s (no server unit rebuilt) |
| Ingest handler edit, contributor tests | 94.4 s | ≤ 5 s |
| Server module edit, server `--lib` | 75.4 s | ≤ 0.40 × baseline CPU, about ≤ 32 s |
| Server module edit, ingest lib tests | 75.4 s (today's combined harness) | ≤ 80 s (no regression beyond noise) |
| Incremental, second consecutive edit (step 0) | 22.7 s | ≤ 25 s, and still at or below that after step 2 |

There is also one CI number: the wall time of `cargo test (default features)`
and of the ingest-bin-postgres job on an ingest-only PR, taken from the five
runs before step 2 and the five after.

If the ingest handler target is missed by more than the noise band, the
line-share model is wrong. The likely reason is that `tests.rs`
monomorphises much of the library inside the ingest harness. Step 2 is then
still justified by the `--bin`, `--no-run` and contributor rows, but section
6 item 4 should not proceed.

## 9. Open questions for the owner

1. **Q1. Local incremental against CI parity.** Is it acceptable for local
   edit loops to run with incremental on (step 0) while CI stays at
   `CARGO_INCREMENTAL=0`? `-D warnings` behaves the same either way.
2. **Q2. A fourth AGPL crate.** Approve `trace-commons-ingest` as
   `AGPL-3.0-or-later`. That changes the AGPL set in `license_boundary.rs`
   and in `LICENSE`, `AGENTS.md`, `README.md`, `CONTRIBUTING.md` and
   `CLAUDE.md`. Is the crate name right?
3. **Q3. Embedding binaries.** Does any out-of-tree (proprietary) binary
   already call `trace_commons_server::ingest::run` after #1428? It would
   have to switch to `trace_commons_ingest::run`. A re-export from the
   server crate is impossible, because it would be a dependency cycle.
4. **Q4. File names.** The default in section 5.2 keeps
   `src/bin/trace-commons-ingest.rs` and `src/bin/trace_commons_ingest_internal/`
   at the same depth in the new crate. That gives the smallest diff, keeps
   blame, and leaves every relative path inside the tree unchanged. Renaming
   to `src/app.rs` / `src/app/` reads better, but shifts every relative
   `#[path]` and `include_str!`, and could be a follow-up PR.
5. **Q5. Integration-test consolidation.** Is consolidating `tests/*.rs`
   into one harness per crate acceptable, given that CI selects suites by
   `--test <name>` and by `#[ignore]` (section 6 item 3)? If not, that item
   is dropped.
6. **Q6. Debuginfo.** Is `line-tables-only` acceptable for workspace crates
   in test builds, if it measures well? Debugger variable inspection would
   be lost there.

## Appendix A: measurements

Measured at `a5ebb90bc` on a 16-core macOS machine with cargo 1.96.1,
sccache 0.18.0 as the rustc wrapper, `RUSTFLAGS="-D warnings"`,
`CARGO_INCREMENTAL=0` (except scenario 7, where it was unset), the default
Apple linker and no `.cargo/config.toml`. The dev profile is at its defaults
for workspace crates (full debuginfo, 256 codegen units); third-party crates
have `debug = false`.

The machine was shared with other work. The load average ranged from 35 to
223 during the later runs, and the same lib-test rebuild took 67.7–97.2 s
wall across scenarios. Treat wall times as ±30%.

"Units" counts units with a duration of at least 0.05 s. "lib (test)" is the
crate's unit-test harness.

| # | Scenario | Command | Wall s | Units rebuilt / total | sccache Rust hits / misses / non-cacheable |
|---|---|---|---|---|---|
| 1a | Cold | `test -p trace-commons-server --bin trace-commons-ingest --no-run` | 84.3 | 549 / 558 | 0 / 428 / 109 |
| 1b | Cold | `test -p trace-commons-contributor --no-run` | 119.6 | 595 / 596 | 394 / 28 / 141 |
| 1c | Deps warm | `test -p trace-commons-server --lib --no-run` | 101.2 | 36 / 557 | 31 / 1 / 3 |
| 1d | Deps warm | `test -p trace-commons-server --no-run` | 70.9 | 86 / 644 | 0 / 1 / 85 |
| 2a | Edit ingest handler | server `--lib --no-run` | 79.9 | 1 / 557 | 0 / 0 / 1 |
| 2b | Same edit | server `--bin trace-commons-ingest --no-run` | 54.6 | 2 / 558 | 0 / 1 / 1 |
| — | Revert after 2 | server `--lib --no-run` | 78.2 | 1 / 557 | 0 / 0 / 1 |
| — | Revert after 2 | server `--bin trace-commons-ingest --no-run` | 4.1 | 2 / 558 | 1 / 0 / 1 |
| 3 | Edit `tests.rs` | server `--lib --no-run` | 77.9 | 1 / 557 | 0 / 0 / 1 |
| — | Revert after 3 | server `--lib --no-run` | 78.3 | 1 / 557 | 0 / 0 / 1 |
| 4a | Edit `audit_chain.rs` | server `--lib --no-run` | 75.4 | 1 / 557 | 0 / 0 / 1 |
| 4b | Same edit | server `--no-run` | 64.0 | 86 / 644 | 0 / 1 / 85 |
| 4c | Same edit | contributor `--no-run` | 94.4 | 45 / 596 | 0 / 1 / 44 |
| — | Revert after 4 | server `--no-run` | 97.7 | 87 / 644 | 1 / 0 / 86 |
| — | Revert after 4 | contributor `--no-run` | 77.7 | 45 / 596 | 1 / 0 / 44 |
| 5 | Edit contributor `managed/copy.rs` | contributor `--no-run` | 74.4 | 48 / 596 | 0 / 1 / 47 |
| — | Revert after 5 | contributor `--no-run` | 51.4 | 48 / 596 | 1 / 0 / 47 |
| 6a | Edit protocol `canonical_json.rs` | server `--lib --no-run` | 99.0 | 3 / 557 | 0 / 2 / 1 |
| 6b | Same edit | contributor `--no-run` | 143.9 | 50 / 596 | 0 / 3 / 47 |
| — | Revert after 6 | server `--lib --no-run` | 75.7 | 3 / 557 | 2 / 0 / 1 |
| — | Revert after 6 | contributor `--no-run` | 50.0 | 50 / 596 | 3 / 0 / 47 |
| 7 | Incremental base build | server `--lib --no-run` | 151.6 | 557 / 557 | 0 / 420 / 104 |
| 7a | Incremental, first edit (as 2a) | server `--lib --no-run` | 45.8 | 1 / 557 | 0 / 0 / 1 |
| 7b | Incremental, second edit | server `--lib --no-run` | 22.7 | 1 / 557 | 0 / 0 / 1 |
| 8 | Check base | `check -p trace-commons-server --lib --profile test` | 80.2 | 397 / 568 | 0 / 395 / 2 |
| 8a | Check after handler edit (under heavy load) | same | 148.5 | 1 / 568 | 0 / 0 / 1 |
| 8b | Rerun of 8a | same | 109.3 | 1 / 568 | 0 / 0 / 1 |

Unit durations from `--timings` (wall seconds):

| Scenario | Unit costs |
|---|---|
| 1a | server rlib 52.9, contributor 25.7 |
| 1b | server rlib 50.7 (dev-dep), contributor `lib (test)` 46.6, 40 test binaries about 118 s CPU in total |
| 1c | server `lib (test)` 74.0, contributor lib 23.5 |
| 1d | 73 test/bin units about 154 s CPU in total, plus the server rlib |
| 2b | server rlib 53.5, bin 0.8 |
| 4b | server rlib 44, plus 73 test/bin units about 165 s CPU in total |
| 4c | server rlib 46, contributor `lib (test)` 46.7 |
| 5 | contributor rlib 25.9, `lib (test)` 45.9, 40 test binaries about 115 s CPU in total. Server rlib not rebuilt |
| 6a | protocol 5.6, contributor 25.3, server `lib (test)` 67.7 |
| 6b | server rlib 93.2, contributor rlib 70.0, contributor `lib (test)` 43.0 |

Integration-test units take 1–5 s each, with `versioned_pipeline_runtime_pg`
the slowest at 17 s.

Frontend against full build, after the same handler edit, run back to back
with `/usr/bin/time -l`:

| Run | `cargo test --lib --no-run` real / user s | `cargo check --lib --profile test` real / user s |
|---|---|---|
| 1 | 199.4 / 220.9 | 67.7 / 58.7 |
| 2 | 213.2 / 250.2 | 223.4 / 80.6 |
| 3 | 328.9 / 201.2 | 87.2 / 66.8 |
| 4 | 205.9 / 156.0 | 66.3 / 57.6 |

Peak RSS was about 8.6–8.8 GB for the test build and 5.1 GB for check.

Other figures:

- **Dependency graph:** `trace-commons-protocol` is depended on by
  contributor, contributor-ffi and server. Server and contributor are
  dev-dependencies of each other. Contributor-ffi is the only non-rlib crate
  type (cdylib, staticlib, rlib).
- **Line counts:**
  - server `src` 410,044, of which 163,288 test-named;
  - server `tests/` 101,643 lines in 61 files;
  - contributor `src` 275,517 (19,705 test-named);
  - contributor `tests/` 18,810 lines in 36 files;
  - protocol `src` 35,711.
- **Test attributes:**
  - server `src` 3,177, `tests/` 937;
  - contributor `src` 4,392, `tests/` 312;
  - protocol `src` 567, `tests/` 69.
