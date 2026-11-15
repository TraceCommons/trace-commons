import XCTest
import TCShellCore
@testable import TraceCommonsApp

/// The map draws managed sessions from `tool_destinations`. It was read
/// again only on a status change, a resync or a model call, so a session
/// started or stopped from the list stayed off the map, or kept its old
/// count, until one of those happened.
@MainActor
final class ManagedMapRefreshTests: XCTestCase {
    func test_aManagedSessionChangeRereadsTheMapsDestinations() async throws {
        let client = SampleDaemonClient(.normalDay)
        let store = InferenceStore(client: client)
        let running = Task { await store.run() }
        defer { running.cancel() }
        // The opening load, and the snapshot the stream opens with.
        while store.destinationReads < 2 { await Task.yield() }
        for _ in 0..<50 { await Task.yield() }
        let before = store.destinationReads

        client.emit(.unknown("something_else"))
        client.emit(.unknown("managed_changed"))
        // A yield budget alone runs out on a loaded machine before the
        // reread is scheduled, so bound the wait by the clock.
        let deadline = ContinuousClock.now + .seconds(10)
        while store.destinationReads == before, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(1))
        }
        for _ in 0..<50 { await Task.yield() }
        XCTAssertEqual(store.destinationReads, before + 1,
                       "managed_changed rereads the destinations once; another unknown event does not")
    }
}
