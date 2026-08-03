# Repository Guidance — trace-commons-server

This file pins repo-specific facts that override the global CLAUDE.md guidance.

## What this repo is

`trace-commons-server` is the **standalone hosted server-side control plane for
Trace Commons / TraceCommons**. It is no longer part of Ironclaw — extracted from
Ironclaw's `gecko-pass` worktree and now owns its own database schema, object
storage, encrypted artifact store, upload-claim issuer, and shared protocol
crate. Ironclaw should depend on `crates/trace-commons-protocol` when the
client-side integration is rewired, but it does not live in this tree.

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
- `docs/operator/README.md` — operator runbook index. Current runbooks:
  A2.6 result handler, pilot-bootstrap first-100-traces, HF cache hygiene,
  GPU cost ledger.

When in doubt about what to build next, read the **"Production Gap Queue"**
section of the roadmap.

## Binaries

- `trace-commons-ingest` — hosted ingest / review / admin / worker API.
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
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test trace_corpus_storage_contract
cargo test -p trace-commons-server --test trace_corpus_pg_store   # requires PostgreSQL
```

CI applies `RUSTFLAGS=-D warnings` to check + test, so plain `cargo check`
will not catch what CI catches. Always use the `RUSTFLAGS` form locally
before claiming green. Clippy is CI-enforced — run it locally too.

The protocol crate is `crates/trace-commons-protocol`; the server crate is
`crates/trace-commons-server`. Migrations live in `migrations/`.

## CI

Eight jobs gate every PR (see `.github/workflows/ci.yml`):

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
- `cargo test (default features)` (with `RUSTFLAGS=-D warnings`).
- `pilot-bootstrap smoke` — `scripts/operator/pilot-bootstrap-smoke.sh`,
  exercising the JSONL loader path. Do not break it.
- `operator-binaries smoke`.

GitHub Actions runners are on Node 24; pinned actions are
`actions/checkout@v6` and `actions/cache@v5`. Future CI edits should hold
those versions unless intentionally upgrading.

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
- `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` is ~61k LOC of
  production code. Its ~60k LOC test module has been extracted to a sibling
  file via `#[cfg(test)] #[path = "trace_commons_ingest_internal/tests.rs"] mod
  tests;`. Reuse this pattern if other binaries need the same split; do not
  inline the tests back.
- Pilot-bootstrap loads JSONL session files. Parquet + arrow deps were
  removed; do not reintroduce them. See
  `crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs`.

## Known gotchas

- Plain `cargo check` does not apply `-D warnings`; CI does. Always verify
  with `RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins` and
  `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --no-run` before
  claiming green. Dead-code and warning regressions slip through plain
  `cargo check`.
- Pilot-bootstrap loader streams JSONL session files; parquet support was
  removed. See `crates/trace-commons-server/src/bin/pilot_bootstrap/hf_dataset.rs`.
- `trace-commons-ingest.rs` test module is extracted to
  `trace_commons_ingest_internal/tests.rs` via `#[path = ...]`.

## Memory

Persistent memory for this project is under
`~/.claude/projects/-Users-zakimanian-code-trace-commons-server/memory/`. Check it
on session start for any project-specific facts not captured here.
