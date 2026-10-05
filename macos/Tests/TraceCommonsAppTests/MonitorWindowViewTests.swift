#if DEBUG
import Foundation
import XCTest
import TCBridge
import TCDesign
import TCShellCore
@testable import TraceCommonsApp

/// The Inference tab's dot draws what the listener is doing, never what the
/// switch asked for, and still tells a daemon's silence from an explicit off.
@MainActor
final class MonitorWindowViewTests: XCTestCase {
    private let calls = PrivateInferenceCalls.testing

    // MARK: Unreported and unknown draw nothing

    func testMissingSettingsDoNotDrawAnOffDot() {
        XCTAssertNil(MonitorWindowView.inferenceDot(settings: nil, state: Self.unreported, calls: calls))
    }

    func testMissingPrivateInferenceDoesNotDrawAnOffDot() throws {
        XCTAssertNil(dot(try settings()))
    }

    func testNullPrivateInferenceDoesNotDrawAnOffDot() throws {
        XCTAssertNil(dot(try settings(NSNull())))
    }

    func testSwitchOnWithAnUnreportedStateDrawsNoDot() throws {
        XCTAssertNil(dot(try settings(true)))
    }

    func testSwitchOnWithAnOffStateDrawsNoDot() throws {
        XCTAssertNil(dot(try settings(true, state: "off")))
    }

    // MARK: Explicit off

    func testExplicitFalseDrawsAnOffDot() throws {
        XCTAssertEqual(dot(try settings(false)), .off)
    }

    func testExplicitFalseWithAnOffStateDrawsAnOffDot() throws {
        XCTAssertEqual(dot(try settings(false, state: "off")), .off)
    }

    // MARK: Only a running listener is on

    func testRunningDrawsAnOnDot() throws {
        XCTAssertEqual(dot(try settings(true, state: "running")), .on)
    }

    func testRunningDrawsAnOnDotWhateverTheSwitchSays() throws {
        XCTAssertEqual(dot(try settings(false, state: "running")), .on)
    }

    func testSwitchOnOverARefusedListenerIsNotOn() throws {
        let drawn = dot(try settings(true, state: "port_in_use", port: 8463))
        XCTAssertNotNil(drawn)
        XCTAssertNotEqual(drawn, .on)
    }

    func testSwitchOnOverAStoppingListenerIsNotOn() throws {
        let drawn = dot(try settings(true, state: "stopping"))
        XCTAssertNotNil(drawn)
        XCTAssertNotEqual(drawn, .on)
    }

    func testSwitchOnOverAListenerWithoutBackendsIsNotOn() throws {
        let drawn = dot(try settings(true, state: "running_no_backends", port: 8463))
        XCTAssertNotNil(drawn)
        XCTAssertNotEqual(drawn, .on)
    }

    func testHeldAndRefusedDrawDistinctDots() throws {
        XCTAssertNotEqual(
            dot(try settings(true, state: "stopping")),
            dot(try settings(true, state: "port_in_use")))
    }

    // MARK: The segment's accessibility value

    func testAccessibilityValueIsAbsentWithoutTheCopy() {
        XCTAssertNil(MonitorWindowView.inferenceAccessibilityValue(
            state: PrivateInferenceState(label: "running", port: 8463), copy: nil, calls: calls))
    }

    func testAccessibilityValueFallsBackToUnknownWhenUnreported() throws {
        let copy = try Self.fixtureCopy()
        XCTAssertEqual(
            MonitorWindowView.inferenceAccessibilityValue(state: Self.unreported, copy: copy, calls: calls),
            copy.stateUnknown)
    }

    func testAccessibilityValueIsTheStateLine() throws {
        XCTAssertEqual(
            MonitorWindowView.inferenceAccessibilityValue(
                state: PrivateInferenceState(label: "port_in_use", port: 8463), copy: try Self.fixtureCopy(),
                calls: calls),
            "port_in_use")
    }

    // MARK: Helpers

    private static let unreported = PrivateInferenceState(label: "", port: nil)

    private static func fixtureCopy() throws -> PrivateInferenceCopy {
        try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: try XCTUnwrap(TCPrivateInference.copyJSON())))
    }

    /// The dot as the window computes it: the state from the same payload.
    private func dot(_ settings: DaemonSettingsView) -> GlassStatus? {
        MonitorWindowView.inferenceDot(
            settings: settings,
            state: settings.privateInferenceState?.surfaceState ?? Self.unreported,
            calls: calls)
    }

    private func settings(
        _ privateInference: Any? = nil, state: String? = nil, port: Int? = nil
    ) throws -> DaemonSettingsView {
        var payload: [String: Any] = [
            "quiescence_secs": 45, "digest_interval_secs": 3600,
            "local_notifications": true, "queue_ttl_days": 14,
            "max_queue_entries": 500, "max_uploads_per_day": 100,
            "near_ai_configured": false, "claude_root_configured": true,
            "codex_root_configured": true,
        ]
        if let privateInference {
            payload["private_inference"] = privateInference
        }
        if let state {
            var object: [String: Any] = ["state": state]
            if let port {
                object["port"] = port
            }
            payload["private_inference_state"] = object
        }
        return try JSONDecoder().decode(
            DaemonSettingsView.self, from: JSONSerialization.data(withJSONObject: payload))
    }
}
#endif
