import XCTest
@testable import TCShellCore

/// Arming a project -- setting it to contribute without asking -- is the
/// strongest thing this app can be set to do, and until now macOS was the
/// only shell that could not do it at all. The Linux shell has offered it
/// from Settings for some time (`crates/trace-commons-contributor-gtk/src/ui/
/// settings.rs`, `mode_choices` and `confirm_arming`) and so has Windows
/// (`windows/src/TraceCommons.Interop/UnresolvedBucketCopy.cs`,
/// `OfferableModes`).
///
/// These tests pin the two things that must not drift when macOS catches up:
/// which modes a row may be offered, and the words used to confirm the one
/// that matters. The offer rule is a correctness claim about what the daemon
/// will accept, not a presentation choice -- `Policy` refuses `auto_upload`
/// for the unresolvable bucket in two independent places
/// (`daemon/policy.rs`, `set_mode` and `resolve`), so a picker that offered
/// it there would invite a contributor to believe they had armed something
/// that cannot be armed, and the refusal would be silent.
final class ProjectArmingCopyTests: XCTestCase {
    private func row(bucket: Bool) -> ProjectRow {
        ProjectRow(
            projectId: bucket ? "bucket" : "abc123",
            projectLabel: bucket ? "unknown-project" : "api",
            mode: .ask,
            isUnresolvedBucket: bucket
        )
    }

    // MARK: - What a row may be offered

    func testAnOrdinaryProjectMayBeArmed() {
        XCTAssertEqual(row(bucket: false).offerableModes, [.ask, .autoUpload, .ignore])
    }

    /// The bucket keeps `ignore`: refusing to contribute it unattended is not
    /// a reason to refuse to silence it. Only arming is withheld.
    func testTheBucketIsOfferedEverythingButArming() {
        XCTAssertEqual(row(bucket: true).offerableModes, [.ask, .ignore])
    }

    /// Omitting the choice is the honest answer; offering it disabled still
    /// puts an arming affordance on a row that has none. Windows states this
    /// in the same words on `MayOfferAutoUpload`.
    func testTheOfferRuleAgreesWithCanBeArmed() {
        for bucket in [true, false] {
            let r = row(bucket: bucket)
            XCTAssertEqual(
                r.offerableModes.contains(.autoUpload),
                r.canBeArmed,
                "bucket=\(bucket)"
            )
        }
    }

    /// The order is the order a picker shows, and it is deliberate: ask-first
    /// is the default and leads, arming sits in the middle, and the
    /// irreversible-feeling one is last. Windows' `OfferableModes` documents
    /// the same ordering for the same reason.
    func testOrderIsStableAndLeadsWithTheDefault() {
        XCTAssertEqual(row(bucket: false).offerableModes.first, .ask)
        XCTAssertEqual(row(bucket: true).offerableModes.first, .ask)
    }

    // MARK: - The confirmation

    func testTheHeadingNamesTheProject() {
        XCTAssertEqual(
            ProjectArmingCopy.confirmationTitle(project: "api"),
            "Contribute from api automatically?"
        )
    }

    /// The body has to say the part a contributor would otherwise discover by
    /// noticing traces they never saw: that review stops. The Linux shell's
    /// `ARMING_BODY` says it in these words and this shell says the same
    /// thing -- two shells describing the same irreversible-feeling switch
    /// differently is worse than either wording alone.
    func testTheBodySaysReviewStops() {
        let body = ProjectArmingCopy.confirmationBody
        XCTAssertTrue(body.contains("without asking you"), body)
        XCTAssertTrue(body.contains("You won't review them first."), body)
    }

    /// Every confirmation in this app names the way back. Arming is
    /// reversible and the body must say so, or it reads as a door that only
    /// opens one way.
    func testTheBodyNamesTheWayBack() {
        XCTAssertTrue(
            ProjectArmingCopy.confirmationBody.contains("turn this off at any time"),
            ProjectArmingCopy.confirmationBody
        )
    }

    /// The scrubbing promise is load-bearing here in a way it is not under
    /// ask-first: nobody reads a preview once this is on. It is stated in the
    /// body for that reason.
    func testTheBodyStatesTheScrubbing() {
        XCTAssertTrue(
            ProjectArmingCopy.confirmationBody.contains("scrubbed"),
            ProjectArmingCopy.confirmationBody
        )
    }

    /// A confirm button that says "OK" makes the reader reconstruct what they
    /// are agreeing to from the heading. This one carries the action.
    func testTheButtonsCarryTheirActions() {
        XCTAssertEqual(ProjectArmingCopy.confirm, "Turn on automatic contributing")
        XCTAssertEqual(ProjectArmingCopy.cancel, "Not now")
    }

    // MARK: - Picker labels

    /// The picker offers choices, so its labels are actions ("Contribute
    /// automatically"), not the state sentences Settings shows beside a row
    /// ("Contributed without asking"). Both exist; they are not
    /// interchangeable, and these are the Linux shell's `mode_choices` words.
    func testChoiceLabelsAreActionsNotStates() {
        XCTAssertEqual(ProjectCopy.modeChoiceLabel(.ask), "Ask me first")
        XCTAssertEqual(ProjectCopy.modeChoiceLabel(.autoUpload), "Contribute automatically")
        XCTAssertEqual(ProjectCopy.modeChoiceLabel(.ignore), "Never offer this one")
    }
}
