import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// History against the legacy screens: withdrawal, the session detail and
/// the public-run editor keep every binding, rule and core-copy source.
final class HistoryParityTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// The owner's rule: an unknown status is terminal and offers no
    /// Withdraw; `received` and `rejected` stay withdrawable.
    func test_theWithdrawRuleIsTheOwners() {
        XCTAssertTrue(ContributionStatusPresentation.offersWithdraw("received"))
        XCTAssertTrue(ContributionStatusPresentation.offersWithdraw("rejected"))
        XCTAssertFalse(ContributionStatusPresentation.offersWithdraw("a-status-from-the-future"))
        XCTAssertFalse(ContributionStatusPresentation.offersWithdraw("withdrawn"))
    }

    /// A failed `history_rollup` read shows a dash in every History cell the
    /// rollup feeds, never the store's stale value, and History says the
    /// read failed (the Summary inspector's `SummaryFacts.fresh` rule).
    func test_aFailedRollupReadIsNeverShownAsCurrent() throws {
        let source = try Self.text("Views/Monitor/HomeViews.swift")
        let fresh = #"SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])"#
        for name in ["HistoryStats", "HistoryCommunityCard", "HistoryCreditCard"] {
            let start = try XCTUnwrap(source.range(of: "struct \(name): View {"), "\(name) is gone")
            let end = source.range(of: "\n}\n", range: start.upperBound..<source.endIndex)?.lowerBound ?? source.endIndex
            let body = String(source[start.upperBound..<end])
            XCTAssertTrue(body.contains(fresh), "\(name) does not read the rollup through SummaryFacts.fresh")
            XCTAssertFalse(body.contains("store.rollup?"), "\(name) reads store.rollup directly")
        }
        let page = try XCTUnwrap(source.range(of: "private struct HistoryPage: View {"))
        let pageEnd = try XCTUnwrap(source.range(of: "\n}\n", range: page.upperBound..<source.endIndex))
        let pageBody = String(source[page.upperBound..<pageEnd.lowerBound])
        XCTAssertTrue(pageBody.contains(#"if let failure = store.failures["history_rollup"] {"#),
                      "History draws no notice for a failed history_rollup read")
        XCTAssertFalse(pageBody.contains("store.rollup?"), "HistoryPage reads store.rollup directly")
        XCTAssertFalse(pageBody.contains("rollup: store.rollup)"), "HistoryPage reads store.rollup directly")
        // Home's stat cards read the same rollup, by the same rule.
        XCTAssertFalse(source.contains("store.rollup?"), "HomeViews.swift reads store.rollup directly")
    }

    /// A failed `commons_credit_summary` read shows neither the earlier
    /// settlement sentence nor a pending figure beside it: Home's stat
    /// cards, History's stat cards and the credit record read the summary
    /// through `SummaryFacts.fresh`, as the Summary inspector does.
    /// `HomeStore.read` keeps the old value when a read fails.
    func test_aFailedCreditReadIsNeverShownAsCurrent() throws {
        let source = try Self.text("Views/Monitor/HomeViews.swift")
        let fresh = #"SummaryFacts.fresh(store.credit, unless: store.failures["commons_credit_summary"])"#
        for name in ["private struct HomeOverview", "private struct HistoryStats", "struct HistoryCreditCard"] {
            let start = try XCTUnwrap(source.range(of: "\(name): View {"), "\(name) is gone")
            let end = source.range(of: "\n}\n", range: start.upperBound..<source.endIndex)?.lowerBound ?? source.endIndex
            let body = String(source[start.upperBound..<end])
            XCTAssertTrue(body.contains(fresh), "\(name) does not read the credit summary through SummaryFacts.fresh")
        }
        XCTAssertFalse(source.contains("credit: store.credit"), "HomeViews.swift reads store.credit directly")
        XCTAssertFalse(source.contains("pendingCondition(store.credit"), "HomeViews.swift reads store.credit directly")
    }

    /// The overview's processing status reads the core's unavailable word
    /// for a status it does not name, the same word History's rows read.
    func test_anUnknownStatusReadsStatusUnavailable() throws {
        let copy = try XCTUnwrap(PublicRunCopy.decode(fromJSON: TCPublicRun.copyJSON() ?? ""))
        XCTAssertEqual(copy.contributionStatusLabel(for: "a-status-from-the-future"), copy.contributionStatusUnavailable)
        XCTAssertEqual(copy.historyStatusLabel(for: "a-status-from-the-future"), copy.contributionStatusUnavailable)
    }

    func test_withdrawalKeepsItsConfirmationAndOutcome() throws {
        let source = try Self.text("Views/SessionContributionOverview.swift")
        for needle in [
            "ContributionStatusPresentation.offersWithdraw(currentStatus)", "model.withdrawals[record.submissionID]",
            "model.withdrawing.contains(record.submissionID)", "model.withdraw(record)", "WithdrawalCopy.confirmation(for:",
            "confirmation.gravest", "confirmation.credit", "confirmation.confirmLabel", ".keyboardShortcut(.cancelAction)",
            "WithdrawalCopy.resultSentence(", "WithdrawalCopy.accountSessionRequired", "WithdrawalCopy.failureSentence(label:",
            "copy.historyStatusLabel(for: detail.contributionStatus ?? record.status)", "copy.permittedUsesUnavailable",
            "copy.noPermittedUses", "copy.contentUnavailable", "minHeight: 44",
        ] {
            XCTAssertTrue(source.contains(needle), "SessionContributionOverview.swift lacks \(needle)")
        }
        // One History table for a status (ruling R-17): never the session
        // detail's "Submitted", which reads as done.
        XCTAssertFalse(source.contains("contributionStatusLabel("),
                       "SessionContributionOverview.swift reads the session-detail status table")
        try LegacySymbols.assertClean("Views/SessionContributionOverview.swift")
    }

    /// The glass shapes the brief names: the next-action card gates on the
    /// withdraw rule outside it, Keep is the cancel action in both branches,
    /// and the gravest body is the one drawn as outside.
    func test_withdrawalIsDrawnOnGlass() throws {
        let source = try Self.text("Views/SessionContributionOverview.swift")
        for needle in [
            "GlassEyebrowCard(copy.task)", "GlassEyebrowCard(copy.decisiveCorrection)",
            "GlassEyebrowCard(copy.supportingEvidence)", "GlassEyebrowCard(copy.contributionDetails)",
            "GlassNotice(tone: .off) {", "GlassStatusLabel(copy.permittedUseLabel(for: use), status: .on)",
            "GlassKeyValueList([", ".init(copy.envelopeVersion, detail.contributedVersion, mono: true)",
            ".init(copy.consentPolicyVersion, detail.consentPolicyVersion, mono: true)",
            ".init(copy.redactionVersion, detail.redactionPipelineVersion, mono: true)",
            "GlassEyebrowCard(copy.nextAction)", "status: index == confirmation.gravest ? .outside : .off",
            "Button(inFlight ? confirmation.busyLabel : confirmation.confirmLabel, action: onConfirm)",
            ".buttonStyle(GlassButtonStyle(.primary))", "GlassWell {",
        ] {
            XCTAssertTrue(source.contains(needle), "SessionContributionOverview.swift lacks \(needle)")
        }
        XCTAssertTrue(source.contains("""
                if isWithdrawable || model.withdrawals[record.submissionID] != nil {
                    GlassEyebrowCard(copy.nextAction) {
        """), "the withdraw rule must gate the next-action card from outside it")
        let flat = source.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let keep = "Button(keepLabel, action: onKeep) .buttonStyle(GlassButtonStyle(.glass)) "
            + ".keyboardShortcut(.cancelAction) .frame(minHeight: 44)"
        XCTAssertEqual(flat.components(separatedBy: keep).count - 1, 2,
                       "Keep is the cancel action with and without the core's words")
        XCTAssertTrue(GlassSurfaceRulesTests.files.contains("Views/SessionContributionOverview.swift"))
    }

    /// Withdraw first asks: the button only opens the confirmation, the
    /// confirmation is what withdraws, and nothing else does but Retry. The
    /// gravest consequence is set in a heavier weight, not only a colour.
    func test_withdrawAlwaysConfirmsFirst() throws {
        let source = try Self.text("Views/SessionContributionOverview.swift")
        let flat = source.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        for needle in [
            "Button(copy.withdraw) { confirming = true } .buttonStyle(GlassButtonStyle(.glass)) .frame(minHeight: 44)",
            "} else if confirming { WithdrawalConfirmationView(",
            "onConfirm: { model.withdraw(record) }",
            ".buttonStyle(GlassButtonStyle(.primary)) .frame(minHeight: 44) .disabled(inFlight)",
            "GlassStatusLabel(body, status: index == confirmation.gravest ? .outside : .off) "
                + ".fontWeight(index == confirmation.gravest ? .semibold : nil)",
        ] {
            XCTAssertTrue(flat.contains(needle), "SessionContributionOverview.swift lacks \(needle)")
        }
        XCTAssertEqual(source.components(separatedBy: "model.withdraw(record)").count - 1, 2,
                       "only Retry and the confirmation withdraw")
    }

    /// The session detail keeps every legacy gate: the reloads, the shared
    /// Skills predicate (asked, never re-derived here), the editor only for
    /// an active contribution, the core's validation, the evidence cap and
    /// the reuse choice.
    func test_theSessionDetailKeepsItsGatesAndTheEditor() throws {
        let source = try Self.text("Views/SessionDetailView.swift")
        for needle in [
            "model.loadSessionDetail(record)", "NSApplication.didBecomeActiveNotification",
            "model.ensureLocalInstalledSkillStatus(for: record)",
            "SessionContributionOverview(record: record, detail: detail, copy: copy)",
            Self.withdrawCall,
            "SkillLearningView(record: record, copy: skillCopy)",
            "SkillLearningGate.offersLearning(detail, recordStatus: record.status, withdrawalCompleted: withdrawalCompleted)",
            "Text(copy.publicationAfterAcceptance)",
            "PublicRunEditor(record: record, detail: detail, copy: copy)", "TCPublicRun.validateEditorJSON(json)",
            "model.publishPublicRun(record, draft: draft)", "model.unpublishPublicRun(record)",
            "maximum: 100)", "maximum: 600)", "maximum: 4_000)", "ForEach(copy.reusePermissions)",
            "Text(copy.exactPublicPreview)", "model.publicRunErrors[record.submissionID]",
            "Text(copy.choosePermission)", "Text(copy.publicationDisclosure)",
        ] {
            XCTAssertTrue(source.contains(needle), "SessionDetailView.swift lacks \(needle)")
        }
        XCTAssertEqual(source.components(separatedBy: "Button(copy.retryRead)").count - 1, 2,
                       "the read error and the installed-skill status each offer Retry")
        XCTAssertTrue(source.contains("let onBack: (() -> Void)?"))
        XCTAssertTrue(source.contains("init(record: HistoryRecord, offersWithdrawal: Bool = true, onBack: (() -> Void)? = nil)"),
                      "the inspector draws it without a back control, and History's pane without Withdraw")
        try LegacySymbols.assertClean("Views/SessionDetailView.swift")
    }

    /// The glass shapes, and the publish gates pinned where they sit: Review
    /// page needs a draft, the fifth evidence item is refused, and nothing
    /// publishes or unpublishes twice.
    func test_theSessionDetailIsDrawnOnGlass() throws {
        let source = try Self.text("Views/SessionDetailView.swift")
        let flat = source.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        for needle in [
            // The reloads hang on a container that is always there.
            "VStack(alignment: .leading, spacing: GlassTokens.Space.s6) { if let copy = model.publicRunCopy "
                + "{ content(copy) } } .frame(maxWidth: .infinity, alignment: .leading) .onAppear {",
            // One heading: the trail's current crumb, or the title when there
            // is no trail (ruling R-31).
            "if let onBack { GlassBreadcrumb([GlassCrumb(copy.allContributions, action: onBack), "
                + "GlassCrumb(copy.sessionDetail)], backLabel: copy.allContributions, onBack: onBack) "
                + ".frame(minHeight: 44, alignment: .leading) } else { Text(copy.sessionDetail) "
                + ".glassType(GlassTokens.TypeScale.title)",
            "GlassNotice(tone: .outside, title: message) { Button(copy.retryRead) { model.loadSessionDetail(record) } "
                + ".buttonStyle(GlassButtonStyle(.glass)) .frame(minHeight: 44) }",
            "GlassEyebrowCard(copy.publicWorkflow) {",
            "GlassTag(copy.published, tone: .on)",
            "Link(copy.openPage, destination: url) .buttonStyle(GlassButtonStyle(.primary)) .frame(minHeight: 44)",
            "Button(working ? copy.unpublishing : copy.unpublish) { model.unpublishPublicRun(record) } "
                + ".buttonStyle(GlassButtonStyle(.glass)) .frame(minHeight: 44) .disabled(working)",
            "Text(\"\\(count)/\\(maximum)\") .glassType(GlassTokens.TypeScale.mono)",
            ".toggleStyle(GlassCheckboxStyle()) .disabled( selectedEvidence.count >= 4 "
                + "&& !selectedEvidence.contains(evidence.eventID) ) .frame(minHeight: 44, alignment: .leading)",
            "Toggle(copy.publishCorrection, isOn: $includeCorrection) .toggleStyle(GlassCheckboxStyle()) "
                + ".frame(minHeight: 44, alignment: .leading)",
            "GlassCheckMark(checked: selected)",
            ".accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)",
            "GlassStatusLabel(problem, status: .outside)",
            "Button(copy.reviewPage) { reviewDraft = makeDraft() } .buttonStyle(GlassButtonStyle(.primary)) "
                + ".frame(minHeight: 44) .disabled(makeDraft() == nil)",
            "Button(copy.cancelEdit) { editingPublished = false } .buttonStyle(GlassButtonStyle(.glass))",
            "Button(copy.editDraft) { reviewDraft = nil } .buttonStyle(GlassButtonStyle(.glass))",
            "Button(working ? copy.publishing : detail.publication == nil ? copy.publishPage : copy.updatePage) "
                + "{ model.publishPublicRun(record, draft: draft) } .buttonStyle(GlassButtonStyle(.primary)) "
                + ".frame(minHeight: 44) .disabled(working)",
            // Fields carry their names for VoiceOver.
            "GlassTextField(copy.pageTitle, text: $title, prompt: copy.pageTitle, showsLabel: false)",
            "GlassTextArea(copy.publicOutcome, text: $outcomeSummary, showsLabel: false)",
            "GlassTextArea(copy.reusableInstructions, text: $workflow, showsLabel: false)",
            "GlassTextField(copy.sourcePlaceholder, text: $source, prompt: copy.sourcePlaceholder, showsLabel: false)",
            // A publication error can be put away; the next attempt shows it again.
            "Button(ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord) { dismissedError = message }",
        ] {
            XCTAssertTrue(flat.contains(needle), "SessionDetailView.swift lacks \(needle)")
        }
        XCTAssertTrue(GlassSurfaceRulesTests.files.contains("Views/SessionDetailView.swift"))
    }

    /// One implementation of the core's dismiss word: the notices that can
    /// be put away read it from `ActionNoticeWords`, beside its fallback.
    func test_theDismissWordIsDecodedOnce() throws {
        let decode = "MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())?.dismiss"
        let words = try Self.text("Views/SettingsView.swift")
        XCTAssertTrue(words.contains("static let coreDismissWord = \(decode)"))
        for rel in ["Views/SessionDetailView.swift", "Views/SkillLearningView.swift", "Views/Settings/ProjectsSection.swift"] {
            let source = try Self.text(rel)
            XCTAssertFalse(source.contains(decode), "\(rel) decodes the dismiss word again")
        }
        for rel in ["Views/SessionDetailView.swift", "Views/SkillLearningView.swift"] {
            let source = try Self.text(rel)
            XCTAssertTrue(source.contains("Button(ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord) {"),
                          "\(rel) lacks the shared dismiss word")
        }
        // Projects puts its error away with the banner's x, named by the
        // banner's word, never Traces' dismiss verb (ActionNoticeDismissTests).
        let projects = try Self.text("Views/Settings/ProjectsSection.swift")
        XCTAssertFalse(projects.contains("ActionNoticeWords.coreDismissWord"))
    }

    /// Publishing a public page always passes the exact preview: the only
    /// publish call is in the review state, which only a validated draft
    /// reaches, and a published page shows itself until Edit is chosen.
    func test_publishingRequiresTheExactPreview() throws {
        let source = try Self.text("Views/SessionDetailView.swift")
        let flat = source.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        XCTAssertEqual(source.components(separatedBy: "model.publishPublicRun(").count - 1, 1,
                       "only the review state publishes")
        XCTAssertEqual(source.components(separatedBy: "model.unpublishPublicRun(").count - 1, 1,
                       "only the published state unpublishes")
        let review = try XCTUnwrap(source.range(of: "private func review(_ draft: PublicRunDraftInput)"))
        let after = try XCTUnwrap(source.range(of: "private func previewField(", range: review.upperBound..<source.endIndex))
        let publish = try XCTUnwrap(source.range(of: "model.publishPublicRun("))
        XCTAssertTrue(review.upperBound <= publish.lowerBound && publish.upperBound <= after.lowerBound,
                      "the publish call lies inside review(_:)")
        for needle in [
            "if let publication = detail.publication, !editingPublished { published(publication) } "
                + "else if let reviewDraft { review(reviewDraft) } else { editor }",
            // Fail closed: an unreadable validation is a problem, never a pass,
            // and the binding refuses a fifth item even if the box is reached.
            "guard let editorValidation else { return copy.publicationUnavailable }",
            "if selected { guard selectedEvidence.count < 4 else { return } selectedEvidence.insert(id) }",
        ] {
            XCTAssertTrue(flat.contains(needle), "SessionDetailView.swift lacks \(needle)")
        }
    }

    /// The legacy window passes Back as a trailing closure; the inspector
    /// passes nothing.
    @MainActor
    func test_theSessionDetailIsBuiltWithAndWithoutBack() {
        let record = HistoryRecord(
            submissionID: "sub-1", submittedAt: Date(timeIntervalSince1970: 0), projectID: "project",
            projectLabel: "project", source: "claude_code", status: "accepted", consentScopes: [],
            creditPointsPending: 0, creditPointsFinal: nil, explanations: [], lastRefreshedAt: nil)
        XCTAssertNil(SessionDetailView(record: record).onBack)
        XCTAssertNotNil(SessionDetailView(record: record) {}.onBack)
    }

    func test_heldExplanationsAreDistinctAndCarryNoDigest() {
        let lines = HeldExplanations.lines(in: [
            ["Held for a closer look.", "Attributed to tenant tenant_sha256:abc"],
            ["Held for a closer look.", "A second reason."],
        ])
        XCTAssertEqual(lines, ["Held for a closer look.", "A second reason."])
    }

    /// The selected row's inspector offers Withdraw only through the
    /// resolved record and its rule, shows the status in the core's words,
    /// and draws the session detail (public run, Skills) beneath. A row
    /// whose record does not resolve draws its status alone.
    func test_theInspectorHostsWithdrawTheDetailAndTheStatus() throws {
        let inspector = try Self.text("Views/Monitor/HistoryInspector.swift")
        let flat = inspector.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        for needle in [
            "HistorySelection.record(for: row.submissionId, in: model.history)",
            "if let record { explanations(record) SessionDetailView(record: record, offersWithdrawal: false) .id(record.submissionID) }",
            // The resolved record's status when there is one, the word its
            // Withdraw outcome agrees with; the list row's otherwise.
            "let status = record?.status ?? row.status",
            // Said once: the tag only while the session detail's overview,
            // which says the status from the same table, is not drawn.
            "if HistorySelection.tagsStatus(record: record, "
                + "detail: record.flatMap { model.sessionDetails[$0.submissionID] }), "
                + "let tag = HomeFormat.statusWord(status, label: { HomeFormat.historyStatusLabel(copy: model.publicRunCopy, $0) }) "
                + "{ GlassTag(tag, tone: HomeFormat.tone(status))",
            // The folder and day only while no record resolves: the session
            // detail's own heading carries them otherwise.
            "if record == nil { Text(row.projectLabel ?? \"—\")",
            "Text(HomeFormat.meta(row, compact: false))",
            "let failures = WithdrawalCopyCheck.failures()",
            "if !failures.isEmpty { GlassNotice(tone: .outside, title: HistoryLegacyWords.withdrawalWordingDefect) "
                + "{ VStack(alignment: .leading, spacing: GlassTokens.Space.s2) { ForEach(failures, id: \\.self) { Text($0)",
            // The record's own reasons, digest-free; a held record the server
            // said nothing about reads the held sentence.
            "let lines = HeldExplanations.lines(in: [record.explanations])",
            "ForEach(lines, id: \\.self) { Text($0)",
            // The core's held sentence (R-37), and no blank line without it.
            "if lines.isEmpty, record.status == \"quarantined\", !MonitorWords.heldExplanation.isEmpty "
                + "{ Text(MonitorWords.heldExplanation)",
        ] {
            XCTAssertTrue(flat.contains(needle), "HistoryInspector.swift lacks \(needle)")
        }
        // One Withdraw, one Skills panel, one reading state, one reload on
        // activation: the session detail's, never a second copy beside it.
        for absent in ["SessionWithdrawalAction(", "SkillLearningView(", "SkillLearningGate.", "readingRecord",
                       "sessionDetailErrors", "didBecomeActiveNotification"] {
            XCTAssertFalse(inspector.contains(absent), "HistoryInspector.swift draws its own \(absent)")
        }

        let home = try Self.text("Views/Monitor/HomeViews.swift")
        let homeFlat = home.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        for needle in [
            // Never an empty title or sentence: only with the core's words.
            // Ron's PRIVACY REVIEW card, its count as the heading.
            "if let quarantined = rollup?.quarantined, quarantined > 0, let words = MonitorWords.table { "
                + "GlassEyebrowCard(words.shell.filterQuarantined) {",
            "Text(words.homeHistory.held(quarantined))",
            "Text(words.heldExplanation)", "Text(HistoryLegacyWords.typicalWait)",
            // Every record the app holds, not the list's capped page.
            "ForEach(HeldExplanations.lines(in: model.history.filter { $0.status == \"quarantined\" }.map(\\.explanations)), "
                + "id: \\.self)",
            "Text(WithdrawalCopy.noBulkAction)",
            // The daemon's view on every visit, on the always-present container.
            ".scrollIndicators(.never) .onAppear { model.clearHistoryRefresh() model.refreshHistory() model.refreshAccountSession() }",
        ] {
            XCTAssertTrue(homeFlat.contains(needle), "HomeViews.swift lacks \(needle)")
        }

        // The legacy screen is gone (R15); its words are the core's now
        // (#1146 parity, 2026-10-07), and its file holds none.
        let legacy = try Self.text("Views/HistoryView.swift")
        for sentence in ["Typical wait", "Do not trust the withdrawal wording"] {
            XCTAssertFalse(legacy.contains(sentence), "\(sentence) is written in Swift")
        }
        XCTAssertEqual(HistoryLegacyWords.typicalWait, "Typical wait: we don't have a reliable number yet.")
        XCTAssertEqual(HistoryLegacyWords.withdrawalWordingDefect, "Do not trust the withdrawal wording on this screen.")
    }

    static let withdrawCall = "SessionWithdrawalAction(record: record, "
        + "currentStatus: Self.withdrawalStatus(record, detail: model.sessionDetails[record.submissionID]), copy: copy)"

    /// Withdraw stands on the app's own record, as the legacy row did: one
    /// call, after the detail's states rather than inside the read detail,
    /// so reading or a failed read never hides it or its outcome.
    func test_withdrawDoesNotWaitForTheDetailRead() throws {
        let source = try Self.text("Views/SessionDetailView.swift")
        let flat = source.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        XCTAssertEqual(source.components(separatedBy: "SessionWithdrawalAction(").count - 1, 1)
        XCTAssertTrue(flat.contains(
            "if let detail = model.sessionDetails[record.submissionID] { detailContent(detail, copy: copy) } "
                + "if offersWithdrawal { \(Self.withdrawCall) } localInstalledSkillSurface(copy)"),
            "Withdraw sits after the detail's reading/failed/read states, outside them")
        XCTAssertTrue(source.contains(
            "static func withdrawalStatus(_ record: HistoryRecord, detail: SessionDetail?) -> String {\n"
                + "        detail?.contributionStatus ?? record.status\n"))
    }

    // MARK: - History in the left pane (Task 8 of the #1146 port)

    static func flat(_ source: String) -> String {
        source.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    static func record(_ status: String) -> HistoryRecord {
        HistoryRecord(
            submissionID: "sub-\(status)", submittedAt: Date(timeIntervalSince1970: 0), projectID: "proj_a",
            projectLabel: "a", source: "claude_code", status: status, consentScopes: [],
            creditPointsPending: 0, creditPointsFinal: nil, explanations: [], lastRefreshedAt: nil)
    }

    static func rows(_ statuses: [String]) throws -> [DaemonData.HistoryRow] {
        let objects = statuses.enumerated().map { index, status in
            #"{"submission_id":"s\#(index)","project_id":"proj_\#(index % 2)","project_label":"p\#(index % 2)","status":"\#(status)"}"#
        }
        return try DaemonDataDecoding.decoder().decode(
            [DaemonData.HistoryRow].self, from: Data("[\(objects.joined(separator: ","))]".utf8))
    }

    /// Ron's `HistoryRow`: Open and Withdraw sit on the row, and nowhere
    /// else on History's page. Withdraw asks first, with the existing
    /// confirmation and outcome, and stands on the app's own record: a row
    /// whose record has not resolved offers none. The opened row's detail,
    /// drawn below the list on the same page, carries no Withdraw of its own.
    func test_withdrawIsOnTheRow() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        let flat = Self.flat(home)
        for needle in [
            "HistorySelection.record(for: row.submissionId, in: model.history)",
            "Button(MonitorWords.table?.shell.open ?? copy.viewSession, action: open)",
            "case .withdraw: Button(copy.withdraw) { confirming = true }",
            "WithdrawalConfirmationView( status: SessionDetailView.withdrawalStatus(record, detail: detail), "
                + "keepLabel: copy.keepContribution, inFlight: model.withdrawing.contains(record.submissionID), "
                + "onKeep: { confirming = false }, onConfirm: { model.withdraw(record) } )",
            "WithdrawalOutcomeView(result: result)",
        ] {
            XCTAssertTrue(flat.contains(needle), "HomeViews.swift lacks \(needle)")
        }
        XCTAssertEqual(home.components(separatedBy: "model.withdraw(record)").count - 1, 2,
                       "only the row's confirmation and its Retry withdraw")

        // Every view History's page draws: one Withdraw entry point, the row's.
        let inspector = try Self.text("Views/Monitor/HistoryInspector.swift")
        XCTAssertTrue(Self.flat(inspector).contains("SessionDetailView(record: record, offersWithdrawal: false)"),
                      "the opened row's detail leaves Withdraw to the row")
        for absent in ["model.withdraw(", "SessionWithdrawalAction(", "WithdrawalConfirmationView("] {
            XCTAssertFalse(inspector.contains(absent), "HistoryInspector.swift draws \(absent)")
        }
        let detail = try Self.text("Views/SessionDetailView.swift")
        XCTAssertFalse(detail.contains("model.withdraw("), "the session detail withdraws only through its action")
        XCTAssertEqual(detail.components(separatedBy: "SessionWithdrawalAction(").count - 1, 1)
        XCTAssertTrue(Self.flat(detail).contains("if offersWithdrawal { SessionWithdrawalAction("),
                      "the session detail's Withdraw is behind the flag History's pane turns off")
        let overview = try Self.text("Views/SessionContributionOverview.swift")
        XCTAssertFalse(overview.contains("SessionDetailView("), "the overview does not draw a second detail")
        XCTAssertEqual(home.components(separatedBy: "SessionWithdrawalAction(").count - 1, 0)
    }

    /// The row decides from the status the detail decides from (the
    /// detail's when read, else the record's) and offers Retry after a
    /// failed withdrawal, as the session detail's action did; a withdrawn
    /// row offers nothing more.
    @MainActor
    func test_theRowFollowsTheDetailStatusAndRetries() throws {
        let accepted = Self.record("accepted")
        let decide = { (record: HistoryRecord?, detail: SessionDetail?, result: AppModel.WithdrawalResult?) in
            HistoryList.rowWithdraw(record: record, detail: detail, result: result, account: .signedIn)
        }
        XCTAssertEqual(decide(accepted, nil, nil), .withdraw)
        XCTAssertEqual(decide(Self.record("received"), nil, nil), .withdraw)
        XCTAssertEqual(decide(Self.record("withdrawn"), nil, nil), .none)
        XCTAssertEqual(decide(Self.record("a-status-from-the-future"), nil, nil), .none)
        XCTAssertEqual(decide(nil, nil, nil), .none, "no record, no Withdraw")
        // The detail's terminal status wins over the record's, as it does
        // in `SessionDetailView.withdrawalStatus`.
        XCTAssertEqual(decide(accepted, try Self.detail("withdrawn"), nil), .none)
        XCTAssertEqual(decide(Self.record("withdrawn"), try Self.detail("accepted"), nil), .withdraw)
        // A failed withdrawal is retried from the row; a done one is not.
        XCTAssertEqual(decide(accepted, nil, .failed("withdraw-failed")), .retry)
        XCTAssertEqual(decide(accepted, nil, .noAccountSession), .retry)
        XCTAssertEqual(decide(accepted, nil, .withdrawn(nil)), .none)
        XCTAssertEqual(decide(accepted, try Self.detail("withdrawn"), .failed("withdraw-failed")), .none)
        let home = Self.flat(try Self.text("Views/Monitor/HomeViews.swift"))
        XCTAssertTrue(home.contains("case .retry: if let record { Button(WithdrawalCopy.tryAgain) { model.withdraw(record) }"))
    }

    /// Ron's `HistoryRow`: wherever Withdraw would be offered and the
    /// account session is not known to be active, the row offers the core's
    /// sign-in instead, through the native identity path; while the session
    /// is first read it says so. A row with nothing to withdraw offers neither.
    @MainActor
    func test_aSignedOutRowOffersSignInNotWithdraw() throws {
        let accepted = Self.record("accepted")
        XCTAssertEqual(HistoryList.rowWithdraw(record: accepted, detail: nil, result: nil, account: .signedOut), .signIn)
        XCTAssertEqual(HistoryList.rowWithdraw(record: accepted, detail: nil, result: nil, account: .unread), .signIn,
                       "an unread session is not a signed-in one")
        XCTAssertEqual(HistoryList.rowWithdraw(record: accepted, detail: nil, result: nil, account: .checking), .checkingAccount)
        XCTAssertEqual(HistoryList.rowWithdraw(record: accepted, detail: nil, result: .noAccountSession, account: .signedOut),
                       .signIn, "a rejected session asks for sign-in, not a Retry that would fail again")
        XCTAssertEqual(HistoryList.rowWithdraw(record: Self.record("withdrawn"), detail: nil, result: nil, account: .signedOut), .none)
        XCTAssertEqual(HistoryList.rowWithdraw(record: accepted, detail: nil, result: .withdrawn(nil), account: .signedOut), .none)

        let home = Self.flat(try Self.text("Views/Monitor/HomeViews.swift"))
        for needle in [
            "case .signIn: if let words { HistorySignInControl(words: words) }",
            "case .checkingAccount: if let words { Text(words.checkingAccount)",
            "Button(model.accountSigningIn ? words.waitingForSignIn : words.signInToWithdraw) "
                + "{ Task { await model.signInToWithdraw() } }",
            "if model.accountSigningIn { Text(words.completeSignIn)",
        ] {
            XCTAssertTrue(home.contains(needle), "HomeViews.swift lacks \(needle)")
        }
    }

    /// The model's account session: read through `account_session_status`,
    /// set to signed out when the commons rejects the stored session, and
    /// after a sign-in trusted only as the status read again says.
    @MainActor
    func test_theAccountSessionIsReadAndReReadAfterSignIn() async throws {
        let daemon = HistoryActionsDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        XCTAssertEqual(model.accountSession, .unread)

        daemon.signedIn = true
        model.refreshAccountSession()
        try await waitUntil { model.accountSession == .signedIn }

        // The commons rejects the stored session: the row asks for sign-in.
        daemon.withdrawFails = "account-session-required"
        model.withdraw(Self.record("accepted"))
        try await waitUntil { model.withdrawals["sub-accepted"] == .noAccountSession }
        XCTAssertEqual(model.accountSession, .signedOut)

        // Sign-in returns, but the session read again is not active.
        daemon.signedIn = false
        await model.signInToWithdraw()
        XCTAssertEqual(model.accountSession, .signedOut)
        XCTAssertEqual(model.accountSignInFailure, .inactive)
        XCTAssertEqual(daemon.calls.filter { $0 != "hello" }.suffix(2), ["account_sign_in", "account_session_status"],
                       "the session is read again after sign-in")

        daemon.signedIn = true
        await model.signInToWithdraw()
        XCTAssertEqual(model.accountSession, .signedIn)
        XCTAssertNil(model.accountSignInFailure)
        // The rejection is answered: the row offers Withdraw again, which
        // asks first, rather than a Retry under a line about no sign-in.
        XCTAssertNil(model.withdrawals["sub-accepted"], "a signed-in row forgets the no-session refusal")
        XCTAssertEqual(HistoryList.rowWithdraw(record: Self.record("accepted"), detail: nil,
                                               result: model.withdrawals["sub-accepted"], account: model.accountSession),
                       .withdraw)

        daemon.signInFails = true
        await model.signInToWithdraw()
        XCTAssertEqual(model.accountSignInFailure, .failed)
    }

    /// A refusal for want of an account session is never described on a
    /// History row: the row offers sign-in (or, once signed in, Withdraw)
    /// in its place, and the legacy sentence says this build cannot sign
    /// in. Every other outcome is drawn.
    @MainActor
    func test_aRejectedSessionIsNotDescribedBesideSignIn() throws {
        XCTAssertFalse(HistoryList.showsOutcome(.noAccountSession), "the no-sign-in sentence on a row")
        XCTAssertTrue(HistoryList.showsOutcome(.failed("withdraw-failed")))
        XCTAssertTrue(HistoryList.showsOutcome(.withdrawn(nil)))
        // Whatever the row offers, sign-in (signed out) or Withdraw (signed
        // in again), the refusal's sentence is not beside it.
        XCTAssertEqual(HistoryList.rowWithdraw(record: Self.record("accepted"), detail: nil,
                                               result: .noAccountSession, account: .signedOut), .signIn)
        let home = Self.flat(try Self.text("Views/Monitor/HomeViews.swift"))
        XCTAssertTrue(home.contains(
            "if let result { if HistoryList.showsOutcome(result) { WithdrawalOutcomeView(result: result) }"),
            "the row draws its outcome only through the rule")
    }

    /// The refresh control (Ron's #1146 HistoryRefreshControl): asks the daemon's poller to check the
    /// server sooner (`refresh_history`), then reads History again. A refusal,
    /// or a reply that does not say it was requested, is a failure.
    @MainActor
    func test_refreshAsksTheDaemonToCheckTheServer() async throws {
        let daemon = HistoryActionsDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        let asked = await model.requestHistoryRefresh()
        XCTAssertTrue(asked)
        XCTAssertEqual(model.historyRefresh, .requested)
        XCTAssertTrue(daemon.calls.contains("refresh_history"))
        try await waitUntil { daemon.calls.contains("list_history") }

        daemon.refreshRequested = false
        let notAsked = await model.requestHistoryRefresh()
        XCTAssertFalse(notAsked)
        XCTAssertEqual(model.historyRefresh, .failed)
        // A later visit starts clean: the last outcome is not left standing.
        model.clearHistoryRefresh()
        XCTAssertEqual(model.historyRefresh, .idle)
        _ = await model.requestHistoryRefresh()
        XCTAssertEqual(model.historyRefresh, .failed)
        daemon.refreshRequested = true
        _ = await model.requestHistoryRefresh()
        XCTAssertEqual(model.historyRefresh, .requested)
        model.clearHistoryRefresh()
        XCTAssertEqual(model.historyRefresh, .idle)

        let home = Self.flat(try Self.text("Views/Monitor/HomeViews.swift"))
        for needle in [
            "if let words = MonitorWords.table?.historyActions {",
            // Ron's SUBMISSIONS card: its heading beside the refresh.
            "submissionsHeader refreshOutcome list", "Spacer(minLength: 0) refresh }",
            "Button(model.historyRefresh == .requesting ? words.requesting : words.requestRefresh) "
                + "{ Task { if await model.requestHistoryRefresh() { await store.load() } } }",
            "case .requested: Text(words.refreshRequested)", "case .failed: GlassStatusLabel(words.refreshFailed, status: .outside)",
            ".onAppear { model.clearHistoryRefresh() model.refreshHistory() model.refreshAccountSession() }",
        ] {
            XCTAssertTrue(home.contains(needle), "HomeViews.swift lacks \(needle)")
        }
    }

    /// Every Monitor label comes from the core: no view under Views/Monitor
    /// authors one with `String(localized:`.
    func test_noMonitorViewAuthorsALabel() throws {
        let dir = GlassSurfaceRulesTests.root.appendingPathComponent("Views/Monitor")
        let files = try FileManager.default.contentsOfDirectory(atPath: dir.path).filter { $0.hasSuffix(".swift") }
        XCTAssertFalse(files.isEmpty)
        for file in files {
            let source = try String(contentsOf: dir.appendingPathComponent(file), encoding: .utf8)
            XCTAssertFalse(source.contains("String(localized:"), "Views/Monitor/\(file) authors a label")
        }
    }

    static func detail(_ status: String) throws -> SessionDetail {
        let json = #"{"contribution_status":"\#(status)","evidence":[],"contributed_version":"trace-contribution/1","consent_policy_version":"consent/1","redaction_pipeline_version":"redaction/3","publication":null,"publication_version":0}"#
        return try JSONDecoder().decode(SessionDetail.self, from: Data(json.utf8))
    }

    @MainActor
    private func waitUntil(_ condition: @escaping () -> Bool, file: StaticString = #filePath, line: UInt = #line) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTFail("condition never held", file: file, line: line)
    }

    /// Ron's filter is an exact status match, counted over every row; its
    /// labels are the core's (`historyUi`, and the public-run copy's "all"),
    /// and without them there is no filter at all.
    func test_theFilterMatchesTheStatusExactly() throws {
        let rows = try Self.rows(["accepted", "accepted", "processing", "submitted", "quarantined", "revoked", "withdrawn"])
        let counts = HistoryList.counts(rows)
        XCTAssertEqual(counts[.all], 7)
        XCTAssertEqual(counts[.accepted], 2)
        XCTAssertEqual(counts[.submitted], 1)
        XCTAssertEqual(counts[.quarantined], 1)
        XCTAssertEqual(counts[.withdrawn], 1)
        XCTAssertEqual(HistoryList.rows(rows, filter: .all).count, 7)
        XCTAssertEqual(HistoryList.rows(rows, filter: .submitted).map(\.submissionId), ["s3"])
        XCTAssertEqual(HistoryList.Filter.allCases, [.all, .accepted, .submitted, .quarantined, .withdrawn])

        // Ron's #1146 filter words, from the core (owner ruling, 2026-10-06).
        let shell = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())).shell
        XCTAssertEqual(HistoryList.label(.all, shell: shell), "All")
        XCTAssertEqual(HistoryList.label(.accepted, shell: shell), "In commons")
        XCTAssertEqual(HistoryList.label(.quarantined, shell: shell), "Privacy review")
        XCTAssertEqual(HistoryList.label(.withdrawn, shell: shell), "Withdrawn")
        for filter in HistoryList.Filter.allCases {
            XCTAssertNil(HistoryList.label(filter, shell: nil))
        }
        // The real export labels every filter, so the filter is drawn.
        let labels = try XCTUnwrap(HistoryList.labels(shell: shell),
                                   "a filter label is missing from the core, and no filter is drawn")
        XCTAssertEqual(labels.count, HistoryList.Filter.allCases.count)
        XCTAssertFalse(labels.values.contains(where: \.isEmpty))
        XCTAssertTrue(Self.flat(try Self.text("Views/Monitor/HomeViews.swift")).contains(
            "HistoryFolders.folders( HistoryList.rows(rows, filter: shownFilter),"), "rows grouped by project")
    }

    /// The record by period, which Home's summary inspector drew until Ron's
    /// Summary replaced it (#1241 Task 4), stays on History: the week's and
    /// month's accepted counts and the withdrawn count, all from the rollup
    /// (never a count of the loaded page). #1146's History has no place for
    /// it, so it closes the page, and Ron's three stat cards are his.
    func test_theRecordByPeriodLivesInHistory() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        let period = try XCTUnwrap(home.range(of: "private struct HistoryPeriodCard: View"))
        let periodEnd = try XCTUnwrap(home[period.upperBound...].range(of: "\n}\n"))
        let card = Self.flat(String(home[period.lowerBound..<periodEnd.lowerBound]))
        for needle in [
            #"SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])"#,
            ".init(MonitorWords.week, HomeFormat.count(rollup?.week?.accepted))",
            ".init(MonitorWords.month, HomeFormat.count(rollup?.month?.accepted))",
            ".init(MonitorWords.withdrawn, HomeFormat.count(rollup?.takenBack))",
        ] {
            XCTAssertTrue(card.contains(needle), "the record by period lacks \(needle)")
        }
        let start = try XCTUnwrap(home.range(of: "private struct HistoryStats: View"))
        let end = try XCTUnwrap(home[start.upperBound...].range(of: "\n}\n"))
        let stats = Self.flat(String(home[start.lowerBound..<end.lowerBound]))
        for needle in [
            "HomeStatTile(label: HomeFormat.creditPendingWord, value: HomeFormat.pendingFigure(rollup?.creditPending, credit: credit))",
            "HomeStatTile(label: MonitorWords.contributed, value: HomeFormat.count(rollup?.allTime?.accepted))",
            "HomeStatTile(label: MonitorWords.held, value: HomeFormat.count(rollup?.quarantined))",
        ] {
            XCTAssertTrue(stats.contains(needle), "History's stat cards lack \(needle)")
        }
        XCTAssertFalse(stats.contains("GlassLegendCell("), "Ron's stat cards, not legend wells")
        XCTAssertFalse(stats.contains("MonitorWords.week"), "the record by period is not among the stat cards")
        XCTAssertFalse(home.contains("struct HomeSummaryInspector"), "Ron's Summary replaced Home's")
    }

    /// Ron's `HistoryPage` order: the stat cards, community, the
    /// contribution list, the credit record, the privacy review, and the
    /// record by period last. The opened row's details are not on the page:
    /// they are the inspector's.
    func test_historyIsInRonsOrderAndTheDetailIsTheInspectors() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        let page = try XCTUnwrap(home.range(of: "private struct HistoryPage: View"))
        let tail = home[page.lowerBound...]
        let stats = try XCTUnwrap(tail.range(of: "HistoryStats(store: store)"))
        let community = try XCTUnwrap(tail.range(of: "HistoryCommunityCard(store: store)"))
        let list = try XCTUnwrap(tail.range(of: "submissionsHeader\n"))
        let credit = try XCTUnwrap(tail.range(of: "HistoryCreditCard(store: store)"))
        let held = try XCTUnwrap(tail.range(of: "                held\n"))
        let period = try XCTUnwrap(tail.range(of: "HistoryPeriodCard(store: store)"))
        XCTAssertLessThan(stats.lowerBound, community.lowerBound)
        XCTAssertLessThan(community.lowerBound, list.lowerBound)
        XCTAssertLessThan(list.lowerBound, credit.lowerBound)
        XCTAssertLessThan(credit.lowerBound, held.lowerBound)
        XCTAssertLessThan(held.lowerBound, period.lowerBound)
        XCTAssertFalse(home.contains("HistoryDetailInspector("), "the opened row is not drawn on History's page")
        let inspector = try Self.text("Views/Monitor/HistoryInspector.swift")
        XCTAssertTrue(Self.flat(inspector).contains(
            "struct HistoryInspectorPane: View { let row: DaemonData.HistoryRow var body: some View { ScrollView { HistoryDetailInspector(row: row) }"))
        // Ron's headings and project groups, in the core's words.
        let flat = Self.flat(home)
        for needle in [
            "Text(words.historyDescription)", "Text(words?.submissions ?? MonitorWords.history)",
            "Text(words.contributionHistory)", "Text(words.project)",
            "Text(words?.records(group.count) ?? \"\\(group.count)\")", "Text(words.readingHistory)",
            ".accessibilityLabel(words?.filterLabel ?? MonitorWords.history)",
            "Text(words.status(word))", "Text(words.credit(figure))", "Text(HomeFormat.rowMeta(row))",
        ] {
            XCTAssertTrue(flat.contains(needle), "HomeViews.swift lacks \(needle)")
        }
        // The filter wraps; it is no longer a strip that scrolls sideways.
        XCTAssertFalse(home.contains("ScrollView(.horizontal)"))
        XCTAssertTrue(home.contains("HistoryFilterFlow(spacing: GlassTokens.Space.s2)"))
    }

    /// Ron's inspector auto-open: opening a History row selects it and opens
    /// the inspector, which shows the row while History is shown and the
    /// Traces selection's card otherwise. Clearing it closes nothing.
    func test_openingAHistoryRowOpensTheInspectorOnIt() throws {
        var selected = ""
        var shows = false
        MonitorWindowView.openHistory("s1", selected: &selected, showsInspector: &shows)
        XCTAssertEqual(selected, "s1")
        XCTAssertTrue(shows, "opening a row opens the inspector")
        MonitorWindowView.openHistory("", selected: &selected, showsInspector: &shows)
        XCTAssertEqual(selected, "")
        XCTAssertTrue(shows, "clearing the row closes nothing")
        shows = false
        MonitorWindowView.openHistory("", selected: &selected, showsInspector: &shows)
        XCTAssertFalse(shows)

        let rows = try Self.rows(["accepted", "submitted"])
        XCTAssertEqual(HistorySelection.opened("s1", onHistory: true, in: rows)?.submissionId, "s1")
        XCTAssertNil(HistorySelection.opened("s1", onHistory: false, in: rows), "only while History is shown")
        XCTAssertNil(HistorySelection.opened("", onHistory: true, in: rows))
        XCTAssertNil(HistorySelection.opened("gone", onHistory: true, in: rows), "a row no longer listed")
        XCTAssertNil(HistorySelection.opened("s1", onHistory: true, in: nil))

        let window = try Self.text("Views/MonitorWindowView.swift")
        let code = window.split(separator: "\n")
            .filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
            .joined(separator: "\n")
        XCTAssertTrue(Self.flat(code).contains(
            "case .home, .traces: if tab == .home, let row = HistorySelection.opened(selectedHistory, "
                + "onHistory: homePage == .history, in: home.history) { HistoryInspectorPane(row: row) } "
                + "else { TracesInspectorHost(traces: traces, home: home, selection: selection) }"))
        XCTAssertTrue(Self.flat(window).contains(
            "set: { Self.openHistory($0, selected: &selectedHistory, showsInspector: &showsInspector) }"))
    }
}

/// A daemon that answers History's refresh, the account session, sign-in and
/// withdraw, recording each method it is asked.
private final class HistoryActionsDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var _calls: [String] = []
    var calls: [String] { lock.withLock { _calls } }
    var signedIn: Bool? = nil
    var signInFails = false
    var withdrawFails: String?
    var refreshRequested = true

    func call(_ method: String, params: String) -> String {
        lock.withLock { _calls.append(method) }
        let result: Any
        switch method {
        case "hello":
            result = ["methods": ["account_session_status", "account_sign_in"]]
        case "account_session_status":
            result = ["state": signedIn == true ? "signed_in" : "signed_out", "signed_in": signedIn as Any? ?? NSNull(),
                      "expires_at": NSNull()]
        case "account_sign_in":
            if signInFails { return #"{"error":{"code":"unavailable","message":"account-sign-in-failed"}}"# }
            result = ["state": "signed_in", "signed_in": true, "expires_at": NSNull()]
        case "withdraw":
            if let label = withdrawFails { return #"{"error":{"code":"unavailable","message":"\#(label)"}}"# }
            result = ["withdrawn": true]
        case "refresh_history":
            result = ["requested": refreshRequested]
        case "list_history":
            result = ["history": []]
        default:
            return #"{"error":{"code":"unknown_method","message":"unknown-method"}}"#
        }
        return String(decoding: try! JSONSerialization.data(withJSONObject: ["result": result]), as: UTF8.self)
    }
    func openPreview(entryID: String) throws -> TCPreview { fatalError("No preview in History tests") }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
}
