import XCTest
@testable import TraceCommonsApp

/// The two app-wide patterns the owner ruled on 2026-10-09, pinned at their
/// reference sites by source scan, as the parity tests pin structure.
///
/// 1. On a card, offer, prompt, notice or inline panel the affirmative
///    action comes first as a glass or primary button, and the decline,
///    dismiss or cancel comes after it as a link.
/// 2. A failed request's line is a plain red caption (`GlassAlert`), with no
///    box, directly under the buttons it is about.
final class ActionFirstFailureBelowTests: XCTestCase {
    private static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// The source of one declaration: a type from its `struct` to the next
    /// top-level declaration, a member from its opening to the next member.
    private static func declaration(_ opening: String, in source: String) throws -> String {
        let start = try XCTUnwrap(source.range(of: opening), "\(opening) is missing")
        let rest = source[start.upperBound...]
        let next = opening.contains("struct ")
            ? #"\n(public |private |fileprivate )?(struct|extension|enum) "#
            : #"\n    (private |fileprivate |static |@ViewBuilder|func |var )"#
        let end = rest.range(of: next, options: .regularExpression)?.lowerBound ?? rest.endIndex
        return String(rest[..<end])
    }

    /// Whitespace collapsed to single spaces, so a needle spans lines.
    private static func flat(_ text: String) -> String {
        text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    /// Pattern 1: the accept is drawn before the decline, the accept is not
    /// a link and the decline is.
    func test_theActionComesFirstAndTheDeclineIsALink() throws {
        let sites: [(file: String, opening: String, accept: String, decline: String)] = [
            ("Views/Settings/NudgeSettingsSection.swift", "struct NudgeOfferCard: View {",
             "Button(offer.accept)", "Button(offer.decline)"),
            ("Views/Monitor/TracesOffers.swift", "struct PrivateAIOfferGlassCard: View {",
             "Button(copy.offerAccept, action: onAccept)", "Button(copy.offerDecline, action: onDecline)"),
            ("Views/Monitor/TracesOffers.swift", "struct ArmingOfferGlassCard: View {",
             "Button(copy.confirm) { confirming = true }", "Button(copy.decline, action: onDecline)"),
            ("Views/ShellNotices.swift", "struct GrantVoidNoticeCard: View {",
             "Button(action, action: onRearm)", "Button(notice.acknowledge, action: onAcknowledge)"),
            ("Views/ShellNotices.swift", "struct ArmingRewordedNoticeCard: View {",
             "Button(action, action: onAskFirst)", "Button(notice.acknowledge, action: onAcknowledge)"),
            ("Views/SessionDetailView.swift", "private func review(_ draft: PublicRunDraftInput)",
             "model.publishPublicRun(record, draft: draft)", "Button(copy.editDraft)"),
            ("Views/Settings/WitnessSection.swift", "private func inferenceEvidence(",
             "Button(copy.inferenceEnable)", "Button(copy.inferenceDisable)"),
        ]
        for site in sites {
            let body = try Self.declaration(site.opening, in: try Self.text(site.file))
            let accept = try XCTUnwrap(body.range(of: site.accept), "\(site.opening) lacks \(site.accept)")
            let decline = try XCTUnwrap(body.range(of: site.decline), "\(site.opening) lacks \(site.decline)")
            XCTAssertLessThan(accept.lowerBound, decline.lowerBound,
                              "\(site.file) \(site.opening): the decline is drawn before the action")
            // The decline's own style, within its call's modifiers.
            let declineTail = Self.flat(String(body[decline.lowerBound...].prefix(400)))
            XCTAssertTrue(declineTail.contains(".buttonStyle(GlassButtonStyle(.link))"),
                          "\(site.file) \(site.opening): the decline is not a link")
        }
        // The session card: Contribute leads both of its layouts, and its
        // Dismiss is a link wherever it is drawn.
        let source = try Self.text("Views/Monitor/SessionReviewCard.swift")
        let actions = Self.flat(try Self.declaration("private func actions(", in: source))
        XCTAssertTrue(actions.contains("{ contribute(entry, words) dismiss(entry, words) lookInside(entry, review) }"))
        XCTAssertTrue(actions.contains("{ contribute(entry, words) dismiss(entry, words) }"))
        XCTAssertFalse(actions.contains("dismiss(entry, words) contribute(entry, words)"), "Dismiss leads again")
        XCTAssertTrue(Self.flat(source).contains(
            "Button(words.dismissAction) { act(.dismiss, entry) } .buttonStyle(GlassButtonStyle(.link))"))
        // The offers' doc comments no longer say the decline-first order
        // awaits a ruling.
        let offers = try Self.text("Views/Monitor/TracesOffers.swift")
        XCTAssertFalse(offers.contains("left to the owner"), "the accept-first order is ruled")
        XCTAssertFalse(offers.contains("declining\n/// first"), "the arming offer still says it declines first")
    }

    /// Pattern 2: `GlassAlert` is a plain red caption with no box, and the
    /// reference sites draw their failure with it after their buttons.
    func test_aFailureIsAPlainRedLineUnderItsButtons() throws {
        let alertSource = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCDesign/Components/StatCard.swift"),
            encoding: .utf8)
        let alert = try Self.declaration("public struct GlassAlert: View {", in: alertSource)
        XCTAssertTrue(alert.contains(".foregroundStyle(GlassTokens.Color.statusOutsideText.color)"))
        XCTAssertTrue(alert.contains(".glassType(GlassTokens.TypeScale.caption)"))
        for box in [".background(", ".overlay(", ".glassTier(", ".glassEdge(", ".padding(", "GlassNotice("] {
            XCTAssertFalse(alert.contains(box), "GlassAlert draws a box again: \(box)")
        }

        // (file, declaration, the last button of the row, the failure line)
        let sites: [(file: String, opening: String, button: String, failure: String)] = [
            ("Views/Settings/NudgeSettingsSection.swift", "struct NudgeOfferCard: View {",
             "Button(offer.decline)", "GlassAlert(line)"),
            ("Views/Monitor/ToolFolderInspectors.swift", "private func decisions(",
             "Button(copy.button)", "GlassAlert(words.line(for: refused))"),
            ("Views/ShellNotices.swift", "struct GrantVoidNoticeCard: View {",
             "Button(notice.acknowledge, action: onAcknowledge)", "GlassAlert(failed)"),
            ("Views/ShellNotices.swift", "struct ArmingRewordedNoticeCard: View {",
             "Button(notice.acknowledge, action: onAcknowledge)", "GlassAlert(failed)"),
            ("Views/Monitor/SessionReviewCard.swift", "private func actions(",
             "Button(words.keep)", "TracesRefusal(store: store, entryId: entry.entryId)"),
            ("Views/Monitor/SessionReviewCard.swift", "private func actions(",
             "Button(words.keep)", "GlassAlert(outcome.correctionCredentialHeadline)"),
            ("Views/Settings/WitnessSection.swift", "private func inferenceEvidence(",
             "Button(copy.inferenceDisable)", "GlassAlert(copy.inferenceSaveFailed)"),
            ("Views/SessionDetailView.swift", "private func localInstalledSkillSurface(",
             "Button(copy.retryRead)", "GlassAlert(message)"),
        ]
        for site in sites {
            let body = try Self.declaration(site.opening, in: try Self.text(site.file))
            let button = try XCTUnwrap(body.range(of: site.button), "\(site.opening) lacks \(site.button)")
            let failure = try XCTUnwrap(body.range(of: site.failure), "\(site.opening) lacks \(site.failure)")
            XCTAssertLessThan(button.lowerBound, failure.lowerBound,
                              "\(site.file) \(site.opening): the failure is drawn above its buttons")
            XCTAssertFalse(body.contains("GlassNotice(tone: .outside"),
                           "\(site.file) \(site.opening): a failure is boxed again")
        }
        // The notices' own refusal label is gone with its box.
        XCTAssertFalse(try Self.text("Views/ShellNotices.swift").contains("NoticeRefusal"))
        // TracesRefusal itself says the line with GlassAlert.
        let refusal = try Self.declaration("struct TracesRefusal: View {",
                                           in: try Self.text("Views/Monitor/TracesOffers.swift"))
        XCTAssertTrue(refusal.contains("GlassAlert(line)"))
    }
}
