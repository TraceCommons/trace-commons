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

    /// No write without the preview, and a first connect asks the exposure
    /// question first; dismissing either sheet is saying no.
    func test_connectingAToolKeepsBothQuestions() throws {
        let source = try Self.text("Views/HarnessListView.swift")
        for needle in ["model.beginHarnessAction(id: row.id, action: action)", "model.answerHarnessExposure(accepted: false)",
                       "model.answerHarnessExposure(accepted: true)", "model.cancelHarnessPreview()", "model.confirmHarnessPreview()",
                       "HarnessSurface.canCommit(", "HarnessSurface.outcomeSentence(", "HarnessSurface.occupiedSentence(",
                       "CredentialSurface.harnessNotice(", "HarnessSurface.spendSentence(", "copy.harnessesSpendScope",
                       "copy.harnessesNoneFound", "copy.offerExposure", ".keyboardShortcut(.cancelAction)", "model.harnessBusy"] {
            XCTAssertTrue(source.contains(needle), "HarnessListView.swift lacks \(needle)")
        }
        XCTAssertFalse(source.contains("copy.offerAskedOnce"), "the asked-once sentence is false on this surface")
        try LegacySymbols.assertClean("Views/HarnessListView.swift")
    }

    /// Both sheets hang off the section's always-present stack, are shown
    /// exactly while the model holds their question, and dismissing one is
    /// its no. Deleting the exposure sheet would leave every call above in
    /// the file and connect with no question asked; these pins catch it.
    func test_bothSheetsArePresentedFromTheAlwaysPresentStack() throws {
        let source = try Self.text("Views/HarnessListView.swift")
        for needle in ["            }\n        }\n"
                           + "        .sheet(isPresented: exposureBinding) {\n"
                           + "            HarnessExposureSheet(copy: copy)\n"
                           + "        }\n"
                           + "        .sheet(isPresented: previewBinding) {\n"
                           + "            if let plan = model.harnessPreview {\n"
                           + "                HarnessPreviewSheet(plan: plan, copy: copy)\n",
                       "get: { model.harnessExposureRequest != nil },\n"
                           + "            set: { if !$0 { model.answerHarnessExposure(accepted: false) } })",
                       "get: { model.harnessPreview != nil },\n"
                           + "            set: { if !$0 { model.cancelHarnessPreview() } })"] {
            XCTAssertTrue(source.contains(needle), "HarnessListView.swift lacks \(needle)")
        }
        XCTAssertEqual(source.components(separatedBy: ".sheet(").count - 1, 2, "exactly the two questions")
        XCTAssertFalse(source.contains(".task(") || source.contains(".onAppear") || source.contains(".onReceive("),
                       "the list is refreshed by the model, not by a view that may not be drawn")
    }

    /// Each sheet's words are the core's, verbatim and in full; every
    /// answer keeps its keyboard role and its busy gate; Confirm exists
    /// only for a committable plan.
    func test_theSheetsCarryTheCoresWordsAndEveryGate() throws {
        let source = try Self.text("Views/HarnessListView.swift")
        // The exposure body is pinned whole, in order, in
        // `test_theExposureBodyAndTheToolDetailsArePinnedWhole`.
        for needle in ["Button(copy.offerDecline) { model.answerHarnessExposure(accepted: false) }\n"
                           + "                    .buttonStyle(GlassButtonStyle(.glass))\n"
                           + "                    .keyboardShortcut(.cancelAction)\n",
                       "Button(copy.offerAccept) { model.answerHarnessExposure(accepted: true) }\n"
                           + "                    .buttonStyle(GlassButtonStyle(.primary))\n"
                           + "                    .keyboardShortcut(.defaultAction)\n"
                           + "                    .disabled(model.harnessBusy)\n",
                       "GlassSheet(title: copy.harnessPreviewTitle) {\n",
                       "Button(copy.harnessPreviewCancel) { model.cancelHarnessPreview() }\n"
                           + "                    .buttonStyle(GlassButtonStyle(.glass))\n"
                           + "                    .keyboardShortcut(.cancelAction)\n",
                       "if HarnessSurface.canCommit(plan, calls: model.harnessCalls) {\n"
                           + "                    Button(copy.harnessPreviewConfirm) { model.confirmHarnessPreview() }\n"
                           + "                        .buttonStyle(GlassButtonStyle(.primary))\n"
                           + "                        .keyboardShortcut(.defaultAction)\n"
                           + "                        .disabled(model.harnessBusy)\n",
                       "if !plan.occupied.isEmpty {\n"
                           + "                Text(HarnessSurface.occupiedSentence(copy: copy))",
                       "ForEach(Array(plan.changes.enumerated()), id: \\.offset) { _, change in\n"
                           + "                Text(change)",
                       ".frame(minWidth: 460)"] {
            XCTAssertTrue(source.contains(needle), "HarnessListView.swift lacks \(needle)")
        }
        XCTAssertEqual(source.components(separatedBy: ".frame(minWidth: 460)").count - 1, 2, "both sheets keep their width")
        XCTAssertEqual(source.components(separatedBy: "Button(").count - 1, 5,
                       "the row's action and the four sheet answers, and nothing that would take a slot over")
    }

    /// A row's state is the shared tone on a worded label, the only primary
    /// is connect, the action carries the tool's name and waits on busy, and
    /// the details open in place.
    func test_aToolRowDrawsItsStateInWordsAndOneAction() throws {
        let source = try Self.text("Views/HarnessListView.swift")
        for needle in ["GlassSectionRule(copy.harnessesTitle)",
                       "GlassStatusLabel(sentence, status: PrivateInferenceIndicator.status(HarnessSurface.tone(state)))",
                       "Button(HarnessSurface.actionLabel(action, copy: copy)) {\n"
                           + "                model.beginHarnessAction(id: row.id, action: action)\n"
                           + "            }\n"
                           + "            .buttonStyle(GlassButtonStyle(action == .connect ? .primary : .glass))\n"
                           + "            .accessibilityLabel(Text(row.name) + Text(verbatim: \": \") + Text(HarnessSurface.actionLabel(action, copy: copy)))\n"
                           + "            .disabled(model.harnessBusy)\n",
                       "if let tool = HarnessToolArt.tool(harness: row.id) {\n"
                           + "                GlassToolTile(.tool(tool))\n"
                           + "            } else {\n"
                           + "                Image(systemName: \"terminal\")\n"
                           + "                    .glassGlyph(14)",
                       "GlassExpander(copy.harnessPreviewTitle, isOpen: $settingsExpanded)\n"
                           + "                if settingsExpanded {\n",
                       "HarnessSurface.restartSentence(row, state: state, copy: copy)",
                       "HarnessSurface.lastCallSentence(row, calls: model.harnessCalls)",
                       "HarnessSurface.rowSentence(\n                    row, copy: copy, calls: model.harnessCalls)",
                       ".glassType(GlassTokens.TypeScale.mono)"] {
            XCTAssertTrue(source.contains(needle), "HarnessListView.swift lacks \(needle)")
        }
        XCTAssertFalse(source.contains("palette("), "the row reads the glass status, not the TC palette")
        XCTAssertEqual(source.components(separatedBy: "GlassStatusLabel(").count - 1, 1, "one worded state per row")
        // One mapping: the artwork lives here and the map forwards to it.
        XCTAssertFalse(source.contains("FlowMapScene"), "the list must not read the flow map's copy of the art")
        let map = try Self.text("Views/Monitor/FlowMapScene.swift")
        XCTAssertTrue(map.contains("static func glassTool(harness id: String) -> GlassTool? {\n"
            + "        HarnessToolArt.tool(harness: id)\n    }"), "one mapping, forwarded")
        XCTAssertEqual(HarnessToolArt.tool(harness: "claude"), .claudeCode)
        XCTAssertNil(HarnessToolArt.tool(harness: "something-new"))
    }

    /// Fail closed: only a call that arrived reads as working, a state a
    /// later daemon grows is unknown, and a list nobody answered draws the
    /// core's unknown word, never "none found".
    func test_aToolNeverReadsAsWorkingWithoutACall() throws {
        XCTAssertEqual(PrivateInferenceIndicator.status(HarnessSurface.tone(.answering)), .on)
        for state in [HarnessState.unknown, .notConnected, .connectedNoCalls, .activityShared] {
            XCTAssertNotEqual(PrivateInferenceIndicator.status(HarnessSurface.tone(state)), .on,
                              "\(state) must never read as working")
        }
        XCTAssertEqual(HarnessState.fromABI(999), .unknown)
        let source = try Self.text("Views/HarnessListView.swift")
        XCTAssertTrue(source.contains("if model.harnesses == HarnessList.none {\n"
            + "                RouteDisclosureUnreadableGlassLine(line: nil)\n"
            + "            } else if model.harnesses.harnesses.isEmpty {\n"
            + "                Text(copy.harnessesNoneFound)"),
                      "an unanswered list must not read as an empty one")
    }

    func test_theInspectorCarriesTheAccountTheSwitchAndTheTools() throws {
        let views = try Self.text("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(views.contains("InferenceAccountSection(store: store)"))
        for needle in ["case .needsRoots", "OnboardingCoordinatorView(startAt: .folders, takesInvites: false, offersJoin: false", "case .refused(let",
                       "GlassHealthBanner(banner:", "model.refreshAll()"] {
            XCTAssertTrue(views.contains(needle), "InferenceViews.swift lacks \(needle)")
        }
        let account = try Self.text("Views/Monitor/InferenceAccount.swift")
        for needle in ["CredentialSection(copy: copy, prominent: true)", "HarnessListSection(copy: copy)",
                       "PrivateAISwitchCard(", "store.setPrivateAI(on:", "copy.writeUnconfirmed", "model.refreshSettings()",
                       "store.privateAI?.on", "model.privateInferenceCopy"] {
            XCTAssertTrue(account.contains(needle), "InferenceAccount.swift lacks \(needle)")
        }
        XCTAssertFalse(account.contains("applyPrivateInference("), "the glass switch writes through the data contract")
    }

    /// The glass switch is the legacy card on the store: drawn only under
    /// the core's copy (no destination without its exposure sentence), its
    /// write refused in the core's decoded words (never nil), the model's
    /// settings re-read after, and its refusal dismissable.
    func test_theGlassSwitchIsTheCardOnTheStore() throws {
        let account = try Self.text("Views/Monitor/InferenceAccount.swift")
        for needle in ["        if let copy = model.privateInferenceCopy {\n"
                           + "            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {\n"
                           + "                GlassCard { CredentialSection(copy: copy, prominent: true) }\n"
                           + "                GlassCard { HarnessListSection(copy: copy) }\n"
                           + "                PrivateAISwitchCard(\n",
                       "isOn: store.privateAI?.on,\n",
                       "state: Self.surfaceState(store.privateAI?.state),\n",
                       "calls: model.privateInferenceCalls,\n",
                       "busy: store.privateAIBusy,\n",
                       "refusal: store.privateAIRefusal,\n",
                       "await store.setPrivateAI(on: on, unconfirmed: copy.writeUnconfirmed)\n"
                           + "                            model.refreshSettings()\n",
                       "onDismiss: { store.dismissPrivateAIRefusal() })"] {
            XCTAssertTrue(account.contains(needle), "InferenceAccount.swift lacks \(needle)")
        }
        XCTAssertFalse(account.contains("truncatingIfNeeded"), "a port out of range is unknown, never another port")
        XCTAssertEqual(account.components(separatedBy: "PrivateAISwitchCard(").count - 1, 1)
        try LegacySymbols.assertClean("Views/Monitor/InferenceAccount.swift")
        XCTAssertFalse(account.hasPrefix("#if DEBUG"), "the Monitor is the release default (R15)")
    }

    /// The daemon's listener report onto the card's state: the label as
    /// the daemon sent it, unreported as the empty label, and a port that
    /// is not a port as no port (truncating would name a wrong one).
    func test_theListenerReportConvertsWithoutInventingAPort() {
        let running = InferenceAccountSection.surfaceState(.init(state: "running", port: 4100))
        XCTAssertEqual(running, PrivateInferenceState(label: "running", port: 4100))
        XCTAssertEqual(InferenceAccountSection.surfaceState(.init(state: "running", port: 70_000)),
                       PrivateInferenceState(label: "running", port: nil))
        XCTAssertEqual(InferenceAccountSection.surfaceState(.init(state: "running", port: -1)),
                       PrivateInferenceState(label: "running", port: nil))
        XCTAssertEqual(InferenceAccountSection.surfaceState(.init(state: "port_in_use", port: nil)),
                       PrivateInferenceState(label: "port_in_use", port: nil))
        XCTAssertEqual(InferenceAccountSection.surfaceState(nil), PrivateInferenceState(label: "", port: nil))
    }

    /// The tab reads the daemon's startup as the legacy destination did:
    /// the first run's Folders step when folders are owed (outside the ledger's scroll,
    /// so it never nests one), a spinner while starting, the core's down
    /// title over the refusal's sentence, the ledger only while running;
    /// and the refresh sits on the always-present stack. The inspector's
    /// account, whose controls need the daemon, is drawn only while running.
    func test_theTabGatesOnTheDaemonsStartup() throws {
        let views = try Self.text("Views/Monitor/InferenceViews.swift")
        for needle in ["        VStack(alignment: .leading, spacing: 0) {\n"
                           + "            switch model.startup {\n"
                           + "            case .needsRoots:\n"
                           + "                OnboardingCoordinatorView(startAt: .folders, takesInvites: false, offersJoin: false, onComplete: {})\n"
                           + "            case .starting:\n"
                           + "                SettingsAwaiting().frame(maxWidth: .infinity)\n"
                           + "            case .refused(let sentence):\n"
                           + "                GlassHealthBanner(banner: .init(\n"
                           + "                    title: TracesHealth.coreDownLine?.title ?? TracesHealth.unknownWord ?? \"\",\n"
                           + "                    detail: sentence, tone: .outside))\n"
                           + "            case .running:\n"
                           + "                ledger\n"
                           + "            }\n"
                           + "        }\n"
                           + "        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)\n"
                           + "        .onAppear { model.refreshAll() }\n",
                       "                if case .running = model.startup {\n"
                           + "                    InferenceAccountSection(store: store)\n"
                           + "                }\n"] {
            XCTAssertTrue(views.contains(needle), "InferenceViews.swift lacks \(needle)")
        }
        XCTAssertEqual(views.components(separatedBy: ".onAppear").count - 1, 1)
        XCTAssertEqual(views.components(separatedBy: "OnboardingCoordinatorView(").count - 1, 1)
        // The read-only tool cards are gone: the section's list is the one
        // with the connect action.
        XCTAssertFalse(views.contains("sentence(row)"), "the inspector draws one tools list, the section's")
        XCTAssertFalse(views.contains("TCCoreCopy.healthCopyJSON("), "the core-down words are decoded once, in TracesHealth")
    }

    /// The exposure sheet's paragraph runs whole, in order, inside its own
    /// sheet; and the tool's details hold its config path and its connect
    /// command, both selectable.
    func test_theExposureBodyAndTheToolDetailsArePinnedWhole() throws {
        let source = try Self.text("Views/HarnessListView.swift")
        for needle in ["GlassSheet(title: copy.offerTitle) {\n"
                           + "            Text(copy.offerWhat)\n"
                           + "                .glassType(GlassTokens.TypeScale.body)\n"
                           + "                .foregroundStyle(GlassColor.textPrimary)\n"
                           + "                .fixedSize(horizontal: false, vertical: true)\n"
                           + "            Text(copy.offerExposure)\n"
                           + "                .glassType(GlassTokens.TypeScale.body)\n"
                           + "                .foregroundStyle(GlassColor.textPrimary)\n"
                           + "                .fixedSize(horizontal: false, vertical: true)\n"
                           + "            Text(copy.offerNoRepoint)\n",
                       "if let path = row.configPath {\n"
                           + "                            Label(path, systemImage: \"doc.text\")\n"
                           + "                                .textSelection(.enabled)\n"
                           + "                        }\n"
                           + "                        Label(row.connectCommand, systemImage: \"terminal\")\n"
                           + "                            .textSelection(.enabled)\n"] {
            XCTAssertTrue(source.contains(needle), "HarnessListView.swift lacks \(needle)")
        }
    }
}
