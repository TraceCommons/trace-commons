# Insights subscription limits review revision

## Scope

Address PR #1666's consent, acquisition, compatibility and native UI review. Current usage is separate from historical analytics. Codex uses only the native client and file-backed default/saved managed subscription profiles. Missing measurements remain unknown. No routing, reset redemption, subscription history, browser cookies or new dependencies.

Claude acquisition is gated off pending review: remove its direct OAuth and Keychain reader and hide the switch. Keyring and automatic Codex storage are also refused before credentials/native launch because background account invalidation cannot safely be established for them. This revision has no native Keychain prompt path.

## Decisions

- Permission lives in Settings → Tools. Enabling only saves permission; no offer appears in Insights while off.
- Only explicit Refresh reads credentials or launches Codex. Startup, opening views, status reads and ticks only synchronize non-secret metadata and expire values.
- Refuse detected/ambiguous native Codex processes, serialize owned probes, and terminate only owned descendants. External process starts cannot be atomically locked with native launch.
- Retry modern rate-limit params without params only for a recognized older-schema rejection. Do not hide other failures with retries.
- Keep account-ID and email evidence in separate namespaces. A failure without comparable identity evidence may retain bounded stale values.
- Invalidate cached readings on file/binding/account changes. A same-account native renewal publishes only with a matching final revision witness. Disabled/re-enabled or replaced profiles never revive old claims.
- Unenroll revokes both legacy flags in memory and on disk, invalidates readings, and finishes cleanup before returning a retryable persistence failure. Never overwrite unreadable settings.
- Share one observable model across Settings and Overview. Wake on explicit actions, cancel old observations on reconnect/navigation, and preserve exact optional window durations and distinct windows.
- Repair the independently demonstrated OpenCode duplicated-descriptor lock lifetime. Do not claim its sporadic original CI trigger was reproduced.

## Completed

- [x] Gate/remove Claude acquisition and refuse unsupported Codex storage.
- [x] Make acquisition manual-only; guard native processes and older protocol schemas.
- [x] Repair identity comparison, revision-witness completion, stale retention and duration contract.
- [x] Move informed consent to Settings; hide off-state card; wake polling; show actionable failure states.
- [x] Persist unenroll revocation with partial-failure/retry coverage.
- [x] Add explicit OpenCode lock release and deterministic red/green regression.
- [x] Verify affected Rust/FFI/native checks, inspect narrow renders and complete independent review.

## Verification evidence

Current main `250beb677` was merged into this branch. Manual resolutions retain 103 synchronous / 84 asynchronous IPC handlers and both the shared subscription model and upstream trace inspector.

Final complete contributor/Insights/platform units: 4,888 passed, 12 ignored. Final daemon recording drift guard: 5 passed, 1 ignored. Full merged native suite: 3,287 XCTest cases (10 opt-in skips, 0 failures) plus 8 Swift Testing cases passed with warnings as errors. The combined suites cover the final identity/witness and unenroll fixes and the native initializer resolutions.

On the merged tree, FFI ABI 196, header 2, shared-copy 7 and license-boundary 4 checks passed. Client/FFI all-target Clippy with standing allowances, formatting and diff checks passed. A fresh production FFI was rebuilt for the merged native suite. Swift Release app compilation passed before integration against debug Rust FFI; it was not repeated afterward and is not signing/release evidence.

Both 408pt Settings/Overview fixture renders were inspected. The duplicated-descriptor lock regression failed before and passed after explicit unlock; original OpenCode CI test passed 100 repetitions both before and after, so its precise trigger remains unconfirmed. Independent final source and merge-resolution reviews approved. No live credentials, installed-app settings, dependency changes or release operations were used. Update the existing PR; no merge is authorized.
