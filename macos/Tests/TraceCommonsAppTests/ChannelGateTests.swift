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

    func testTheGateRefusesComputeToo() throws {
        guard case .refused = BuildChannelGate.verdict(plistChannel: try otherChannel()) else {
            return XCTFail("a mismatch must refuse")
        }
        guard case .agreed = BuildChannelGate.verdict(
            plistChannel: try XCTUnwrap(BuildChannel.decode(fromJSON: TCBuildChannel.json())).channel)
        else { return XCTFail("the dylib's own channel agrees") }
    }
}
