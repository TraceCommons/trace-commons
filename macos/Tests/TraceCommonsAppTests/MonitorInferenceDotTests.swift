#if DEBUG
import Foundation
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The monitor's Inference dot says what the listener is doing, from the
/// daemon's report, never what the switch asked for: a switch that is on
/// over a listener that refused to start is not drawn as on. And the dot is
/// not colour-only: the tab carries the core's state sentence.
final class MonitorInferenceDotTests: XCTestCase {
    private let calls = PrivateInferenceCalls(
        stateLine: { "line for \($0)" },
        stateTone: { label in
            switch label {
            case "serving": return 22
            case "held": return 21
            case "port_in_use": return 24
            default: return 0
            }
        },
        servingLine: { _ in nil },
        shouldOffer: { _, _ in false },
        quitNeedsNotice: { _, _ in false }
    )

    /// #1146's rule (`monitor-shell.tsx`): on while working, outside (red)
    /// for every other reported state, refused, held and off alike.
    func test_onlyAServingListenerIsDrawnOn() {
        XCTAssertEqual(MonitorWindowView.inferenceDot(PrivateInferenceState(label: "serving", port: 1), calls: calls), .on)
        XCTAssertEqual(MonitorWindowView.inferenceDot(PrivateInferenceState(label: "port_in_use", port: nil), calls: calls), .outside)
        XCTAssertEqual(MonitorWindowView.inferenceDot(PrivateInferenceState(label: "held", port: nil), calls: calls), .outside)
        XCTAssertEqual(MonitorWindowView.inferenceDot(PrivateInferenceState(label: "off", port: nil), calls: calls), .outside)
    }

    /// No report is no dot: unknown is neither on nor off.
    func test_anUnreportedStateDrawsNoDot() {
        XCTAssertNil(MonitorWindowView.inferenceDot(PrivateInferenceState(label: "", port: nil), calls: calls))
        XCTAssertNil(MonitorWindowView.inferenceDot(nil, calls: calls))
    }

    /// The same from the decoded wire (#1189): an older daemon that omits
    /// `private_inference_state`, or reports it as null, draws no dot, even
    /// with the switch on. Unknown is never drawn as on or off.
    func test_aMissingOrNullStateOnTheWireDrawsNoDot() throws {
        XCTAssertNil(dot(try settings()))
        XCTAssertNil(dot(try settings(state: NSNull())))
        XCTAssertNil(dot(try settings(privateInference: true)))
        XCTAssertNil(dot(try settings(privateInference: true, state: NSNull())))
        XCTAssertEqual(dot(try settings(privateInference: true, state: ["state": "serving", "port": 1])), .on)
    }

    func test_theDotHasTheCoresSentenceAsItsTextEquivalent() {
        XCTAssertEqual(
            MonitorWindowView.inferenceDotDescription(PrivateInferenceState(label: "port_in_use", port: nil), calls: calls),
            "line for port_in_use")
        XCTAssertNil(MonitorWindowView.inferenceDotDescription(nil, calls: calls))
    }

    // MARK: Helpers

    /// The dot as the window computes it from a decoded `get_settings`.
    private func dot(_ settings: DaemonSettingsView) -> GlassStatus? {
        MonitorWindowView.inferenceDot(settings.privateInferenceState?.surfaceState, calls: calls)
    }

    private func settings(privateInference: Bool? = nil, state: Any? = nil) throws -> DaemonSettingsView {
        var payload: [String: Any] = [
            "quiescence_secs": 45, "digest_interval_secs": 3600,
            "local_notifications": true, "queue_ttl_days": 14,
            "max_queue_entries": 500, "max_uploads_per_day": 100,
            "near_ai_configured": false, "claude_root_configured": true,
            "codex_root_configured": true,
        ]
        if let privateInference { payload["private_inference"] = privateInference }
        if let state { payload["private_inference_state"] = state }
        return try JSONDecoder().decode(
            DaemonSettingsView.self, from: JSONSerialization.data(withJSONObject: payload))
    }
}
#endif
