# Agent configuration through the CLI

Let a terminal-capable AI agent configure the running Trace Commons app using supported, machine-readable operations, with user documentation and a copyable sample prompt.

## Approach

Use the existing `near-ai` CLI and local daemon API. The user selected CLI first over adding both CLI and MCP. A separate MCP service would add deployment and protocol maintenance; directly editing settings files would bypass validation and race the app. Neither is needed for this version.

## Interface

Add `near-ai config schema`, `near-ai config show`, `near-ai config preview --file PATCH.json`, and `near-ai config apply --file PATCH.json`, supporting the existing global `--json` and `--config-dir` options. JSON patches are partial objects of supported daemon settings, not JSON Patch operations. Schema discovery describes accepted types, enums, bounds, and effects. Show returns the daemon's redacted settings. Preview validates with the daemon's existing parser on a cloned candidate and returns proposed before/after changes without persistence or runtime effects. Apply uses the existing daemon setter, then reports the effective redacted settings. Errors return nonzero status without leaking submitted values.

Validation and writes use the running daemon, so its supported schema is authoritative. Existing `daemon settings --set` remains compatible. No new dependencies are expected.

## Scope and boundaries

Cover settings already accepted by `set_settings`, including notification timing, source declarations, upload limits, private inference, and capture preferences. Preserve existing authentication, consent, and account-binding behavior. Do not expose credentials as configuration fields or edit persisted settings directly. Keep dedicated existing CLI commands for account selection and project upload modes; document them alongside configuration. Native window/layout preferences and a new MCP server are outside this version.

Preview reports a proposal, not a reserved transaction: apply validates again against current state. It updates only supplied fields. Documentation explicitly describes this limitation and verifies the resulting state. Production and staging configuration directories must be selected explicitly in examples to avoid changing the wrong running app.

## Documentation and sample prompt

Add an agent-configuration guide linked from the contributor documentation. Include locating the bundled CLI, channel/config-directory targeting, schema discovery, inspection, preview/apply examples, verification, and common errors. Include a standalone copyable prompt directing an agent to inspect the schema, translate the user's requested preferences into a minimal patch, preview it, apply within the user's authorization, and verify the result. The prompt must not imply that ordinary configuration grants trace-upload or account-login permission.

## Verification

Test schema/accepted-key consistency, typed and structured settings, unknown keys and invalid ranges, no preview persistence or runtime side effects, redacted output, partial-update preservation, and an end-to-end CLI/daemon preview/apply flow using a temporary configuration directory. Run focused Rust tests, formatting, scoped Clippy, and the license-boundary check. Do not modify the user's installed app settings during development.

## Approved follow-up: trace and folder discovery

The user also requested discovery of estimated trace value and folder ignore/automatic policies. Reuse the daemon's trace estimate data and project listing/policy commands, making the CLI output and documentation discoverable. Estimates must retain their units, uncertainty and provenance and must not be presented as guaranteed payouts. Inspecting data must not approve or submit traces. Automatic folder changes must distinguish new sessions from the existing backlog.

Implementation clarification: configuration commands require a reachable daemon and never use the existing CLI's offline fallback. Preview compares validated internal settings to detect canonical/alias and redacted changes, then redacts displayed values. Apply preserves runtime result fields; persisted preferences are not proof of a running proxy.
