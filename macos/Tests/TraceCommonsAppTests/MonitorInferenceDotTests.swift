#if DEBUG
import TCBridge
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The monitor's Inference dot says what the listener is doing, from the
/// daemon's report, never what the switch asked for: a switch that is on
/// over a listener that refused to start is not drawn as on. And the dot is
/// not colour-only: the tab carries the core's state sentence.
///
/// One state is one colour on every Private AI dot (owner, 2026-10-29: the
/// tab was red while the heading was an amber "On"): the tab, the map's
/// segment, the heading's Status row, the menu bar and the Settings switch
/// card all read the core's menu tone.
@MainActor
final class MonitorInferenceDotTests: XCTestCase {
    /// The core's own table, as `AppModel` wires it.
    private let calls = PrivateInferenceCalls(
        stateLine: { TCPrivateInference.stateLine(state: $0) },
        stateTone: { TCPrivateInference.stateTone(state: $0) },
        servingLine: { TCPrivateInference.servingLine(port: $0) },
        shouldOffer: { TCPrivateInference.shouldOffer(answered: $0, on: $1) },
        quitNeedsNotice: { TCPrivateInference.quitNeedsNotice(on: $0, state: $1) },
        menuWord: { TCPrivateInference.menuWord(state: $0) },
        menuTone: { TCPrivateInference.menuTone(state: $0) })

    private static let labels = [
        "off", "stopping", "running", "running_no_backends", "running_answered_elsewhere",
        "running_destination_unknown", "running_elsewhere", "port_in_use", "start_failed", "crashed",
        "a_label_from_a_later_daemon",
    ]

    private func dot(_ label: String) -> GlassStatus? {
        MonitorWindowView.inferenceDot(PrivateInferenceState(label: label, port: nil), calls: calls)
    }

    private func coreCopy() throws -> PrivateInferenceCopy {
        try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? ""))
    }

    private func source(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// Green only while NEAR AI answers; an "On" with something to look at
    /// (signed out of NEAR AI, answered elsewhere, unproven) is amber; a
    /// listener that is not working is red; off and stopping are grey.
    func test_onlyAServingListenerIsDrawnOn() {
        XCTAssertEqual(dot("running"), .on)
        for label in ["running_answered_elsewhere", "running_no_backends", "running_destination_unknown",
                      "running_elsewhere"] {
            XCTAssertEqual(dot(label), .ask, label)
        }
        for label in ["port_in_use", "start_failed", "crashed"] {
            XCTAssertEqual(dot(label), .outside, label)
        }
        XCTAssertEqual(dot("off"), .off)
        XCTAssertEqual(dot("stopping"), .off)
        XCTAssertEqual(dot("a_label_from_a_later_daemon"), .off)
    }

    /// No report is no dot: unknown is neither on nor off.
    func test_anUnreportedStateDrawsNoDot() {
        XCTAssertNil(MonitorWindowView.inferenceDot(PrivateInferenceState(label: "", port: nil), calls: calls))
        XCTAssertNil(MonitorWindowView.inferenceDot(nil, calls: calls))
    }

    /// The tab's text equivalent is the heading's help and hint sentence.
    func test_theDotHasTheCoresSentenceAsItsTextEquivalent() throws {
        let copy = try coreCopy()
        for label in Self.labels {
            XCTAssertEqual(
                MonitorWindowView.inferenceDotDescription(PrivateInferenceState(label: label, port: nil), calls: calls),
                PrivateAIInspectorView.statusRow(.init(state: label, port: nil), copy: copy, calls: calls).sentence,
                label)
        }
        XCTAssertNil(MonitorWindowView.inferenceDotDescription(nil, calls: calls))
    }

    /// Every reported label paints the same colour on the tab and the map
    /// (one function), the heading's Status row, the menu bar and the
    /// Settings switch card.
    func test_everyPrivateAIDotPaintsOneStateOneColour() throws {
        let copy = try coreCopy()
        for label in Self.labels {
            let state = PrivateInferenceState(label: label, port: nil)
            let tab = try XCTUnwrap(MonitorWindowView.inferenceDot(state, calls: calls), label)
            let heading = PrivateAIInspectorView.statusRow(.init(state: label, port: nil), copy: copy, calls: calls).dot
            let menuBar = MenuPanelStatus.privateAIDot(
                MenuPanelStatus.privateAI(on: true, menu: PrivateInferenceSurface.menuState(state, calls: calls)))
            let card = PrivateAISwitchCard.stateLabel(state: state, copy: copy, calls: calls).status
            XCTAssertEqual(tab, heading, "\(label): the tab and the heading disagree")
            XCTAssertEqual(tab, menuBar, "\(label): the tab and the menu bar disagree")
            XCTAssertEqual(tab, card, "\(label): the tab and the switch card disagree")
        }
    }

    /// Signing in to NEAR AI moves the listener from answered elsewhere to
    /// running: the tab and the heading turn green together, and signing
    /// out turns both amber again.
    func test_signingInToNearAIMovesTheTabAndTheHeadingTogether() throws {
        let copy = try coreCopy()
        func both(_ label: String) -> (tab: GlassStatus?, heading: GlassStatus) {
            (dot(label), PrivateAIInspectorView.statusRow(.init(state: label, port: 4000), copy: copy, calls: calls).dot)
        }
        let signedOut = both("running_answered_elsewhere")
        let signedIn = both("running")
        XCTAssertEqual(signedOut.tab, .ask)
        XCTAssertEqual(signedOut.heading, .ask)
        XCTAssertEqual(signedIn.tab, .on)
        XCTAssertEqual(signedIn.heading, .on)
    }

    /// The tab and the map read the window's store, the heading's own
    /// source; the stores re-read when the app's settings read sees Private
    /// AI move; and a status change re-reads the settings.
    func test_theTabTheMapAndTheHeadingReadOneSource() throws {
        let window = try source("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("inference.privateAI.map { InferenceAccountSection.surfaceState($0.state) }"))
        XCTAssertTrue(window.contains("inferenceDot: Self.inferenceDot(privateAIState, calls: model.privateInferenceCalls)"))
        XCTAssertTrue(window.contains("privateAIDot: Self.inferenceDot(privateAIState, calls: model.privateInferenceCalls)"))
        XCTAssertFalse(window.contains("inferenceDot(model.daemonSettings"),
                       "the tab reads the app's settings snapshot again while the heading reads the store")
        XCTAssertTrue(window.contains(
            ".onChange(of: model.daemonSettings?.privateInferenceState) { _, _ in\n            Task { await inference.reloadPrivateAI() }"))
        let settings = try source("Views/Settings/PrivateAISection.swift")
        XCTAssertTrue(settings.contains(
            ".onChange(of: model.daemonSettings?.privateInferenceState) { _, _ in\n            Task { await store.reloadPrivateAI() }"))
        let model = try source("AppModel.swift")
        let statusArm = try XCTUnwrap(model.components(separatedBy: "        case .statusChanged:\n").dropFirst().first)
        let arm = statusArm.components(separatedBy: "        case ").first ?? ""
        XCTAssertTrue(arm.contains("refreshSettings()"),
                      "a status change leaves the menu bar's Private AI dot on the old settings read")
    }
}
#endif
