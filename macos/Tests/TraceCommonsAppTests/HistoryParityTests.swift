#if DEBUG
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
            "copy.contributionStatusLabel(for: detail.contributionStatus ?? record.status)", "copy.permittedUsesUnavailable",
            "copy.noPermittedUses", "copy.contentUnavailable", "minHeight: 44",
        ] {
            XCTAssertTrue(source.contains(needle), "SessionContributionOverview.swift lacks \(needle)")
        }
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
            #"Button(inFlight ? "Withdrawing..." : confirmation.confirmLabel, action: onConfirm)"#,
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
            "SessionWithdrawalAction(record: record, currentStatus: currentStatus, copy: copy)",
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
        XCTAssertTrue(source.contains("init(record: HistoryRecord, onBack: (() -> Void)? = nil)"),
                      "the inspector draws it without a back control")
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
            "if let onBack { GlassBreadcrumb([GlassCrumb(copy.allContributions, action: onBack)], "
                + "backLabel: copy.allContributions, onBack: onBack)",
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
            "Toggle(copy.publishCorrection, isOn: $includeCorrection) .toggleStyle(GlassCheckboxStyle()) .frame(minHeight: 44",
            "GlassCheckMark(checked: selected)",
            ".accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)",
            "GlassStatusLabel(problem, status: .outside)",
            "Button(copy.reviewPage) { reviewDraft = makeDraft() } .buttonStyle(GlassButtonStyle(.primary)) "
                + ".frame(minHeight: 44) .disabled(makeDraft() == nil)",
            "Button(copy.cancelEdit) { editingPublished = false } .buttonStyle(GlassButtonStyle(.glass))",
            "Button(copy.editDraft) { reviewDraft = nil } .buttonStyle(GlassButtonStyle(.glass))",
            "{ model.publishPublicRun(record, draft: draft) } .buttonStyle(GlassButtonStyle(.primary)) "
                + ".frame(minHeight: 44) .disabled(working)",
            // Fields carry their names for VoiceOver.
            "TextField(copy.pageTitle, text: $title)", ".accessibilityLabel(copy.publicOutcome)",
            ".accessibilityLabel(copy.reusableInstructions)", "TextField(copy.sourcePlaceholder, text: $source)",
            // A publication error can be put away; the next attempt shows it again.
            "Button(Self.dismissLabel ?? ActionMessageBanner.dismissWord) { dismissedError = message }",
        ] {
            XCTAssertTrue(flat.contains(needle), "SessionDetailView.swift lacks \(needle)")
        }
        XCTAssertTrue(GlassSurfaceRulesTests.files.contains("Views/SessionDetailView.swift"))
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
}
#endif
