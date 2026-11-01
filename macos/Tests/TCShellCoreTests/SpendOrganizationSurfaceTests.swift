import XCTest

@testable import TCShellCore

/// When the spending-organization picker and the Private AI prompt draw.
///
/// The case this file is mostly about is the common one: an account with a
/// single organization sees nothing new. The others are the ways a shell
/// could mistake "not known" for "several".
final class SpendOrganizationSurfaceTests: XCTestCase {
    nonisolated(unsafe) static let view: [String: Any] = [
        "title": "TITLE", "what": "WHAT", "picker": "PICKER", "switching": "SWITCHING", "failed": "FAILED",
        "session_expired": "EXPIRED", "refused": "REFUSED", "revoke_pending": "PENDING",
        "prompt_title": "PROMPT-TITLE", "prompt_line": "PROMPT-LINE",
        "prompt_action": "PROMPT-ACTION", "prompt_action_accessibility": "PROMPT-ACTION-AX",
        "prompt_dismiss": "PROMPT-DISMISS", "prompt_dismiss_accessibility": "PROMPT-DISMISS-AX",
    ]

    static func payload(
        state: String = "listed", organizations: [(String, Bool)], current: String? = "org-a",
        explicit: Bool = false, prompt: Bool? = nil, revokePending: Bool = false,
        view: [String: Any]? = SpendOrganizationSurfaceTests.view
    ) -> String {
        var object: [String: Any] = [
            "state": state,
            "organizations": organizations.map { ["id": $0.0, "name": "Name \($0.0)", "role": "owner", "current": $0.1] },
            "current_organization_id": current as Any? ?? NSNull(),
            "explicit": explicit,
            "multiple": organizations.count >= 2,
            "prompt": prompt ?? (organizations.count >= 2 && !explicit && current != nil),
            "revoke_pending": revokePending,
        ]
        if let view { object["view"] = view }
        let data = try! JSONSerialization.data(withJSONObject: object)
        return String(decoding: data, as: UTF8.self)
    }

    func testASingleOrganizationDrawsNeitherPickerNorPrompt() {
        let status = SpendOrganizations.parse(fromJSON: Self.payload(organizations: [("org-a", true)]))
        XCTAssertEqual(status.state, "listed")
        XCTAssertEqual(status.organizations.count, 1)
        XCTAssertFalse(status.showsPicker)
        XCTAssertFalse(status.showsPrompt)
    }

    func testSeveralOrganizationsWithNoChoiceDrawBoth() {
        let status = SpendOrganizations.parse(
            fromJSON: Self.payload(organizations: [("org-a", true), ("org-b", false)]))
        XCTAssertTrue(status.showsPicker)
        XCTAssertTrue(status.showsPrompt)
        XCTAssertEqual(status.organizations.first { $0.current }?.id, "org-a")
        XCTAssertEqual(status.words?.promptLine, "PROMPT-LINE")
    }

    func testEndingThePromptKeepsEverythingElse() {
        let status = SpendOrganizations.parse(
            fromJSON: Self.payload(organizations: [("org-a", true), ("org-b", false)]))
        XCTAssertTrue(status.showsPrompt)
        let ended = status.withPromptEnded()
        XCTAssertFalse(ended.showsPrompt)
        XCTAssertTrue(ended.showsPicker)
        XCTAssertEqual(ended.organizations, status.organizations)
        XCTAssertEqual(ended.words, status.words)
    }

    func testAnExplicitChoiceKeepsThePickerAndEndsThePrompt() {
        let status = SpendOrganizations.parse(
            fromJSON: Self.payload(organizations: [("org-a", true), ("org-b", false)], explicit: true))
        XCTAssertTrue(status.showsPicker)
        XCTAssertFalse(status.showsPrompt)
    }

    /// The daemon's `prompt` decides; the shell never infers one from a
    /// count it was told not to prompt for.
    func testThePromptFollowsTheDaemon() {
        let status = SpendOrganizations.parse(
            fromJSON: Self.payload(organizations: [("org-a", true), ("org-b", false)], prompt: false))
        XCTAssertTrue(status.showsPicker)
        XCTAssertFalse(status.showsPrompt)
    }

    func testAListingThatFailedDrawsNothing() {
        for state in ["unavailable", "session_expired", "no_session", "no_organization"] {
            let status = SpendOrganizations.parse(
                fromJSON: Self.payload(state: state, organizations: [], current: nil, prompt: false))
            XCTAssertFalse(status.showsPicker, state)
            XCTAssertFalse(status.showsPrompt, state)
        }
    }

    func testUnreadablePayloadsAreUnreported() {
        for json in ["", "not json", "[]", #"{"organizations":[]}"#,
                     #"{"state":"listed","organizations":[{"id":"org-a"}]}"#] {
            XCTAssertEqual(SpendOrganizations.parse(fromJSON: json), .unreported, json)
        }
        XCTAssertFalse(SpendOrganizations.unreported.showsPicker)
        XCTAssertFalse(SpendOrganizations.unreported.showsPrompt)
    }

    /// Without the daemon's words there is nothing to say, so nothing draws.
    func testMissingWordsDrawNothing() {
        var partial = Self.view
        partial.removeValue(forKey: "prompt_line")
        for view in [nil, partial] as [[String: Any]?] {
            let status = SpendOrganizations.parse(
                fromJSON: Self.payload(organizations: [("org-a", true), ("org-b", false)], view: view))
            XCTAssertNil(status.words)
            XCTAssertFalse(status.showsPicker)
            XCTAssertFalse(status.showsPrompt)
        }
    }

    /// No key, no current organization: nothing spends, so nothing prompts.
    func testNoCurrentOrganizationDrawsNothing() {
        let status = SpendOrganizations.parse(
            fromJSON: Self.payload(organizations: [("org-a", false), ("org-b", false)], current: nil, prompt: false))
        XCTAssertFalse(status.showsPicker)
        XCTAssertFalse(status.showsPrompt)
    }

    func testFailureLinesByLabel() {
        let status = SpendOrganizations.parse(
            fromJSON: Self.payload(organizations: [("org-a", true), ("org-b", false)]))
        XCTAssertEqual(status.failureLine(for: "near_ai_spend_org_session_expired"), "EXPIRED")
        XCTAssertEqual(status.failureLine(for: "near_ai_spend_org_no_session"), "EXPIRED")
        XCTAssertEqual(status.failureLine(for: "near_ai_spend_org_refused"), "REFUSED")
        XCTAssertEqual(status.failureLine(for: "near_ai_spend_org_unavailable"), "FAILED")
        XCTAssertEqual(status.failureLine(for: "near_ai_spend_org_something_new"), "FAILED")
        // Not the daemon's refusal: the switch may have happened, so no line
        // may say nothing changed.
        XCTAssertNil(status.failureLine(for: "unparseable-response"))
        XCTAssertNil(status.failureLine(for: ""))
        XCTAssertNil(SpendOrganizations.unreported.failureLine(for: "near_ai_spend_org_refused"))
    }

    func testRevokePendingIsCarried() {
        let status = SpendOrganizations.parse(
            fromJSON: Self.payload(organizations: [("org-a", true), ("org-b", false)], revokePending: true))
        XCTAssertTrue(status.revokePending)
    }
}
