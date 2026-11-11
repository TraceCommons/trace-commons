# GTK live Overview implementation plan

Spec: ../specs/2026-11-11-linux-live-overview-design.md

- [x] Add insights_live.rs with safe typed decoding, shared-copy formatting and reusable GTK work cards; tests cover missing/zero/partial values, no fabricated trends, malformed live data.
- [x] Inject optional weak App into existing Insights analytics after App creation. Request insights_week via App worker, gate enabled/readable, handle ISO week selection/stale callbacks; standalone uses local service unchanged. Render live work cards, avoid saved drill-downs on live data.
- [ ] Integrate independently validated PR2083 tab-row correction.
- [x] Final GTK test/fmt/clippy and review. No new dependencies. Use RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0 consistently; final tests once after all tasks per repo rules.

Review focus: disconnected daemon, disabled/unreadable feed, old daemon missing work, partial prices, rapid week changes, standalone startup side effects.

## Execution record

Implemented live connection through the existing App worker with weak ownership, one active request and one latest queued request. Live totals are noninteractive and never route to saved-snapshot evidence. Added API-price spend, top model, model shares and valid spend changes. Subscriptions, NEAR AI credit, daily/token trends, recaps and Benchmarks remain next slices.

Review resolved canceled-Quit loading: invalidate old results immediately, then resume only if the view remains mapped after close handlers. Hidden or destroyed views remain suspended.

Validation so far: 614 GTK tests passed, seven display tests ignored on macOS; formatting and CI-equivalent Clippy passed. Standalone GTK Insights launched/exited against a temporary path without creating contributor state. GDK emitted its skipped-frame timing warning. Independent review found no remaining source-level blockers. Full Linux live-data and canceled-modal interaction qualification remains pending.

PR2083 diagnosis correction: OCR misses tab labels, but Linux hierarchy reproduction and the actual rooted GTK view on macOS show all four labels with nonzero size. No speculative product sizing fix. Direct Rust widget regressions will be added to Weston CI; screenshot artifact upload was blocked by automatic approval review and is omitted.

Required license_boundary suite: four passed.
