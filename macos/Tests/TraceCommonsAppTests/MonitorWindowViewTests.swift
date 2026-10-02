#if DEBUG
import Foundation
import XCTest
import TCDesign
@testable import TraceCommonsApp

/// The tab must distinguish a daemon's silence from an explicit off answer.
@MainActor
final class MonitorWindowViewTests: XCTestCase {
    func testMissingSettingsDoNotDrawAnOffDot() {
        XCTAssertNil(MonitorWindowView.inferenceDot(settings: nil))
    }

    func testMissingPrivateInferenceDoesNotDrawAnOffDot() throws {
        XCTAssertNil(MonitorWindowView.inferenceDot(settings: try settings()))
    }

    func testNullPrivateInferenceDoesNotDrawAnOffDot() throws {
        XCTAssertNil(MonitorWindowView.inferenceDot(settings: try settings(NSNull())))
    }

    func testExplicitFalseDrawsAnOffDot() throws {
        XCTAssertEqual(MonitorWindowView.inferenceDot(settings: try settings(false)), .off)
    }

    func testExplicitTrueDrawsAnOnDot() throws {
        XCTAssertEqual(MonitorWindowView.inferenceDot(settings: try settings(true)), .on)
    }

    private func settings(_ privateInference: Any? = nil) throws -> DaemonSettingsView {
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
        return try JSONDecoder().decode(
            DaemonSettingsView.self, from: JSONSerialization.data(withJSONObject: payload))
    }
}
#endif
