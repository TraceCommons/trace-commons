# Persistent Monitor Safeguard Dismissals Implementation Plan

> **For agentic workers:** Use the executing-plans workflow for the scoped tasks below.

**Goal:** Keep a dismissed safeguard closed through unchanged polls, window recreation and app restart.

**Architecture:** Persist an explicit allowlist of logical safeguard incidents in UserDefaults. Hash configuration directory, channel and account identity into the scope; persist only condition codes and incident digests. Preserve all auth, consent, action, undo and existing daemon acknowledgement behavior.

**Tech Stack:** Swift 6, Observation, CryptoKit, UserDefaults, existing SwiftUI notices.

## Approved policy

Persist queue-full, daily-cap-reached, witness-saturated and automatic-contribution-held. Counts, title/detail wording and retry timestamps do not create new incidents. Different automatic-hold reasons and a newly reported daily quota window (`daily_budget.resets_at`, the daemon's next UTC midnight) do. A missing quota-window value retains the known dismissal. Retain through unread, failed, masked or reattached status. Clear daily/witness/automatic-hold incidents on an explicit independent resolved signal, which also suppresses a stale copy of that health label. Clear queue-full only on a fully loaded, explicitly healthy status with no active queue banner: pending queue depth excludes Approved entries, so it cannot establish spare capacity. If recovery happens entirely while the app is closed or another health condition masks it, retain the dismissal conservatively. Other notices retain their existing behavior.

## Tasks

- [ ] Add behavioral tests for restart/window persistence, isolation by account/config/channel, unchanged observations, unknown/masked reads, authoritative recovery and changed hold reasons.
- [ ] Add the small scoped dismissal store and semantic safeguard identities; connect close/filter/reconciliation to MonitorNotices.
- [ ] Verify focused lifecycle and native notice tests, including a persistence-removal mutation; run the app suite, inspect the diff, and prepare for parent review without pushing or merging.

## Constraints and review focus

- No private paths, account identifiers, or banner text in persisted keys/values.
- No Rust, dependency, deployment-target or installed-app changes.
- No status failure may look like resolution; no previous account's dismissal may hide a new account's notice.
- No persistent suppression of auth, privacy failures, consent decisions, or undo/action outcomes.
