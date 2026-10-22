import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Spec 4.3 at the app: a bundle whose `TCChannel` disagrees with the dylib,
/// or names none, refuses with the core's sentence and starts no daemon.
/// Only refusals are exercised: an agreeing start would open the
/// developer's real state directory.
@MainActor
final class ChannelGateTests: XCTestCase {
    private func otherChannel() throws -> String {
        let channel = try XCTUnwrap(BuildChannel.decode(fromJSON: TCBuildChannel.json()))
        return channel.channel == "production" ? "staging" : "production"
    }

    func testAChannelMismatchRefusesToStart() throws {
        let model = AppModel()
        model.start(plistChannel: try otherChannel())
        XCTAssertEqual(model.startup, .refused(try XCTUnwrap(TCBuildChannel.mismatchLine())))
        XCTAssertEqual(model.configDirectory, "", "no state directory was resolved")
    }

    func testAMissingPlistChannelRefusesToStart() throws {
        let model = AppModel()
        model.start(plistChannel: nil)
        XCTAssertEqual(model.startup, .refused(try XCTUnwrap(TCBuildChannel.mismatchLine())))
        XCTAssertEqual(model.configDirectory, "")
    }

    func testTheDylibsOwnChannelAgrees() throws {
        guard case .refused = BuildChannelGate.verdict(plistChannel: try otherChannel()) else {
            return XCTFail("a mismatch must refuse")
        }
        guard case .agreed = BuildChannelGate.verdict(
            plistChannel: try XCTUnwrap(BuildChannel.decode(fromJSON: TCBuildChannel.json())).channel)
        else { return XCTFail("the dylib's own channel agrees") }
    }

    /// Compute opens the same state directory the daemon does, so it is
    /// gated the same way, and it says the same sentence.
    func testAChannelMismatchRefusesCompute() async throws {
        let model = ComputeModel()
        await model.start(plistChannel: try otherChannel())
        XCTAssertEqual(model.failureLabel, "channel-mismatch")
        XCTAssertEqual(model.refusalSentence, try XCTUnwrap(TCBuildChannel.mismatchLine()))
        XCTAssertNil(model.requestedDirectory, "no state directory was opened")
        XCTAssertNil(model.snapshot)
    }

    func testAMissingPlistChannelRefusesCompute() async throws {
        let model = ComputeModel()
        await model.start(plistChannel: nil)
        XCTAssertEqual(model.failureLabel, "channel-mismatch")
        XCTAssertEqual(model.refusalSentence, try XCTUnwrap(TCBuildChannel.mismatchLine()))
        XCTAssertNil(model.requestedDirectory)
    }
}
