# Contributor Uploader CLI — Design

Date: 2026-07-07
Status: Approved

## Purpose

A general contributor CLI (`trace-commons-contributor`) that lets an individual
developer submit their local coding-agent session transcripts (Claude Code and
Codex) to a trace-commons-server deployment. Devfolio hackathon participants
are the first target audience, but the CLI is instance-agnostic: any vouching
instance that can mint enrollment attestations can onboard its users.

The CLI is the contributor-side counterpart of two server features that
already exist:

- Instance-vouched enrollment (PR #150): per-user tenants derived as
  `tenant-<sha256(instance_id || user_subject)>`, contributor account created
  at enrollment.
- Per-user subjects on upload claims (slice0 branch): one device key can mint
  claims whose principal is namespaced per user
  (`instance:{tenant_id}:{device_key_id}:user:{subject}`).

## Decisions (settled during brainstorming)

| Question | Decision |
|---|---|
| Audience | General contributor CLI; Devfolio is the first vouching instance, not a hardcoded target. |
| Runtime / distribution | Rust crate in this workspace; prebuilt binaries via GitHub Releases + curl installer. No npx wrapper in v1. |
| Sources | Claude Code and Codex, both in v1, behind a `TraceSource` adapter trait. |
| Auth path | Instance-vouched enrollment only in v1 (no invite-code login). |
| Redaction | Deterministic local redaction + server rescrub, plus an opt-in NEAR AI PII pass keyed by the contributor's own NEAR AI API key. |
| Envelope | Map into existing `ironclaw.trace_contribution.v1`; no schema changes, no server changes. The `ironclaw` field name is accepted cosmetic debt. |
| Submit UX | Interactive numbered-list picker (stdin, no TUI dependency) plus flags for scripted use. |
| Structure | New workspace crate `crates/trace-commons-contributor` (lib + bin), reusing `trace-commons-protocol` and `trace-commons-operator-client`. |

## Architecture

```
crates/trace-commons-contributor/
  src/
    lib.rs           # public surface for future wrappers (e.g. npx shim)
    config.rs        # ~/.config/trace-commons/contributor.toml + keystore
    identity.rs      # device keypair, enrollment, claim minting/refresh
    source/
      mod.rs         # TraceSource trait, SessionRef, SessionTranscript
      claude_code.rs # ~/.claude/projects/**/*.jsonl parser
      codex.rs       # ~/.codex/sessions parser
    envelope.rs      # SessionTranscript -> TraceContributionEnvelope
    submit.rs        # redact -> canary-check -> upload -> record receipt
    bin/trace-commons-contributor.rs  # clap subcommands
```

Dependencies: `trace-commons-protocol` (envelope types, deterministic
redaction, subject/tenant derivation, upload-claim request types),
`trace-commons-operator-client` (HTTP transport, host allowlist, error-envelope
mapping), plus workspace-existing clap / ed25519-dalek / serde. No new external
dependencies without explicit approval; the interactive picker is a plain
numbered-list stdin prompt precisely to avoid a TUI crate.

Subcommands: `login`, `list`, `submit`, `status`, `whoami`, `logout`.

## Identity and auth flow

1. Contributor obtains a one-shot enrollment attestation from their instance
   (Devfolio's backend mints it; for dogfooding we mint one with existing
   operator tooling) and runs `trace-commons-contributor login --attestation
   <blob>` (or pastes at a prompt).
2. CLI generates an Ed25519 device keypair, stored under
   `~/.config/trace-commons/` with `0600` permissions. File keystore only in
   v1; OS keychain integration is explicitly out of scope.
3. CLI calls the issuer's enroll endpoint; the server derives the per-user
   tenant and creates the contributor account (merged behavior).
4. `submit` mints short-lived upload claims via `POST /v1/trace-upload-claim`,
   signing with the device key and passing the per-user `subject` (slice0
   path). Claims refresh on expiry using the existing 60-second skew constant.
   Bearer tokens live in memory only and are never written to disk.
5. `whoami` prints the local view (instance id, tenant id, key fingerprint)
   with no network call. `status` queries
   `POST /v1/contributors/me/submission-status` (Slice 1 read-back surface).
6. `logout` deletes the local keystore and receipts.

All CLI logging and error output is hash/label-only, matching the repo
convention: no bearer tokens, no raw URLs with credentials, no trace bodies,
no contributor identity in log strings.

## Source adapters

```rust
trait TraceSource {
    fn name(&self) -> &'static str;               // "claude-code" | "codex"
    fn discover(&self) -> Result<Vec<SessionRef>>; // metadata only
    fn load(&self, r: &SessionRef) -> Result<SessionTranscript>;
}
```

`SessionRef` carries discovery metadata only: path, project, started_at, size,
model(s). `SessionTranscript` is the normalized internal form: an ordered list
of events (user message, assistant message, tool call, tool result, opaque)
plus session metadata (agent name/version, model, working directory,
timestamps).

Parsers are lenient by design: unknown record types are preserved as opaque
events, never errors, because both vendors change transcript formats without
notice. A session that fails to parse is reported and skipped; it never aborts
a batch.

- **claude-code**: walks `~/.claude/projects/**/*.jsonl`, one session per
  file; project directory decoded from Claude's path-encoding of the cwd.
- **codex**: walks `~/.codex/sessions/`; the format is reverse-engineered
  during implementation against real local files, with sanitized fixtures
  checked in.

## Envelope mapping

Target schema: `ironclaw.trace_contribution.v1`, unchanged.

- `channel: Cli`.
- `model_name` from the transcript.
- `feature_flags`: `agent=claude-code|codex`, `agent_version=<x>`.
- Events map onto `TraceContributionEvent`.
- `consent` filled from scopes confirmed at login and echoed at submit. At
  login the contributor picks allowed-use scopes from the existing
  `ConsentScope` set (default preselection: `debugging_evaluation`,
  `benchmark_only`, `ranking_training`, `model_training`; `public_attribution`
  opt-in separately). The choice is stored in config and applied to every
  envelope.
- `replay` marked non-replayable (transcripts, not replay fixtures).
- `value` / cards left at defaults for server-side scoring.
- Working-directory paths reduced to project-name basename plus a path hash;
  full local paths never leave the machine.

## Redaction pipeline

Per session: parse → normalize → deterministic redaction
(`redact_sensitive_json` over every event payload) → optional NEAR AI PII pass
→ canary check (`canary_leaked_tokens`; any hit hard-fails that session's
upload) → envelope assembly → upload.

The privacy metadata block records the deterministic pipeline version string,
plus the `privacy-filter-near-ai-v1` suffix when the NEAR AI pass ran, so the
server knows exactly what ran client-side. Server-side rescrub layers on top.

**NEAR AI PII pass (opt-in).** Enabled with `--pii-filter near-ai` or
`pii_filter = "near-ai"` in config; keyed by the contributor's own
`NEAR_AI_API_KEY` (env) or `near_ai_api_key` (config). It runs after
deterministic secret redaction, so secrets are already stripped before any
content reaches NEAR AI (TEE-hosted). If the flag is set but the key is
missing or the endpoint fails, the CLI fails closed for that session — it
refuses to upload rather than silently downgrading to deterministic-only.
First use prints a one-time notice that redacted-but-unscrubbed prose will be
sent to NEAR AI under the contributor's key.

**Deliberate exclusions:** no local prose-level PII scrubbing beyond the
optional NEAR AI pass. The contract is: secrets deterministically removed
locally; PII handled by the NEAR AI pass (if enabled) and server rescrub. The
login consent text states this.

## Submit flow and UX

`submit` with no args: discovers sessions across both adapters, prints a
numbered list (agent, project, age, event count, model), contributor selects
by number/range on stdin.

Flags for scripted use: `--all`, `--since <duration>`, `--project <path>`,
`--source claude-code|codex`, `--yes`, `--dry-run`, `--pii-filter <backend>`.
`--dry-run` runs the full pipeline including redaction and prints what would
upload; nothing leaves the machine.

Per-session outcome line: submitted (submission id), skipped (parse failure),
refused (canary hit or fail-closed redaction).

**Receipts.** `~/.config/trace-commons/receipts.jsonl`, hash-only rows
(submission id, trace hash, timestamp). Enables: `status` mapping server
responses back to local sessions, idempotent re-submission (already-submitted
sessions marked in the picker and skipped by `--all`).

## Error handling

Sessions are independent; one failure never aborts a batch.

| Category | Behavior |
|---|---|
| Parse failure | Skip session, count and report. |
| Redaction / canary failure | Refuse that session loudly; never upload. |
| Auth failure | Refresh claim once, then instruct re-login. |
| Network failure | Retry with backoff per upload, then mark failed; re-running resumes via receipts. |

Exit code: nonzero if any session was refused or failed; zero if all selected
sessions submitted or were benignly skipped.

## Testing

- Adapter unit tests against checked-in sanitized fixture transcripts for both
  agents, including malformed and unknown-record cases.
- Envelope mapping tests asserting schema validity via the protocol crate's
  validation, plus assertions that no absolute local paths appear anywhere in
  serialized output.
- Redaction tests: seeded secrets in fixtures must be absent post-pipeline;
  canary test asserts hard-fail. NEAR AI pass tested against a mock HTTP
  server, including the fail-closed path.
- One end-to-end test wiring the CLI submit path against the in-process
  issuer + ingest router (same pattern as the slice0 tests): enroll →
  claim-with-subject → upload → status read-back.
- CI: the standard four jobs stay green. Verify locally with
  `RUSTFLAGS="-D warnings" cargo check/test` and the clippy allow-list before
  claiming green.

## Out of scope for v1

- npx / npm wrapper (lib/bin split keeps the door open).
- Invite-code login path.
- Watch mode / auto-submit daemon.
- OS keychain keystore.
- Computer-use or non-coding-agent trace sources.
- Any server-side or schema changes.
