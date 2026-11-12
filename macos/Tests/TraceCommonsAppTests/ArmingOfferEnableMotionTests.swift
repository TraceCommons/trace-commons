import SwiftUI
import XCTest

@testable import TCShellCore
@testable import TraceCommonsApp

/// The arming offer answers its Enable (owner, 2026-11-12): the button
/// waits while the daemon arms, the mark turns to a check once it has, and
/// the card leaves a beat later instead of vanishing on the spot.
@MainActor
final class ArmingOfferEnableMotionTests: XCTestCase {
    private let offer = ArmingOffer(projectId: "p1", projectLabel: "api", contributedCount: 28)

    private static func source(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/" + path)
        return try String(contentsOf: url, encoding: .utf8)
    }

    /// A refresh while Enable is answered keeps the card the person
    /// pressed; otherwise the daemon's answer is the offer.
    func test_aRefreshLeavesTheCardWhileEnableIsAnswered() {
        let other = ArmingOffer(projectId: "p2", projectLabel: "web", contributedCount: 9)
        XCTAssertEqual(ArmingOfferMotion.offerAfterRefresh(current: offer, answered: nil, step: .arming), offer)
        XCTAssertEqual(ArmingOfferMotion.offerAfterRefresh(current: offer, answered: other, step: .armed), offer)
        XCTAssertNil(ArmingOfferMotion.offerAfterRefresh(current: offer, answered: nil, step: nil))
        XCTAssertEqual(ArmingOfferMotion.offerAfterRefresh(current: nil, answered: other, step: nil), other)
    }

    /// The bolt stays while the daemon arms and turns to a check once it
    /// has; Enable is pressed once.
    func test_theMarkTurnsToACheckOnlyOnceArmed() {
        XCTAssertEqual(ArmingOfferMotion.symbol(nil), "bolt")
        XCTAssertEqual(ArmingOfferMotion.symbol(.arming), "bolt")
        XCTAssertEqual(ArmingOfferMotion.symbol(.armed), "checkmark")
        XCTAssertFalse(ArmingOfferMotion.enableLocked(nil))
        XCTAssertTrue(ArmingOfferMotion.enableLocked(.arming))
        XCTAssertTrue(ArmingOfferMotion.enableLocked(.armed))
    }

    /// Reduce Motion: the card goes without moving the list.
    func test_reduceMotionLeavesWithoutMovement() {
        XCTAssertNil(ArmingOfferMotion.exit(reduceMotion: true))
        XCTAssertNotNil(ArmingOfferMotion.exit(reduceMotion: false))
        XCTAssertGreaterThan(ArmingOfferMotion.hold, .zero)
        XCTAssertLessThanOrEqual(ArmingOfferMotion.hold, .seconds(1.5), "the card lingers after it is answered")
    }

    /// With no daemon nothing is sent, so nothing is left waiting.
    func test_enableWithoutADaemonLeavesNoStep() {
        let model = AppModel()
        model.setArmingOfferForTesting(offer)
        model.acceptArmingOffer(offer)
        XCTAssertNil(model.armingOfferStep)
        XCTAssertEqual(model.armingOffer, offer)
    }

    /// The step is set before the call, cleared on a failure, armed on
    /// success, and the card cleared only after the hold.
    func test_enableSetsTheStepAroundTheCall() throws {
        let model = try Self.source("AppModel.swift")
        let start = try XCTUnwrap(model.range(of: "    func acceptArmingOffer(_ offer: ArmingOffer) {"))
        let end = try XCTUnwrap(model.range(of: "    func declineArmingOffer(", range: start.upperBound..<model.endIndex))
        let body = String(model[start.upperBound..<end.lowerBound])
        let arming = try XCTUnwrap(body.range(of: "armingOfferStep = .arming"))
        let call = try XCTUnwrap(body.range(of: "perform("))
        XCTAssertLessThan(arming.lowerBound, call.lowerBound)
        XCTAssertTrue(body.contains("onFailure: { self.armingOfferStep = nil }"))
        let armed = try XCTUnwrap(body.range(of: "self.armingOfferStep = .armed"))
        let hold = try XCTUnwrap(body.range(of: "Task.sleep(for: ArmingOfferMotion.hold)"))
        let clear = try XCTUnwrap(body.range(of: "self.clearArmingOffer()"))
        XCTAssertLessThan(armed.lowerBound, hold.lowerBound)
        XCTAssertLessThan(hold.lowerBound, clear.lowerBound)
    }

    /// Both cards lock Enable while it is answered, and the popover's mark
    /// follows the step.
    func test_bothCardsFollowTheStep() throws {
        let notices = try Self.source("Views/Monitor/MonitorNotices.swift")
        XCTAssertTrue(notices.contains("step: model.armingOfferStep"))
        XCTAssertTrue(notices.contains("systemImage: ArmingOfferMotion.symbol(step)"))
        let offers = try Self.source("Views/Monitor/TracesOffers.swift")
        let prompts = try Self.source("Views/Monitor/InspectorPrompts.swift")
        XCTAssertTrue(prompts.contains("step: model.armingOfferStep"))
        for text in [notices, offers] {
            XCTAssertTrue(text.contains(".disabled(ArmingOfferMotion.enableLocked(step))"))
        }
    }
}
