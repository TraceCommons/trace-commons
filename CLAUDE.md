# Repository Guidance — tracedao-server

This file pins repo-specific facts that override the global CLAUDE.md guidance.

## What this repo is

`tracedao-server` is the **standalone hosted server-side control plane for
Trace Commons / TraceDAO**. It is no longer part of Ironclaw — extracted from
Ironclaw's `gecko-pass` worktree and now owns its own database schema, object
storage, encrypted artifact store, upload-claim issuer, and shared protocol
crate. Ironclaw should depend on `crates/tracedao-protocol` when the
client-side integration is rewired, but it does not live in this tree.

There is **no Ironclaw path dependency**. Do not look for one. Do not propose
adding one.

## Database backend

This repo is **PostgreSQL-only**. There is no libsql build configuration.

The global CLAUDE.md guidance to "verify ALL relevant feature flags compile
cleanly (e.g., both postgres and libsql builds)" does **not** apply here. A
single `cargo check -p tracedao-server` is sufficient. Do not add libsql
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

When in doubt about what to build next, read the **"Production Gap Queue"**
section of the roadmap.

## Binaries

- `tracedao-ingest` — hosted ingest / review / admin / worker API.
- `tracedao-upload-claim-issuer` — EdDSA/Ed25519 upload-claim issuer.

## Local development

```bash
cargo check -p tracedao-server --bins
cargo test -p tracedao-server --test trace_corpus_storage_contract
cargo test -p tracedao-server --test trace_corpus_pg_store   # requires PostgreSQL
```

The protocol crate is `crates/tracedao-protocol`; the server crate is
`crates/tracedao-server`. Migrations live in `migrations/`.

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

## Working with this codebase

- Files have grown large (e.g. `bin/tracedao-ingest.rs` is huge). Do not
  unilaterally split files unless a spec asks for it. Add new modules beside
  existing code when possible.
- Worker routes follow a consistent auth pattern — find an existing handler
  and copy its shape rather than inventing a new one.
- Worker-route credentials are scoped: utility, review, retention, vector,
  benchmark, process-evaluation, revocation, export, and revocation-propagation
  each have their own bearer-token gate. Do not mix them.
- Drills (`/v1/admin/*-drill`) produce hash-only evidence and feed rollout-smoke
  required checks. When you add a drill, wire it into the smoke evidence path.

## Memory

Persistent memory for this project is under
`~/.claude/projects/-Users-zakimanian-code-tracedao-server/memory/`. Check it
on session start for any project-specific facts not captured here.
