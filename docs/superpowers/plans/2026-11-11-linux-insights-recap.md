# Linux weekly Insights recap

## Design

Use the shared local Comparisons operation with the current live daemon history unchanged. The core alone determines the last closed comparable week and opened-card state. Standalone/saved feeds produce no comparisons or card. Missing, malformed or partial closed history never becomes fabricated zero/sample data.

The card uses existing shared copy and native semantics: separate source token figures, optional comparable-week changes, best-cache item, optional user threshold. Native no longer shows PatternUp items; GTK follows that policy.

View week performs explicit local RecapOpened persistence, then selects the returned closed week in the live overview. Hide performs explicit set_settings(insights_recap_card_enabled:false); success removes/reloads the card, failure preserves it and uses the existing setting failure toast. No notification, new wording, off-device action, or provider refresh.

## Plan

1. Add insights_recap.rs as child of analytics, with pure live/history request gate and comparison response identity checks plus presentation strings. Regression tests cover missing history, wrong feed, malformed history, unknown/partial/current-only comparisons, changed week and stale/closed view.
2. Append a bounded asynchronous recap child only to accepted live overview; retain weak view and body handles and overview generation token. Existing reload/close invalidates completion. Actions have an explicit busy guard, no side effects on presentation.
3. Validate GTK suite, formatting and baseline clippy once after tasks. Source review and retain Linux runtime/visual evidence as a separate gate. No Benchmarks changes.

## Validation

- GTK full suite: 614 library, 2 shell and 8 integration tests passed (624 total); 8 library tests explicitly ignored.
- All-target GTK clippy passed with the repository's five documented baseline allowances. Formatting and diff whitespace checks passed.
- No manifests, dependencies, Benchmarks registration, provider probes, or notification paths changed. The parent owns the aggregate license-boundary gate.
- Linux runtime/screenshot validation remains unverified. This slice does not claim full Insights parity.
- Independent source review found no blocker in feed/history validation, response identity, weak generation guards, same-store open persistence, explicit Hide, or single-flight actions. Review did not independently rerun tests.
