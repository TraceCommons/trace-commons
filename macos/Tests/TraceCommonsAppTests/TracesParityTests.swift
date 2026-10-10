import XCTest
import TCBridge
import TCDesign
import TCShellCore
@testable import TraceCommonsApp

/// The Traces tab against the legacy queue: each control, confirmation,
/// state and core-copy source the queue had has a glass home. Rows are
/// removed only when the owner retires the control they name.
@MainActor
final class TracesParityTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    static let words = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())
    static let coreDown = HealthLineCopy.decode(fromJSON: TCCoreCopy.healthCopyJSON(reachable: false, label: nil, maxQueueEntries: nil))

    /// Fail closed: a core that does not answer, a status nobody could read
    /// and a reported label are each a banner; only a read status with
    /// nothing wrong is none.
    func test_anAbsentSignalIsNeverHealthy() throws {
        let words = try XCTUnwrap(Self.words)
        let down = TracesHealth.banners(phase: .failed(.unreachable), status: nil, words: words, coreDown: Self.coreDown)
        XCTAssertEqual(down.map(\.title), [try XCTUnwrap(Self.coreDown).title])
        XCTAssertEqual(down.first?.tone, .outside)
        let unread = TracesHealth.banners(phase: .loaded, status: nil, words: words, coreDown: Self.coreDown)
        XCTAssertEqual(unread.map(\.title), [words.requestFailed])
        XCTAssertTrue(TracesHealth.banners(phase: .loading, status: nil, words: words, coreDown: Self.coreDown).isEmpty)
    }

    func test_aCoreDownLineThatCannotBeReadFallsBackToTheCoresUnreachableWord() throws {
        let words = try XCTUnwrap(Self.words)
        let down = TracesHealth.banners(phase: .failed(.unreachable), status: nil, words: words, coreDown: nil)
        XCTAssertEqual(down.map(\.title), [words.coreUnreachable])
        XCTAssertNil(down.first?.detail)
    }

    func test_aReportedLabelIsTheCoresBanner() throws {
        let words = try XCTUnwrap(Self.words)
        let json = #"{"health":{"last_error_label":"pii-filter-unavailable"}}"#
        let status = try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data(json.utf8))
        let banners = TracesHealth.banners(phase: .loaded, status: status, words: words, coreDown: Self.coreDown)
        XCTAssertEqual(banners.map(\.title), [HealthCopy.core(label: "pii-filter-unavailable", maxQueueEntries: nil).title])
        XCTAssertNotEqual(banners.first?.tone, .on)
    }

    func test_aReadStatusWithNothingWrongDrawsNoBanner() throws {
        let words = try XCTUnwrap(Self.words)
        let status = try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data("{}".utf8))
        XCTAssertTrue(TracesHealth.banners(phase: .loaded, status: status, words: words, coreDown: Self.coreDown).isEmpty)
    }

    /// With no health words at all, a down or unread core still draws the
    /// core's unknown word; it never reads as healthy.
    func test_aDownCoreWithNoHealthWordsStillDrawsABanner() throws {
        let unknown = try XCTUnwrap(TracesHealth.unknownWord)
        XCTAssertFalse(unknown.isEmpty)
        let phases: [TracesStore.Phase] = [.failed(.unreachable), .failed(.notAvailableYet(method: "x")), .loaded]
        for phase in phases {
            let banners = TracesHealth.banners(phase: phase, status: nil, words: nil, coreDown: nil)
            XCTAssertEqual(banners.map(\.title), [unknown])
            XCTAssertEqual(banners.first?.tone, .outside)
        }
    }

    func test_equalTitlesDoNotCollide() {
        let a = TracesHealth.Banner(title: "x", detail: nil, tone: .ask, index: 0)
        let b = TracesHealth.Banner(title: "x", detail: nil, tone: .ask, index: 1)
        XCTAssertNotEqual(a.id, b.id)
    }

    /// Severity per line: budget, witness and gate-held are waiting; the
    /// label line carries the core's own severity.
    func test_eachSafeguardCarriesItsSeverity() throws {
        let json = """
        {"health":{"last_error_label":"pii-filter-unavailable"},
         "daily_budget":{"blocked":true,"blocked_entries":2}}
        """
        let status = try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data(json.utf8))
        let lines = TracesStore.safeguards(status)
        let label = HealthCopy.core(label: "pii-filter-unavailable", maxQueueEntries: nil)
        XCTAssertEqual(lines.first?.title, label.title)
        XCTAssertEqual(lines.first?.severity, label.severity)
        XCTAssertEqual(lines.dropFirst().map(\.severity), [.waiting])
        XCTAssertEqual(lines.dropFirst().first?.title, DailyBudgetCopy.title)
    }

    /// The banners are drawn above the Traces tree (owner, 2026-10-07:
    /// offers, undo and health above the tree, an accepted difference from
    /// #1146), before the prompts, the spinner and the tree; the inspector
    /// host draws none.
    func test_theTreeDrawsTheBannersAboveEverything() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let slot = try XCTUnwrap(tree.range(of: "struct PreviewSlot"))
        let body = String(tree[treeView.lowerBound..<slot.lowerBound])
        XCTAssertTrue(body.contains("TracesHealth.banners("))
        XCTAssertTrue(body.contains("maxQueueEntries: model.daemonSettings?.maxQueueEntries)"),
                      "the queue-full banner names the configured limit, as the main window does")
        XCTAssertTrue(body.contains("GlassHealthBanner(banner:"))
        XCTAssertFalse(body.contains("ForEach(store.safeguards"), "safeguards are drawn through the fail-closed banners")
        XCTAssertTrue(body.contains("coreDown: TracesHealth.coreDownLine,"))
        let banner = try Self.text("Views/Monitor/TracesHealth.swift")
        XCTAssertTrue(banner.contains("GlassNotice(tone: banner.tone, title: banner.title.isEmpty ? nil : banner.title)"),
                      "an empty title would draw a status dot with no words")
        XCTAssertFalse(banner.contains("Button("), "the Traces tab's banners carry no action")
        XCTAssertTrue(banner.contains("TCCoreCopy.healthCopyJSON(reachable: false"))
        let banners = try XCTUnwrap(body.range(of: "TracesHealth.banners("))
        let prompts = try XCTUnwrap(body.range(of: "InspectorPrompts(store: store)"))
        XCTAssertLessThan(banners.lowerBound, prompts.lowerBound, "the banners precede the prompts")
        // #1146: "Reading local queue…" while loading, not a spinner, and
        // drawn after the prompts in the same scroll.
        let loading = try XCTUnwrap(body.range(of: "if store.phase == .loading && isEmpty {"))
        let reading = try XCTUnwrap(body.range(of: "store.words?.tree.readingQueue"))
        let drawn = try XCTUnwrap(body.range(of: "prompts", range: loading.upperBound..<reading.lowerBound))
        XCTAssertLessThan(drawn.lowerBound, reading.lowerBound, "the prompts precede the loading line and the tree")
        let host = try TracesInspectorHostTests.hostBody()
        XCTAssertFalse(host.contains("TracesHealth.banners("), "the inspector repeats the banners")
        XCTAssertFalse(host.contains("InspectorPrompts("), "the inspector repeats the prompts")
    }

    func test_theCaveatAndTheCertificatesAreOnGlass() throws {
        try LegacySymbols.assertClean("Views/ScrubbingCaveat.swift")
        try LegacySymbols.assertClean("Views/CertificateSection.swift")
        XCTAssertEqual(ScrubbingCaveat.status(redactionCount: 0), .ask)
        XCTAssertEqual(ScrubbingCaveat.status(redactionCount: 3), .off)
        let caveat = try Self.text("Views/ScrubbingCaveat.swift")
        let atCommitStart = try XCTUnwrap(caveat.range(of: "struct ScrubbingCaveatAtCommit")).lowerBound
        let atCommit = String(caveat[atCommitStart...])
        for needle in ["GlassStatusLabel(ScrubbingCaveat.canonical, status: .ask)",
                       ".accessibilityLabel(ScrubbingCaveat.beforeYouContribute + \" \" + ScrubbingCaveat.canonical)"] {
            XCTAssertTrue(atCommit.contains(needle), "ScrubbingCaveatAtCommit lacks \(needle)")
        }
        XCTAssertTrue(caveat.contains("Text(ScrubbingCaveat.canonical)"), "the note draws the canonical sentence")
        let certificates = try Self.text("Views/CertificateSection.swift")
        for needle in ["TCCertificate.listTitle(evidenceAdmitted:", "TCCertificate.rowLine(evidenceAdmitted:",
                       "certificateListEmpty", "admissionEvidenceOffered == true", "\\.holdsCertificate"] {
            XCTAssertTrue(certificates.contains(needle), "CertificateSection.swift lacks \(needle)")
        }
    }

    /// The sheet's chrome keeps every binding, confirmation and core-copy
    /// source Look inside still has. It is read-only (#1241, Ron's
    /// `PreviewInspector`): the verdict, the correction, Contribute and Not
    /// this one left for the inspector's session card, and
    /// `PreviewSheetTests.test_lookInsideHasNoApproveControl` holds that they
    /// did. What stays is what shows the session and the native review.
    func test_thePreviewSheetChromeKeepsEveryBinding() throws {
        let sheet = try Self.text("Views/PreviewSheet.swift")
        for needle in [
            // bindings
            "model.openPreview(entryID:", "model.supportsWitnessReview()", "model.witnessReviewOutcome(entryID:",
            "model.daemonSettings?.admissionEvidenceOffered == true", "AdmissionPreparationView(entryID:",
            "SessionSendDisclosureView(", "model.witnessStateCode == 1",
            // Look inside: Ron's words, the turn index, the original search,
            // the paging and the witness offer
            "MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())",
            "model.previewTurns(entryID: entry.entryID, bodyDigest: digest)", "LookInside.bodyDigest(",
            "model.searchOriginal(entryID: entry.entryID, needle: needle)", "LookInside.shownChunks(",
            "LookInside.offersWitnessReview(", "LookInside.showsNativeReview(", "LookInside.holdsCertificate(",
            // the gate statement
            "TCConsentCopy.copyJSON()", "consent?.gateStatement",
            // confirmations
            "WitnessReviewConsent(copy:", "confirmLine: words?.witnessConfirmLine",
            // keyboard
            ".keyboardShortcut(\"f\", modifiers: .command)", ".keyboardShortcut(.cancelAction)",
            // glass
            "GlassSegmentedTabs(", "GlassButtonStyle(.primary",
        ] {
            XCTAssertTrue(sheet.contains(needle), "PreviewSheet.swift lacks \(needle)")
        }
        XCTAssertFalse(sheet.contains(".tcScreen()"))
        XCTAssertFalse(sheet.contains("CenteredNotice("))
        XCTAssertFalse(sheet.contains("struct SheetSecondaryButtonStyle"))
        try LegacySymbols.assertClean("Views/SessionSendDisclosureView.swift")
    }

    /// The chrome is drawn with glass parts, each pinned by its exact call:
    /// the tabs carry the selection in the core's words, the native review's
    /// controls are glass buttons, a notice with no title has no dot, and
    /// the witness consent is a glass sheet whose heading is the core's.
    func test_thePreviewSheetChromeIsDrawnOnGlass() throws {
        let sheet = try Self.text("Views/PreviewSheet.swift")
        for needle in [
            ".glassTier(.pane)",
            "Divider().overlay(GlassColor.hairline)",
            "GlassSegmentedTabs(tab.title(words), selection: $tab,",
            "segments: Tab.allCases.map { item in GlassSegment(item.title(words), value: item) })",
            "Button(words.prepareAdmission) { preparingAdmission = true }\n"
                + "                            .buttonStyle(GlassButtonStyle(.glass))\n",
            "SheetNotice(fills: !inModal, title: copy.heading, detail: copy.working)",
            // The witness consent is #1146's regular glass modal (P30) whose
            // title is the core's heading: cancel (Escape) left, confirm
            // prominent but never on Return, and armed only by Ron's tick.
            "GlassModal(\n            title: copy.heading,\n",
            ".cancel(copy.cancel, action: onCancel),",
            "GlassModalAction(copy.confirm, isEnabled: confirmLine != nil && confirmed, isProminent: true) {",
            "GlassCheckRow(confirmLine, isOn: $confirmed)",
            // Prepare admission, in a regular glass modal over the preview.
            "title: words.prepareAdmission,\n",
            // The preview itself, in a regular glass modal over the window.
            "struct PreviewModal: View {",
            "PreviewSheet(entry: entry, onClose: onClose)",
            ".accessibilityIdentifier(\"transcript-copy-all\")",
            // A notice with no title is drawn without one, never as a bare dot.
            "GlassNotice(tone: .ask, title: title?.isEmpty == false ? title : nil) {",
            // The witness consent's body scrolls once it no longer fits.
            "GlassModalBody { disclosure }",
        ] {
            XCTAssertTrue(sheet.contains(needle), "PreviewSheet.swift lacks \(needle)")
        }
        XCTAssertFalse(sheet.contains("struct SheetHairline"))
        XCTAssertFalse(sheet.contains("SheetSecondaryButtonStyle"))
        // The witness consent is the one place its heading is drawn: GlassSheet draws it.
        let consentStart = try XCTUnwrap(sheet.range(of: "struct WitnessReviewConsent: View {")).lowerBound
        let consent = String(sheet[consentStart...])
        XCTAssertFalse(consent.contains("Text(copy.heading)"))
        XCTAssertFalse(consent.contains(".tcScreen()"))
        XCTAssertFalse(consent.contains(".sheet("))
        XCTAssertFalse(sheet.contains(".sheet("), "the preview's own questions are glass modals")
        XCTAssertFalse(sheet.contains(".alert("), "the correction refusal is a glass confirmation")
    }

    /// The per-session send disclosure is glass, its unreadable state is the
    /// Settings glass line, and its `.onAppear` hangs on the always-present
    /// stack rather than on a Group of conditional branches.
    func test_theSessionSendDisclosureIsOnGlass() throws {
        let disclosure = try Self.text("Views/SessionSendDisclosureView.swift")
        XCTAssertTrue(disclosure.contains(
            "RouteDisclosureUnreadableGlassLine(line: model.routeDisclosureUnreadableCopy?.session)"))
        XCTAssertFalse(disclosure.contains("struct RouteDisclosureUnreadableLine"))
        XCTAssertFalse(disclosure.contains("Group {"))
        XCTAssertFalse(disclosure.contains("design: .monospaced"))
        XCTAssertTrue(disclosure.contains("}\n        .onAppear {"), "the onAppear closes the outer VStack")
        XCTAssertTrue(GlassSurfaceRulesTests.files.contains("Views/SessionSendDisclosureView.swift"))
    }

    /// The private-inference tone onto a glass status, as the window's
    /// Inference dot maps it: only clear is on.
    func test_thePrivateInferenceToneMapsOntoAGlassStatus() {
        XCTAssertEqual(PrivateInferenceIndicator.status(.clear), .on)
        XCTAssertEqual(PrivateInferenceIndicator.status(.held), .ask)
        XCTAssertEqual(PrivateInferenceIndicator.status(.attention), .ask)
        XCTAssertEqual(PrivateInferenceIndicator.status(.refused), .ask)
        XCTAssertEqual(PrivateInferenceIndicator.status(.neutral), .off)
    }

    /// The tabs keep their searches, the whole body and the copy-all
    /// control, and the sheet is off the legacy palette. What's in it and
    /// Permissions left Look inside with #1241: their facts are the session
    /// card's (the redaction summary, residual risk, consent scopes).
    func test_thePreviewSheetTabsKeepEveryBinding() throws {
        let sheet = try Self.text("Views/PreviewSheet.swift")
        for needle in [
            "preview.search(needle)", "searchOriginal(needle)", "RecentSearches.remember(", "RecentSearches.load()",
            "OriginalSearchOutcome.classify(", "document.snippet(around:", ".onSubmit(commit)", "focus: $focused",
            "\"transcript-copy-all\"", "document.wholeText()", "RedactionMarks.spoken(",
            "TranscriptResidentChunks", "TranscriptRowIndex(", "GlassTokens.TypeScale.mono",
        ] {
            XCTAssertTrue(sheet.contains(needle), "PreviewSheet.swift lacks \(needle)")
        }
        for gone in ["struct WhatsInItTab", "struct PermissionsTab"] {
            XCTAssertFalse(sheet.contains(gone), "PreviewSheet.swift still declares \(gone), which nothing draws")
        }
        try LegacySymbols.assertClean("Views/PreviewSheet.swift")
    }

    /// The tabs are drawn with glass parts, each pinned by its exact call:
    /// the search highlight and the redaction chip on the existing tokens
    /// (no new token), every outcome a dot with its words, and the sheet
    /// held to the glass surface rules.
    func test_thePreviewSheetTabsAreDrawnOnGlass() throws {
        let sheet = try Self.text("Views/PreviewSheet.swift")
        for needle in [
            // Search
            "attributed[found].backgroundColor = GlassTokens.Color.statusAsk.color.opacity(0.32)\n"
                + "        attributed[found].foregroundColor = GlassColor.textPrimary\n",
            "Button(\"Search\", action: commit)\n                    .buttonStyle(GlassButtonStyle(.glass))\n",
            "Button(term) { needle = term }\n                            .buttonStyle(GlassButtonStyle(.link))\n",
            "GlassStatusLabel(outcome.sentence, status: Self.status(for: outcome.emphasis))",
            "GlassStatusLabel(\"0 matches\", status: .on)",
            "GlassStatusLabel(Self.matchCount(offsets!.count), status: .ask)",
            "GlassTextField(prompt, text: $needle, prompt: prompt, showsLabel: false, focus: $focused)",
            ".glassFieldWell(invalid: false)",
            "GlassCard(quiet: true) {\n                            Text(highlighting(snippet, term: needle))\n"
                + "                                .glassType(GlassTokens.TypeScale.mono)\n",
            // Transcript: Ron's caption from the core, the body in mono
            // with its chips, and his Load more and turn separators.
            "Text(words?.transcriptCaption ?? \"\")\n"
                + "                    .glassType(GlassTokens.TypeScale.caption)\n",
            "chip.backgroundColor = GlassTokens.Color.controlSelected.color\n"
                + "            chip.foregroundColor = GlassTokens.Color.textOnSelected.color\n",
            "Text(segment.text)\n                            .glassType(GlassTokens.TypeScale.mono)\n"
                + "                            .textSelection(.enabled)\n",
            "TranscriptMarkers.chipped(text, font: GlassTokens.TypeScale.mono.font)",
            "NSLayoutManager().defaultLineHeight(for: font) + GlassTokens.TypeScale.mono.lineSpacing",
            "GlassCard(quiet: true, flush: true) {",
            "Button(words.addTurnSeparators, action: onAddSeparators)\n"
                + "                            .buttonStyle(GlassButtonStyle(.link))\n",
        ] {
            XCTAssertTrue(sheet.contains(needle), "PreviewSheet.swift lacks \(needle)")
        }
        // The transcript's columns are measured against the very inset the
        // chunks are drawn inside, or every placeholder is the wrong height.
        XCTAssertTrue(sheet.contains(".padding(.horizontal, TranscriptTab.inset)"))
        XCTAssertTrue(sheet.contains("width - 2 * TranscriptTab.inset"))
        // No sentence is drawn in the uppercased eyebrow inside the tabs.
        let tabsStart = try XCTUnwrap(sheet.range(of: "// MARK: - Tabs")).lowerBound
        let tabsEnd = try XCTUnwrap(sheet.range(of: "struct WitnessReviewConsent: View {")).lowerBound
        XCTAssertFalse(sheet[tabsStart..<tabsEnd].contains("TypeScale.eyebrow"))
        XCTAssertTrue(GlassSurfaceRulesTests.files.contains("Views/PreviewSheet.swift"))
    }

    /// A search outcome onto a glass status: still there is the one to slow
    /// down on, removed is clear, and a check that did not run is neither.
    func test_aSearchOutcomeMapsOntoAGlassStatus() {
        XCTAssertEqual(SearchTab.status(for: .attention), .ask)
        XCTAssertEqual(SearchTab.status(for: .clear), .on)
        XCTAssertEqual(SearchTab.status(for: .unchecked), .off)
    }

    /// The preloaded count is still inflected now that it is a plain string
    /// on a status label rather than a `Label`'s localized key.
    func test_thePreloadedMatchCountIsInflected() {
        XCTAssertEqual(SearchTab.matchCount(1), "1 match")
        XCTAssertEqual(SearchTab.matchCount(2), "2 matches")
        XCTAssertEqual(SearchTab.matchCount(20), "20 matches")
    }

    /// Undo and the consent offers sit at the top of the inspector (Ron's
    /// `WaitingPrompts`), which opens itself when one appears
    /// (`InspectorDemand`), and read the same model members and core words
    /// the legacy queue does.
    func test_offersAndUndoLiveAtTheTopOfTheInspector() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let inspector = try XCTUnwrap(tree.range(of: "struct PreviewSlot"))
        XCTAssertFalse(tree[treeView.lowerBound..<inspector.lowerBound].contains("model.undo"))
        XCTAssertTrue(tree.contains("Text(QueueLegacyWords.nothingWaiting)"))
        XCTAssertTrue(tree.contains("Text(QueueLegacyWords.nothingWaitingDetail)"))
        // A refused undo is said beside it, by the one helper the card
        // shares.
        XCTAssertTrue(try Self.text("Views/Monitor/SessionReviewCard.swift")
            .contains("TracesRefusal(store: store, entryId: entry.entryId)"))

        let prompts = try Self.text("Views/Monitor/InspectorPrompts.swift")
        let offers = try Self.text("Views/Monitor/TracesOffers.swift")
        let host = try Self.text("Views/Monitor/TracesInspectorHost.swift")
        // The Summary draws the certificates and the outcomes (Task 4).
        let summary = try Self.text("Views/Monitor/SummaryInspector.swift")
        for needle in [
            "if let undo = model.undo {", "approvalUndo(undo)", "{ model.undoApproval() }", "{ model.dismissUndo() }",
            "QueueLegacyWords.undoWillSend", "QueueLegacyWords.closeNoticeStillSends", "QueueLegacyWords.closeNotice)",
            "QueueLegacyWords.approvedAgo(undo.heldSeconds)", "if let contributed = store.lastContributed,",
            "if let kept = store.lastKept,",
            "store.perform(.undoContribute, on: contributed.entryId)", "store.perform(.undoKeep, on: kept)",
            "model.showsPrivateInferenceOffer", "model.answerPrivateInferenceOffer(accepted: true)",
            "model.answerPrivateInferenceOffer(accepted: false)", "model.privateInferenceBusy",
            "model.armingOffer", "model.acceptArmingOffer(", "model.declineArmingOffer(",
            "model.lastActionError = nil", "model.lastActionNotice = nil",
            "model.witnessCopy?.onboarding", "QueueLegacyWords.undo)", "ActionNoticeWords.dismissWord",
            "words.undo.approvalSaved", "words.undo.undoing", "store.words?.optionalAutomation",
        ] {
            XCTAssertTrue(prompts.contains(needle), "InspectorPrompts.swift lacks \(needle)")
        }
        for needle in [
            "copy.offerWhat", "copy.offerExposure", "copy.offerNoRepoint", "copy.offerAskedOnce",
            "TCCoreCopy.armingOfferCopyJSON(", "TCOutcome.line(label:", "QueueLegacyWords.noLongerWaiting(",
            "QueueLegacyWords.notOfferedScope", "QueueLegacyWords.agentSetup",
        ] {
            XCTAssertTrue(offers.contains(needle), "TracesOffers.swift lacks \(needle)")
        }
        XCTAssertTrue(summary.contains("NotOfferedGlassDisclosure(counts: outcomeCounts, words: words?.summaryPanel)"))
        XCTAssertTrue(host.contains("queueAnswered: model.queueAnswered, outcomeCounts: model.outcomeCounts)"))
        // Both undos are drawn by the prompts' body, not merely defined.
        let promptsBody = try XCTUnwrap(prompts.range(of: "    var body: some View {"))
        let firstHelper = try XCTUnwrap(prompts.range(of: "    private func approvalUndo("))
        let body = prompts[promptsBody.upperBound..<firstHelper.lowerBound]
        XCTAssertTrue(body.contains("approvalUndo(undo)\n"), "the approval undo is not drawn")
        XCTAssertTrue(body.contains("            storeUndo\n"), "the store's undos are not drawn")
        // Return is bound to Undo itself: no other button between the two.
        let undoButton = try XCTUnwrap(prompts.range(
            of: "Button(store.words?.undoContribute ?? QueueLegacyWords.undo) { model.undoApproval() }"))
        let shortcut = try XCTUnwrap(prompts.range(
            of: ".keyboardShortcut(.defaultAction)", range: undoButton.upperBound..<prompts.endIndex))
        XCTAssertFalse(prompts[undoButton.upperBound..<shortcut.lowerBound].contains("Button("))
        XCTAssertEqual(prompts.components(separatedBy: ".keyboardShortcut(.defaultAction)").count, 2)
        XCTAssertFalse(offers.contains(".keyboardShortcut(.defaultAction)"))
        // The arming card draws nothing without the core's words, and its
        // confirm only opens the confirmation; arming is the dialog's.
        let armingCard = try XCTUnwrap(offers.range(of: "struct ArmingOfferGlassCard"))
        let armingBody = try XCTUnwrap(offers.range(
            of: "    var body: some View {\n", range: armingCard.upperBound..<offers.endIndex))
        XCTAssertTrue(offers[armingBody.upperBound...].hasPrefix("        if let copy {\n"))
        XCTAssertTrue(offers.contains("Button(copy.confirm) { confirming = true }"))
        // A whole-window glass confirmation (Ron's GlassModal), in the
        // core's words.
        XCTAssertTrue(offers.contains(".glassModal(isPresented: $confirming)"))
        XCTAssertTrue(offers.contains("title: copy.question, message: copy.body,"))
        XCTAssertEqual(offers.components(separatedBy: "onArm()").count - 1, 1, "only the confirmation arms")
        // No control without words: an absent core word falls back to an
        // existing one, never to an empty title.
        for text in [prompts, offers, host] {
            XCTAssertFalse(text.contains("?? \"\""), "a control would be wordless without the core")
        }
        // One dismiss accessor everywhere a glass notice is put away, and
        // never the review's "Not this one".
        XCTAssertTrue(prompts.contains(
            "store.words?.dismissAction ?? ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord"))
        for text in [prompts, offers] {
            XCTAssertNil(text.range(of: #"words\??\.dismiss\b(?!Action)"#, options: .regularExpression))
        }
        // An unread list is not an empty one: the certificate list and the
        // first-contribution note wait for the daemon's answer, and draw
        // nothing before it (standing rule: fail closed).
        let flatSummary = summary.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        XCTAssertTrue(flatSummary.contains("if queueAnswered { CertificateSection(entries: awaitingDecision) }"))
        let flatPrompts = prompts.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        XCTAssertTrue(flatPrompts.contains(
            "if model.historyAnswered, model.queueAnswered, model.history.isEmpty, "
                + "let copy = model.witnessCopy?.onboarding { "
                + "FirstContributionGlassNote(copy: copy, reviewing: !model.awaitingDecision.isEmpty) }"))
        XCTAssertEqual(summary.components(separatedBy: "CertificateSection(").count - 1, 1)
        XCTAssertFalse(host.contains("CertificateSection("))
        XCTAssertEqual(prompts.components(separatedBy: "FirstContributionGlassNote(copy:").count - 1, 1)
        for rel in ["Views/Monitor/TracesOffers.swift", "Views/Monitor/InspectorPrompts.swift"] {
            try LegacySymbols.assertClean(rel)
            XCTAssertTrue(GlassSurfaceRulesTests.files.contains(rel))
        }

        // The legacy queue is gone (R15); its words are the core's now
        // (#1146 parity, 2026-10-07), and its file holds none.
        let queue = try Self.text("Views/QueueView.swift")
        XCTAssertTrue(queue.contains("enum QueueLegacyWords"))
        for sentence in ["Nothing is waiting.", "Close this notice.", "\"Undo\"", "\"Agent setup\""] {
            XCTAssertFalse(queue.contains(sentence), "\(sentence) is written in Swift")
        }
        XCTAssertEqual(QueueLegacyWords.undo, "Undo")
        XCTAssertEqual(QueueLegacyWords.agentSetup, "Agent setup")
    }

    /// The words table holds the legacy sentences verbatim.
    func test_theQueueWordsAreTheLegacySentences() {
        XCTAssertEqual(QueueLegacyWords.nothingWaiting, "Nothing is waiting.")
        XCTAssertEqual(
            QueueLegacyWords.nothingWaitingDetail,
            "When a trace finishes and goes quiet, it shows up here. Nothing is sent unless you say so.")
        XCTAssertEqual(
            QueueLegacyWords.undoWillSend,
            "Approved traces will send automatically. You can undo until uploading starts.")
        XCTAssertEqual(
            QueueLegacyWords.closeNoticeStillSends,
            "Close this notice. Approved traces will still send automatically.")
        XCTAssertEqual(QueueLegacyWords.closeNotice, "Close this notice.")
        XCTAssertEqual(QueueLegacyWords.approvedAgo(7), "Approved 7s ago")
        XCTAssertEqual(QueueLegacyWords.approvedAgo(AppModel.Undo.tickCeiling), "Approved 120s+ ago")
        XCTAssertEqual(QueueLegacyWords.approvedAgo(500), "Approved 120s+ ago")
        XCTAssertEqual(QueueLegacyWords.noLongerWaiting(3), "Traces no longer waiting (3)")
        XCTAssertEqual(
            QueueLegacyWords.notOfferedScope,
            "This covers traces that reached the queue. Traces that were never queued at all are not counted here.")
    }

    /// The accept comes first as a glass button and the decline after it as
    /// a link, on both offers (Ron, 2026-10-09, which replaced the legacy
    /// queue's decline-first order); neither answer is the primary action.
    func test_bothOffersPutTheAcceptFirstAndTheDeclineAsALink() throws {
        let offers = try Self.text("Views/Monitor/TracesOffers.swift")
        let decline = try XCTUnwrap(offers.range(of: "copy.offerDecline"))
        let accept = try XCTUnwrap(offers.range(of: "copy.offerAccept"))
        XCTAssertLessThan(accept.lowerBound, decline.lowerBound)
        let armDecline = try XCTUnwrap(offers.range(of: "Button(copy.decline"))
        let armConfirm = try XCTUnwrap(offers.range(of: "Button(copy.confirm"))
        XCTAssertLessThan(armConfirm.lowerBound, armDecline.lowerBound)
        for declineCall in ["Button(copy.offerDecline, action: onDecline)\n                        .buttonStyle(GlassButtonStyle(.link))",
                            "Button(copy.decline, action: onDecline)\n                            .buttonStyle(GlassButtonStyle(.link))"] {
            XCTAssertTrue(offers.contains(declineCall), "the decline is not a link: \(declineCall)")
        }
        let arming = try XCTUnwrap(offers.range(of: "struct ArmingOfferGlassCard"))
        XCTAssertFalse(offers[arming.lowerBound...].prefix(1500).contains("GlassButtonStyle(.primary"))
        let privateAI = try XCTUnwrap(offers.range(of: "struct PrivateAIOfferGlassCard"))
        XCTAssertFalse(offers[privateAI.lowerBound..<arming.lowerBound].contains("GlassButtonStyle(.primary"))
        // #1146's head: the destination as a mono accent eyebrow over the
        // h2 title, beside the X that answers Not now; the paragraphs drawn
        // alike, the first and the one-line exposure always, the rest
        // behind Learn more (owner, 2026-10-08).
        let card = String(offers[privateAI.lowerBound..<arming.lowerBound])
        let eyebrow = try XCTUnwrap(card.range(of: "Text(copy.destination)\n                    .glassType(Self.destinationType)"))
        let title = try XCTUnwrap(card.range(of: "Text(copy.offerTitle)\n                    .glassType(GlassTokens.TypeScale.title)"))
        XCTAssertLessThan(eyebrow.lowerBound, title.lowerBound)
        XCTAssertTrue(card.contains("CardClose(label: copy.offerDecline, action: onDecline)"),
                      "the X answers as Not now does")
        XCTAssertEqual(PrivateAIOfferGlassCard.destinationType.design, .monospaced)
        XCTAssertTrue(PrivateAIOfferGlassCard.destinationType.uppercase)
        XCTAssertTrue(card.contains("Text(copy.offerWhat)\n                    Text(copy.offerExposureShort)\n                    if learnMore {"),
                      "what turning it on exposes must stay in sight without Learn more")
        XCTAssertTrue(card.contains("Text(copy.offerAskedOnce)\n                    }\n                }\n                .glassType(GlassTokens.TypeScale.body)"),
                      "the paragraphs behind Learn more are drawn as the first two")
    }

    /// The folder row offers Submit all only when the shared table offers
    /// Contribute, with the daemon's counts, and says what it withheld.
    /// Submit all as is the Folder inspector's modal (Task 5 of the #1146
    /// port), in the core's outcome words, with Submit all beside it.
    func test_theFolderRowSubmitsAllAsTheQueueDid() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        for needle in ["store.groupOffer(", "offer.offersContribute", "Self.submitTitle(offer.count, words: $0)",
                       "Self.submitHelp(offer.withheldLine, words: words)", "store.contributeFolder(folder, verdict: nil)",
                       "offer.withheldLine", "submitTitle: submits ?", "onSubmit: submits && !busy ?",
                       "store.mayContributeFolder(folder)", "words.tree.submitCount", "words?.tree.submitTip"] {
            XCTAssertTrue(tree.contains(needle), "TracesViews.swift lacks \(needle)")
        }
        let inspector = try Self.text("Views/Monitor/ToolFolderInspectors.swift")
        for needle in ["store.groupOffer(folder)", "store.mayContributeFolder(folder)", "outcome.submitAllAs",
                       "outcome.submitAllAsTooltip", "store.contributeFolder(folder, verdict: verdict)",
                       "store.contributeFolder(folder, verdict: nil)", "offer.withheldLine",
                       "Self.submitAllTitle(offer.count, words: words)"] {
            XCTAssertTrue(inspector.contains(needle), "ToolFolderInspectors.swift lacks \(needle)")
        }
        let store = try Self.text("Views/Monitor/TracesStore.swift")
        for needle in ["projectId: folder.id, verdict: verdict, filter: idleOnly ? .idleSessions : nil", "verdict:", "excludedIneligible", "withheldLine(", "cancelFolder(projectId:",
                       "EligibilitySurface.groupSubmit("] {
            XCTAssertTrue(store.contains(needle), "TracesStore.swift lacks \(needle)")
        }
        XCTAssertTrue(try Self.text("Views/Monitor/InspectorPrompts.swift").contains("store.lastContributedFolder"))
        XCTAssertTrue(try Self.text("Views/QueueFolderRow.swift").contains("enum QueueFolderWords"))
    }

    /// The legacy entry is found by id; a session the legacy queue does not
    /// hold opens no sheet (fail closed), rather than a sheet for another.
    func test_theSheetOpensOnlyForTheSameSession() throws {
        let json = #"[{"entry_id":"e-1","session_hash":"h","source":"claude-code","project_id":"p","project_label":"payments","size_bytes":10,"discovered_at":"2026-10-01T00:00:00Z","state":"pending","attempts":0}]"#
        let awaiting = try DaemonDecoding.decoder().decode([QueueEntry].self, from: Data(json.utf8))
        XCTAssertEqual(QueueEntryBridge.legacyEntry(for: "e-1", in: awaiting)?.entryID, "e-1")
        XCTAssertNil(QueueEntryBridge.legacyEntry(for: "e-2", in: awaiting))
        XCTAssertNil(QueueEntryBridge.legacyEntry(for: "e-1", in: []))
    }

    /// A secret the scan found and left in is said in the core's words,
    /// counted by site; removals alone, or no summary yet, say nothing.
    func test_theSurvivorLineIsTheCoresAndOnlyForASurvivor() throws {
        func summary(_ redactions: String) throws -> DaemonData.PreviewSummary {
            try DaemonDataDecoding.decoder().decode(
                DaemonData.PreviewSummary.self, from: Data(#"{"redactions":\#(redactions)}"#.utf8))
        }
        XCTAssertNil(TracesStore.survivorLine(nil))
        XCTAssertNil(TracesStore.survivorLine(
            try DaemonDataDecoding.decoder().decode(DaemonData.PreviewSummary.self, from: Data("{}".utf8))))
        XCTAssertNil(TracesStore.survivorLine(try summary(#"{"api_key":2}"#)))
        let counts = ["api_key": 2, "residual_secret_at:events.3.correction": 1]
        let line = try XCTUnwrap(TracesStore.survivorLine(
            try summary(#"{"api_key":2,"residual_secret_at:events.3.correction":1}"#)))
        XCTAssertEqual(line, TCCoreCopy.residualSecretLine(
            count: RedactionLabels.survivorTotal(counts), sites: RedactionLabels.survivors(counts).map(\.site)))
        XCTAssertTrue(line.contains("events.3.correction"), line)
    }

    /// The queue card's facts, drawn by the inspector from the same sources:
    /// the surviving secret, the scrubbing caption, the subagent line, and
    /// Look inside opening the preview sheet for this session only.
    func test_theInspectorSaysWhatTheQueueCardSaid() throws {
        let card = try Self.text("Views/Monitor/SessionReviewCard.swift")
        let inspector = try XCTUnwrap(card.range(of: "struct SessionReviewCard: View"))
        let body = card[inspector.lowerBound...]
        for needle in ["TracesStore.survivorLine(summary)", "GlassStatusLabel(survivor, status: .ask)",
                       "RedactionLabels.removedTotal(redactions)",
                       "ScrubbingCaveat.rowLine(redactionCount: removed)",
                       "ScrubbingCaveat.status(redactionCount: removed)",
                       "SubagentCopy.line(count: entry.subagentCount ?? 0, dropped: entry.subagentsDropped ?? 0)",
                       ".glassModal(item: $previewing)",
                       "PreviewModal(entry: $0) { previewing = nil }", "@EnvironmentObject private var model: AppModel",
                       #"ProcessInfo.processInfo.environment["TRACE_COMMONS_DEMO_PREVIEW"] == "1","#,
                       ".onChange(of: model.awaitingDecision.count)", "previewing == nil,"] {
            XCTAssertTrue(body.contains(needle), "SessionReviewCard lacks \(needle)")
        }
        // Look inside is always offered (#1146), on this session's own
        // entry: the legacy queue's when it holds it, carried over when not.
        let flat = body.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        XCTAssertTrue(flat.contains(
            "Button(review.lookInside) { previewing = QueueEntryBridge.previewEntry(entry, in: model.awaitingDecision) }"
                + " .buttonStyle(GlassButtonStyle(.link))"), "Look inside must open this session's entry, always")
        XCTAssertFalse(body.contains("if let legacy = QueueEntryBridge.legacyEntry("),
                       "Look inside is drawn only when the legacy queue holds the session")
        // The preview modal and the demo hook hang off the always-present
        // container, not the selected-session branch.
        XCTAssertTrue(body.contains("""
                        Spacer(minLength: 0)
                    }
                }
                // Over the whole window (`glassModalHost` at its root), not a sheet.
                .glassModal(item: $previewing) { PreviewModal(entry: $0) { previewing = nil }.environmentObject(model) }
                .onChange(of: model.awaitingDecision.count) { _, _ in
        """), "the preview modal and the demo hook must sit on the inspector's outer VStack")
        XCTAssertFalse(body.contains(".sheet("), "the preview is a glass modal, never a stock sheet")
        XCTAssertTrue(try Self.text("Views/Monitor/TracesStore.swift")
            .contains("TCCoreCopy.residualSecretLine(count: total,"))
    }

    // MARK: Selection (Task 2 of the #1146 inspector port)

    /// Ron's `useInspectorDemand` opens the inspector for a selected
    /// session only: selecting a folder moves the selection and leaves the
    /// inspector as the person set it. A session opens it, as Review does.
    func test_selectingAFolderDoesNotOpenTheInspector() throws {
        var selection: MonitorSelection?
        var showsInspector = false
        MonitorWindowView.select(.folder(projectID: "p1"), selection: &selection, showsInspector: &showsInspector)
        XCTAssertEqual(selection, .folder(projectID: "p1"))
        XCTAssertFalse(showsInspector, "a folder is not a demand to open the inspector")

        MonitorWindowView.select(.session(entryID: "e1"), selection: &selection, showsInspector: &showsInspector)
        XCTAssertEqual(selection, .session(entryID: "e1"))
        XCTAssertTrue(showsInspector, "a session is")

        // Never closes itself: back to a folder, or to nothing, it stays open.
        MonitorWindowView.select(.folder(projectID: "p1"), selection: &selection, showsInspector: &showsInspector)
        XCTAssertTrue(showsInspector)
        MonitorWindowView.select(nil, selection: &selection, showsInspector: &showsInspector)
        XCTAssertNil(selection)
        XCTAssertTrue(showsInspector)

        // The tree routes its selection through that rule, and Return opens
        // a review only for a session.
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("Self.select($0, selection: &selection, showsInspector: &showsInspector)"))
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        XCTAssertTrue(tree.contains("guard case .session(let entryID) = selection else { return .ignored }"))
    }

    /// The selection is restored per window under one key, through a
    /// stored form that round-trips both kinds and refuses anything else.
    func test_theSelectionIsRestoredPerWindow() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("@SceneStorage(\"monitor.selection\") private var selection: MonitorSelection?"))
        XCTAssertFalse(window.contains("monitor.selectedSession"), "the session-only key is retired")

        for value: MonitorSelection in [
            .folder(projectID: "/Users/me/api"), .session(entryID: "e1"), .session(entryID: "with:colon"),
        ] {
            XCTAssertEqual(MonitorSelection(rawValue: value.rawValue), value)
            let coded = try JSONDecoder().decode(MonitorSelection.self, from: JSONEncoder().encode(value))
            XCTAssertEqual(coded, value)
        }
        XCTAssertNil(MonitorSelection(rawValue: ""))
        XCTAssertNil(MonitorSelection(rawValue: "tool:claude-code"), "the tree has no tool level")
        XCTAssertNil(MonitorSelection(rawValue: "e1"), "a bare id from the old key is not a selection")

        // The inspector reads the resolved selection, never the stored one:
        // the window hands the stored one to the host, which resolves it.
        XCTAssertTrue(window.contains("TracesInspectorHost(traces: traces, home: home, selection: selection"))
        XCTAssertTrue(try Self.text("Views/Monitor/TracesInspectorHost.swift")
            .contains("traces.selectedSession(selection)"))
        XCTAssertFalse(window.contains("selectedEntry"), "no entry is picked by the raw stored selection")
    }
}
