# Linux Settings navigation implementation plan

> **For agentic workers:** Use superpowers:executing-plans for this bounded implementation.

**Goal:** Add macOS-ordered Settings sections and responsive navigation to GTK.

**Architecture:** A nested settings_navigation module builds the sidebar, compact chooser, and one scroll body. SettingsView builds each existing widget once and places it into named section containers; its public interface and handlers remain intact.

**Tech Stack:** Existing gtk4 0.9 and libadwaita 0.7 with v1_2.

**Spec:** ../specs/2026-11-11-linux-settings-navigation-design.md

## Global constraints

No new notices, dependencies, grants, auth behavior, or UI shell changes. Use shared settings_nav copy. RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0 on all Cargo commands.

## Review focus

- Short sections near the bottom must scroll within valid bounds.
- Narrow windows retain a reachable section selector.
- Resizing preserves selected section and edited widget values.
- Dynamic card height changes do not invalidate stored widget anchors.
- Account recovery and watcher controls remain reachable through the appropriate group.

## Task

- [x] Add navigation tests for section order, clamping and real GTK selection/resizing.
- [x] Build nested settings_navigation.rs and regroup settings.rs cards without changing handlers.
- [x] Run format, full GTK suite, display-dependent test and clippy; record platform limits.
- [ ] Review diff and commit, without merge.

Repository multi-task verification guidance takes precedence over per-step test runs: tests run in the final batch.

## Validation record

The local macOS GTK suite passes: 606 library tests and 10 integration tests, with nine display-dependent tests ignored. The new real-widget test compiles but local execution is blocked before constructing widgets by GTK's macOS main-thread requirement (Rust's test harness runs tests on worker threads). The exact test is registered in the Weston run_ignored_test loop for Linux CI. Do not treat local suite success as Linux visual/interaction evidence.

Format checking, whitespace checking, and Weston script shell syntax validation pass. Clippy passes with the repository's documented allowances (`type_complexity`, `collapsible_if`, `manual_option_as_slice`, `useless_vec`, `redundant_pattern_matching`). Strict Clippy without those allowances reports existing warnings in unchanged GTK code; none identify settings_navigation.rs.
