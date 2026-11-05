import SwiftUI
import XCTest
@testable import TraceCommonsApp

/// Each snapshot must relinquish its hosted view and polling task before
/// a subsequent snapshot creates another AppKit renderer.
@MainActor
final class InsightsRenderLifecycleTests: XCTestCase {
    private final class Probe {
        var started = false
        var stopped = false
        var disappeared = false
    }

    func testRenderFinishesHostedTasksBeforeReturning() async throws {
        let probe = Probe()
        let view = Text("Lifecycle probe")
            .task {
                probe.started = true
                defer { probe.stopped = true }
                do { try await Task.sleep(for: .seconds(60 * 60)) }
                catch { }
            }
            .onDisappear { probe.disappeared = true }
        let png = try await InsightsGetStartedRenderTests.render(
            AnyView(view), width: 440, height: 240, ready: { probe.started })
        XCTAssertFalse(png.isEmpty)
        XCTAssertTrue(probe.disappeared, "The old hosted root must disappear before its renderer is destroyed")
        XCTAssertTrue(probe.stopped, "A rendered view's task must finish before the next snapshot begins")
    }
}
