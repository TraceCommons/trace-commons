# Ironclaw Source Snapshot

This repository was initially extracted from the Ironclaw `gecko-pass` worktree.

- Ironclaw source commit: `49dcf88e4ae300290cd29d3a8856264d62085839`
- Source branch: `gecko-pass`
- Extracted server binaries:
  - `src/bin/trace_commons_ingest.rs` -> `crates/tracedao-server/src/bin/tracedao-ingest.rs`
  - `src/bin/trace_commons_upload_claim_issuer.rs` -> `crates/tracedao-server/src/bin/tracedao-upload-claim-issuer.rs`
- Extracted database schema:
  - `migrations/V26__trace_commons_schema.sql` -> `migrations/V1__trace_commons_schema.sql`

The Ironclaw worktree had uncommitted Trace Commons consolidation and hardening
changes at extraction time. Treat this repo as the new server-side home and
port future server changes here instead of extending the Ironclaw binary path.

