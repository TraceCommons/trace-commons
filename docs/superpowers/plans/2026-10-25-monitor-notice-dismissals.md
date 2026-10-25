# Persistent Monitor Safeguard Dismissals Implementation Plan

> **For agentic workers:** Use the executing-plans workflow for the scoped tasks below.

**Goal:** Keep a dismissed safeguard closed through unchanged polls, window recreation and app restart.

**Architecture:** Persist an explicit allowlist of logical safeguard incidents in UserDefaults. Hash configuration directory, channel and account identity into the scope; persist only condition codes and incident digests. Preserve all auth, consent, action, undo and existing daemon acknowledgement behavior.

**Tech Stack:** Swift 6, Observation, CryptoKit, UserDefaults, existing SwiftUI notices.

## Dismissal policy

Persist queue-full, daily-cap-reached, witness-saturated and automatic-contribution-held. Counts, title/detail wording and retry timestamps do not create new incidents. Different automatic-hold reasons and a newly reported daily quota window (`daily_budget.resets_at`, the daemon's next UTC midnight) do. A missing quota-window value or missing/empty hold reasons retain the known dismissal; a dismissal made before these are known binds to their first known value. Retain through unread, failed, masked or reattached status. Clear daily/witness/automatic-hold incidents on an explicit independent resolved signal, which also suppresses a stale copy of that health label. Clear queue-full only on a fully loaded, explicitly healthy status with no active queue banner: pending queue depth excludes Approved entries, so it cannot establish spare capacity. If recovery happens entirely while the app is closed or another health condition masks it, retain the dismissal conservatively. Other notices retain their existing behavior.

## Tasks

- [x] Add behavioral tests for restart/window persistence, isolation by account/config/channel, unchanged observations, unknown/masked reads, authoritative recovery and changed hold reasons.
- [x] Add the small scoped dismissal store and semantic safeguard identities; connect close/filter/reconciliation to MonitorNotices.
- [x] Verify focused lifecycle and native notice tests, including a persistence-removal mutation; run the app suite, inspect the diff, and prepare the reviewed follow-up PR.

## Constraints and review focus

- No private paths, account identifiers, or banner text in persisted keys/values.
- No Rust, dependency, deployment-target or installed-app changes.
- No status failure may look like resolution; no previous account's dismissal may hide a new account's notice.
- No persistent suppression of auth, privacy failures, consent decisions, or undo/action outcomes.

## Verification

- Integrated menu Kept repair uses the same waiting-session population and discovery-time buckets as Traces; unknown pending reads stay unknown.
- 152 focused native tests pass with zero compiler diagnostics.
- Full native suite: 3,107 XCTest cases (9 existing skips) plus 8 Swift Testing cases, zero failures. The known local CredentialRenewal OCR/Contacts test was excluded by CLI; CI remains unchanged.
- Omitting only the preferences write produces two expected assertion failures in the restart test. Exact source restoration was verified by SHA256, then all 11 dismissal tests passed.
- Independent review found no persistent suppression of critical notices or cross-account dismissal leakage. No installed-app actions occurred.
