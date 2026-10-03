#if DEBUG
import XCTest
@testable import TCShellCore

/// The sample client answers the switch as the daemon does: the recorded
/// settings until a write, then the written value with the marker set.
final class PrivateAISampleTests: XCTestCase {
    func testTheSampleReadsTheRecordedSettingsThenTheWrite() async throws {
        let client = SampleDaemonClient(.normalDay)
        let before = try await client.privateAI()
        XCTAssertEqual(before.on, false)
        XCTAssertNotNil(before.state)
        let on = try await client.setPrivateAI(on: true)
        XCTAssertEqual(on.on, true)
        XCTAssertEqual(on.offerSeen, true)
        let reread = try await client.privateAI()
        XCTAssertEqual(reread.on, true)
        XCTAssertEqual(client.privateAICalls, [true])
    }

    func testACoreDownSampleIsUnreachable() async {
        let client = SampleDaemonClient(.coreDown)
        do {
            _ = try await client.setPrivateAI(on: true)
            XCTFail("a core-down sample answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .unreachable)
        }
        XCTAssertEqual(client.privateAICalls, [])
    }
}
#endif
