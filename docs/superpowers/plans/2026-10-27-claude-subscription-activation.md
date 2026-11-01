# Claude subscription activation plan

Owner request and implementation boundaries: [design](../specs/2026-10-27-claude-subscription-activation-design.md).

- [x] Create a clean isolated worktree from main using the finished-worktree helper.
- [x] Implement exact native credential selection and explicit-only Keychain reader with wiping buffers and native fixture proof.
- [x] Restore fixed OAuth profile/usage acquisition, default Claude availability and single-login Refresh scope.
- [x] Extend native metadata/wire/model/UI without a Claude opt-in; retain Codex settings and capture context.
- [x] Complete independent security corrections for organization identity, credential-change invalidation, managed-registry cancellation and native environment isolation.
- [x] Verify unenroll cache cancellation, current Rust/FFI/Swift checks and narrow native renders.
- [x] Finish independent review and prepare the branch for a PR. No merge or release.

Native OS calls may outlive their caller; retained leases and dropped receivers are required even after timeout or cancellation. Keychain revisions may have one-second precision, so Claude readings remain explicitly point-in-time. No real provider credential read is part of development verification.

## Verification results

- Rust contributor, Insights and platform units: 4,910 passed, 13 ignored.
- Sample recorder: 5 passed, 1 ignored; recorded samples match.
- FFI ABI, header and shared-copy checks: 205 passed.
- License boundary: 4 passed. Formatting, diff checks and affected-crate all-target Clippy passed with the repository allowances. No dependencies changed.
- Fresh production FFI build and warnings-as-errors native suite: 3,296 XCTest cases, 10 skips, zero failures, plus 8 Swift Testing cases passed. Focused usage/wire tests: 33 passed.
- Synthetic disposable Keychain fixture passed without reading user credentials or changing the user search list. Narrow 408-point native render inspected without clipping.
- Independent final review approved after four demonstrated regressions were repaired. Live user-token Anthropic requests were not exercised.
