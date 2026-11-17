# Linux Insights Benchmarks

Implement the authorized GTK parity slice as a separate Benchmarks destination. Reuse the existing weak App worker binding and read-only `insights_benchmarks` method; standalone creates no backend. Offer last 4/13 weeks and own/spawned populations, using existing shared copy only. Clear old figures when selection changes. Bound work to one pending request plus one coalesced latest selection; generation and active-tab guards reject late results.

Native `InsightsBenchmarksTab.swift` is authoritative: fixed category order, rows from the first trace sorted by sessions then label, missing figures as unavailable, integer USD micros, rounded kept percentages, minimum one minute for durations. Show daemon-provided quality/value/speed leaders and runners-up, never calculate a ranking. Missing picks show the daemon minimum-session comparison copy. Show per-model work mix using compared category traces, excluding left-out totals. No sample figures, new dependencies, background refresh, account reads or new notices.

Approach: a self-contained GTK module owns its UI and bounded request state, registered with the existing analytics lifecycle. Extending Overview would couple independent requests; duplicating daemon calculations would risk semantic drift. Selected approach follows the existing worker pattern.

Scope boundary: observation tips are deferred in this slice; model rows, daemon picks, category counts and model work mix are complete. No changes to live Overview request code.

Verification: pure tests cover malformed/disabled responses, selection identity, stale completions, single-flight coalescing, row ordering and known-zero versus missing numbers. Run GTK tests, format and clippy after implementation; native Linux mounting remains a separate environment check.
