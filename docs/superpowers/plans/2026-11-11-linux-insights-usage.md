# Linux Insights usage cards

Authorized parity slice, following native InsightsUsageCards.swift. Add a pure presentation model plus GTK renderer in insights_usage.rs; reuse shared copy and card styling. Decode only supported Codex/Claude providers. Keep missing measurements unknown, omit absent measured windows, reject expired windows, preserve known partial NEAR AI money, and show NEAR AI only after calls in the last 31 days. Daily bars retain unknown versus measured zero and accessible dates/amounts.

Read subscription_usage with refresh:false and interactive:false through the existing App worker after insights_week. Keep the overview busy until both complete, coalesce changed selections and check generation/visibility before rendering. Standalone remains account-free and never creates a backend.

1. Write model regression tests for unknown/expired/provider windows, partial daily money and NEAR AI visibility.
2. Implement presentation and GTK cards, then wire the passive read into the guarded live request.
3. Run formatting, GTK unit tests and clippy once at the end (repository multi-task rule). Inspect the diff and record limitations.

Remaining parity: explicit per-login refresh, Projects drill, referral earnings, recap, Benchmarks, and full live visual evidence. No disabled buttons or new notices are introduced in this slice.

## Verification

- GTK full suite: 618 library, 2 shell and 8 integration tests passed. After the final default-login label adjustment, all 6 usage regressions passed again.
- GTK all-target clippy passed with the repository's five documented baseline allowances; the initial strict run reported only pre-existing baseline categories outside the changed files.
- Formatting and diff whitespace checks passed. Independent review caught a weekly-pace precedence defect; the corrected selection and regression were re-reviewed.
- Parent owns the aggregate license-boundary gate. No manifests or dependencies changed.
- No Linux runtime, screenshot, or full macOS visual parity claim. Cards currently use accessible horizontal daily bars; explicit provider refresh, Projects navigation, earnings, recap and Benchmarks remain separate slices.
