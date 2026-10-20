//! The rules that must not drift from `src/render.rs`.
//
// A user who runs `ironwire status` and then looks at the menu bar is entitled
// to see the same thing. These pin the arithmetic and the wording so the two
// surfaces cannot quietly disagree about what "nearly full" looks like.

import XCTest

@testable import IronWireKit

final class FormatTests: XCTestCase {
    // MARK: - The bar

    /// The single most likely place for this app to go wrong, per the issue that
    /// asked for it. `ProgressView` wants a `Double`; `unknown` has none, and
    /// every default — 0, 50, greyed — is a number IronWire made up in the one
    /// surface a user takes in without reading.
    func test_an_unobserved_backend_draws_no_bar_at_all() {
        XCTAssertNil(Format.capacityFraction(.unknown))
    }

    func test_a_state_this_build_does_not_recognise_draws_no_bar() {
        XCTAssertNil(Format.capacityFraction(.unrecognised("throttled")))
    }

    /// Neither is a measurement of how much of the window went. A bar pinned to
    /// full would claim one.
    func test_exhaustion_and_a_spend_cap_draw_no_bar_either() {
        XCTAssertNil(Format.capacityFraction(.exhausted(retryInSecs: 60)))
        XCTAssertNil(Format.capacityFraction(.capReached(spentUsd: 5, capUsd: 5, resetsInSecs: 3600)))
    }

    func test_only_an_observation_produces_a_fraction() throws {
        let fraction = try XCTUnwrap(
            Format.capacityFraction(.observed(usedPct: 82, observedSecsAgo: 0, resetsInSecs: nil)))
        XCTAssertEqual(fraction, 0.82, accuracy: 0.0001)
    }

    func test_a_percentage_outside_the_scale_is_clamped_rather_than_drawn_past_the_end() throws {
        let over = try XCTUnwrap(
            Format.capacityFraction(.observed(usedPct: 140, observedSecsAgo: 0, resetsInSecs: nil)))
        let under = try XCTUnwrap(
            Format.capacityFraction(.observed(usedPct: -5, observedSecsAgo: 0, resetsInSecs: nil)))
        XCTAssertEqual(over, 1)
        XCTAssertEqual(under, 0)
    }

    // MARK: - Parity with the CLI

    /// `meter()` in `src/render.rs`: ten cells, rounded, clamped.
    func test_the_meter_fills_the_same_cells_the_cli_fills() {
        XCTAssertEqual(Format.meterFilled(usedPct: 0), 0)
        XCTAssertEqual(Format.meterFilled(usedPct: 4), 0)
        XCTAssertEqual(Format.meterFilled(usedPct: 5), 1)  // .5 rounds away from zero
        XCTAssertEqual(Format.meterFilled(usedPct: 50), 5)
        XCTAssertEqual(Format.meterFilled(usedPct: 82), 8)
        XCTAssertEqual(Format.meterFilled(usedPct: 100), 10)
        XCTAssertEqual(Format.meterFilled(usedPct: 140), 10)
        XCTAssertEqual(Format.meterFilled(usedPct: -3), 0)
    }

    /// `Style::by_usage` in `src/style.rs` colours at 50 and 90.
    func test_the_usage_bands_are_the_cli_thresholds() {
        XCTAssertEqual(Format.usageLevel(usedPct: 0), .good)
        XCTAssertEqual(Format.usageLevel(usedPct: 49.9), .good)
        XCTAssertEqual(Format.usageLevel(usedPct: 50), .warn)
        XCTAssertEqual(Format.usageLevel(usedPct: 89.9), .warn)
        XCTAssertEqual(Format.usageLevel(usedPct: 90), .bad)
        XCTAssertEqual(Format.usageLevel(usedPct: 100), .bad)
    }

    /// `duration()` in `src/render.rs`, digit for digit.
    func test_durations_read_the_way_the_cli_writes_them() {
        XCTAssertEqual(Format.duration(0), "0s")
        XCTAssertEqual(Format.duration(59), "59s")
        XCTAssertEqual(Format.duration(60), "1m")
        XCTAssertEqual(Format.duration(3599), "59m")
        XCTAssertEqual(Format.duration(3600), "1h")
        XCTAssertEqual(Format.duration(3660), "1h1m")
        XCTAssertEqual(Format.duration(7200), "2h")
    }

    /// An elapsed window must never report negative time — the same guard the
    /// daemon applies before the value is ever sent.
    func test_an_elapsed_window_never_counts_backwards() {
        XCTAssertEqual(Format.duration(-30), "0s")
    }

    /// Summing no dollars gives `-0.0` in IEEE 754, and "$-0.00" reads like a
    /// refund. `balance_block` carries the same guard.
    func test_no_spend_is_shown_as_zero_and_never_as_a_refund() {
        XCTAssertEqual(Format.currency(-0.0), "$0.00")
        XCTAssertEqual(Format.currency(0.16), "$0.16")
        XCTAssertEqual(Format.currency(12), "$12.00")
    }

    // MARK: - Wording

    func test_unknown_capacity_says_the_provider_has_not_reported() {
        let summary = Format.headroomSummary(.unknown)
        XCTAssertTrue(summary.contains("unknown"), summary)
        XCTAssertFalse(summary.contains("0%"), "a number nobody measured must not appear")
    }

    func test_an_observation_carries_its_own_age() {
        let summary = Format.headroomSummary(
            .observed(usedPct: 82, observedSecsAgo: 40, resetsInSecs: 1800))
        XCTAssertTrue(summary.contains("82% used"), summary)
        XCTAssertTrue(summary.contains("resets in 30m"), summary)
        XCTAssertTrue(summary.contains("observed 40s ago"), summary)
    }

    func test_a_spend_cap_is_worded_as_the_users_limit_not_the_providers() {
        let summary = Format.headroomSummary(
            .capReached(spentUsd: 5, capUsd: 5, resetsInSecs: 3600))
        XCTAssertTrue(summary.contains("cap reached"), summary)
        XCTAssertFalse(summary.contains("exhausted"), "the provider would have served this one")
    }

    /// A healthy backend gets no health line. One on every row is noise that
    /// trains people to stop reading the block.
    func test_a_healthy_backend_says_nothing_about_its_circuit() {
        XCTAssertNil(Format.healthSummary(HealthView()))
    }

    func test_a_skipped_backend_says_so_and_says_when_it_comes_back() throws {
        let summary = try XCTUnwrap(
            Format.healthSummary(HealthView(circuit: "open", consecutiveFailures: 3, retryInSecs: 45)))
        XCTAssertTrue(summary.contains("skipping"), summary)
        XCTAssertTrue(summary.contains("45s"), summary)
    }

    func test_a_backend_with_failures_that_is_still_in_use_says_that_too() throws {
        let summary = try XCTUnwrap(
            Format.healthSummary(HealthView(circuit: "closed", consecutiveFailures: 2)))
        XCTAssertTrue(summary.contains("still in use"), summary)
    }

    func test_a_route_that_moved_names_where_it_came_from() {
        let moved = LastRouteView(backend: "nearai", from: "claude-sub", rung: .crossFamily)
        XCTAssertEqual(Format.routeSummary(moved), "claude-sub → nearai")
    }

    /// The ordinary case is a conversation staying put, and an arrow pointing at
    /// itself is noise — `events.rs` avoids the same thing in its log line.
    func test_a_route_that_stayed_put_does_not_draw_a_pointless_arrow() {
        let stayed = LastRouteView(backend: "claude-sub", from: "claude-sub")
        XCTAssertEqual(Format.routeSummary(stayed), "claude-sub")
        XCTAssertEqual(Format.routeSummary(LastRouteView(backend: "claude-sub")), "claude-sub")
    }

    /// A banner is read as a sentence or not at all.
    func test_names_are_joined_as_a_sentence_not_a_list() {
        XCTAssertEqual(Format.list([]), "")
        XCTAssertEqual(Format.list(["Claude Code"]), "Claude Code is")
        XCTAssertEqual(Format.list(["Claude Code", "Codex"]), "Claude Code and Codex are")
        XCTAssertEqual(
            Format.list(["Claude Code", "Codex", "Zed"]), "Claude Code, Codex and Zed are")
    }
}
