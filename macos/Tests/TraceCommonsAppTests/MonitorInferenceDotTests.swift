#if DEBUG
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

    func test_theDotHasTheCoresSentenceAsItsTextEquivalent() {
        XCTAssertEqual(
            MonitorWindowView.inferenceDotDescription(PrivateInferenceState(label: "port_in_use", port: nil), calls: calls),
            "line for port_in_use")
        XCTAssertNil(MonitorWindowView.inferenceDotDescription(nil, calls: calls))
    }
}
#endif
