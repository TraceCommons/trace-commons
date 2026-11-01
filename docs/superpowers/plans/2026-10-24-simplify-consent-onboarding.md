# Simplify consent onboarding implementation plan

**Goal:** Implement the requested separation of data uses, sharing and public attribution, and include existing eligible watched traces when native onboarding explicitly grants Automatic.

**Architecture:** Keep scope keys and independent choices; shared Rust tables own labels and the exact reward sentence. Native onboarding uses a distinct RPC with an explicit scope acknowledgement, so older daemons cannot silently accept a narrower grant. Old callers and persisted grants keep snapshot protection.

## Tasks

- [x] Verify failing consent presentation tests: three visible optional data uses, required statement once, no attribution row in onboarding.
- [x] Shorten shared scope titles; add the approved reward line to shared copy and native onboarding/Settings; keep descriptions available beside individual permissions.
- [x] Move the existing attribution-scope control into Public profile settings without changing roster/public-profile consent or auto-enabling attribution.
- [x] Add explicit existing-trace grant tests and implementation. Apply to existing and future watched roots, replacing Ask with Automatic while preserving Ignore and dismissed/manual-held entries, and existing upload gates. Selection sends nothing until setup is confirmed.
- [x] Rebuild FFI and run contributor regressions, native Swift tests, wording guard, formatting and independent review. Prepare a follow-up PR stacked on #1610 with the final scope and evidence.

## Result and validation

- Automatic includes existing/new eligible traces in all watched sources, except ignored folders. Fresh confirmation supersedes earlier global modes and old folder history limits; legacy grants are not expanded on load.
- Manual approvals retain priority. Automatic backlog is ordered by original trace age, with durable modification timestamps as fallback; full discovery admits older source-age history first. Existing full queues are preserved and older undiscovered history enters as capacity frees.
- Ask is saved before scope changes and withdraws the standing grant. Setup cannot finish as Ask after a failed Automatic grant unless that stopping choice is confirmed.
- Signed-in onboarding reuses the explicit reward-recipient picker. Arrival monitoring follows the freshly verified payout recipient without needing a Private AI key; recipient changes invalidate in-flight and prepared notifications.
- Eligibility still requires 24 hours without modifications; a pass sends at most 20 uploads. Server acceptance, scoring and payout determine actual awards. Incoming-transfer notifications remain generic NEAR AI credit notifications, not proof of trace rewards.
- Full Swift suite: 1,570 app tests (9 skipped), 866 shell-core, 262 design, 277 bridge, 27 updates and 8 Swift Testing tests passed. The final Ignore-affordance change also passed targeted tree/rail/decoder/wording checks.
- Focused backend: 14 onboarding/ordering/rollback checks, dispatcher contract, 30 arrivals checks, 2 notification checks and 1 recipient-verification integration check passed. Contributor/FFI Clippy, formatting and the four license-boundary tests passed.
- Whole contributor lib run: 3,897 passed, 5 failed, 6 ignored. Dispatcher contract was repaired and rechecked; home-directory and Terminal probes passed outside the sandbox. Two unchanged managed OpenCode tests require 1.19.0; this host has 1.19.3: `managed_sign_in_redeem_hosts_the_proxy_and_ends_it` and `opencode_starts_on_the_sign_in_without_a_model`.
- Independent review found two credit-poll issues; both were repaired and re-reviewed without remaining findings. Native layouts were rendered at 860 × 760 and visually inspected. No live user traces were submitted.
