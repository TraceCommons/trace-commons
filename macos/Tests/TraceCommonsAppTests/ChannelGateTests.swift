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
        model.start(plistChannel: try otherChannel(), featureProfileAgrees: true)
        XCTAssertEqual(model.startup, .refused(try XCTUnwrap(TCBuildChannel.mismatchLine())))
        XCTAssertEqual(model.configDirectory, "", "no state directory was resolved")
    }

    func testAMissingPlistChannelRefusesToStart() throws {
        let model = AppModel()
        model.start(plistChannel: nil, featureProfileAgrees: true)
        XCTAssertEqual(model.startup, .refused(try XCTUnwrap(TCBuildChannel.mismatchLine())))
        XCTAssertEqual(model.configDirectory, "")
    }

    func testTheDylibsOwnChannelAgrees() throws {
        guard case .refused = BuildChannelGate.verdict(
            plistChannel: try otherChannel(), featureProfileAgrees: true) else {
            return XCTFail("a mismatch must refuse")
        }
        guard case .agreed = BuildChannelGate.verdict(
            plistChannel: try XCTUnwrap(BuildChannel.decode(fromJSON: TCBuildChannel.json())).channel,
            featureProfileAgrees: true)
        else { return XCTFail("the dylib's own channel agrees") }
    }

    /// Compute opens the same state directory the daemon does, so it is
    /// gated the same way, and it says the same sentence.
    func testAChannelMismatchRefusesCompute() async throws {
        let model = ComputeModel()
        await model.start(plistChannel: try otherChannel(), featureProfileAgrees: true)
        XCTAssertEqual(model.failureLabel, "channel-mismatch")
        XCTAssertEqual(model.refusalSentence, try XCTUnwrap(TCBuildChannel.mismatchLine()))
        XCTAssertNil(model.requestedDirectory, "no state directory was opened")
        XCTAssertNil(model.snapshot)
    }

    func testAMissingPlistChannelRefusesCompute() async throws {
        let model = ComputeModel()
        await model.start(plistChannel: nil, featureProfileAgrees: true)
        XCTAssertEqual(model.failureLabel, "channel-mismatch")
        XCTAssertEqual(model.refusalSentence, try XCTUnwrap(TCBuildChannel.mismatchLine()))
        XCTAssertNil(model.requestedDirectory)
    }

    private func agreedChannel() throws -> String {
        try XCTUnwrap(BuildChannel.decode(fromJSON: TCBuildChannel.json())).channel
    }

    func testChannelMismatchVerdictKeepsItsLabelAndTheCoresSentence() throws {
        guard case .refused(let label, let sentence) = BuildChannelGate.verdict(
            plistChannel: try otherChannel(), featureProfileAgrees: true)
        else { return XCTFail("a channel mismatch must refuse") }
        XCTAssertEqual(label, "channel-mismatch")
        XCTAssertEqual(sentence, try XCTUnwrap(TCBuildChannel.mismatchLine()))
    }

    func testFeatureProfileMismatchVerdictHasItsOwnLabelAndAFullSentence() throws {
        guard case .refused(let label, let sentence) = BuildChannelGate.verdict(
            plistChannel: try agreedChannel(), featureProfileAgrees: false)
        else { return XCTFail("a feature-profile mismatch must refuse") }
        XCTAssertEqual(label, "feature-profile-mismatch")
        XCTAssertEqual(sentence, try XCTUnwrap(TCBuildChannel.mismatchLine()))
        XCTAssertNotEqual(sentence, label, "the person sees a sentence, not the label")
    }

    func testFeatureProfileMismatchRefusesTheDaemon() throws {
        let model = AppModel()
        model.start(plistChannel: try agreedChannel(), featureProfileAgrees: false)
        XCTAssertEqual(model.startup, .refused(try XCTUnwrap(TCBuildChannel.mismatchLine())))
        XCTAssertEqual(model.configDirectory, "")
    }

    func testFeatureProfileMismatchRefusesComputeAndClearsTheSnapshot() async throws {
        let model = ComputeModel()
        await model.start(plistChannel: try agreedChannel(), featureProfileAgrees: false)
        XCTAssertEqual(model.failureLabel, "feature-profile-mismatch")
        XCTAssertEqual(model.refusalSentence, try XCTUnwrap(TCBuildChannel.mismatchLine()))
        XCTAssertNil(model.requestedDirectory)
        XCTAssertNil(model.snapshot)
    }
}
