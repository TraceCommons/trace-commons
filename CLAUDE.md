# Repository Guidance — trace-commons-server

This file pins repo-specific facts that override the global CLAUDE.md guidance.

## What this repo is

`trace-commons-server` is the **standalone hosted server-side control plane for
Trace Commons / TraceCommons**. It is no longer part of Ironclaw — extracted from
Ironclaw's `gecko-pass` worktree and now owns its own database schema, object
storage, encrypted artifact store, upload-claim issuer, and shared protocol
crate. Ironclaw should depend on `crates/trace-commons-protocol` when the
client-side integration is rewired, but it does not live in this tree.

It also holds the contributor side: the contributor CLI and daemon
(`trace-commons-contributor`, whose session-store parsers and routing
overlay live in the leaf crate `-contributor-sources`), the C ABI
(`-contributor-ffi`), and four desktop shells -- macOS (`macos/`, Swift),
Windows (`windows/`, C#), Linux GTK (`crates/trace-commons-contributor-gtk`),
and Tauri (`tauri-desktop/`).

**Native SwiftUI (`macos/`) is the main client on macOS** (#1173 D1): no new
Tauri work on macOS, and the Tauri app is retired there once the native
screens match. Windows and Linux stay on Tauri, frozen, until macOS ships
(#1173 D13, a default for now). Native macOS is also the lead client and the
parity target: GTK, Windows and Tauri match its terminology, states, flows
and affordances, and Tauri drift is low priority.

There is **no Ironclaw path dependency**. Do not look for one. Do not propose
adding one.

## Database backend

This repo is **PostgreSQL-only**. There is no libsql build configuration.

The global CLAUDE.md guidance to "verify ALL relevant feature flags compile
cleanly (e.g., both postgres and libsql builds)" does **not** apply here. A
single `cargo check -p trace-commons-server` is sufficient. Do not add libsql
feature flags, do not propose dual-backend testing, do not run libsql-specific
verification.

## Where to look

- `README.md` — current phase status, open production gaps, operator promotion
  checklist. Lead with this when orienting on the repo.
- `docs/trace-commons.md` — envelope contract and threat model (authoritative).
- `docs/trace-commons-storage.md` — storage contract (authoritative).
- `docs/trace-commons-roadmap.md` — the production-gap queue and phase plan.
- `docs/superpowers/specs/` — per-slice design specs.
- `docs/superpowers/plans/` — per-slice implementation plans.
- `docs/operator/README.md` — operator runbook index; every runbook under
  `docs/operator/` is listed there.

When in doubt about what to build next, read the **"Production Gap Queue"**
section of the roadmap.

## Binaries

- `trace-commons-ingest` — hosted ingest / review / admin / worker API. Built
  from its own crate, `crates/trace-commons-ingest` (`cargo build -p
  trace-commons-ingest --bin trace-commons-ingest`); the other binaries here
  stay in `trace-commons-server`.
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
cargo test -p trace-commons-server --test trace_corpus_storage_contract
cargo test -p trace-commons-server --test trace_corpus_pg_store   # requires PostgreSQL
RUSTFLAGS="-D warnings" cargo check -p trace-commons-ingest --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-ingest --no-run
```

For client work, select the client packages rather than the whole workspace:

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-sources
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-platform
# Only when checking the client/server boundary (also run by workspace CI):
RUSTFLAGS="-D warnings" cargo test -p trace-commons-client-server-tests
```

The unpublished `trace-commons-client-server-tests` package has one `contracts`
test executable holding enrollment/submission, admission and native-auth
cross-checks. It owns the server dev-dependency; the contributor has none, so
even client unit tests no longer build the server. `account_auth::wire` is
exposed only under the test credential-store feature for those cross-checks;
the shipped API keeps only its existing `NATIVE_REDIRECT_PATH` export. Run the
contract package when changing either side of that boundary, not on every
client-only edit. `cargo test --workspace` still runs all of it.

### Shared compile cache (recommended)

Each worktree has its own `target/`, so a new worktree compiles the whole
third-party dependency graph from scratch, and the copies add up: on one
machine on 2026-10-15, sixteen worktree `target/` directories held 523 GB
and filled the disk. Install sccache and point cargo at it in your
user-level config:

```bash
brew install sccache        # or: cargo install sccache --locked
```

```toml
# ~/.cargo/config.toml
[build]
rustc-wrapper = "sccache"
```

Worktrees then share compiled third-party crates. `sccache --show-stats`
shows the hit rate, the cache location and its size limit.

For agent and worktree builds (anything that is not an edit-rebuild loop at
your own terminal), also do these three. Without them, most of a build
misses the cache:

- **Set `CARGO_INCREMENTAL=0`.** The dev profile builds the workspace's own
  crates incrementally, and sccache refuses to cache an incremental
  compile, so without this `trace-commons-contributor`, the server and the
  ingest bin recompile in every worktree. CI sets the same variable.
- **Set `RUSTFLAGS="-D warnings"` for every cargo command, clippy
  included.** The flags are part of both sccache's key and cargo's
  fingerprint, so alternating `-D warnings` (check, test) with no flags
  (clippy) builds the dependency graph twice. CI sets the same value for
  every job, clippy too, so this is also what CI runs.
- **Raise the cache size limit.** The default is 10 GiB; on one machine
  on 2026-10-17 the cache held 17 GiB against a 20 GiB limit, building this
  repository's three Cargo workspaces (root, GTK, Tauri) and IronWire. 50 GiB
  is a working size; on macOS set `[cache.disk]` `size = 53687091200` in
  `~/Library/Application Support/Mozilla.sccache/config`, then
  `sccache --stop-server`.

For Claude Code, put the two variables in the `env` block of
`~/.claude/settings.json` so every session and subagent gets them; for
other agents, use their equivalent user-level environment. Keep them out of
the repository's `.cargo/config.toml`, which would change every human's
edit loop too. Binaries, test binaries and proc-macros are never cached
whatever the settings, so linking still costs the same.

With the cache in place, deleting the `target/` of a worktree whose PR has
merged is cheap.

Never set `rustc-wrapper` in the repository's own `.cargo/config.toml`:
CI runners do not install sccache, and every build there would fail.

The ingest service's PostgreSQL tests (`ingest::app::`, in the
`trace-commons-ingest` library)
self-skip without `TRACE_COMMONS_PG_TEST_DATABASE_URL`, and the
`postgres-suites` job runs only named slices of them. To run all of them
locally, use a fresh
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

Without the resolver URL, about 74 account, passkey and login tests fail with
`redeem succeeds: left 400 right 303`. CI runs the same thing in the
`ingest-bin-postgres` job (`trace-commons-ingest tests, whole bin, against
PostgreSQL`), which is a required status check, in the merge queue too.

CI applies `RUSTFLAGS=-D warnings` to check + test, so plain `cargo check`
will not catch what CI catches. Always use the `RUSTFLAGS` form locally
before claiming green. Clippy is CI-enforced — run it locally too.

The protocol crate is `crates/trace-commons-protocol`; the server crate is
`crates/trace-commons-server`; the ingest service is
`crates/trace-commons-ingest`. Migrations live in `migrations/`.

Two crates are separate Cargo workspaces with their own lockfiles, so `-p`
from the repo root does not reach them: the GTK shell
(`--manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`) and the
Tauri app (`--manifest-path tauri-desktop/src-tauri/Cargo.toml`, plus
`pnpm test` and `pnpm build` in `tauri-desktop/frontend`). No CI job builds or
tests the Tauri app; run those commands by hand when you change it.

## Executing a multi-task plan

When a plan's tasks are carried out one after another, test and check once,
after the last task is written, not after each task. This overrides the
global guidance to run each change's targeted test straight away. A full
compile-and-test cycle of the server and contributor crates takes minutes,
and paying it once per task made a nine-task plan take hours.

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

`.github/workflows/ci.yml` holds the server-side jobs and runs every one of
them on every PR. The list below covers the long-standing ones and is not a
full inventory -- read the workflow for that.

The client shells are in `.github/workflows/clients.yml` since 2026-09-22:
`macOS app tests`, `windows named-pipe ACL`, `windows contributor crate
tests`, `windows update conformance`, `install.ps1 against the real release`,
`windows contributor app`, `linux-shell desktop entry and metainfo`,
`linux-shell desktop integration (weston + portal)`. That workflow runs on
every push to `main`, on `workflow_dispatch`, and on a pull request only when
it touches a client path (its `paths:` filter). None of its jobs is a
required status check, so a server-only PR never waits on a macOS or Windows
runner, and a client regression surfaces on `main` after the merge.

A job that caches `target` restores with `actions/cache/restore@v5` and
saves with `actions/cache/save@v5` on a push to `main` only, after
`./.github/actions/trim-cargo-cache`, which deletes linked test and bin
executables, `incremental/`, and the workspace's own `trace-commons-*`
crates (checkout makes cargo rebuild those every run anyway). The
repository's caches were over GitHub's 10 GB ceiling and a main-written
cache was evicted within minutes: a pull request's cache is restorable by
that pull request alone, so saving one only evicted `main`'s. A job that
adds a `target` cache must follow the same shape, or it re-creates the
churn, and the jobs in `clients.yml` keep no cache at all. The three
`cargo test` jobs (`cargo test (default features)`, `staging channel
tests`, `wallet SDK and production CSP`) restore through
`./.github/actions/restore-cargo-test-cache`, which builds the key in one
place, falls back to another job's key when asked, and warns on the run
when nothing at all was restored. Push-to-main
runs are exempt from `cancel-in-progress` for the same reason: a cancelled
job saves no cache, and merges to `main` land close together.

Running is not the same as blocking. **Thirteen** of the twenty-three are required
status checks on `main`, and only those block a merge -- `README.md` lists
them. `main` is also behind a merge queue (`main merge queue`), so the
required checks are re-run against `main` at merge time; a PR that never
receives them times out of the queue instead of merging.

- `cargo fmt --check` — runs `cargo fmt --all -- --check`. Run `cargo fmt --all`
  before committing.
- `cargo clippy` with the allow-list above (`-A clippy::type_complexity
  -A clippy::collapsible_if -A clippy::manual_option_as_slice
  -A clippy::useless_vec -A clippy::redundant_pattern_matching`). Do not
  widen the allow-list without explicit approval.
- `cargo check (default features)` (with `RUSTFLAGS=-D warnings`).
- `cargo check (local-gpu-models, non-CUDA)` (with `RUSTFLAGS=-D warnings`).
  Note this feature IS checked in CI even though it cannot link locally without
  a CUDA toolchain — a change that only compiles under default features will
  fail here.
- `cargo check (near-ai-scorer)` (with `RUSTFLAGS=-D warnings`). This is the
  configuration the pilot builds.
- `cargo check (permissive crates, standalone)` — builds each MIT/Apache crate
  alone with `--no-default-features`. Every other job builds the workspace or
  the server crate, and cargo unifies features across those, so a permissive
  crate can silently come to need one of this workspace's optional features.
  That is the configuration a third-party harness gets, and this job is the
  only thing that exercises it. See #486.
- `cargo test (default features)` (with `RUSTFLAGS=-D warnings`). Two
  parts of it run as their own jobs since 2026-10-30, and both are
  required checks, as they were inside it: `staging channel tests` (the
  `near-ai-staging` feature build) and `wallet SDK and production CSP`
  (Playwright/Chromium and the ignored wallet-page test). Each builds a
  different feature set or package selection, so inside the workspace job
  each recompiled crates the workspace run had already built, and the job
  ran into its 30-minute timeout. Branch protection matches these three
  names exactly; renaming one drops it from the gate.
- `pilot-bootstrap smoke` — `scripts/operator/pilot-bootstrap-smoke.sh`,
  exercising the JSONL loader path.
- `operator-binaries smoke`.
- `database suites against a real PostgreSQL` and `trace-commons-ingest
  tests, whole bin, against PostgreSQL` — the only jobs that point the
  PostgreSQL-backed tests at a real database; everywhere else they self-skip.
  Both are required, and both end by refusing a run whose database shows no
  committed transactions, because a pass count alone does not prove the
  tests reached PostgreSQL.
- `pipeline qualification and restore` — runs `python3
  scripts/operator/pipeline.py qualify` against its own digest-pinned
  PostgreSQL container (no `services:` block), signed with a throwaway check
  key that the job makes with `pipeline.py keygen` under the runner's temp
  directory (trusted nowhere, never uploaded). Not a required status check
  (owner decision P4-D17): it runs on every PR and in the merge queue, but
  blocks nothing until the owner promotes it. See
  `docs/operator/pipeline-qualification.md`. The job also runs
  `pipeline.py compare --self-test` after `qualify`; see
  `docs/operator/pipeline-comparison.md`.
- `builds at the declared MSRV floor` — the only job that does NOT use
  `dtolnay/rust-toolchain@stable`. It reads `rust-version` out of `cargo
  metadata` (never a literal in the workflow) for both the root workspace and
  the separate GTK workspace, audits each resolved dependency graph for a crate
  demanding a newer rustc, and then checks the contributor CLI, the FFI dylib
  and the GTK shell under the derived floor toolchain. It is a required
  check. Every `@stable` job is structurally blind to an MSRV regression --
  stable is always at or above the floor -- so this job is the only thing
  that catches a dependency demanding a newer rustc before a release build
  does. Logic lives in `scripts/ci/msrv-floor.sh`.

Notes on three `clients.yml` jobs (none of them is a required check):

- `linux-shell desktop entry and metainfo` — `desktop-file-validate`,
  `appstreamcli validate`, the icon-name pair and the metainfo-vs-crate
  version pin, from `crates/trace-commons-contributor-gtk/scripts/validate-desktop-entry.sh`,
  in about a minute. The weston job runs the same script.
- `macOS app tests` — `swift test` in `macos/`, on `macos-26`. The only
  thing that runs the Swift suite. Needs
  `cargo build -p trace-commons-contributor-ffi` first, because the Swift
  package links that dylib.
- `windows named-pipe ACL` — runs `scripts/windows/verify-pipe-acl.ps1`,
  which creates a second, non-administrator local account and requires it to
  be denied when opening the contributor daemon's named pipe. That DACL is
  the sole access control on the daemon IPC socket on Windows, and it is not
  observable from a cross-compile, so this job is the only thing standing
  behind the claim that it works. Do not weaken it to a smoke test.

GitHub Actions runners are on Node 24; pinned actions are
`actions/checkout@v6` and `actions/cache@v5`. Future CI edits should hold
those versions unless intentionally upgrading.

## Licensing

This repo is **split-licensed**, and the split is load-bearing. `AGENTS.md`
carries the full statement of it; the essentials are repeated here.

- **AGPL-3.0-or-later**: `trace-commons-server`, `trace-commons-gate-api`,
  `trace-commons-gate-enclave`, `trace-commons-ingest`. Copyright K&Z Partners
  LLC. Every `.rs` file in these crates carries a two-line copyright + SPDX
  header; new files need one.
- **MIT OR Apache-2.0**: everything else (`-protocol`, `-contributor`,
  `-contributor-sources`, `-contributor-insights`, `-contributor-platform`,
  `-contributor-ffi`, `-contributor-gtk`,
  `-operator-client`, `-mark`, `-build-info`, `-attestation`, the unpublished
  `-client-server-tests`, and `tauri-desktop/src-tauri`
  (`trace-commons-tauri-desktop`)). The shipped clients can be embedded in
  proprietary agent harnesses and must stay permissive.

**Contributions are licensed inbound under `MIT OR Apache-2.0`**, including to
the AGPL crates -- deliberately not "inbound = outbound", so the project keeps
the right to relicense what it distributes. Stated in `CONTRIBUTING.md`, the PR
template, `LICENSE`, and the README; keep those four consistent.

**Permissive code may flow into the AGPL crates. Never the reverse.** Do not add
`trace-commons-gate-api`, `-gate-enclave`, `-server`, or `-ingest` to a
permissive crate's dependencies -- not even to reuse a single trait. That
silently makes a shipped client copyleft.
`crates/trace-commons-server/tests/license_boundary.rs` fails if you do; do not
"fix" it by editing the expected sets. Test-only dev-dependencies across the
boundary are permitted but pinned in that same file.

Note that `trace-commons-gate-api`'s traits being AGPL is deliberate: the
proprietary-backend seam described below is available to the copyright holder
and closed to third parties. The inbound MIT/Apache term is what keeps that
possible -- an outside contribution offered under other terms would end it.

AGPL section 13 obliges a network operator to offer the Corresponding Source.
`trace-commons-ingest` does this at `GET /v1/source` -- unauthenticated, no
tenant context, outside every fail-closed gate. Do not put it behind auth.

`cargo deny check licenses` audits dependency licenses against `deny.toml`.
`--features` is a **global** flag on `cargo deny`, not a `check` flag -- it goes
before the subcommand. `cargo deny check licenses --features near-ai-scorer`
errors with `unexpected argument '--features' found`. Run all four:

```bash
cargo deny check licenses
cargo deny --features near-ai-scorer check licenses
cargo deny --features local-gpu-models check licenses
cargo deny --all-features check licenses
```

The `--all-features` run is not redundant with the other three: 54 crates
resolve only under it -- the whole `google-cloud-*` tree behind `gcs-client` /
`gcp-kms`, the `cudarc` / `ug-cuda` stack behind `local-gpu-models-cuda`, and
`dcap-qvl`'s collateral client behind `near-attestation-collateral`. Do not
delete it as duplicated coverage.

A new dependency must be combinable into an AGPL-3.0 work: GPL-2.0-only, SSPL,
or proprietary is a hard conflict. CI runs `check licenses` and `check sources`
under `--all-features`, and runs `check advisories` under `--all-features`
too, so the manual runs above are a pre-flight, not the only enforcement.

If the boundary test fails, remove the dependency. Do not edit the expected sets
in `license_boundary.rs` to match your diff -- those sets are the specification.

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

- Files have grown large (e.g. `bin/trace-commons-ingest.rs` is huge). Do not
  unilaterally split files unless a spec asks for it. Add new modules beside
  existing code when possible.
- Worker routes follow a consistent auth pattern — find an existing handler
  and copy its shape rather than inventing a new one.
- Worker-route credentials are scoped: utility, review, retention, vector,
  benchmark, process-evaluation, revocation, export, and revocation-propagation
  each have their own bearer-token gate. Do not mix them.
- Drills (`/v1/admin/*-drill`) produce hash-only evidence and feed rollout-smoke
  required checks. When you add a drill, wire it into the smoke evidence path.
- `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs` is tens of
  thousands of lines of production code. Despite its path it is a library
  module, `trace_commons_ingest::ingest::app`, declared by `#[path]` in
  `src/ingest.rs`; the `trace-commons-ingest` binary is the thin
  `src/bin/trace_commons_ingest_main.rs` over `trace_commons_ingest::run`,
  so a build that registers its own Score policies can start the same service
  (`IngestOptions::with_score_policies`). Its unit tests therefore run under
  `--lib` with names prefixed `ingest::app::` (`cargo test -p
  trace-commons-ingest --lib`); `--bin trace-commons-ingest` runs none.
  Inside it and its `trace_commons_ingest_internal/` modules, `crate::` is
  the `trace_commons_ingest` library root, so a reference to the service's
  own items is `crate::ingest::app::...` or `super::`, and a reference to
  the server library is `trace_commons_server::...`. Its test module, larger
  still, lives in a sibling file via `#[cfg(test)] #[path =
  "trace_commons_ingest_internal/tests.rs"] mod tests;`. Reuse this pattern if other binaries need the same split; do not
  inline the tests back.
- Pilot-bootstrap loads JSONL session files. Parquet + arrow deps were
  removed; do not reintroduce them. See
  `crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs`.

## Known gotchas

- Plain `cargo check` does not apply `-D warnings`; CI does. Always verify
  with `RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins` and
  `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --no-run` (and
  the same two with `-p trace-commons-ingest` for an ingest change) before
  claiming green. Dead-code and warning regressions slip through plain
  `cargo check`.
- Pilot-bootstrap loader streams JSONL session files; parquet support was
  removed. See `crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs`.
- `trace-commons-ingest.rs` test module is extracted to
  `trace_commons_ingest_internal/tests.rs` via `#[path = ...]`. The file is
  compiled into the `trace-commons-ingest` library (`ingest::app`), so its
  tests run under `-p trace-commons-ingest --lib`; a `--bin
  trace-commons-ingest <filter>` test run, or a `-p trace-commons-server
  --lib ingest::app::` run, matches nothing and passes.
- `cargo test -p <crate>` compiles none of a module behind a non-default
  feature, then reports a green run of the tests it did build. Measured on
  `main`: `-p trace-commons-attestation` runs 35 tests, `--features receipt`
  runs 98 -- the 63 in `receipt.rs` never built. Two more crates have the same
  shape: `trace-commons-gate-enclave` 99 -> 136 under `near-ai-scorer`, and
  `trace-commons-protocol` 251 -> 289 under `near-ai-privacy-filter`. Pass
  `--features`, or use `cargo test --workspace`, which is what CI runs and
  which reaches them through feature unification.
- The session-store parsers (`source/`) and the routing row, ledger trait,
  attested capture and enriched source (`routing/{mod,attested,enriched}`)
  live in `crates/trace-commons-contributor-sources`, a permissive leaf the
  contributor re-exports at `trace_commons_contributor::source`, `::routing`,
  and `daemon::settings::SourceDeclaration`, so callers keep their paths.
  Their 231 unit tests run under `cargo test -p
  trace-commons-contributor-sources`; `-p trace-commons-contributor` no longer
  builds or runs them, and runs only the assertions that need the daemon
  (`source/cross_layer_tests.rs`, `routing/cross_layer_tests.rs`). The split
  means a daemon-only edit does not rebuild the parsers or their tests; no
  build-time speedup has been measured yet. Fix a parser bug in the leaf
  crate, which must never depend on the contributor.
- Private local Insights lives in `trace-commons-contributor-insights`,
  re-exported at `trace_commons_contributor::insights`. Its 495 unit tests
  move with it. It depends on sources, protocol and the shared platform
  crate, never on the daemon; daemon edits no longer invalidate Insights,
  and Insights edits no longer invalidate the source parsers. Platform owns
  OS credential storage and shared local-file primitives, with their tests.
  The test credential-store feature is forwarded to both crates, but never
  enabled in production. It still refuses OS credential access outside the
  platform crate's own tests, and Insights' seeded digest key remains
  `cfg(test)` only. Run the platform package's ignored OS round-trip tests
  on the native OS (CI does), not under the contributor package's old path.
- The shell test projects copy the FFI dylib only `Condition="Exists(...)"`
  (`windows/tests/TraceCommons.Interop.Tests/*.csproj`), so a stale or absent
  artifact fails nothing and the suite tests whatever ABI it last built. CI
  builds `trace-commons-contributor-ffi` before running them; a local run does
  not. The exported-vs-declared assertions do not catch it, because both halves
  move together -- an old dylib compared against a shell that has not added the
  new fields agrees with itself. Rebuild the dylib before trusting a shell
  suite result, green or red.
