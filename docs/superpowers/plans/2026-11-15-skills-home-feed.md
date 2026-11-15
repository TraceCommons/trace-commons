# Skills Home feed implementation plan

Goal: build the approved default Home feed using existing Skills data.
Architecture: a bounded Swift projection supplies a native Insights view; a validated internal route targets catalog entries and Usage charts. Shared words remain in the Rust copy owner.
Spec: ../specs/2026-11-15-skills-home-feed.md

- [x] Add projection tests for used-skill updates, comparable usage changes, followed-creator changes, stale dates, incomplete history, and disabled analysis.
- [x] Build the projection with stable IDs and bounded sections.
- [x] Add Home as the default tab and route View actions to exact targets; preserve explicit zero-use selections without changing ordinary ranking.
- [x] Add shared copy and native pane/wide rendering coverage.
- [x] Run consolidated native and copy suites, Release build, rendered visual inspection, and independent review.

Review focus: source identity is not a skill name; daily buckets are not rolling-window totals; missing coverage is not zero; creator events come only from followed catalogs; stale navigation must not silently choose another skill.

Validation: 731 Insights crate tests passed (3 ignored); 57 focused Skills tests passed; full Swift suite passed 3,689 XCTest cases (10 skipped) plus 8 Swift Testing cases. Release build passed. Clippy with the repository baseline, formatting, four license-boundary tests, and exported Home labels passed. Native renders at 408pt and 800pt were visually inspected. Independent review has no remaining findings. Existing capture warnings in unchanged AccountLoginChangeViews.swift remain.

Feed limits: three used-skill updates, three usage changes, five creator catalog changes. Trends require complete history spanning two completed weeks and an analysis run today; uncertain comparisons are omitted. Creator items are catalog changes, not creator posts. No installed-app upgrade or publication was performed.
