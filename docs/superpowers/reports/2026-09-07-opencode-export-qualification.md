# OpenCode export and routing qualification

The export adapter is eligible for review independently of routing catalog
admission. **Generic OpenCode provider-block catalog enablement remains refused:**
adding a provider changes the default model in a fresh profile. This checkpoint
makes no live NEAR AI, receipt, witness, native UI, or release claim.

## Version and official contract

Tested executable: OpenCode **1.18.29**, existing Homebrew macOS ARM64 binary;
SHA-256 `2f24593f1b8e578d0b7ed7ca399440d4b6c125330eece20a69ad8d380190d669`.
Official release tag revision:
`16747470f976aca3d362ad730bcd3fe82ecc2c9a`.

- [Supported export CLI](https://opencode.ai/docs/cli/#export):
  `opencode export SESSION_ID > session.json`.
- [Pinned export producer](https://github.com/anomalyco/opencode/blob/16747470f976aca3d362ad730bcd3fe82ecc2c9a/packages/opencode/src/cli/cmd/export.ts)
  emits `{ "info": ..., "messages": [{ "info": ..., "parts": [...] }] }`.
- [Pinned session/message/part schema](https://github.com/anomalyco/opencode/blob/16747470f976aca3d362ad730bcd3fe82ecc2c9a/packages/schema/src/v1/session.ts).
- [Pinned model selection](https://github.com/anomalyco/opencode/blob/16747470f976aca3d362ad730bcd3fe82ecc2c9a/packages/opencode/src/provider/provider.ts),
  `defaultModel`: explicit configuration, then valid recent selection, then
  configured-provider fallback.

The machine also has a distinct `~/.opencode/bin/opencode` 1.18.27 executable.
An initial exploratory run used that binary; it is **not** the basis of the
1.18.29 qualification below. Use an explicit executable path, not PATH order.
No binary was installed or upgraded for this work.

## Reproduce the model-selection counterexample

On macOS, using an already installed 1.18.29 binary:

```sh
python3 scripts/qualification/opencode-routing.py --opencode /opt/homebrew/bin/opencode
```

The harness creates synthetic, separate HOME/XDG profiles, denies external
network with `sandbox-exec`, disables updates/models fetching and external
plugins (`--pure`), and uses a loopback mock with a synthetic key. It stops after
observing primary model selection, without waiting for a completed inference.
It neither edits real configuration nor obtains real credentials. The
fresh-before case observes the model selection in diagnostics before the
external provider request can succeed; all provider traffic that is answered
comes from the local mock. This is not a live NEAR AI qualification.

| Profile | Primary model | Configuration bytes unchanged |
| --- | --- | --- |
| Fresh, before provider block | `opencode/big-pickle` | yes |
| Fresh, after provider block | `trace-test/trace-model` | yes |
| Empty-string model, before provider block | `opencode/big-pickle` | yes |
| Empty-string model, after provider block | `trace-test/trace-model` | yes |
| Explicit model, before provider block | `existing/existing-model` | yes |
| Explicit model, after provider block | `existing/existing-model` | yes |
| Valid recent model, after provider block | `existing/existing-model` | yes |

These are generated synthetic configurations, **not** a test of IronWire's
preview/commit writer or conflict handling. Existing digest-bound writes,
unreadable-file refusals and full-slot preservation must remain intact. The
fresh-profile counterexample is sufficient to reject broad catalog admission.
The empty-string counterexample also means the existing presence-only
`AgentSetting.requires = "model"` cannot establish safe explicit selection.
A future explicitly selected-model routing flow or a proven restrictive
precondition needs its own upstream implementation and qualification before
catalog enablement. This PR does not change the IronWire catalog or pin.

## Export adapter boundary

`source::opencode::OpenCodeSource` consumes direct `.json` children of an
**explicitly declared export directory**. The source ID is `opencode`.
`SourceRoots::new()` and `SourceRoots::conventional()` do not construct it.
`Off` constructs nothing; a caller must supply a `Watch` declaration to opt in.
There is no native database scan, application config mutation or implicit import
of historical sessions. The backend settings key is `opencode_source`, accepting the existing source
object form (`{"mode":"watch","path":"/chosen/exports"}` or `{"mode":"off"}`).
Absent/null constructs nothing, including old settings files. Native picker
and shared-copy wiring are a separate coordinated workstream; this checkpoint
is not that UI. IPC settings expose only `opencode_source_mode`
(`unset`, `off`, or `watch`), never the selected path; registered source fields
share the same redaction rule.

The reader caps input at 16 MiB before JSON parsing and limits messages/parts to
100,000. Discovery refuses directories exceeding 256 entries, counting non-JSON
entries too. A bounded 64 KiB leading header supplies the original session time
and directory for discovery and `--since`; export modification time is never
substituted for session time. Missing or later headers leave these fields absent.
It hashes the exact imported bytes and uses the export's session ID,
message IDs, part IDs and tool call IDs. It rejects duplicate/cross-session IDs,
missing parents, backward message timestamps, invalid roles/part types/tool
states and malformed/truncated JSON. It preserves supplied order for equal
timestamps. Its tested version allowlist currently accepts only
`info.version == "1.18.29"`; this conservative check can refuse an older-created
session even if exported by a newer compatible executable. Expand the allowlist
only with fixtures and schema review, not a best-effort parse. Unsupported
versions raise the standing `opencode-export-version-unsupported` health label;
a complete successful discovery pass clears it after repair.

Text/reasoning content and tool inputs/results enter the ordinary redaction
pipeline. Completed/error tools preserve true/false; pending/running tools do
not manufacture a result. File URLs, attachment bytes, snapshots and harness
metadata are not opened or copied; recognized non-text part types become empty
opaque events. Synthetic or ignored text is withheld as an empty opaque event.
Message/part IDs are validated locally, while session and tool-call IDs retain
their typed roles. Text events do not carry import-ID objects that would falsely
declare tool-payload consent. Tool arguments contain the actual input object;
tool results retain import identity and supplied model metadata. Arbitrary
readable structured payloads still require consent on both client and server.
Mixed models leave the transcript-wide model absent; per-text model metadata is
not exported. Usage/pricing is left absent because the export does not establish
the complete cache-duration accounting contract.

Discovery/load checks path containment and rejects final symlinks. Shipped Unix
opens use nonblocking/no-follow flags, then validate the same opened inode;
Windows opens the reparse point and rejects non-regular/reparse handles. This
does not claim resistance to adversarial replacement of ancestor directories,
Windows DACL validation, or immutable files. The size bound applies to the bytes
actually read even if an export grows after discovery.

## Admission and evidence

A valid invite permits this local OpenCode trace through ordinary contribution
validation, redaction and approval **without NEAR AI receipts**. The adapter
contains no receipt prerequisite or invitation assertion.

Without an invite, the separate admission path must verify NEAR AI receipts,
exact-byte binding, witness redaction and contributor approval for that
contribution. OpenCode's normalized export and `--sanitize` output are not
verbatim request/response carriers, receipt verification, or witness
certification. The adapter assigns no `attested_call` and no routing records.
Existing optional routing enrichment still requires explicit session-ID matches;
time proximity alone is not a join. No request/session-header propagation was
qualified here, so automatic OpenCode-to-receipt correlation remains unproven.
Use the independent evidence-intake contract or refuse an ambiguous join.

## Fixtures and validation

`tests/fixtures/opencode/completed.json` is hand-authored synthetic data against
the pinned official schema, not an independently generated or signed provider
vector. It has no credentials, real user identifiers or provider receipt.
Adapter regressions mutate it for malformed/version/type/ID/order failures and
pending/error outcomes; a declared-directory test exercises actual discovery and
load, and a symlink test exercises final-link refusal. These tests do not prove
end-to-end witness admission or production routing.

Local checks use `RUSTFLAGS='-D warnings'` and a private target with two build
jobs. The source-only regression selection excludes `compute::resource::`:
Cargo's substring filter `source::` also matches that unrelated module. The
initial broad selection ran its signed-worker lifecycle fixture and hit an
`Error`/deadline failure; no compute code or assertion was changed or suppressed
in CI. Source coverage and that incidental failure are separate evidence.
