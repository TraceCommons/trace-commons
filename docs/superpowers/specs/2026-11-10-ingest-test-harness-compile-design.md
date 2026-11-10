# Ingest test harness: cutting what it costs to compile

Status: draft for owner review, 2026-11-10. Design only. No Rust code, no
workflow and no manifest changes in this PR, and no test suite was run for
it. Every number below is labelled measured or estimated, with the command
that produced it.

## Intent

The ingest service is one very large compilation unit. Its production code
(`crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs`, 88,080 lines,
the library module `trace_commons_ingest::ingest::app`) and its tests
(`trace_commons_ingest_internal/tests.rs`, 104,913 lines, plus the files it
mounts) compile together into a single test executable. CI builds that
executable eight times per run, and a local run of one filtered ingest test
pays for all of it.

The owner asked for this cost to be reduced. `CLAUDE.md` forbids splitting
large files without a spec; this document is that spec. It measures first,
because the obvious change (more modules in the same crate) does nothing,
and the change that looks most promising (moving tests to `tests/`) saves
less in CI than expected.

The short version:

- The harness takes 55 s to compile on a developer Mac and a median of
  104 s on a CI runner. Of that, 59% is the production code being compiled
  a second time under `cfg(test)`; the tests themselves are the other 41%.
- About 90% of the harness build is single-threaded whole-crate work.
  Changing `codegen-units` or debug info moves the time by under 1 s.
- No test can move to an integration target today. All 36 test files reach
  at least one private item of `app`, and the tests reference about 1,040 of
  its 3,053 top-level items.
- The cheapest real savings are fewer builds (CI steps that rebuild the same
  harness on another runner) and fewer dependencies (84 packages reach the
  harness through one dev-dependency used by 8 of 2,006 tests).

## 1. What is compiled, and where

`cargo test -p trace-commons-ingest --lib` builds one unit: the library in
test mode. It contains `app`, its sibling modules, and the test module.
It lists 2,006 tests (50 ignored), 1,893 under `ingest::app::tests::` and
113 in the inline test modules of sibling files. Measured with the built
binary's `--list`, not by counting attributes.

| | files | lines |
|---|---|---|
| test corpus (`tests.rs` and everything it mounts) | 39 | 164,255 |
| everything else (`app`, its sibling modules with their inline test modules, and test files mounted from outside `tests.rs`) | 36 | 111,810 |

The plain library (what the `trace-commons-ingest` binary links) is a
different unit. A job that builds the binary and then runs a `--lib` test
compiles the production code twice.

### Every CI step that compiles it

From `.github/workflows/ci.yml` on `main` at 037d03f06. "Harness" is the
library in test mode; "lib" is the plain library.

| job | what it builds | features |
|---|---|---|
| `cargo test (default features)` | harness, lib, 3 integration targets | default (workspace-unified) |
| `trace-commons-ingest tests, whole bin, against PostgreSQL` | harness | default |
| `database suites against a real PostgreSQL` | harness, for 14 filtered steps | default |
| `... whole bin, near-ai-scorer ...` | harness | `near-ai-scorer` |
| `... whole bin, near-ai-staging ...` | lib and one integration target, then harness | `near-ai-staging` |
| `cargo check (near-ai-scorer)` | lib check, lib build, then harness for 1 test | `near-ai-scorer`, then `gcs-client` |
| `cargo check (near-attestation-collateral)` | lib check, lib build, then harness for 11 tests | `near-attestation-collateral` |
| `pipeline qualification and restore` | harness, through `pipeline.py keygen` and `qualify` | default |
| `cargo clippy` | harness and lib in check mode, twice | default, then `near-ai-staging` |
| `cargo check (default features)`, `(local-gpu-models, non-CUDA)` | lib check | various |
| `pilot-bootstrap smoke` | lib, release profile | default |

That is eight harness builds. Earlier counts put it at about seven; the eighth
is the qualification job. Its cargo output is captured by the tool and does
not show in the job log, so it is confirmed from the tool's source instead:
`pipeline.py` runs its key writer, corpus checks and restore drill as
`cargo test -p trace-commons-ingest --lib <name>` (`INGEST_TEST_ARGS`), and
the job's `keygen` step, the first of them, took 2 m 37 in run 42696115058.

## 2. Measurements

### 2.1 Local, isolated

Apple M4 Max, rustc 1.99.1, `CARGO_BUILD_JOBS=4`, `RUSTFLAGS="-D warnings"`,
`CARGO_INCREMENTAL=0`, dependencies already built, only the named unit
rebuilding (forced with `touch crates/trace-commons-ingest/src/lib.rs`).
Unit times are from the cargo timing report
(`target/cargo-timings/cargo-timing-*.html`). Load average was 6 to 8
during these runs, from other work on the machine.

| unit | command | time |
|---|---|---|
| harness, build | `cargo rustc --timings -p trace-commons-ingest --lib --profile test` | 54.6 s |
| plain lib, build | `cargo rustc --timings -p trace-commons-ingest --lib -- -C codegen-units=16` | 32.0 s |
| harness, check only | `cargo check --timings -p trace-commons-ingest --lib --tests` | 28.8 s |
| plain lib, check only | same run | 13.7 s |

The explicit `-C codegen-units=16` on the plain lib is the default value;
it is there only so sccache cannot answer from its cache.

So the production code costs 32.0 s to build on its own, and the harness
costs 22.6 s more than that. The tests are 41% of the harness; the other
59% is `app` compiled again. In check mode the split is 13.7 s and 15.1 s.

The first, cold build of the worktree (`cargo build --timings -p
trace-commons-ingest --lib --tests`, 3 m 08 s wall, load average 22) gave
67.9 s for the harness and 46.5 s for the plain lib while they compiled
side by side. Those two numbers are contended and are not used below.

### 2.2 Where the harness's time goes

One harness build with rustc's pass timings. `-Z time-passes` is unstable,
so this run set `RUSTC_BOOTSTRAP=1` through a `RUSTC_WRAPPER` script for the
ingest crate only; nothing else was rebuilt. Total 60.9 s, against the
54.6 s baseline without the flag.

| pass | time | share |
|---|---|---|
| macro expansion and name resolution | 3.0 s | 5% |
| type checking (includes 5.5 s coherence) | 12.5 s | 21% |
| MIR borrow checking | 12.7 s | 21% |
| monomorphization collection and partitioning | 15.8 s | 26% |
| generating LLVM IR | 10.7 s | 18% |
| LLVM passes | 13.8 s, overlapping the row above | |
| finishing codegen and linking | 2.3 s | 4% |

Peak resident memory was 6.4 GB.

Everything down to and including LLVM IR generation runs on one thread and
sees the whole crate: 54.7 s of 60.9 s, about 90%. Only the LLVM passes are
split across codegen units, and they already overlap IR generation. Linking
the 250 MB test executable takes 0.8 s.

### 2.3 Flag experiments

Same harness command, with one flag appended after `--`. Measured, one
sample each, against the single baseline sample.

| flag | harness time | against 54.6 s |
|---|---|---|
| none | 54.6 s | |
| `-C codegen-units=256` | 55.2 s | +0.6 s |
| `-C debuginfo=0` | 54.9 s | +0.3 s |

Three builds of the same unit within 0.6 s of each other: neither flag
has an effect that one sample can see.

### 2.4 CI

From the three most recent green `ci.yml` runs on `main` on 2026-11-09
(run ids 42696115058, 42689031259, 42657279018). "Harness" is the time from
cargo's `Compiling trace-commons-ingest` line to its `Finished` line in a
step that builds only the harness. "Cargo step" is the total cargo reports
for that step, which also includes the server and the dev-dependencies.

| job | harness, three runs | cargo step |
|---|---|---|
| whole bin, default | 78.5, 110.4, 107.3 s | 3 m 51, 5 m 27, 5 m 24 |
| whole bin, `near-ai-scorer` | 76.1, 103.6, 76.4 s | 2 m 07, 2 m 49, 2 m 04 |
| whole bin, `near-ai-staging` | 90.4, 108.7, 106.2 s | 1 m 30, 1 m 48, 1 m 46 |
| database suites, first ingest step | 104.2, 64.8, 92.1 s | 3 m 24, 2 m 05, 2 m 55 |
| collateral job, 11 tests | 86.6, 94.1, 103.9 s | 2 m 20, 2 m 35, 2 m 49 |
| scorer check job, 1 `gcs-client` test | 105.0, 109.7, 105.5 s | 2 m 50, 2 m 54, 2 m 49 |
| workspace test | not separable | 6 m 05, 5 m 52, 5 m 39 |
| pipeline qualification | not visible in the log | |

The harness takes 65 to 110 s on a runner, median 104 s over these 18
samples. The usual figure of 2.5 to 3 minutes each matches the cargo step,
not the harness: a third to a half of each step is the server and the
dev-dependency chain.

The six measurable harness builds sum to 9.0, 9.9 and 9.9 minutes per run.
With the two that cannot be read from the logs, at the median 104 s each,
the harness costs an estimated 12 to 13 runner-minutes per run. All jobs in run 42696115058 sum
to 117.6 minutes, so that is roughly a tenth of the run.

Other ingest units, for scale, same runs: plain lib build 48 to 62 s; lib
check 14 to 26 s; clippy over the harness 55 to 102 s per pass; release
build 170 to 206 s.

Two more facts from the same logs matter for the options:

- The longest job is `database suites against a real PostgreSQL`, 15.0
  minutes in run 42696115058. The next is `cargo test (default features)`
  at 11.8.
- A whole-bin job spends about 250 s running the tests on one thread
  against about 100 s compiling them. Compile time is not most of those
  jobs.

### 2.5 The contributor dev-dependency

The ingest crate's dev-dependencies include `trace-commons-contributor`.
Measured with `cargo tree -p trace-commons-ingest -e normal,build` against
the same with `dev` added: 84 of the 430 packages in the test graph are
there only through dev-dependencies. They include
`trace-commons-contributor`, `-contributor-insights`, `-contributor-sources`,
`ironclaw_llm`, `rig-core`, the three `ironwire_*` crates and `aws-lc-sys`.

In the cold local build the server finished at 98.0 s and the contributor
at 120.1 s. The harness started at 120.1 s. It waited 22 s for a crate that
three test files use (full `grep -rn trace_commons_contributor
crates/trace-commons-ingest/`, 7 lines):

- `nearai_ceremony_pg_tests.rs`, 6 tests
- `token_bundle_journey_test.rs`, 1 test
- `tests/near_ai_provisioning_tests.rs`, 1 test (both uses are inside it)

That is 8 tests of 2,006. In the CI collateral job, 47 to 58 s pass between
the first workspace crate starting and the harness starting.

### 2.6 What the tests reach

`app` is declared `pub(crate) mod app`, and the crate's public API is
`run`, `run_with_score_policies`, `IngestOptions` and `OPEN_BUILD_BIN_NAME`.
An integration target sees only those. So the count of tests that use only
the public API is zero by construction, and the useful question is how much
would have to be opened.

A name-based count, from a script that lists column-0 item definitions in
the `app` file and searches the test corpus with comments and string
literals removed:

| | count |
|---|---|
| top-level items in `app` | 3,053 |
| of those, private | 3,046 (5 `pub(crate)`, 2 `pub`) |
| referenced from the test corpus, any word match | 1,041 |
| referenced, counting only unqualified uses and excluding names the tests define themselves | 1,036 |
| of the referenced items, declared `#[cfg(test)]` | 14 |

The two matching rules agree to within 5, which is the only cross-check
this count has. It is a lower bound on the work: it does not count private
struct fields, private methods, or the items of the 35 sibling modules. A
compiler-derived number is the first step of phase 1.

By file, every one of the 36 test-bearing files reaches at least one
private `app` item directly. `tests.rs` itself reaches 981;
`pipeline_http_pg_tests.rs` 64; the 22 mounted files under `tests/` reach between 1
and 35 each. Those small counts understate the coupling, because the
mounted files also use helpers defined in `tests.rs` through their own
`use super::*`.

Two more seams an external target would cross:

- The production source carries 101 `#[cfg(test)]` attributes and 4
  `cfg(not(test))`. A plain library is compiled without `cfg(test)`, so
  every test-only constructor and fault hook the tests use would need a
  different gate.
- `tests.rs` inherits `app`'s 73 top-level `use` statements through
  `use super::*`. An external target has to restate the ones it needs. That
  is mechanical.

## 3. What the measurements rule out

**Splitting the file into more modules in the same crate.** rustc
type-checks, borrow-checks and collects monomorphizations for a crate as
one unit, on one thread. Those passes are 73% of the harness build, and IR
generation, also single-threaded, is another 18%. Module boundaries change
none of it. The codegen-unit experiment is the direct test: going from 16
units to 256 changed the build by 0.6 s.

What this spec authorizes, for the rule on large files: moving test files,
and tests out of `tests.rs`, into separate test targets or packages, in the
order and behind the gates of section 6. It does not authorize splitting
`trace-commons-ingest.rs`, or splitting `tests.rs` into more modules of the
same crate, for compile time.

**Profile settings.** Debug info is already line tables only; removing it
entirely changed nothing. Linking is 0.8 s, so a different linker has
nothing to win on macOS. The parallel front end (`-Z threads`) would attack
the right 73%, but it is nightly-only and CI builds on stable; it was not
measured.

## 4. Options

Savings are per harness build unless stated. "Local" is the M4 Max at four
jobs; "CI" scales by the measured ratio of about 1.9 (104 s against 55 s).

### (a) CI changes only

No Rust changes. Build the harness fewer times.

1. **Move the 14 ingest steps out of `database suites against a real
   PostgreSQL`.** That job builds the default-feature harness only to run
   14 filtered steps. `trace-commons-ingest tests, whole bin, against
   PostgreSQL` builds the same harness, with the same features, against a
   real database. Running the 14 steps there removes one harness build, and
   the dev-dependency chain with it, from the longest job.
   - Saving: the first ingest step's cargo build, 2 m 05 to 3 m 24
     (measured). The tests' own run time moves; it is not saved. The
     critical path drops from 15.0 to an estimated 12 to 13 minutes, where
     `cargo test (default features)` (11.8) is next.
   - Cost: a workflow edit. Each moved step needs its database, roles and
     environment reproduced in the other job, and the PostgreSQL steps must
     stay one at a time.
   - Risk: six of the 14 steps are plain filters with no `--ignored`, so
     the whole-bin run may already execute them; each has to be checked
     against the whole-bin job's database before it is dropped as a
     duplicate rather than moved. Several steps have no "at least one test
     passed" guard, so a step that silently matches nothing after the move
     still goes green. Add the guard as part of the move. Both job names
     are matched by branch protection and must not change.

2. **Stop the workspace job building the ingest harness** (`cargo test
   --workspace --exclude trace-commons-ingest`, plus the ingest integration
   targets by name). The whole-bin job runs the same tests with a database.
   - Saving: one harness build and a 47 s test run, an estimated 2.5
     minutes off an 11.8-minute job.
   - Risk: high enough that this is not recommended without a check.
     `cargo test --workspace` reaches feature-gated tests in other crates
     through feature unification (`CLAUDE.md` records
     `trace-commons-protocol` going from 251 to 289 tests under a feature
     the ingest crate enables). Excluding ingest can silently drop those.
     The gate is a per-binary `-- --list` count before and after; any count
     that falls blocks the change.

3. **The two tiny feature runs** (1 `gcs-client` test, 11 collateral tests)
   each build a whole harness under a feature set no other job builds.
   Nothing in CI alone fixes that; see (b).

### (b) Move tests to integration targets

No test can move as the code stands (2.6). Moving any needs a seam:

- a `test-support` cargo feature on the ingest crate, enabled for the
  crate's own tests through a self dev-dependency, under which `app` is
  declared `pub mod` instead of `pub(crate) mod`;
- `pub` on each private item the moved tests use. Because `app` stays
  `pub(crate)` in a normal build, this does not widen the shipped API;
- `#[cfg(any(test, feature = "test-support"))]` in place of `#[cfg(test)]`
  on the test-only items they use;
- the shared helpers of `tests.rs` moved to a support module under the same
  gate;
- `scripts/ci/check-release-credential-store.sh` extended to refuse any
  shipped build that enables `test-support`, as it already does for the
  contributor's test-only features.

Following the server crate, moved files would be modules of one
`tests/it.rs` binary, not one target each: every extra target links
another executable of this size.

- Saving, local (estimate): a test-only edit rebuilds the test target, not
  the production code. With every test moved that is about 23 s instead of
  55 s. With tests in several targets, a small target is an estimated 3 to
  8 s. An edit to `app` still costs the lib (32 s) plus the target.
- Saving, CI (estimate): close to nothing for a job that runs everything.
  The lib built with `test-support` (32 s local) plus the tests (23 s) is
  the same work as today's harness. The lib and the tests can overlap on
  two cores, so wall time might fall by about 10%. The two tiny feature
  runs would build the lib and a small target, an estimated 40 to 50 s less
  each on a runner.
- Cost: at least 1,036 visibility edits in an 88,080-line file that most
  open branches touch, plus fields and methods not yet counted; every
  `--lib <filter>` that names a moved test rewritten.
- Risk:
  - While the move is partial, a job that runs both the harness and the
    external target compiles the production code once more than it does
    today, about 55 s per job on a runner. A half-finished (b) makes CI
    slower.
  - The self dev-dependency is how other projects turn a feature on for
    their own tests; it has not been tried in this workspace and is
    unproven until phase 1, step 1. Cargo unifies features across the
    packages a command selects, so once any workspace package enables
    `test-support`, a `--workspace` build (the workspace test job, the
    first clippy pass) only ever sees the ingest crate with it on. The
    `-p`-selected check, release and binary builds are what still see it
    off, and the release guard has to hold that line.
  - Filters. `ci.yml` has 19 `cargo test -p trace-commons-ingest ... --lib`
    lines. `scripts/operator/pipeline.py`, `pipeline_tooling/checks.py` and
    `pipeline_tooling/promote.py` hard-code `-p trace-commons-ingest --lib`
    and full test paths such as
    `ingest::app::tests::pipeline_restore_pg_tests::pipeline_restore_seed`;
    those names are part of pipeline qualification and promotion. Operator
    runbooks and `CLAUDE.md` quote the same form. A moved test whose old
    filter still parses matches nothing and passes.
  - The PostgreSQL tests share the `tenant-a` fixture and run with
    `--test-threads=1`. In one `it.rs` binary that still holds. Split
    across targets, `cargo test` runs the targets one after another, but
    two CI steps or two local shells would not.

### (c) Split `app` into crates

Carve the service into several crates so each area's tests compile against
a smaller unit. This is the only option that shrinks the 59%.

- Saving (estimate, not measurable without doing it): proportional to the
  split. Four even crates would put a single-area edit-and-test cycle near
  a quarter of today's and let a runner compile them in parallel.
- Cost: very high. `tests.rs` alone reaches 981 `app` items; the sibling
  modules reach back into `app` through `super::`; application state is
  shared. Each new crate is AGPL and needs its SPDX headers, an entry in
  the licence lists and `license_boundary.rs`, and a place in the feature
  forwarding for all seven ingest features.
- Risk: a long-running refactor of the file every other branch edits.

Not recommended now. If phase 2 shows the seam is cheap along one boundary,
that boundary is where a crate split would start.

### (d) Gate test groups behind features or cfg

Compile only the groups a job runs.

- Saving (estimate): at most the tests' 41%, and only for a job that needs
  none of `tests.rs`. `tests.rs` is 64% of the test corpus and holds the
  shared helpers, so a realistic filtered job saves an estimated 10 to 20 s
  on a runner. The production code is still compiled in full.
- Cost: a feature per group, forwarded in every job.
- Risk: the gate is itself a way to run nothing. A job that forgets a
  feature compiles the group out and passes. Passing cfg through
  `RUSTFLAGS` instead would change every dependency's fingerprint and
  rebuild the graph.

Not recommended: a small saving bought with a new silent-omission path.

### (e) Profile tweaks

Measured in 2.3: no effect from codegen units or debug info, and linking is
under a second. Optimizing dependencies would not shorten the compile; it
might shorten the 250 s single-threaded test run, which is larger than the
compile in the whole-bin jobs. That is a different question and was not
measured here.

Not recommended. No profile change.

### (f) Take the contributor out of the harness's dependencies

Not among the options first listed; the measurements raised it (2.5). Move the 8 tests that
use `trace-commons-contributor` out of the ingest package and drop the
dev-dependency. Dev-dependencies are per package, so another target in the
same package would not help; the tests need a different package.

- Saving, local (measured gap): 22 s of wall time on a cold build or after
  the contributor changes, and 84 fewer packages in the ingest test graph.
  No change to a warm edit loop.
- Saving, CI (estimate): 20 to 55 s per harness build, because CI rebuilds
  the workspace's own crates on every run. It applies to six builds, not
  eight: the workspace job builds the contributor anyway, and after (a)1
  the database-suites job no longer builds the harness. 2 to 5.5
  runner-minutes.
- Cost: those tests use private `app` items and `tests.rs` helpers, so they
  need the seam from (b), at small scale.
- Where they go is an owner decision, because the three files carry AGPL
  headers:
  - `trace-commons-client-server-tests`, the unpublished package that
    already exists for client/server cross-checks, is `MIT OR Apache-2.0`.
    Moving the files there relicenses that test code, and adds a pinned
    dev-dependency edge to `trace-commons-ingest` in `license_boundary.rs`.
    That list is the specification and changes only with owner approval.
  - A new unpublished AGPL test package keeps the licence and needs no new
    crossing edge, but is a new crate: SPDX headers, the AGPL lists in
    `CLAUDE.md`, `AGENTS.md` and `license_boundary.rs`.
- Risk: the moved tests leave the three whole-bin PostgreSQL jobs. The
  ceremony tests need a database and would need their own step.

## 5. Recommendation

Do (a)1 now, then (f), then pilot (b) on a small group with a stop/go
measurement. Do not split files inside the crate, do not gate test groups,
and do not change profiles: the measurements show each either does nothing
or trades a small saving for a silent-omission risk. Leave (c) until the
pilot shows what the seam costs.

The reasoning: in CI the harness is rebuilt because jobs are separate, not
because it is badly shaped, so the CI savings come from fewer builds and
fewer dependencies. The shape matters locally, where a test-only edit
recompiles 88,080 lines of production code it did not touch, and (b) is
what fixes that. But (b) costs a thousand-line visibility change and slows
CI while it is half done, so it should prove itself on one group first.

## 6. Phased plan

| phase | change | saving per CI run | saving per local ingest test cycle |
|---|---|---|---|
| 0 | (a)1: move the 14 ingest steps to the whole-bin job, with pass-count guards | 2.1 to 3.4 runner-minutes (measured step build); longest job 15.0 to an estimated 12 to 13 minutes | none |
| 1 | The `test-support` seam and its release guard; (f): the 8 contributor tests out of the ingest package | estimated 2 to 5.5 runner-minutes | 22 s on a cold build or after a contributor change (measured gap); none warm |
| 2 | Pilot (b): 21 of the files under `tests/` (147 tests, about 12,500 lines) into one `tests/it.rs` target | about zero; an estimated +55 s per job that builds both, until phase 3 | for a moved test, an estimated 3 to 8 s plus link for a test-only edit, against 55 s |
| 3 | Only if phase 2 passes its gate: the rest of `tests.rs`, then the PostgreSQL files | about zero for full runs; an estimated 40 to 50 s for each of the two tiny feature runs | an estimated 23 s or less for a test-only edit, against 55 s; unchanged for an `app` edit |

Today, for reference: the harness costs an estimated 12 to 13
runner-minutes per CI run, and 55 s per local cycle for any edit.

Phases 0 and 1 together are an estimated 4 to 9 runner-minutes per run, of
about 118, and 2 to 3 minutes off the longest job. Phases 2 and 3 are for
the local cycle.

### Phase 0

One workflow PR. For each of the 14 steps: confirm whether the whole-bin
run already executes it; move it or drop it; add the "at least one passed"
guard. Verify by comparing the set of test names run before and after, from
the job logs. Job names unchanged.

### Phase 1

1. Add the `test-support` feature, the two `mod app` declarations and the
   self dev-dependency. Extend the release guard and its self-test.
2. Compile the three contributor-using files as external tests and count
   the privacy errors. That count, not the name-based one in 2.6, sizes the
   work. Stop and report if it is over a few dozen items.
3. Ask the owner which package takes them (option (f)); move them; give
   the ceremony tests a PostgreSQL step.
4. Measure: `cargo tree` package count, and harness start against server
   finish in the timing report.

### Phase 2 gate

Phase 3 goes ahead only if all three hold, measured as in 2.1:

- a test-only edit to a moved file rebuilds in under 10 s;
- the harness is no slower than before the move;
- the visibility changes for the pilot came to under 200 items.

If not, stop at phase 1 and close this spec with the numbers.

### Phase 3 ordering

The pipeline test files go last. Their names are pinned by the operator
tooling, so they move together with `pipeline.py`, `checks.py`,
`promote.py` and `test_pipeline_tooling.py`, in one PR, with the
qualification job green on it.

## 7. Risks that apply to every phase

- **Vacuous passes.** A filter that matches nothing passes. Every moved or
  renamed test needs its filter updated in `ci.yml`, the operator tooling,
  the runbooks and `CLAUDE.md`, and every filtered CI step needs a pass
  count check.
- **Fixture sharing.** PostgreSQL tests that share `tenant-a` must not run
  at once. Any new target holding them needs the same `--test-threads=1`
  and must not run in parallel with the harness against the same database.
- **Required checks.** Thirteen job names are matched by branch protection.
  No phase renames a job.
- **Licensing.** New `.rs` files in the ingest crate carry the AGPL header.
  The one licence-boundary change is the dev-dependency edge in phase 1.
- **Concurrent branches.** Phases 2 and 3 edit the files most branches
  touch. Each lands as one mechanical PR, announced beforehand.

## 8. Not measured

- The two harness builds whose cargo output is not in the job log
  (workspace test, pipeline qualification).
- The cost of an external test target. The 23 s is the harness minus the
  plain lib; a separate crate also instantiates the generic code it calls,
  which could move it either way.
- Linux link time. The 0.8 s is macOS.
- Private fields and methods the tests use.
- Whether optimized dependencies would shorten the 250 s test run.

## 9. Reproducing

```bash
export CARGO_BUILD_JOBS=4 RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0

# Cold build, both units (contended).
cargo build --timings -p trace-commons-ingest --lib --tests

# Each unit alone. Touch first so cargo rebuilds it.
touch crates/trace-commons-ingest/src/lib.rs
cargo rustc --timings -p trace-commons-ingest --lib --profile test
touch crates/trace-commons-ingest/src/lib.rs
cargo rustc --timings -p trace-commons-ingest --lib -- -C codegen-units=16
cargo check --timings -p trace-commons-ingest --lib --tests

# Flag experiments.
cargo rustc --timings -p trace-commons-ingest --lib --profile test -- -C codegen-units=256
cargo rustc --timings -p trace-commons-ingest --lib --profile test -- -C debuginfo=0

# Pass timings on the stable toolchain. The wrapper is a two-line script:
#   #!/bin/sh
#   RUSTC_BOOTSTRAP=1 exec "$@"
# Only the ingest unit rebuilds, so only it is compiled without sccache.
touch crates/trace-commons-ingest/src/lib.rs
RUSTC_WRAPPER=/path/to/wrapper.sh \
  cargo rustc -p trace-commons-ingest --lib --profile test -- -Z time-passes

# Test counts, from the harness binary cargo prints.
cargo test -p trace-commons-ingest --lib -- --list | tail -1
cargo test -p trace-commons-ingest --lib -- --list --ignored | tail -1

# Dev-only packages.
cargo tree -p trace-commons-ingest -e normal,build --prefix none | sort -u
cargo tree -p trace-commons-ingest -e normal,build,dev --prefix none | sort -u
```

Unit times are the `duration` of the `trace-commons-ingest` entries in the
timing report's `UNIT_DATA`. CI times are the gap between the `Compiling
trace-commons-ingest` and `Finished` lines in `gh run view --job <id>
--log` for the three runs named in 2.4.
