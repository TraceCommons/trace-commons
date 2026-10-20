//! Which events are loud enough to interrupt someone.
//
// The filter is narrow because the channel is expensive. Announcing a rung the
// user cannot act on trains them to ignore the channel, and then the
// announcement that matters gets ignored too (`docs/DESIGN.md` §3).

import XCTest

@testable import IronWireKit

final class NotificationPolicyTests: XCTestCase {
    private func routed(_ rung: Rung, translated: Bool = false) -> Event {
        .routed(
            at: Date(), conversation: "abc123", from: "claude-sub", to: "nearai",
            rung: rung, translated: translated, reason: "no same-family capacity available")
    }

    /// The sentence this whole feature exists to produce.
    func test_a_family_change_is_announced_and_names_where_traffic_went() throws {
        let notification = try XCTUnwrap(NotificationPolicy.notification(for: routed(.crossFamily, translated: true)))
        XCTAssertTrue(notification.body.contains("claude-sub → nearai"), notification.body)
        XCTAssertTrue(notification.body.contains("no same-family capacity available"), notification.body)
    }

    /// Rungs 0–2 change nothing the user can observe. This is the same test
    /// `events.rs` runs against `Event::is_user_visible`.
    func test_a_descent_the_user_cannot_act_on_is_not_announced() {
        XCTAssertNil(NotificationPolicy.notification(for: routed(.preferred)))
        XCTAssertNil(NotificationPolicy.notification(for: routed(.smallerModel)))
        XCTAssertNil(NotificationPolicy.notification(for: routed(.alternateCredential)))
    }

    /// A rung from a newer ladder is degraded, but it is not the cross-family
    /// descent — announcing it would be this app inventing a policy.
    func test_an_unrecognised_rung_is_not_announced() {
        XCTAssertNil(NotificationPolicy.notification(for: routed(.unrecognised("different_provider"))))
    }

    func test_a_request_that_could_not_be_served_is_always_announced() throws {
        let notification = try XCTUnwrap(
            NotificationPolicy.notification(
                for: .failed(at: Date(), conversation: "abc", detail: "every backend is rate limited")))
        XCTAssertEqual(notification.body, "every backend is rate limited")
    }

    /// `Event::is_user_visible` returns true for this, and the issue's problem
    /// statement names a spend cap among the events that matter. It is published
    /// once per backend per window, so it cannot become the per-request noise
    /// the issue warns about.
    func test_reaching_a_spend_cap_the_user_set_is_announced() throws {
        let notification = try XCTUnwrap(
            NotificationPolicy.notification(
                for: .capReached(at: Date(), backend: "openai", spentUsd: 5, capUsd: 5)))
        XCTAssertTrue(notification.body.contains("$5.00 of $5.00"), notification.body)
        XCTAssertTrue(notification.body.contains("openai"), notification.body)
    }

    /// The whole point of the breaker is that the request was still served.
    /// Useful in a live view, noise as an alert.
    func test_a_circuit_opening_is_not_worth_a_notification() {
        XCTAssertNil(
            NotificationPolicy.notification(for: .health(at: Date(), backend: "nearai", circuit: "open")))
    }

    func test_an_event_from_a_newer_daemon_is_never_announced() {
        XCTAssertNil(NotificationPolicy.notification(for: .unrecognised("catalog_updated")))
    }
}
