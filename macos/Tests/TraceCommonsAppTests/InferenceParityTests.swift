#if DEBUG
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The Inference tab against the legacy Private AI destination: the
/// switch, sign-in and tools keep every binding, gate and core sentence.
@MainActor
final class InferenceParityTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// A write is taken only when the core's rule confirms it: the marker
    /// echoed and the switch where it was asked to be.
    func test_aWriteIsConfirmedByTheCoresRule() {
        let state = DaemonData.PrivateInferenceState(state: "running", port: 4100)
        let echoed = DaemonData.PrivateAISwitch(on: true, offerSeen: true, state: state)
        var asked: (Bool?, Bool?, Bool?)?
        XCTAssertTrue(InferenceStore.confirmed(echoed, requested: true) { asked = ($0, $1, $2); return true })
        XCTAssertEqual(asked?.0, true)
        XCTAssertEqual(asked?.1, true)
        XCTAssertEqual(asked?.2, true)
        XCTAssertFalse(InferenceStore.confirmed(echoed, requested: true) { _, _, _ in false })
    }

    /// The rule is asked (requested, echoed marker, echoed switch), in that
    /// order: an asymmetric reply tells a swapped pair apart.
    func test_theRuleIsAskedInItsOwnOrder() {
        let echoed = DaemonData.PrivateAISwitch(on: false, offerSeen: true, state: nil)
        var asked: (Bool?, Bool?, Bool?)?
        _ = InferenceStore.confirmed(echoed, requested: false) { asked = ($0, $1, $2); return true }
        XCTAssertEqual(asked?.0, false)
        XCTAssertEqual(asked?.1, true)
        XCTAssertEqual(asked?.2, false)
    }

    /// Through the core's real rule: a confirmed write (off, which a
    /// swapped argument order would refuse) is taken and clears any refusal.
    func test_aConfirmedWriteIsTakenThroughTheRealRule() async {
        let store = InferenceStore(client: SampleDaemonClient(.normalDay))
        await store.setPrivateAI(on: false, unconfirmed: "refused")
        XCTAssertEqual(store.privateAI?.on, false)
        XCTAssertNil(store.privateAIRefusal)
        XCTAssertFalse(store.privateAIBusy)
        await store.setPrivateAI(on: true, unconfirmed: "refused")
        XCTAssertEqual(store.privateAI?.on, true)
        XCTAssertNil(store.privateAIRefusal)
    }

    /// A write the daemon never answered is refused in the core's words
    /// and never reads as on.
    func test_anUnansweredWriteIsRefusedAndNeverReadsAsOn() async {
        let store = InferenceStore(client: SampleDaemonClient(.coreDown))
        await store.setPrivateAI(on: true, unconfirmed: "refused")
        XCTAssertEqual(store.privateAIRefusal, "refused")
        XCTAssertNil(store.privateAI)
        XCTAssertFalse(store.privateAIBusy)
        store.dismissPrivateAIRefusal()
        XCTAssertNil(store.privateAIRefusal)

        let detached = InferenceStore(client: nil)
        await detached.setPrivateAI(on: true, unconfirmed: "refused")
        XCTAssertEqual(detached.privateAIRefusal, "refused")
        XCTAssertNil(detached.privateAI)
    }

    /// The switch is read with the rest of the tab, and a new client
    /// forgets the old one's switch and refusal.
    func test_theSwitchIsReadOnLoadAndForgottenOnAttach() async {
        let store = InferenceStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertNotNil(store.privateAI)
        store.attach(nil)
        XCTAssertNil(store.privateAI)
        XCTAssertNil(store.privateAIRefusal)
    }

    /// A switch nobody could read is never drawn on, and the state comes
    /// from the tone, never from the switch.
    func test_theSwitchFailsClosedAndTheStateReadsTheTone() throws {
        let source = try Self.text("Views/PrivateInferenceView.swift")
        for needle in ["copy.offerWhat", "copy.offerExposure", "copy.settingsToggle", "copy.settingsAppliesAtOnce",
                       "PrivateInferenceSurface.stateLine(", "PrivateInferenceSurface.tone(", "PrivateInferenceSurface.servingLine(",
                       "isOn ?? false", "busy || isOn == nil", "GlassToggleStyle(.settings)",
                       "CredentialSection(copy: copy, prominent: true)", "HarnessListSection(copy: copy)",
                       "GlassExpander(copy.settingsTitle, isOpen:",
                       "GlassNotice(tone: .outside, title: refusal)",
                       "Button(ActionMessageBanner.coreDismissWord ?? ActionMessageBanner.dismissWord, action: onDismiss)"] {
            XCTAssertTrue(source.contains(needle), "PrivateInferenceView.swift lacks \(needle)")
        }
        XCTAssertFalse(source.contains("privateInferenceOn ? .on"), "the state must come from the tone, not the switch")
        XCTAssertFalse(source.contains("static func palette("), "the TC palette moved to QueueView.swift")
        XCTAssertFalse(source.contains("ActionMessageBanner(text:"), "the refusal is the card's, not a legacy banner")
        XCTAssertEqual(PrivateInferenceIndicator.status(.clear), .on)
        for tone in [PrivateInferenceTone.held, .attention, .refused, .neutral] {
            XCTAssertNotEqual(PrivateInferenceIndicator.status(tone), .on, "\(tone) must never read as working")
        }
        try LegacySymbols.assertClean("Views/PrivateInferenceView.swift")
    }

    /// The legacy window draws the same card, bound to the model's switch,
    /// its busy flag, its write and its refusal.
    func test_theLegacyDestinationDrawsTheCardOnTheModel() throws {
        let source = try Self.text("Views/PrivateInferenceView.swift")
        for needle in ["isOn: model.daemonSettings?.privateInference,", "state: model.privateInferenceState,",
                       "busy: model.privateInferenceBusy,", "refusal: model.lastActionError,",
                       "onSet: model.applyPrivateInference,", "onDismiss: { model.lastActionError = nil }"] {
            XCTAssertTrue(source.contains(needle), "PrivateInferenceContent lacks \(needle)")
        }
    }

    func test_theStoreWritesThroughTheDataContract() throws {
        let store = try Self.text("Views/Monitor/InferenceStore.swift")
        for needle in ["$0.privateAI()", "client.setPrivateAI(on: on)", "check: TCPrivateInference.writeConfirmed)",
                       "privateAIRefusal = unconfirmed", "async let privateAI: Void = loadPrivateAI()"] {
            XCTAssertTrue(store.contains(needle), "InferenceStore.swift lacks \(needle)")
        }
    }

    /// The window's Inference dot and the card map the tone one way.
    func test_theDotReadsTheIndicator() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains(
            "return PrivateInferenceIndicator.status(PrivateInferenceSurface.tone(state, calls: calls))"))
    }
}
#endif
