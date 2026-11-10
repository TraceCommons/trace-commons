# Repository Guidance — trace-commons

Full guidance for this repository: layout, build and test commands, CI,
conventions and known gotchas. Where it differs from a global CLAUDE.md, this
file wins.

`AGENTS.md` is the short file every coding agent reads, and it carries the
complete licensing rules; this file repeats only the essentials. It is not
imported here, so **read `AGENTS.md` before you touch a dependency, a
`Cargo.toml`, a license header or `GET /v1/source`**.

## What this repo is

This repository (`TraceCommons/trace-commons`, formerly `trace-commons-server`)
holds both sides of Trace Commons.

- **Server side**: the hosted control plane. It owns its database schema,
  object storage, encrypted artifact store, upload-claim issuer, and the
  shared protocol crate.
- **Contributor side**: the contributor CLI and daemon
  (`trace-commons-contributor`; its session-store parsers and routing overlay
  live in the leaf crate `-contributor-sources`), the C ABI
  (`-contributor-ffi`), and four desktop shells: macOS (`macos/`, Swift),
  Windows (`windows/`, C#), Linux GTK (`crates/trace-commons-contributor-gtk`),
  and Tauri (`tauri-desktop/`).

**Native SwiftUI (`macos/`) is the main client on macOS** (#1173 D1): no new
Tauri work on macOS, and the Tauri app is retired there once the native
screens match. Windows and Linux stay on Tauri, frozen, until macOS ships
(#1173 D13, a default for now). Native macOS is also the lead client and the
parity target: GTK, Windows and Tauri match its terminology, states, flows
and affordances, and Tauri drift is low priority.

Ironclaw is a separate project and does not live in this tree; the surface it
shares with this repo is `crates/trace-commons-protocol`. There is **no
Ironclaw path dependency**. Do not look for one. Do not propose adding one.

## Database backend

This repo is **PostgreSQL-only**. There is no libsql build configuration.
Global guidance to verify several database feature builds ("both postgres and
libsql") does **not** apply here: a single `cargo check -p trace-commons-server`
is sufficient. Do not add libsql feature flags, do not propose dual-backend
testing, do not run libsql-specific verification.

## Where to look

- `README.md` — what Trace Commons and Near AI Gateway are, install paths,
  repository layout, build commands and the required checks. Lead with this
  when orienting on the repo.
- `docs/status.md` — pilot deployment status and public reference notes.
- `docs/trace-commons.md` — envelope contract and threat model (authoritative).
- `docs/trace-commons-storage.md` — storage contract (authoritative).
- `docs/trace-commons-roadmap.md` — the production-gap queue and phase plan.
  When in doubt about what to build next, read its **"Production Gap Queue"**
  section.
- `docs/superpowers/specs/` and `docs/superpowers/plans/` — per-slice design
  specs and implementation plans.
- `docs/operator/README.md` — operator runbook index; every runbook under
  `docs/operator/` is listed there.
- Crates: protocol `crates/trace-commons-protocol`, server
  `crates/trace-commons-server`, ingest service `crates/trace-commons-ingest`.
  Migrations live in `migrations/`.

## Binaries

- `trace-commons-ingest` — hosted ingest / review / admin / worker API. Built
  from its own crate, `crates/trace-commons-ingest` (`cargo build -p
  trace-commons-ingest --bin trace-commons-ingest`). The other server-side
  binaries are in `trace-commons-server`.
- `trace-commons-upload-claim-issuer` — EdDSA/Ed25519 upload-claim issuer.
- `trace-commons-gate-calibrate` — calibration tooling. Subcommands include
  `tail-floor --sidecar <path> --db-url <url> --percentile <n>` for tail-floor
  calibration, and the scorer flags `--scorer perplexity|token-rarity|both`
  with `--token-rarity-k <N>` for per-token rarity. The real-scorer path
  (`LocalPerplexityScorer`) is deferred at `BakeoffRealRarityNotImplemented`
  pending A.5a.

## Local development

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server -p trace-commons-ingest --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test it -- 'trace_corpus_storage_contract::'
cargo test -p trace-commons-server --test trace_corpus_pg_store   # requires PostgreSQL
RUSTFLAGS="-D warnings" cargo check -p trace-commons-ingest --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-ingest --no-run
```

**CI applies `RUSTFLAGS=-D warnings`; plain `cargo check` does not**, so
dead-code and warning regressions slip through it. Before claiming green,
always run the `RUSTFLAGS` form of `cargo check --bins` and `cargo test
--no-run` for `-p trace-commons-server`, and the same two for
`-p trace-commons-ingest` on an ingest change. Clippy is CI-enforced; run it
locally too. CI's clippy job is wider than the command above: `--workspace
--all-targets` with `-D warnings`, plus three `near-ai-staging` feature builds.

### Integration tests: one `it` binary per crate

The server, contributor, FFI and protocol crates build most of their
integration tests as one binary each, `tests/it.rs`, with each
`tests/<name>.rs` as a module (`autotests = false`), so the dependency graph
links once per crate instead of once per file.

- Run one file's tests with `--test it -- '<name>::'`. Quote the filter; in a
  workflow `run:` line an unquoted trailing `::` breaks the YAML.
- A new `tests/*.rs` file needs a `mod <name>;` line in `tests/it.rs` or its
  own `[[test]]`. `it_harness::every_test_file_is_built_once` fails until it
  has exactly one.
- A file that cannot share a process with the others gets its own `[[test]]`
  with the reason beside it: every PostgreSQL suite, and files that set
  process-wide env vars, install a global tracing subscriber, signal their
  own pid, or are built by the near-ai-staging CI step.
- PostgreSQL suites pointed at the same database must not run at once. Run
  them one at a time.
- Separate databases on one cluster are not enough either: every fresh
  database's migrations write cluster-wide role rows (`CREATE ROLE`, `ALTER
  ROLE`, `GRANT <guard> TO CURRENT_USER`), and two suites migrating at once
  fail with `tuple concurrently updated`. To run CI's PostgreSQL suites
  (the `postgres-suites` job's and the ingest slices in `ingest-bin-postgres`)
  locally in parallel, use `scripts/dev/pg-suites-local.sh [--jobs N]
  [--only a,b]`: it builds the test binaries once, starts one throwaway
  cluster per lane (needs `initdb`/`pg_ctl` on PATH), runs the suites
  round-robin across the lanes with each suite's log under
  `target/pg-suites-local/`, refuses a suite whose database committed
  nothing, prints a summary, and removes the clusters. `--list` names the
  suites; keep its table in step with `ci.yml`.

Workspace crates build with `debug = "line-tables-only"` (root `Cargo.toml`):
backtraces keep file:line, but a debugger has no variables. Set `debug = true`
locally to step through.

### Client work

Select the client packages rather than the whole workspace:

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-sources
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-platform
# Only when checking the client/server boundary (also run by workspace CI):
RUSTFLAGS="-D warnings" cargo test -p trace-commons-client-server-tests
```

The unpublished `trace-commons-client-server-tests` package has one `contracts`
test executable holding the enrollment/submission, admission and native-auth
cross-checks. It owns the server dev-dependency; the contributor has none, so
client unit tests never build the server. `account_auth::wire` is exposed only
under the test credential-store feature for those cross-checks; the shipped
API keeps only its existing `NATIVE_REDIRECT_PATH` export. Run the contract
package when changing either side of that boundary, not on every client-only
edit. `cargo test --workspace` still runs all of it.

Two crates are separate Cargo workspaces with their own lockfiles, so `-p`
from the repo root does not reach them: the GTK shell
(`--manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`) and the
Tauri app (`--manifest-path tauri-desktop/src-tauri/Cargo.toml`, plus
`pnpm test` and `pnpm build` in `tauri-desktop/frontend`). No CI job builds or
tests the Tauri app; run those commands by hand when you change it.

### Shared compile cache (recommended)

Each worktree has its own `target/`, so without a shared cache every new
worktree recompiles the whole third-party dependency graph, and the copies
fill the disk. Install sccache (`brew install sccache`, or `cargo install
sccache --locked`) and point cargo at it in your user-level config:

```toml
# ~/.cargo/config.toml
[build]
rustc-wrapper = "sccache"
```

**Never set `rustc-wrapper` in the repository's own `.cargo/config.toml`**: CI
runners do not install sccache, and every build there would fail.

For agent and worktree builds (anything that is not an edit-rebuild loop at
your own terminal), also do these three. Without them, most of a build misses
the cache:

- **Set `CARGO_INCREMENTAL=0`.** sccache refuses to cache an incremental
  compile, which is how the dev profile builds the workspace's own crates.
- **Set `RUSTFLAGS="-D warnings"` for every cargo command, clippy included.**
  The flags are part of sccache's key and cargo's fingerprint, so alternating
  them builds the dependency graph twice. CI sets both variables.
- **Raise the cache size limit** from the 10 GiB default to about 50 GiB: on
  macOS, `[cache.disk]` `size = 53687091200` in
  `~/Library/Application Support/Mozilla.sccache/config`, then
  `sccache --stop-server`.

For Claude Code, put the two variables in the `env` block of
`~/.claude/settings.json` so every session and subagent gets them; for other
agents, use their equivalent user-level environment. Keep them out of the
repository's `.cargo/config.toml`, which would change every human's edit loop
too. `sccache --show-stats` shows the hit rate and size limit. Binaries, test
binaries and proc-macros are never cached, so linking costs the same.

### Ingest PostgreSQL tests

The ingest service's PostgreSQL tests (`ingest::app::`, in the
`trace-commons-ingest` library) self-skip without
`TRACE_COMMONS_PG_TEST_DATABASE_URL`. CI runs them in the `ingest-bin-postgres`
job: the whole library single-threaded, plus named slices with their own
databases and env. To run all of them locally, use a fresh
`admission_test_*` database on the literal host `127.0.0.1`, set the
login-resolver URL (a LOGIN role that is a member of `trace_login_resolver`,
see `docs/operator/login-resolver-role.md`), and run single-threaded, because
the tests share the `tenant-a` fixture tenant:

```bash
createdb -h 127.0.0.1 admission_test_ingest_local
TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://$USER@127.0.0.1:5432/admission_test_ingest_local \
TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=postgres://tc_login_resolver_login@127.0.0.1:5432/admission_test_ingest_local \
cargo test -p trace-commons-ingest --lib -- --test-threads=1
```

Without the resolver URL, the account and login tests fail with
`redeem succeeds: left 400 right 303`. CI runs the same thing in the
`ingest-bin-postgres` job (`trace-commons-ingest tests, whole bin, against
PostgreSQL`), a required status check.

## Executing a multi-task plan

When a plan's tasks are carried out one after another, test and check once,
after the last task is written, not after each task. This overrides global
guidance to run each change's targeted test straight away: a full
compile-and-test cycle of the server and contributor crates takes minutes, and
paying it per task turns a plan into hours.

- Each task still writes every test its plan step specifies, with the
  assertions, as TDD would. Only running them is deferred.
- Per task, run at most a plain `cargo check -p <crate> --all-targets` on
  the crates it touched (add `--features` for a feature-gated module), so
  the next task builds on code that compiles. No test runs, no clippy, no
  `RUSTFLAGS=-D warnings`, no Swift suite, no PostgreSQL suites.
- Do not report a task as passing. Report which tests it wrote and did not
  run.
- After the last task, run everything once: the commands in "Local
  development", clippy, `cargo fmt --all -- --check`, the PostgreSQL suites
  the plan touched, the FFI build followed by `swift test`, and the
  separate GTK and Tauri workspaces if touched. Fix every failure against
  the spec, never by weakening a test. A failure that also fails on `main`
  is pre-existing: note it and leave it.
- Reviews of the branch come after that run, so they read code that passes.

This covers implementation inside one branch. It does not relax "never claim
green from a partial run", and it does not apply to a single fix, where the
targeted test still runs immediately.

## CI

`.github/workflows/ci.yml` holds the server-side jobs and runs them on every
PR, with two exceptions. A `changes` job (`scripts/ci/ci-changes.sh`) lets a
pull request that changes only documents no other tracked file names skip the
compiling jobs; it fails open, and push and merge-queue runs always run
everything. And the two `near-ai-*` ingest whole-bin variants run only on
push to `main` and in the merge queue. The lists below are not a full
inventory; read the workflow for that.

Running is not the same as blocking. **Thirteen** jobs are required status
checks on `main`, and only those block a merge; `README.md` lists them too.
`main` is also behind a merge queue (`main merge queue`), so the required
checks are re-run against `main` at merge time, and a PR that never receives
them times out of the queue instead of merging. Branch protection matches job
names exactly: renaming a required job drops it from the gate.

### Required checks

- `cargo fmt --check` — runs `cargo fmt --all -- --check`. Run
  `cargo fmt --all` before committing.
- `cargo clippy` — with the allow-list under "Local development". Do not
  widen the allow-list without explicit approval.
- `cargo check (default features)`.
- `cargo check (local-gpu-models, non-CUDA)`. Checked in CI even though it
  cannot link locally without a CUDA toolchain, so a change that only
  compiles under default features fails here.
- `cargo check (near-ai-scorer)`. The configuration the pilot builds.
- `cargo check (permissive crates, standalone)` — builds each MIT/Apache crate
  alone with `--no-default-features`, which is what a third-party harness
  gets. Every other job unifies features across the workspace, so only this
  job notices a permissive crate that has come to need one of the workspace's
  optional features.
- `builds at the declared MSRV floor` — the only job that does not use
  `dtolnay/rust-toolchain@stable`, so the only one that can see an MSRV
  regression. It reads `rust-version` out of `cargo metadata` (never a
  literal in the workflow) for the root and GTK workspaces, audits each
  dependency graph, and checks the contributor CLI, the FFI dylib and the GTK
  shell under that toolchain. Logic lives in `scripts/ci/msrv-floor.sh`.
- `cargo test (default features)`, plus two jobs split out of it because
  each builds a different feature set or package selection: `staging channel
  tests` (the `near-ai-staging` feature build) and `wallet SDK and production
  CSP` (Playwright/Chromium and the ignored wallet-page test).
- `pilot-bootstrap smoke` — `scripts/operator/pilot-bootstrap-smoke.sh`,
  exercising the JSONL loader path.
- `database suites against a real PostgreSQL` and `trace-commons-ingest
  tests, whole bin, against PostgreSQL` — the only jobs that point the
  PostgreSQL-backed tests at a real database; everywhere else they self-skip.
  Both end by refusing a run whose database shows no committed transactions,
  because a pass count alone does not prove the tests reached PostgreSQL.
  `postgres-suites` builds every target once, then runs its suites in two
  lanes on two separate PostgreSQL servers (lane B through
  `scripts/ci/pg-lane.sh`, with per-suite log groups and a results step that
  names each failure): separate databases on one server race on cluster-wide
  role rows. The ingest lib slices run in `ingest-bin-postgres`, which builds
  the same harness, each step with `if: !cancelled()`.

### Not required (block nothing)

- `operator-binaries smoke`.
- `trace-commons-ingest tests, whole bin, near-ai-scorer, against PostgreSQL`
  and `... near-ai-staging, against PostgreSQL` — run on push to `main` and in
  the merge queue, not on pull requests, so a regression in either surfaces
  after the PR.
- `pipeline qualification and restore` — runs `python3
  scripts/operator/pipeline.py qualify` against its own digest-pinned
  PostgreSQL container (no `services:` block), signed with a throwaway check
  key that the job makes with `pipeline.py keygen` under the runner's temp
  directory (trusted nowhere, never uploaded), then `pipeline.py compare
  --self-test`. Not required by owner decision (P4-D17): it also runs in the
  merge queue, but blocks nothing until the owner promotes it. See
  `docs/operator/pipeline-qualification.md` and
  `docs/operator/pipeline-comparison.md`.

### Client shells: `clients.yml`

`.github/workflows/clients.yml` holds the client jobs: `macOS app tests`,
`macOS 15 fallback tests`, `windows contributor crates (tests, named-pipe
ACL, update conformance)`, `install.ps1 against the real release`, `windows contributor app`, `linux-shell desktop entry and metainfo`,
`linux-shell desktop integration (weston + portal)`. It runs on every push to
`main`, on `workflow_dispatch`, and on a pull request only when it touches a
client path (its `paths:` filter). None of its jobs is a required status
check, so a server-only PR never waits on a macOS or Windows runner, and a
client regression surfaces on `main` after the merge.

- `macOS app tests` — `swift test` in `macos/`, on `macos-26`. The only PR
  job that runs the whole Swift suite (`macOS 15 fallback tests` runs a
  filtered subset). Needs `cargo build -p trace-commons-contributor-ffi`
  first, because the Swift package links that dylib.
- `windows contributor crates (tests, named-pipe ACL, update conformance)` —
  one job, so one compile serves the crate tests, the pipe-ACL check and the
  update conformance; each keeps its own named step. Its named-pipe ACL step
  runs `scripts/windows/verify-pipe-acl.ps1`,
  which creates a second, non-administrator local account and requires it to
  be denied when opening the contributor daemon's named pipe. That DACL is
  the sole access control on the daemon IPC socket on Windows, and it is not
  observable from a cross-compile, so this job is the only thing standing
  behind the claim that it works. Do not weaken it to a smoke test.
- `linux-shell desktop entry and metainfo` — runs
  `crates/trace-commons-contributor-gtk/scripts/validate-desktop-entry.sh`
  (`desktop-file-validate`, `appstreamcli validate`, the icon-name pair, the
  metainfo-vs-crate version pin). The weston job runs the same script.

### Caching rules

- A job that caches `target` restores with `actions/cache/restore@v5` and
  saves with `actions/cache/save@v5` **on a push to `main` only**, after
  `./.github/actions/trim-cargo-cache`, which deletes linked test and bin
  executables, `incremental/`, and the workspace's own `trace-commons-*`
  crates (checkout makes cargo rebuild those every run anyway).
- A job that adds a `target` cache must follow the same shape. A pull
  request's cache is restorable by that pull request alone, so saving one only
  evicts `main`'s from GitHub's 10 GB allowance.
- The jobs in `clients.yml` keep no cache at all.
- The two PostgreSQL jobs restore (never save) the `pg-ingest-bin-near-ai-staging`
  cache, which resolves the same third-party graph; `pilot-bootstrap smoke`
  caches its `--release` build in the same restore-trim-save shape.
- `./.github/actions/prune-cargo-cache` runs after each save on `main` and
  deletes that cache family's older lockfile generations (never the key just
  saved), so `main`'s caches stay inside the allowance.
- `./.github/actions/free-runner-disk` starts its deletions in the background
  and waits (until done or 40 GB free) just before the first compile.
- The three `cargo test` jobs restore through
  `./.github/actions/restore-cargo-test-cache`, which builds the key in one
  place, falls back to another job's key when asked, and warns on the run
  when nothing at all was restored.
- Push-to-main runs are exempt from `cancel-in-progress`: a cancelled job
  saves no cache, and merges to `main` land close together.

GitHub Actions runners are on Node 24; pinned actions are
`actions/checkout@v6` and `actions/cache/{restore,save}@v5`. Future CI edits
should hold those versions unless intentionally upgrading.

## Licensing

This repo is **split-licensed**, and the split is load-bearing. `AGENTS.md`
carries the full statement and working rules; the essentials are repeated here.

- **AGPL-3.0-or-later**: `trace-commons-server`, `trace-commons-gate-api`,
  `trace-commons-gate-enclave`, `trace-commons-ingest`. Copyright K&Z Partners
  LLC. Every `.rs` file in these crates carries a two-line copyright + SPDX
  header; new files need one.
- **MIT OR Apache-2.0**: everything else (`-protocol`, `-contributor`,
  `-contributor-sources`, `-contributor-insights`, `-contributor-platform`,
  `-contributor-ffi`, `-contributor-gtk`, `-operator-client`, `-mark`,
  `-build-info`, `-attestation`, the unpublished `-client-server-tests`, and
  `tauri-desktop/src-tauri` (`trace-commons-tauri-desktop`)). The shipped
  clients can be embedded in proprietary agent harnesses and must stay
  permissive.

**Contributions are licensed inbound under `MIT OR Apache-2.0`**, including to
the AGPL crates — deliberately not "inbound = outbound", so the project keeps
the right to relicense what it distributes. Stated in `CONTRIBUTING.md`, the PR
template, `LICENSE`, and the README; keep those four consistent.

**Permissive code may flow into the AGPL crates. Never the reverse.** Do not add
`trace-commons-gate-api`, `-gate-enclave`, `-server`, or `-ingest` to a
permissive crate's dependencies, not even to reuse a single trait: that
silently makes a shipped client copyleft.
`crates/trace-commons-server/tests/license_boundary.rs` fails if you do. If it
fails, remove the dependency. Do not edit the expected sets in that file to
match your diff: those sets are the specification. Test-only dev-dependencies
across the boundary are permitted but pinned in that same file.

`trace-commons-gate-api`'s traits being AGPL is deliberate: the
proprietary-backend seam (see "Gate contracts" below) is open to the copyright
holder and closed to third parties. The inbound MIT/Apache term keeps that
possible; an outside contribution offered under other terms would end it.

AGPL section 13 obliges a network operator to offer the Corresponding Source.
`trace-commons-ingest` does this at `GET /v1/source`: unauthenticated, no
tenant context, outside every fail-closed gate. Do not put it behind auth.

A new dependency must be combinable into an AGPL-3.0 work: GPL-2.0-only, SSPL,
or proprietary is a hard conflict. `cargo deny check licenses` audits
dependency licenses against `deny.toml`. Run all four:

```bash
cargo deny check licenses
cargo deny --features near-ai-scorer check licenses
cargo deny --features local-gpu-models check licenses
cargo deny --all-features check licenses
```

- `--features` is a **global** flag on `cargo deny`, not a `check` flag: it
  goes before the subcommand. `cargo deny check licenses --features
  near-ai-scorer` errors with `unexpected argument '--features' found`.
- The `--all-features` run is not redundant with the other three. Some crates
  resolve only under it: the whole `google-cloud-*` tree behind `gcs-client` /
  `gcp-kms`, the `cudarc` / `ug-cuda` stack behind `local-gpu-models-cuda`,
  and `dcap-qvl`'s collateral client behind `near-attestation-collateral`. Do
  not delete it as duplicated coverage.
- CI runs `check licenses`, `check sources` and `check advisories` under
  `--all-features`, so the manual runs are a pre-flight, not the only
  enforcement.

## Product text

When adding or changing user-facing UI text, follow
[docs/ui-copy-guide.md](docs/ui-copy-guide.md). It covers short action labels,
preserved context, separate titles and accessibility names, and shared-copy
review checks.

**No new notices without explicit approval.** Do not add a banner, card,
one-time message, interruption, notification, or a daemon status channel that
feeds one, unless the owner has explicitly approved both the notice and its
wording. A spec or design principle calling for one is not approval: ask.
See rule 9 of the copy guide.

## Conventions specific to this repo

- **Hash-only audit and logging.** Audit rows, error logs, and operational
  surfaces are hash-only or label-only. Never include raw URLs, bearer tokens,
  ARNs, account references, transaction hashes, contributor identity, trace
  bodies, or any operator-secret material in stored rows or log strings.
- **Fail-closed by default.** When a required gate is configured but its
  dependency is missing, refuse the path with a safe missing-control name.
  Never silently fall back to plaintext or to a less-restricted backend.
- **Tenant scoping.** Every read/write path must be driven by auth-derived
  tenant + actor context. Envelope tenant fields are attribution only.
- **PostgreSQL RLS is forced** on every Trace Commons table. Tenant predicates
  go through `trace_current_tenant_id()`; the raw pool is restricted.
- **No emojis in commits, PRs, code, or reports.** Match the existing commit
  style (short imperative subjects without `feat:` / `fix:` prefixes).
- **Gate contracts live in `trace-commons-gate-api`.** Traits, result types, and
  decision types belong there; scoring implementations do not. Code that holds a
  scorer, embedder, or vector index must hold it as a trait object, never as a
  concrete type — a proprietary backend substitutes at that seam, and a concrete
  type cannot participate in one. The `Reference*` implementations in that crate
  are real but deliberately simple and uncalibrated; the `Mock*` types in
  `trace-commons-gate-enclave` are hash-derived test doubles and must never gate
  anything. See `crates/trace-commons-gate-api/README.md`.

## Working with this codebase

- Files have grown large. Do not unilaterally split files unless a spec asks
  for it. Add new modules beside existing code when possible.
- Worker routes follow a consistent auth pattern — find an existing handler
  and copy its shape rather than inventing a new one.
- Worker-route credentials are scoped: utility, review, retention, vector,
  benchmark, process-evaluation, revocation, export, and revocation-propagation
  each have their own bearer-token gate. Do not mix them.
- Drills (`/v1/admin/*-drill`) produce hash-only evidence and feed rollout-smoke
  required checks. When you add a drill, wire it into the smoke evidence path.
- Pilot-bootstrap loads JSONL session files. Parquet + arrow deps were
  removed; do not reintroduce them. See
  `crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs`.
- `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs` is tens of
  thousands of lines of production code. Despite its path it is a library
  module, `trace_commons_ingest::ingest::app`, declared by `#[path]` in
  `src/ingest.rs`; the `trace-commons-ingest` binary is the thin
  `src/bin/trace_commons_ingest_main.rs` over `trace_commons_ingest::run`, so
  a build that registers its own Score policies can start the same service
  (`IngestOptions::with_score_policies`).
  - Its unit tests run under `--lib` with names prefixed `ingest::app::`
    (`cargo test -p trace-commons-ingest --lib`). A `--bin
    trace-commons-ingest <filter>` test run, or a `-p trace-commons-server
    --lib ingest::app::` run, matches nothing and passes.
  - Inside it and its `trace_commons_ingest_internal/` modules, `crate::` is
    the `trace_commons_ingest` library root: the service's own items are
    `crate::ingest::app::...` or `super::`, and the server library is
    `trace_commons_server::...`.
  - Its test module, larger still, lives in a sibling file via `#[cfg(test)]
    #[path = "trace_commons_ingest_internal/tests.rs"] mod tests;`. Reuse this
    pattern if other binaries need the same split; do not inline the tests
    back.

## Known gotchas

- **Feature-gated tests pass by not building.** `cargo test -p <crate>`
  compiles none of a module behind a non-default feature, then reports a green
  run of the tests it did build. Three crates have this shape:
  `trace-commons-attestation` (`receipt.rs`, under `--features receipt`),
  `trace-commons-gate-enclave` (under `near-ai-scorer`), and
  `trace-commons-protocol` (under `near-ai-privacy-filter`). Pass `--features`,
  or use `cargo test --workspace`, which is what CI runs and which reaches
  them through feature unification.
- **Parsers and routing live in the leaf crate.** The session-store parsers
  (`source/`) and the routing row, ledger trait, attested capture and enriched
  source (`routing/{mod,attested,enriched}`) live in
  `crates/trace-commons-contributor-sources`, a permissive leaf the
  contributor re-exports at `trace_commons_contributor::source`, `::routing`,
  and `daemon::settings::SourceDeclaration`. Their unit tests run under
  `cargo test -p trace-commons-contributor-sources`;
  `-p trace-commons-contributor` runs only the assertions that need the
  daemon (`source/cross_layer_tests.rs`, `routing/cross_layer_tests.rs`). Fix
  a parser bug in the leaf crate, which must never depend on the contributor.
- **Insights and platform are their own crates.** Private local Insights lives
  in `trace-commons-contributor-insights`, re-exported at
  `trace_commons_contributor::insights`, with its unit tests. It depends on
  sources, protocol and the platform crate, never on the daemon.
  `trace-commons-contributor-platform` owns OS credential storage and shared
  local-file primitives. The test credential-store feature is forwarded to
  both crates but never enabled in production; it refuses OS credential
  access outside the platform crate's own tests, and Insights' seeded digest
  key remains `cfg(test)` only. Run the platform package's ignored OS
  round-trip tests on the native OS (CI does).
- **A stale FFI dylib fakes shell-suite results.** The shell test projects
  copy the FFI dylib only `Condition="Exists(...)"`
  (`windows/tests/TraceCommons.Interop.Tests/*.csproj`), so a stale or absent
  artifact fails nothing and the suite tests whatever ABI it last built. CI
  builds `trace-commons-contributor-ffi` before running them; a local run does
  not. The exported-vs-declared assertions do not catch it, because both
  halves move together. Rebuild the dylib before trusting a shell suite
  result, green or red.
