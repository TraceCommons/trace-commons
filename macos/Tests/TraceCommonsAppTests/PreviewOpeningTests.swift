import Foundation
import os
import TCBridge
import XCTest
@testable import TraceCommonsApp

private final class PreviewOpeningDaemon: DaemonCalling, Sendable {
    private let mainThreadCalls = OSAllocatedUnfairLock(initialState: [Bool]())

    var calls: [Bool] { mainThreadCalls.withLock { $0 } }

    private enum Refusal: Error, CustomStringConvertible {
        case synthetic
        var description: String { "synthetic_preview_refusal" }
    }

    func openPreview(entryID: String) throws -> TCPreview {
        mainThreadCalls.withLock { $0.append(Thread.isMainThread) }
        throw Refusal.synthetic
    }

    func call(_ method: String, params: String) -> String {
        #"{"id":1,"error":{"code":"unavailable","message":"unexpected_test_method"}}"#
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
}

final class PreviewOpeningTests: XCTestCase {
    /// Redaction can block. Opening must leave the UI actor while keeping
    /// the existing refusal mapped to the preview's failure outcome.
    @MainActor
    func testOpeningRunsOffTheMainThreadAndReturnsTheFailure() async {
        let daemon = PreviewOpeningDaemon()
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))

        let outcome = await model.openPreview(entryID: "synthetic-entry")

        XCTAssertEqual(daemon.calls, [false], "Preview redaction must not block the main thread")
        switch outcome {
        case .failed(let label):
            XCTAssertEqual(label, "synthetic_preview_refusal")
        case .opened(let preview):
            preview.close()
            XCTFail("The preview refusal must remain a failed outcome")
        }
    }
}
