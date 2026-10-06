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

    // The confirmation's words are the core's (`project_copy`, through
    // `tc_arming_offer_copy_json`); the heading, the whole body, and that it
    // states the scrubbing, that review stops and the way back, are asserted
    // against the real export in `TCBridgeTests/CoreCopyExportTests`.

    // MARK: - Mode names

    /// A project mode reads by the core's one name for it, looked up in the
    /// pill's table by wire mode (owner decision, 2026-10-02). The words here
    /// are a fixture; `TraceCommonsAppTests/ProjectModeWordsTests` checks the
    /// real export.
    func testAModeReadsByItsNameInThePillTable() throws {
        let json = #"""
            {"title":"t","mixed":"m","override_active":"o","clear":"c","auto_partial":"p",
             "choices":[{"mode":"notify_only","label":"A","line":"a"},
                        {"mode":"auto_upload","label":"B","line":"b"},
                        {"mode":"ignore","label":"C","line":"c"}]}
            """#
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: json))
        XCTAssertEqual(copy.label(for: .ask), "A")
        XCTAssertEqual(copy.label(for: .autoUpload), "B")
        XCTAssertEqual(copy.label(for: .ignore), "C")
    }
}
