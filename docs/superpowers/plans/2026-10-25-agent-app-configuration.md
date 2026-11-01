# Agent App Configuration Implementation Plan

> **For agentic workers:** Use the approved design and implement the scoped tasks below; independent daemon and CLI work can proceed in parallel, with final integration review.

**Goal:** Let agents discover, inspect, preview, and apply app settings, and discover trace estimates and folder upload policies through the CLI.

**Architecture:** The running daemon owns validation, schema discovery and redaction. CLI configuration commands call it directly without offline fallback. Existing trace/project APIs supply facts and policy changes.

**Tech Stack:** Rust, clap, serde_json, existing local daemon IPC. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-25-agent-app-configuration-design.md` (approved).

## Global Constraints

- Preserve existing consent, credential, and upload-admission boundaries.
- Do not modify the user's installed app settings.
- Always use `RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0` for Cargo builds/tests.
- No credentials in schema, preview, or error output; read-only settings remain read-only.
- Preserve existing CLI behavior. Document staging/production targeting and unsupported older daemons.

## Review Focus

- No running daemon: fail explicitly instead of reconstructing local state.
- Aliases and partial nested settings: preview shows canonical changes, including redacted changes.
- Invalid/duplicate/oversized JSON patches: refuse before any write.
- Saved preferences versus running service state: report effective/runtime outcome without claiming service success.
- Trace estimates: preserve unknowns, units and estimate labels; never imply guaranteed payout or trigger uploads from inspection.

## Tasks

### 1. Daemon configuration discovery and preview

Files: new `daemon/agent_config.rs`, `daemon/mod.rs`, `daemon/ipc.rs`, IPC reference documentation.

Interfaces: `get_config_schema` accepts `{}` and returns versioned metadata plus a JSON Schema for the accepted settings patch. `preview_settings` accepts the same partial object as `set_settings`, validates a clone with `apply_settings_object`, and returns `{schema_version, changes:[{key,before,after,redacted}]}`. Preview never persists or reconciles runtime state. Existing `set_settings` remains the apply operation.

- [x] Add regression tests for accepted-key/schema consistency, invalid and structured fields, no side effects, redaction and aliases.
- [x] Implement schema metadata from existing ranges/parsers and cloned-candidate preview.
- [x] Wire both methods into IPC dispatch/capability lists; document them.
- [x] Run focused daemon tests.

### 2. Agent CLI commands

Files: new `cli/config.rs`, `cli.rs`, CLI tests.

Interfaces: `near-ai config schema|show|preview --file PATH|apply --file PATH`, existing global `--json` and `--config-dir`. Use `daemon::client::try_call` directly. Schema maps to `get_config_schema`, show to `get_settings`, preview to `preview_settings`, apply to `set_settings`; retain the async setter's runtime result.

- [x] Test CLI parsing and nonzero structured errors, bounded/duplicate-safe patch decoding, and unavailable daemon behavior.
- [x] Implement commands without generic arbitrary-IPC access or offline writes.
- [x] Add a temporary-store CLI/daemon integration proving preview no-write, apply persistence and unrelated-setting preservation.
- [x] Verify existing pending/project CLI covers new discovery requirements; fill any specific gaps without adding alternative policy logic.

### 3. Guide, sample prompt and integration

Files: `docs/agent-configuration.md`, `docs/examples/agent-configuration-prompt.md`, README links, approved spec addendum.

- [x] Document CLI discovery, app channel targeting, settings schema, inspect/preview/apply, trace estimated value and folder modes.
- [x] Explain unknown estimates and new-only versus existing backlog automatic policy choices.
- [x] Provide a copyable prompt that inspects before acting and verifies authorized changes.
- [x] Run focused tests, formatting, scoped Clippy and license-boundary check.
- [x] Review actual CLI examples and all changes.
- [ ] Commit and open a PR.
