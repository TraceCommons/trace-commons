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

Initial complete contributor/Insights/platform unit run: 4,840 passed, 12 ignored. Final backend follow-up: 42 passed. Final unenroll module: 24 passed; configuration: 8 passed; Claude gate: 1 passed; copy contracts: 2 passed. FFI ABI: 195 passed; header: 2 passed; shared native copy: 7 passed; license boundary: 4 passed. All-target client Clippy with standing allowances, formatting and diff checks pass.

Native focused tests: 24 passed including 408pt Settings/Overview renders. Full Swift run covered 3,265 XCTest cases and 8 Swift Testing cases; the sole failure was an obsolete source-wiring assertion. After updating that assertion to include the shared model, 32 affected cases completed with 31 passed and 1 opt-in skip. Swift Release app compile passed against the fresh debug Rust FFI; this is not signed-release validation.

The broad Rust unit run precedes the final identity/witness and unenroll edge fixes; their final targeted suites cover those changes. Original OpenCode CI test passed 100 repetitions before and after the repair; the new inherited-descriptor case independently failed before and passed after explicit unlock. No live credentials, installed-app settings, dependency changes or release operations were used. Update the existing PR; no merge is authorized.
