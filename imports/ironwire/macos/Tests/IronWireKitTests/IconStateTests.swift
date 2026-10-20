//! What the icon says, for every state it can be in.
//
// The icon is read at a glance or not at all, so the mapping has to be exact and
// it has to come from fields the daemon published. Nothing here looks at a
// backend's name.

import XCTest

@testable import IronWireKit

final class IconStateTests: XCTestCase {
    private func status(
        backends: [BackendView] = [BackendView(id: "a", name: "A")],
        balance: BalanceView = BalanceView(available: 1, freeAvailable: 1),
        lastRoute: LastRouteView? = nil
    ) -> StatusView {
        StatusView(version: "0.1.0", port: 8463, backends: backends, balance: balance, lastRoute: lastRoute)
    }

    func test_no_daemon_dims_the_icon_rather_than_raising_an_alarm() {
        // The daemon being down is the state a machine is in most of the time.
        XCTAssertEqual(IconState.from(nil), .unreachable)
    }

    func test_a_preferred_route_is_the_quiet_icon() {
        let state = IconState.from(status(lastRoute: LastRouteView(backend: "claude-sub", rung: .preferred)))
        XCTAssertEqual(state, .healthy)
    }

    func test_a_daemon_that_has_not_routed_anything_yet_is_not_treated_as_degraded() {
        XCTAssertEqual(IconState.from(status()), .healthy)
    }

    /// Rungs 1 and 2 are not worth interrupting anyone over, but they are worth
    /// being able to see — which is the difference between a dot and a red one.
    func test_a_descent_short_of_a_family_change_shows_a_dot_but_no_colour() {
        for rung in [Rung.smallerModel, .alternateCredential] {
            let state = IconState.from(status(lastRoute: LastRouteView(backend: "b", rung: rung)))
            XCTAssertEqual(state, .degraded, "for \(rung)")
        }
    }

    /// The descent IronWire is obliged to announce.
    func test_a_cross_family_route_colours_the_icon() {
        let state = IconState.from(
            status(lastRoute: LastRouteView(backend: "nearai", from: "claude-sub", rung: .crossFamily)))
        XCTAssertEqual(state, .attention)
    }

    /// The acceptance criterion in the issue: it goes back on its own.
    func test_the_icon_returns_when_the_route_does() {
        let degraded = IconState.from(
            status(lastRoute: LastRouteView(backend: "nearai", from: "claude-sub", rung: .crossFamily)))
        let recovered = IconState.from(
            status(lastRoute: LastRouteView(backend: "claude-sub", from: "nearai", rung: .preferred)))
        XCTAssertEqual(degraded, .attention)
        XCTAssertEqual(recovered, .healthy)
    }

    /// A backend the breaker is skipping is otherwise indistinguishable from one
    /// that simply is not being chosen.
    func test_an_open_circuit_outranks_an_otherwise_perfect_route() {
        let state = IconState.from(
            status(
                backends: [BackendView(id: "a", name: "A", health: HealthView(circuit: "open", consecutiveFailures: 3))],
                lastRoute: LastRouteView(backend: "a", rung: .preferred)))
        XCTAssertEqual(state, .attention)
    }

    func test_nowhere_left_to_route_raises_the_icon() {
        let state = IconState.from(status(balance: BalanceView(available: 0, unknown: 0, unavailable: 2)))
        XCTAssertEqual(state, .attention)
    }

    /// "We have not heard from it" is not "it is down" — `BalanceView` keeps the
    /// two apart for the same reason, and the daemon will still route there.
    func test_a_pool_that_has_not_reported_still_counts_as_somewhere_to_go() {
        let state = IconState.from(status(balance: BalanceView(available: 0, unknown: 1)))
        XCTAssertEqual(state, .healthy)
    }

    /// A rung added after this build shipped is not `preferred`, so it is a
    /// descent — but it is not the one descent worth alarming someone over.
    func test_an_unrecognised_rung_shows_as_degraded_and_not_as_an_alarm() {
        let state = IconState.from(
            status(lastRoute: LastRouteView(backend: "b", rung: .unrecognised("different_provider"))))
        XCTAssertEqual(state, .degraded)
    }
}
