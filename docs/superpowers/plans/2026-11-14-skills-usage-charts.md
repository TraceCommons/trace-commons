# Skills Usage charts implementation plan

Goal: build the approved chart-first Skills Usage layout with concise shared copy.
Architecture: a pure Swift projection of the existing daemon snapshot feeds native Charts and the existing evidence drill-down. No data collection or dependency changes.
Spec: ../specs/2026-11-14-skills-usage-charts.md

- [x] Add InsightsSkillsUsage projection and regression tests for disjoint counts, periods, load-only exclusions, incomplete coverage, collisions and undated evidence.
- [x] Replace Usage list with ranked uses, conservative removal candidates and selected-skill daily chart using InsightsGlass/GlassTokens.
- [x] Trim redundant Index helper prose; shared Rust copy is a parallel task.
- [x] Rebuild FFI, run the complete Swift suite, then rerun focused regressions on the final empty-state changes.
- [x] Independent review and read-only staging data validation.
- [ ] Open draft PR; CI and rendered native UI verification remain separate gates.

Review focus: missing windows cannot become zero; undated uses cannot imply unused; unsupported agents stay excluded; chart dates use daemon local-day buckets; no session-count aggregation or new authority.

Validation: Insights crate 730 passed, 3 ignored; native broad suite 3,663 XCTest cases (10 skipped) and 8 Swift Testing cases passed; final focused suite 38 passed including 9 new regressions. Clippy with the repository baseline, formatting, four license-boundary tests, and 55 literal Skills labels exported through the rebuilt FFI passed. Release build validation is recorded in the PR. Staging has partial history, so removal candidates are conservatively omitted there. No rendered native UI inspection was performed.
