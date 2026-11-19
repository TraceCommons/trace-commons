# Linux Insights Benchmarks Implementation Plan

**Goal:** Add usable read-only live Benchmarks to GTK Insights.
**Architecture:** Dedicated presentation and single-flight state module, weak existing App binding; small registration changes only.
**Tech Stack:** Existing Rust, GTK4/libadwaita, serde.
**Spec:** ../specs/2026-11-11-linux-insights-benchmarks-design.md

- [x] Add typed response and pure formatting/state helpers with tests.
- [x] Add selectors, category/model/pick/mix rendering and bounded App request.
- [x] Register destination and lifecycle; update tab navigation expectation.
- [x] Run GTK tests, formatting and clippy with fixed flags; independent review and address findings.

No new dependencies, backend creation, fake samples or copy. Preserve live Overview logic.

Validation: GTK full suite passed (608 library tests, 8 ignored; 10 binary/integration tests). Formatting passed. Clippy passed using the repository documented allow list. Independent review found no blocking issues. Native Linux display tests remain ignored locally on macOS. Optional observation tips remain out of scope.
