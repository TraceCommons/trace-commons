#if DEBUG
import TCBridge
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
                       "let label = Self.stateLabel(state: state, copy: copy, calls: calls)",
                       "GlassStatusLabel(label.line, status: label.status)",
                       "CredentialSection(copy: copy, prominent: true)", "HarnessListSection(copy: copy)",
                       "GlassExpander(copy.settingsTitle, isOpen:",
                       "GlassNotice(tone: .outside, title: refusal)",
                       "Button(ActionMessageBanner.coreDismissWord ?? ActionMessageBanner.dismissWord, action: onDismiss)"] {
            XCTAssertTrue(source.contains(needle), "PrivateInferenceView.swift lacks \(needle)")
        }
        XCTAssertFalse(source.contains("privateInferenceOn ? .on"), "the state must come from the tone, not the switch")
        XCTAssertEqual(source.components(separatedBy: "GlassStatusLabel(").count - 1, 1,
                       "the card's one state label is the only status drawn")
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

    /// The core's own copy and calls, as `AppModel` wires them.
    private static let coreCalls = PrivateInferenceCalls(
        stateLine: { TCPrivateInference.stateLine(state: $0) },
        stateTone: { TCPrivateInference.stateTone(state: $0) },
        servingLine: { TCPrivateInference.servingLine(port: $0) },
        shouldOffer: { TCPrivateInference.shouldOffer(answered: $0, on: $1) },
        quitNeedsNotice: { TCPrivateInference.quitNeedsNotice(on: $0, state: $1) })

    /// The state label reads the listener's report alone: a refusal asks,
    /// an unreported state is never on, and a line the core could not give
    /// falls back to its unknown sentence.
    func test_theStateLabelReadsTheReportNeverTheSwitch() throws {
        let copy = try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? ""))
        let refused = PrivateAISwitchCard.stateLabel(
            state: PrivateInferenceState(label: "port_in_use", port: nil), copy: copy, calls: Self.coreCalls)
        XCTAssertEqual(refused.status, .ask)
        XCTAssertEqual(refused.line, TCPrivateInference.stateLine(state: "port_in_use"))
        let unreported = PrivateAISwitchCard.stateLabel(
            state: PrivateInferenceState(label: "", port: nil), copy: copy, calls: Self.coreCalls)
        XCTAssertNotEqual(unreported.status, .on)
        XCTAssertFalse(unreported.line.isEmpty)
        let silent = PrivateInferenceCalls(
            stateLine: { _ in nil }, stateTone: { _ in 0 }, servingLine: { _ in nil },
            shouldOffer: { _, _ in false }, quitNeedsNotice: { _, _ in false })
        let unknown = PrivateAISwitchCard.stateLabel(
            state: PrivateInferenceState(label: "", port: nil), copy: copy, calls: silent)
        XCTAssertEqual(unknown.line, copy.stateUnknown)
        XCTAssertNotEqual(unknown.status, .on)
    }

    /// A reply the daemon gave but the core's rule refused is not taken:
    /// the refusal shows and the switch is what the re-read says.
    func test_aReplyTheRuleRefusesIsNotTaken() async throws {
        let client = SampleDaemonClient(.normalDay)
        let store = InferenceStore(client: client)
        await store.load()
        let before = try XCTUnwrap(store.privateAI)
        let asked = !(before.on ?? false)
        await store.setPrivateAI(on: asked, unconfirmed: "refused") { _, _, _ in false }
        XCTAssertEqual(store.privateAIRefusal, "refused")
        XCTAssertFalse(store.privateAIBusy)
        let reread = try await client.privateAI()
        XCTAssertEqual(store.privateAI, reread)
        XCTAssertNil(store.failures["set_private_ai"])
    }

    /// A write that threw is recorded by method.
    func test_aThrownWriteIsRecorded() async {
        let store = InferenceStore(client: SampleDaemonClient(.coreDown))
        await store.setPrivateAI(on: true, unconfirmed: "refused")
        XCTAssertEqual(store.failures["set_private_ai"], .unreachable)
    }

    /// A read that started before a confirmed write never undoes it.
    func test_aReadOlderThanAWriteDoesNotLand() async {
        let store = InferenceStore(client: SampleDaemonClient(.normalDay))
        let stale = DaemonData.PrivateAISwitch(on: true, offerSeen: true, state: nil)
        let startedAt = store.beginPrivateAIRead()
        await store.setPrivateAI(on: false, unconfirmed: "refused")
        XCTAssertEqual(store.privateAI?.on, false)
        store.landPrivateAIRead(stale, startedAt: startedAt)
        XCTAssertEqual(store.privateAI?.on, false, "a read from before the write drew over it")
        store.landPrivateAIRead(stale, startedAt: store.beginPrivateAIRead())
        XCTAssertEqual(store.privateAI?.on, true, "a read after the write lands")
        store.landPrivateAIRead(nil, startedAt: store.beginPrivateAIRead())
        XCTAssertEqual(store.privateAI?.on, true, "a failed read keeps the last value")
    }

    func test_theStoreWritesThroughTheDataContract() throws {
        let store = try Self.text("Views/Monitor/InferenceStore.swift")
        for needle in ["$0.privateAI()", "client.setPrivateAI(on: on)", "check: (Bool?, Bool?, Bool?) -> Bool = TCPrivateInference.writeConfirmed",
                       "Self.confirmed(reply, requested: on, check: check)",
                       "privateAIRefusal = unconfirmed", "async let privateAI: Void = loadPrivateAI()"] {
            XCTAssertTrue(store.contains(needle), "InferenceStore.swift lacks \(needle)")
        }
    }

    /// Sign-in keeps every decision the core makes for it, its four writes,
    /// its poll and its busy gate, and the balance and funding rows it hosts.
    func test_signInKeepsItsDecisionsAndItsPoll() throws {
        let source = try Self.text("Views/CredentialSection.swift")
        for needle in ["CredentialSurface.tone(", "CredentialSurface.action(", "CredentialSurface.stateLine(",
                       "CredentialSurface.actionExplains(", "CredentialSurface.actionLabel(", "BalanceSurface.actionToDraw(",
                       "Self.providerOptions(copy)", "copy.credentialWalletNotice", "model.startNearAiCredential(provider:",
                       "model.cancelNearAiCredential()", "model.forgetNearAiCredential()", "model.migrateNearAiCredential()",
                       "model.refreshNearAiCredential()", "pollInterval", "model.credentialBusy",
                       "BalanceRow(copy: copy, credentialAction: action, run: run)", "FundingRow(copy: copy)"] {
            XCTAssertTrue(source.contains(needle), "CredentialSection.swift lacks \(needle)")
        }
        for file in ["Views/CredentialSection.swift", "Views/BalanceRow.swift", "Views/FundingRow.swift", "Views/NearAiJoinView.swift"] {
            try LegacySymbols.assertClean(file)
        }
    }

    /// The card's state is the core's tone on a worded glass label, the
    /// provider chooser writes the caller's binding and is named on screen,
    /// and no control is drawn without words.
    func test_signInDrawsTheToneWithWordsAndNamesTheChooser() throws {
        let source = try Self.text("Views/CredentialSection.swift")
        for needle in ["GlassStatusLabel(\n                CredentialSurface.stateLine(status, copy: copy, calls: model.credentialCalls),\n                status: PrivateInferenceIndicator.status(tone))",
                       "set: { if let value = $0 { selection.wrappedValue = value } }",
                       "Text(copy.credentialProviderLabel)",
                       "GlassPicker(\n                        copy.credentialProviderLabel,",
                       "placeholder: copy.credentialProviderLabel)",
                       "Text(copy.credentialProviderLabel)\n"
                           + "                        .glassType(GlassTokens.TypeScale.label)\n"
                           + "                        .foregroundStyle(GlassColor.textSecondary)\n"
                           + "                        .accessibilityHidden(true)",
                       ".buttonStyle(GlassButtonStyle(prominent && action == .obtain ? .primary : .glass))\n"
                           + "                .disabled(model.credentialBusy)",
                       // The poll sits on the card's always-present stack,
                       // never on a branch that may not be drawn.
                       "            }\n        }\n        .task(id: action) {\n",
                       "while action == .cancel, !Task.isCancelled {",
                       "try? await Task.sleep(for: Self.pollInterval)"] {
            XCTAssertTrue(source.contains(needle), "CredentialSection.swift lacks \(needle)")
        }
        XCTAssertEqual(source.components(separatedBy: ".task(").count - 1, 1, "the card has exactly one poll")
        XCTAssertFalse(source.contains("palette("), "the card reads the glass status, not the TC palette")
        let balance = try Self.text("Views/BalanceRow.swift")
        for needle in ["GlassStatusLabel(sentence, status: PrivateInferenceIndicator.status(tone))",
                       "BalanceSurface.showsFigures(status, calls: model.balanceCalls)", "Text(copy.balanceWhat)",
                       "CredentialSurface.actionLabel(action, copy: copy)", "Button(label) { run(action) }",
                       ".disabled(model.credentialBusy)"] {
            XCTAssertTrue(balance.contains(needle), "BalanceRow.swift lacks \(needle)")
        }
        XCTAssertFalse(balance.contains("palette("), "the balance reads the glass status, not the TC palette")
        let funding = try Self.text("Views/FundingRow.swift")
        for needle in ["Text(status?.view.message ?? copy.fundingUnavailable)", "Text(copy.fundingWhat)",
                       "Text(status?.destination == nil ? copy.fundingRefresh : copy.fundingManage)",
                       ".disabled(model.credentialBusy || request != nil)",
                       ".onReceive(model.$credentialBusy.removeDuplicates().dropFirst())",
                       ".onReceive(model.$credentialStatus.map(\\.sessionState).removeDuplicates().dropFirst())"] {
            XCTAssertTrue(funding.contains(needle), "FundingRow.swift lacks \(needle)")
        }
    }

    /// The commons field is drawn only under its own word, and Join only
    /// beside that field: with the wallet copy absent there is neither, so
    /// no dead control and no value sent from a field nobody can see.
    func test_theCommonsFieldIsNeverWordless() throws {
        let source = try Self.text("Views/NearAiJoinView.swift")
        XCTAssertTrue(source.contains("let commonsLabel = model.witnessCopy?.wallet?.commons\n"))
        XCTAssertTrue(source.contains("if let commonsLabel {\n"
            + "                        GlassTextField(commonsLabel, text: $commons)"))
        XCTAssertTrue(source.contains("} else if signedIn {\n"
            + "                        if commonsLabel != nil {\n"
            + "                            Button(copy.nearAiEnrollAction) { join() }"))
        XCTAssertEqual(source.components(separatedBy: "wallet?.commons").count - 1, 1,
                       "the field and Join read one word")
        XCTAssertFalse(source.contains("wallet?.commons ?? \"\""), "an absent word must not draw an unlabelled field")
    }

    /// The window's Inference dot and the card map the tone one way.
    func test_theDotReadsTheIndicator() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains(
            "return PrivateInferenceIndicator.status(PrivateInferenceSurface.tone(state, calls: calls))"))
    }
}
#endif
