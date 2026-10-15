# Managed OpenCode implementation plan

Goal: launch OpenCode from native macOS or near-ai with a selected saved key and model; optionally collect that exact session automatically on exit.

Design authorized in conversation: extend existing managed accounts, ticket redemption, process supervision and export importer. No global provider changes or attestation claims. The follow-up automatic discovery and direct rusqlite dependency were explicitly authorized in conversation. Use OpenCode 1.19.0, qualified with an isolated synthetic localhost model and external networking denied. Subscription login is unavailable for OpenCode until independently qualified.

## Tasks
- [x] Qualify real export and add failing version/account tests; accept 1.19.0 alongside 1.18.29 and preserve actual producer version.
- [x] Add optional model and collect_traces launch fields (old requests default to no collection). OpenCode requires a nonempty model and key connection. Persist choices with session identity. CLI exposes --model and --collect-traces.
- [x] Isolate HOME/XDG/config per launch under account profile, remove inherited OpenCode settings and provider credentials, disable project configuration/plugins and auto-update, constrain provider/model. Import an empty owned session and launch --session with its exact ID. Export only that ID on exit, validate before atomically staging, report failure visibly and preserve profile for manual recovery.
- [x] Add managed export staging alongside the explicitly selected ordinary export folder under one OpenCode source; keep project policy, approval, redaction and admission unchanged. Do not collect other sessions created in the TUI.
- [x] Add native macOS tool, model field and collection toggle with shared copy; constrain connection options. Preserve existing shell compatibility and document CLI use on other platforms.
- [x] Run managed/source/daemon regressions, contributor check/clippy, license boundary, Swift checks; qualify the actual adapter against installed OpenCode with synthetic credentials and localhost server. Review complete diff.

## Review focus
- Project or environment settings must not redirect selected credentials.
- Export ID mismatch, malformed/unsupported output, timeout or symlink must never stage a trace.
- Export failure must preserve local data and remain distinct from native process exit.
- Existing ordinary watched folder and managed exports must both resolve correctly through by-name lookups.
- Old launch requests and saved sessions remain decodable; consent is never inferred from merely launching a tool.

## Evidence ledger
- Synthetic OpenCode 1.19.0 import/run/export succeeded with --pure, isolated HOME/XDG, explicit model, external networking denied; same session ID, two messages, only fixture-model observed.
- Initial regressions test real fixture parsing and saved OpenCode key accounts.

## Final verification
- Managed integration suites: 33 passed; installed-binary qualification is normally ignored and was explicitly run separately (1 passed).
- Source adapter library selection: 189 passed, excluding unrelated compute resource tests matched by the substring.
- Managed library tests: 7 passed, including explicitly selecting the second NEAR model with distinct per-session credentials.
- Logout cleanup: 6 passed; shared source copy: 10 passed; OpenCode settings regression: 1 passed.
- License boundary: 4 passed. The original managed-launch change added no dependencies. See the automatic-discovery follow-up below.
- Contributor all-target Clippy passed with RUSTFLAGS=-D warnings and the repo allow-list; cargo fmt check and git diff check passed.
- Rebuilt contributor FFI and ran native macOS ManagedSessions, SourceCheckExport, SessionRoots and SettingsParity tests successfully. OpenCode launch dialog rendered and visually inspected.
- Fresh read-only reviewer identified two issues: explicit Off bypass and indefinitely pending collection after unredeemed launch failure. Both reproduced in tests and fixed; regressions pass.
- Ruling: collection opt-in defaults off and applies to the seeded session only. Export happens on exit. Subscription auth,  forced-helper recovery, and attested contribution qualification remain outside this implementation.
- Ruling: managed staging discovery is bounded at 65,536 entries; the ordinary declared folder retains its 256-entry bound. Explicit Off disables both. Logout removes collected staging without deleting native session profiles.
- No real provider credentials, paid calls, deployment, or release performed. Windows/Linux desktop launch UI remains frozen; CLI shares the implementation, but no Windows/Linux live-native qualification was performed.

## Automatic discovery follow-up

- Offer the native `opencode.db` at `$XDG_DATA_HOME/opencode` or `~/.local/share/opencode` during source discovery. Selection is explicit; old undeclared desktop settings remain off.
- Preserve existing export-folder declarations. Native selections name the database file, so adding a database to an old export folder never widens its consent.
- Read qualified session/message/part tables through read-only SQLite transactions. Retain export validation, version checks, redaction, project policy, preview and upload approval. No credentials or provider configuration are queried.
- Use stable per-session references, session/message/part update times, and DB/WAL preview cache invalidation. Existing polling observes committed changes.
- Bound session counts, payload bytes, record counts and SQLite instruction work. Fail closed on malformed/unsupported input. No database migrations or transcript writes; SQLite may coordinate readers through an existing shared-memory sidecar.
- `rusqlite` 0.37 with bundled SQLite and hooks was approved by the user. It already exists transitively through IronWire; no new package is introduced.
- Qualification fixture `tests/fixtures/opencode/native-1.19.0.sql` comes from the installed OpenCode 1.19.0 importing synthetic `completed.json` in an isolated temporary profile. It exercises assistant parent links, reasoning, tool call/result, ordering and native identity; imported original version 1.18.29 is preserved.

### Follow-up verification

- After integrating main at 5b954bf34: complete contributor suite, 4,557 passed, 14 ignored.
- FFI ABI and header suites: 197 passed. Rebuilt native FFI.
- Native macOS managed sessions, source setup, chooser, source-copy, session-roots and settings suites: 79 passed against a freshly rebuilt, isolated FFI dylib.
- License boundary: 4 passed; all four required dependency license configurations passed.
- Contributor and FFI all-target Clippy passed with `RUSTFLAGS=-D warnings` and repository allowances; formatting and diff checks passed.
- Native SQLite qualification uses OpenCode 1.19.0 on macOS. No Windows/Linux live-native qualification or release was performed.
