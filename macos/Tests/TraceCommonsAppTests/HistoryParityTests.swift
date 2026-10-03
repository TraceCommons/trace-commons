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
}
#endif
