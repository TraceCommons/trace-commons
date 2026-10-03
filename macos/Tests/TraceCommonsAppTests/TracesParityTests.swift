#if DEBUG
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

    func test_theTreeDrawsTheBannersAboveItself() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let inspector = try XCTUnwrap(tree.range(of: "struct SessionInspectorView"))
        let body = tree[treeView.lowerBound..<inspector.lowerBound]
        XCTAssertTrue(body.contains("TracesHealth.banners("))
        XCTAssertTrue(body.contains("GlassHealthBanner(banner:"))
        XCTAssertFalse(body.contains("ForEach(store.safeguards"), "safeguards are drawn through the fail-closed banners")
        let banner = try Self.text("Views/Monitor/TracesHealth.swift")
        XCTAssertFalse(banner.contains("Button("), "the Traces tab's banners carry no action")
        XCTAssertTrue(banner.contains("TCCoreCopy.healthCopyJSON(reachable: false"))
        XCTAssertTrue(body.contains("coreDown: TracesHealth.coreDownLine)"))
        let banners = try XCTUnwrap(body.range(of: "TracesHealth.banners("))
        let spinner = try XCTUnwrap(body.range(of: "ProgressView()"))
        XCTAssertLessThan(banners.lowerBound, spinner.lowerBound, "the banners precede the spinner and the tree")
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
                       ".accessibilityLabel(\"Before you contribute. \\(ScrubbingCaveat.canonical)\")"] {
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
    /// source the legacy sheet had, and the gate still decides Contribute.
    func test_thePreviewSheetChromeKeepsEveryBinding() throws {
        let sheet = try Self.text("Views/PreviewSheet.swift")
        for needle in [
            // bindings and writes
            "model.openPreview(entryID:", "model.supportsWitnessReview()", "model.witnessReviewOutcome(entryID:",
            "model.dismiss(entry)", "model.approve(entry, verdict: verdict)", "model.approve(entry, verdict: verdict, correction: text)",
            "model.daemonSettings?.admissionEvidenceOffered == true", "AdmissionPreparationView(entryID:",
            "SessionSendDisclosureView(", "model.witnessStateCode == 1",
            // the gate
            "ReadGate.canContribute(hasPinnedPreview: summary?.enrolled == true)", "EligibilitySurface.mayProceed(",
            "EligibilitySurface.current(", "TCConsentCopy.copyJSON()", "TCConsentCopy.gateHelp(pinned:",
            // confirmations and alerts
            "CorrectionCopy.credentialHeadline", "CorrectionCopy.credentialBody", "WitnessReviewConsent(copy:",
            // verdict and correction
            "VerdictCopy.question", "VerdictCopy.caption", "CorrectionCopy.question", "CorrectionCopy.placeholder",
            "CorrectionCopy.caption", "CorrectionCopy.maxCharacters", "CorrectionCopy.toSend(",
            // keyboard
            ".keyboardShortcut(\"f\", modifiers: .command)", ".keyboardShortcut(.cancelAction)",
            // glass
            "GlassSegmentedTabs(", "GlassButtonStyle(.primary", "ScrubbingCaveatAtCommit()",
        ] {
            XCTAssertTrue(sheet.contains(needle), "PreviewSheet.swift lacks \(needle)")
        }
        XCTAssertFalse(sheet.contains(".tcScreen()"))
        XCTAssertFalse(sheet.contains("CenteredNotice("))
        XCTAssertFalse(sheet.contains("struct SheetSecondaryButtonStyle"))
        // Contribute has no keyboard shortcut: an irreversible send is never one keystroke away.
        let contribute = try XCTUnwrap(sheet.range(of: "contribute()\n"))
        XCTAssertFalse(sheet[contribute.upperBound...].prefix(240).contains(".keyboardShortcut("))
        try LegacySymbols.assertClean("Views/SessionSendDisclosureView.swift")
    }

    /// The chrome is drawn with glass parts, each pinned by its exact call:
    /// the tabs carry the selection, the eligibility line is a dot with its
    /// words, the verdict chips say which is chosen, and the witness consent
    /// is a glass sheet whose heading is the core's.
    func test_thePreviewSheetChromeIsDrawnOnGlass() throws {
        let sheet = try Self.text("Views/PreviewSheet.swift")
        for needle in [
            ".glassTier(.pane)",
            "Divider().overlay(GlassColor.hairline)",
            "GlassSegmentedTabs(model.publicRunCopy?.sessionDetail ?? \"\", selection: $tab,",
            "GlassStatusLabel(line, status: PrivateInferenceIndicator.status(",
            ".accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)",
            "GlassTag(\"nothing sent yet\", tone: .neutral)",
            "GlassTag(Self.scrubbingFound(summary), tone: .ask)",
            "SheetNotice(title: copy.heading, detail: copy.working)",
            "GlassSheet(title: copy.heading) {",
            "Button(copy.cancel, role: .cancel) { dismiss() }",
            "Button(copy.confirm) { dismiss(); onConfirm() }",
            ".accessibilityIdentifier(\"transcript-copy-all\")",
            // The two questions are sentences, not field labels: never the
            // uppercased eyebrow.
            "Text(VerdictCopy.question)\n                .glassType(GlassTokens.TypeScale.caption)",
            "Text(CorrectionCopy.question)\n                .glassType(GlassTokens.TypeScale.caption)",
            // The one irreversible control: primary, disarmed by the gate,
            // its tooltip the gate's own, and nothing after it.
            "Button(\"Contribute\") {\n                    contribute()\n                }\n"
                + "                .buttonStyle(GlassButtonStyle(.primary))\n"
                + "                .disabled(!canContribute)\n"
                + "                .help(gateHelp)\n            }\n",
            // A notice with no title is drawn without one, never as a bare dot.
            "GlassNotice(tone: .ask, title: title?.isEmpty == false ? title : nil) {",
            // The witness consent: Escape cancels, and the pane fills the sheet.
            "Button(copy.cancel, role: .cancel) { dismiss() }\n"
                + "                    .buttonStyle(GlassButtonStyle(.glass))\n"
                + "                    .keyboardShortcut(.cancelAction)\n",
            "}\n            Spacer(minLength: 0)\n            HStack {",
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
        XCTAssertTrue(consent.contains(".frame(width: 560, height: 390)"))
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

    /// The four tabs keep their searches, their figures, the whole body and
    /// the copy-all control, and the sheet is off the legacy palette.
    func test_thePreviewSheetTabsKeepEveryBinding() throws {
        let sheet = try Self.text("Views/PreviewSheet.swift")
        for needle in [
            "preview.search(needle)", "searchOriginal(needle)", "RecentSearches.remember(", "RecentSearches.load()",
            "OriginalSearchOutcome.classify(", "document.snippet(around:", ".onSubmit(commit)", ".focused($focused)",
            "TCCoreCopy.redactionSummaryJSON(", "summary.piiLabelsPresent", "summary.residualRisk", "summary.tokenDistributionSummary",
            "ScrubbingCaveatNote()", "\"transcript-copy-all\"", "document.wholeText()", "RedactionMarks.spoken(",
            "TranscriptResidentChunks", "TranscriptRowIndex(", "ScopeCopy.title(for:", "summary.consentScopes",
            "GlassTokens.TypeScale.mono",
        ] {
            XCTAssertTrue(sheet.contains(needle), "PreviewSheet.swift lacks \(needle)")
        }
        try LegacySymbols.assertClean("Views/PreviewSheet.swift")
    }

    /// The tabs are drawn with glass parts, each pinned by its exact call:
    /// the search highlight and the redaction chip on the existing tokens
    /// (no new token), every outcome a dot with its words, the figures a
    /// key-value list, and the sheet held to the glass surface rules.
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
            ".textFieldStyle(.plain)\n                    .glassType(GlassTokens.TypeScale.label.weight(.regular))\n",
            ".fill(GlassTokens.Color.fieldFill.color)",
            "GlassCard(quiet: true) {\n                            Text(highlighting(snippet, term: needle))\n"
                + "                                .glassType(GlassTokens.TypeScale.mono)\n",
            // What's in it
            "GlassKeyValueList(items)",
            "GlassKeyValueList.Item(\"Agent\", entry.agentName)",
            "GlassKeyValueList.Item(\"Would send\", Format.bytes(summary.wouldSendBytes))",
            "GlassSectionRule(\"What scrubbing removed\")",
            "GlassSectionRule(\"Found, and still in what would be sent\")",
            "GlassSectionRule(\"Personal-information categories seen\")",
            "GlassSectionRule(\"Residual risk\")",
            "GlassStatusLabel(row.description, status: .ask)",
            "GlassNotice(tone: .ask) {",
            // Transcript
            "Text(TranscriptMarkers.chipped(Self.caption, font: GlassTokens.TypeScale.caption.font))\n"
                + "                .glassType(GlassTokens.TypeScale.caption)\n",
            "chip.backgroundColor = GlassTokens.Color.controlSelected.color\n"
                + "            chip.foregroundColor = GlassColor.textPrimary\n",
            ".glassType(GlassTokens.TypeScale.mono)\n                    .textSelection(.enabled)\n",
            "TranscriptMarkers.chipped(text, font: GlassTokens.TypeScale.mono.font)",
            "NSLayoutManager().defaultLineHeight(for: font) + GlassTokens.TypeScale.mono.lineSpacing",
            "GlassCard(quiet: true, flush: true) {",
            // Permissions
            "GlassSectionRule(\"What this upload asks for\")",
            "Text(ScopeCopy.title(for: scope, options: options))\n                            .glassType(GlassTokens.TypeScale.bodyStrong)\n",
        ] {
            XCTAssertTrue(sheet.contains(needle), "PreviewSheet.swift lacks \(needle)")
        }
        // The transcript's columns are measured against the very inset the
        // chunks are drawn inside, or every placeholder is the wrong height.
        XCTAssertTrue(sheet.contains(".padding(.horizontal, TranscriptTab.inset)"))
        XCTAssertTrue(sheet.contains("width - 2 * TranscriptTab.inset"))
        // No sentence is drawn in the uppercased eyebrow inside the tabs.
        let tabsStart = try XCTUnwrap(sheet.range(of: "// MARK: - Tabs")).lowerBound
        let tabsEnd = try XCTUnwrap(sheet.range(of: "enum ScopeCopy {")).lowerBound
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

    /// Undo and the consent offers sit above the tree, so they are on
    /// screen with the inspector hidden (spec, "Behaviour that must not
    /// depend on the inspector"), and read the same model members and core
    /// words the legacy queue does.
    func test_offersAndUndoLiveAboveTheTree() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let inspector = try XCTUnwrap(tree.range(of: "struct SessionInspectorView"))
        XCTAssertTrue(tree[treeView.lowerBound..<inspector.lowerBound].contains("TracesOffersBar(store: store)"))
        XCTAssertFalse(tree[inspector.lowerBound...].contains("pendingUndo"), "undo must not live only in the inspector")
        XCTAssertTrue(tree.contains("Text(QueueLegacyWords.nothingWaiting)"))
        XCTAssertTrue(tree.contains("Text(QueueLegacyWords.nothingWaitingDetail)"))
        // A refused undo is said beside it, by the one helper the inspector
        // shares.
        XCTAssertTrue(tree.contains("TracesRefusal(store: store, entryId: entry.entryId)"))

        let offers = try Self.text("Views/Monitor/TracesOffers.swift")
        for needle in [
            "if let undo = model.undo {", "approvalUndo(undo)", "{ model.undoApproval() }", "{ model.dismissUndo() }",
            "QueueLegacyWords.undoWillSend", "QueueLegacyWords.closeNoticeStillSends", "QueueLegacyWords.closeNotice)",
            "QueueLegacyWords.approvedAgo(undo.heldSeconds)", "if let contributed = store.lastContributed,",
            "if let kept = store.lastKept,",
            "store.perform(.undoContribute, on: contributed.entryId)", "store.perform(.undoKeep, on: kept)",
            "model.showsPrivateInferenceOffer", "model.answerPrivateInferenceOffer(accepted: true)",
            "model.answerPrivateInferenceOffer(accepted: false)", "model.privateInferenceBusy",
            "copy.offerWhat", "copy.offerExposure", "copy.offerNoRepoint", "copy.offerAskedOnce",
            "model.armingOffer", "model.acceptArmingOffer(", "model.declineArmingOffer(",
            "TCCoreCopy.armingOfferCopyJSON(", "CertificateSection(entries: model.awaitingDecision)",
            "model.outcomeCounts", "TCOutcome.line(label:", "QueueLegacyWords.noLongerWaiting(",
            "QueueLegacyWords.notOfferedScope", "model.lastActionError = nil", "model.lastActionNotice = nil",
            "model.witnessCopy?.onboarding", "QueueLegacyWords.agentSetup", "QueueLegacyWords.undo)",
            "ActionMessageBanner.dismissWord",
        ] {
            XCTAssertTrue(offers.contains(needle), "TracesOffers.swift lacks \(needle)")
        }
        // Both undos are drawn by the bar's body, not merely defined.
        let barBody = try XCTUnwrap(offers.range(of: "    var body: some View {"))
        let firstHelper = try XCTUnwrap(offers.range(of: "    private func approvalUndo("))
        let body = offers[barBody.upperBound..<firstHelper.lowerBound]
        XCTAssertTrue(body.contains("approvalUndo(undo)\n"), "the approval undo is not drawn")
        XCTAssertTrue(body.contains("            storeUndo\n"), "the store's undos are not drawn")
        // Return is bound to Undo itself: no other button between the two.
        let undoButton = try XCTUnwrap(offers.range(
            of: "Button(store.words?.undoContribute ?? QueueLegacyWords.undo) { model.undoApproval() }"))
        let shortcut = try XCTUnwrap(offers.range(
            of: ".keyboardShortcut(.defaultAction)", range: undoButton.upperBound..<offers.endIndex))
        XCTAssertFalse(offers[undoButton.upperBound..<shortcut.lowerBound].contains("Button("))
        XCTAssertEqual(offers.components(separatedBy: ".keyboardShortcut(.defaultAction)").count, 2)
        // The arming card draws nothing without the core's words.
        let armingCard = try XCTUnwrap(offers.range(of: "private struct ArmingOfferGlassCard"))
        let armingBody = try XCTUnwrap(offers.range(
            of: "    var body: some View {\n", range: armingCard.upperBound..<offers.endIndex))
        XCTAssertTrue(offers[armingBody.upperBound...].hasPrefix("        if let copy {\n"))
        // No control without words: an absent core word falls back to an
        // existing one, never to an empty title.
        XCTAssertFalse(offers.contains("?? \"\""), "a control would be wordless without the core")
        // An unread list is not an empty one: the certificate list and the
        // first-contribution note wait for the daemon's answer, and draw
        // nothing before it (standing rule: fail closed).
        let flatOffers = offers.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        for needle in [
            "if model.queueAnswered { CertificateSection(entries: model.awaitingDecision) }",
            "if model.historyAnswered, model.queueAnswered, model.history.isEmpty, "
                + "let copy = model.witnessCopy?.onboarding { "
                + "FirstContributionGlassNote(copy: copy, reviewing: !model.awaitingDecision.isEmpty) }",
        ] {
            XCTAssertTrue(flatOffers.contains(needle), "TracesOffers.swift lacks \(needle)")
        }
        XCTAssertEqual(offers.components(separatedBy: "CertificateSection(").count - 1, 1)
        XCTAssertEqual(offers.components(separatedBy: "FirstContributionGlassNote(copy:").count - 1, 1)
        try LegacySymbols.assertClean("Views/Monitor/TracesOffers.swift")
        XCTAssertTrue(GlassSurfaceRulesTests.files.contains("Views/Monitor/TracesOffers.swift"))

        // The legacy queue reads the same table, one literal per sentence.
        let queue = try Self.text("Views/QueueView.swift")
        for needle in [
            "Text(QueueLegacyWords.undoWillSend)", "QueueLegacyWords.closeNoticeStillSends",
            "QueueLegacyWords.approvedAgo(undo.heldSeconds)", "Text(QueueLegacyWords.noLongerWaiting(",
            "Text(QueueLegacyWords.notOfferedScope)", "title: QueueLegacyWords.nothingWaiting,",
            "detail: QueueLegacyWords.nothingWaitingDetail", "Button(QueueLegacyWords.lookInside,",
            "DisclosureGroup(QueueLegacyWords.agentSetup)", "Button(QueueLegacyWords.undo,",
        ] {
            XCTAssertTrue(queue.contains(needle), "QueueView.swift lacks \(needle)")
        }
    }

    /// The words table holds the legacy sentences verbatim.
    func test_theQueueWordsAreTheLegacySentences() {
        XCTAssertEqual(QueueLegacyWords.nothingWaiting, "Nothing is waiting.")
        XCTAssertEqual(
            QueueLegacyWords.nothingWaitingDetail,
            "When a session finishes and goes quiet, it shows up here. Nothing is sent unless you say so.")
        XCTAssertEqual(
            QueueLegacyWords.undoWillSend,
            "Approved sessions will send automatically. You can undo until uploading starts.")
        XCTAssertEqual(
            QueueLegacyWords.closeNoticeStillSends,
            "Close this notice. Approved sessions will still send automatically.")
        XCTAssertEqual(QueueLegacyWords.closeNotice, "Close this notice.")
        XCTAssertEqual(QueueLegacyWords.approvedAgo(7), "Approved 7s ago")
        XCTAssertEqual(QueueLegacyWords.approvedAgo(AppModel.Undo.tickCeiling), "Approved 120s+ ago")
        XCTAssertEqual(QueueLegacyWords.approvedAgo(500), "Approved 120s+ ago")
        XCTAssertEqual(QueueLegacyWords.noLongerWaiting(3), "Sessions no longer waiting (3)")
        XCTAssertEqual(
            QueueLegacyWords.notOfferedScope,
            "This covers sessions that reached the queue. Sessions that were never queued at all are not counted here.")
    }

    /// Declining comes first and neither answer is the primary action, on
    /// both offers (`QueueView.swift:1262-1268, 1321-1325`).
    func test_neitherOfferLeadsTheEyeToYes() throws {
        let offers = try Self.text("Views/Monitor/TracesOffers.swift")
        let decline = try XCTUnwrap(offers.range(of: "copy.offerDecline"))
        let accept = try XCTUnwrap(offers.range(of: "copy.offerAccept"))
        XCTAssertLessThan(decline.lowerBound, accept.lowerBound)
        let armDecline = try XCTUnwrap(offers.range(of: "Button(copy.decline"))
        let armConfirm = try XCTUnwrap(offers.range(of: "Button(copy.confirm"))
        XCTAssertLessThan(armDecline.lowerBound, armConfirm.lowerBound)
        let arming = try XCTUnwrap(offers.range(of: "struct ArmingOfferGlassCard"))
        XCTAssertFalse(offers[arming.lowerBound...].prefix(1500).contains("GlassButtonStyle(.primary"))
        let privateAI = try XCTUnwrap(offers.range(of: "struct PrivateAIOfferGlassCard"))
        XCTAssertFalse(offers[privateAI.lowerBound..<arming.lowerBound].contains("GlassButtonStyle(.primary"))
    }

    /// The folder row offers Submit all and Submit all as only when the
    /// shared table offers Contribute, with the daemon's counts, and says
    /// what it withheld.
    func test_theFolderRowSubmitsAllAsTheQueueDid() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        for needle in ["store.groupOffer(", "offer.offersContribute", "QueueFolderWords.submitAll(offer.count)",
                       "QueueFolderWords.submitAllHelp(", "store.contributeFolder(", "VerdictCopy.submitAllAs",
                       "ContributorVerdict.allCases", "offer.withheldLine", "submitTitle: submits ?", "onSubmit: submits ?",
                       "onMenu: submits ?", "menuLabel: submits ?", "store.mayContributeFolder(folder)"] {
            XCTAssertTrue(tree.contains(needle), "TracesViews.swift lacks \(needle)")
        }
        let store = try Self.text("Views/Monitor/TracesStore.swift")
        for needle in ["approveFolder(projectId: folder.id, verdict: verdict)", "verdict:", "excludedIneligible", "withheldLine(", "cancelFolder(projectId:",
                       "EligibilitySurface.groupSubmit("] {
            XCTAssertTrue(store.contains(needle), "TracesStore.swift lacks \(needle)")
        }
        XCTAssertTrue(try Self.text("Views/Monitor/TracesOffers.swift").contains("store.lastContributedFolder"))
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
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let inspector = try XCTUnwrap(tree.range(of: "struct SessionInspectorView"))
        let end = try XCTUnwrap(tree.range(of: "struct PreviewSlot", range: inspector.upperBound..<tree.endIndex))
        let body = tree[inspector.lowerBound..<end.lowerBound]
        for needle in ["TracesStore.survivorLine(summary)", "GlassStatusLabel(survivor, status: .ask)",
                       "RedactionLabels.removedTotal(redactions)",
                       "ScrubbingCaveat.rowLine(redactionCount: removed)",
                       "ScrubbingCaveat.status(redactionCount: removed)",
                       "SubagentCopy.line(count: entry.subagentCount ?? 0, dropped: entry.subagentsDropped ?? 0)",
                       ".sheet(item: $previewing)",
                       "PreviewSheet(entry: $0)", "@EnvironmentObject private var model: AppModel",
                       #"ProcessInfo.processInfo.environment["TRACE_COMMONS_DEMO_PREVIEW"] == "1","#,
                       ".onChange(of: model.awaitingDecision.count)", "previewing == nil,"] {
            XCTAssertTrue(body.contains(needle), "SessionInspectorView lacks \(needle)")
        }
        // Look inside is absent, not disabled, when the legacy queue does not
        // hold this session: the guard and the button are one needle.
        XCTAssertTrue(body.contains("""
                                if let legacy = QueueEntryBridge.legacyEntry(for: entry.entryId, in: model.awaitingDecision) {
                                    Button(QueueLegacyWords.lookInside) { previewing = legacy }
                                        .buttonStyle(GlassButtonStyle(.glass))
                                }
        """), "Look inside must be drawn only inside the legacyEntry guard")
        // The sheet and the demo hook hang off the always-present container,
        // not the selected-session branch.
        XCTAssertTrue(body.contains("""
                        Spacer(minLength: 0)
                    }
                }
                .sheet(item: $previewing) { PreviewSheet(entry: $0).environmentObject(model) }
                .onChange(of: model.awaitingDecision.count) { _, _ in
        """), "the sheet and the demo hook must sit on the inspector's outer VStack")
        XCTAssertTrue(try Self.text("Views/Monitor/TracesStore.swift")
            .contains("TCCoreCopy.residualSecretLine(count: total,"))
    }
}
#endif
