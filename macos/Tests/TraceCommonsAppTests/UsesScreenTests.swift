import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's Uses screen (#1030 `uses-screen.tsx`): the always-on use shown
/// unticked and required, Start held until it is ticked (owner, 2026-09-28
/// point 4), the Sharing picker worded by the core, and the Private AI card
/// on Custom only. Read from the real core tables and the screen's source,
/// the house pattern for a SwiftUI view.
final class UsesScreenTests: XCTestCase {
    private func copy() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    private func modes() throws -> ContributionModeCopy {
        try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
    }

    private func grant() throws -> AutomaticGrantCopy {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("tc-uses-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        return try XCTUnwrap(
            AutomaticGrantCopy.decode(fromJSON: TCCoreCopy.automaticContributionCopyJSON(configDir: dir.path)))
    }

    /// `consent_options` as the daemon answers it: the floor first, three
    /// data uses, then the handle, which grants no data use.
    private let options = [
        ConsentScope(name: "debugging_evaluation", description: "d", alwaysOn: true, grantsDataUse: true),
        ConsentScope(name: "benchmark_only", description: "b", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "ranking_training", description: "r", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "model_training", description: "m", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "public_attribution", description: "p", alwaysOn: false, grantsDataUse: false),
    ]

    private static func source() throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/UsesScreen.swift")
        return try String(contentsOf: url, encoding: .utf8)
    }

    func test_theRequiredUseStartsUnticked() throws {
        XCTAssertEqual(UsesScreenLayout.requiredScope(options)?.name, "debugging_evaluation")
        XCTAssertEqual(
            UsesScreenLayout.optionalScopes(options).map(\.name),
            ["benchmark_only", "ranking_training", "model_training"])
        XCTAssertEqual(UsesScreenLayout.handleScopes(options).map(\.name), ["public_attribution"])

        // Nothing is ticked for the person, the floor included.
        let fresh = FirstRunState(tier: .quick, step: .uses, account: .nearAI)
        XCTAssertTrue(fresh.scopes.isEmpty)
        XCTAssertFalse(UsesScreenLayout.isTicked("debugging_evaluation", in: fresh))

        // The row carries Ron's "required" tag, and nothing in the screen
        // ticks a scope except the person's own toggle.
        let source = try Self.source()
        XCTAssertTrue(source.contains("GlassTag(copy.uses.required"))
        XCTAssertEqual(source.components(separatedBy: "scopes.insert(").count - 1, 1)
        XCTAssertFalse(source.contains("scopes = "))

        // The optional group reads Ron's summary, every placeholder filled.
        let uses = try copy().uses
        var some = fresh
        some.scopes = ["ranking_training"]
        let summary = UsesScreenLayout.optionalSummary(uses, scopes: some.scopes, optional: UsesScreenLayout.optionalScopes(options))
        XCTAssertFalse(summary.contains("{"), summary)
        XCTAssertTrue(summary.contains("3"), summary)
        XCTAssertTrue(summary.contains("1"), summary)
        XCTAssertEqual(UsesScreenLayout.group(some.scopes, optional: UsesScreenLayout.optionalScopes(options)), .some)
        XCTAssertEqual(UsesScreenLayout.group([], optional: UsesScreenLayout.optionalScopes(options)), .none)
    }

    func test_startIsDisabledUntilTheRequiredUseIsTicked() throws {
        let grant = try grant()
        var state = FirstRunState(tier: .quick, step: .uses, account: .nearAI)
        let required = UsesScreenLayout.requiredScope(options)
        XCTAssertFalse(UsesScreenLayout.canStart(state, requiredScope: required, grant: grant, isCommitting: false))

        // Every optional use ticked is still not the required one.
        state.scopes = Set(UsesScreenLayout.optionalScopes(options).map(\.name))
        XCTAssertFalse(UsesScreenLayout.canStart(state, requiredScope: required, grant: grant, isCommitting: false))

        state.scopes.insert("debugging_evaluation")
        XCTAssertTrue(UsesScreenLayout.canStart(state, requiredScope: required, grant: grant, isCommitting: false))

        // Without the sharing words, or while a Start is running, it stays
        // disabled; with no required use known, it never enables.
        XCTAssertFalse(UsesScreenLayout.canStart(state, requiredScope: required, grant: nil, isCommitting: false))
        XCTAssertFalse(UsesScreenLayout.canStart(state, requiredScope: required, grant: grant, isCommitting: true))
        XCTAssertFalse(UsesScreenLayout.canStart(state, requiredScope: nil, grant: grant, isCommitting: false))

        // The footer note is Ron's, until the box is ticked.
        let uses = try copy().uses
        XCTAssertNil(UsesScreenLayout.footerNote(uses, state: state, requiredScope: required))
        state.scopes.remove("debugging_evaluation")
        XCTAssertEqual(UsesScreenLayout.footerNote(uses, state: state, requiredScope: required), uses.baseUseNote)
    }

    /// The Sharing card's line is the core's: Ask me reads `path_ask_first`;
    /// Automatic reads `path_automatic` then the scrub's scope and limit; no
    /// copy reads the fallbacks. Watching only offers Ask me alone.
    func test_theSharingLineIsTheCores() throws {
        let uses = try copy().uses
        let grant = try grant()
        let scrub = try XCTUnwrap(grant.scrub)
        let pathAskFirst = try XCTUnwrap(grant.pathAskFirst)
        let pathAutomatic = try XCTUnwrap(grant.pathAutomatic)
        XCTAssertEqual(UsesScreenLayout.sharingLine(uses, path: .askMe, grant: grant), pathAskFirst)
        XCTAssertEqual(
            UsesScreenLayout.sharingLine(uses, path: .automatic, grant: grant),
            [pathAutomatic, scrub.scope, scrub.limit].joined(separator: " "))
        XCTAssertEqual(UsesScreenLayout.sharingLine(uses, path: .automatic, grant: nil), uses.sharingUnavailable)

        let modes = try modes()
        let full = UsesScreenLayout.sharingOptions(for: .nearAI, modes: modes)
        XCTAssertEqual(full.map(\.value), [.askMe, .automatic])
        XCTAssertEqual(full.map(\.title), [modes.label(for: .ask), modes.label(for: .autoUpload)])
        XCTAssertEqual(UsesScreenLayout.sharingOptions(for: .watchOnly, modes: modes).map(\.value), [.askMe])
    }

    func test_privateAIAppearsOnlyInCustom() throws {
        XCTAssertFalse(UsesScreenLayout.showsPrivateAI(FirstRunState(tier: .quick, step: .uses)))
        XCTAssertTrue(UsesScreenLayout.showsPrivateAI(FirstRunState(tier: .custom, step: .uses)))

        // The card is behind that guard and worded by the Private AI copy;
        // the switch stays disabled until the copy is there.
        let source = try Self.source()
        let guardRange = try XCTUnwrap(source.range(of: "UsesScreenLayout.showsPrivateAI("))
        let cardRange = try XCTUnwrap(source.range(of: "privateAICard"))
        XCTAssertLessThan(guardRange.lowerBound, cardRange.lowerBound)
        XCTAssertTrue(source.contains("model.privateInferenceCopy"))
        XCTAssertTrue(source.contains(".disabled(privateAI == nil)"))
    }

    private func privateAI() throws -> PrivateInferenceCopy {
        try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? ""))
    }

    /// Review Focus 3, on the screen: a refused grant comes after setup
    /// finished, so whatever the label, the notice is the first run's own
    /// line saying Automatic was not turned on and sharing is on Ask me. It
    /// is never the folder-override refusal, which says nothing changed or
    /// points at a pill this screen does not have.
    func test_aRefusedGrantSaysSetupFinishedOnAskMe() throws {
        let uses = try copy().uses
        let privateAI = try privateAI()
        let labels = [
            "automatic-grant-witness-changed", "arming-terms-unavailable", "connect", "scrub_disclosure",
            SharingDisclosureFlow.unreadableLabel,
        ]
        for label in labels {
            let line = try XCTUnwrap(
                UsesScreenLayout.notice(for: .grantRefused(label: label), uses: uses, privateAI: privateAI), label)
            XCTAssertEqual(line, uses.sharingRefused, label)
            XCTAssertNotEqual(line, TCCoreCopy.contributionOverrideRefusalLine(label: label), label)
            XCTAssertFalse(line.contains("Nothing changed"), label)
            XCTAssertFalse(line.contains("pill"), label)
        }
        XCTAssertNil(UsesScreenLayout.notice(for: nil, uses: uses, privateAI: privateAI))

        let source = try Self.source()
        XCTAssertTrue(source.contains("UsesScreenLayout.notice(for: runner.failure"))
    }

    /// Each failure that stops Start reads its own line; Private AI's is
    /// the Private AI copy's, and the first run's when that copy is missing.
    func test_eachStartFailureReadsItsOwnLine() throws {
        let uses = try copy().uses
        let privateAI = try privateAI()
        XCTAssertEqual(UsesScreenLayout.notice(for: .scopesFailed, uses: uses, privateAI: privateAI), uses.scopesFailed)
        XCTAssertEqual(UsesScreenLayout.notice(for: .rulesFailed, uses: uses, privateAI: privateAI), uses.rulesFailed)
        XCTAssertEqual(
            UsesScreenLayout.notice(for: .privateAIFailed, uses: uses, privateAI: privateAI), privateAI.writeUnconfirmed)
        XCTAssertEqual(UsesScreenLayout.notice(for: .privateAIFailed, uses: uses, privateAI: nil), uses.privateAiFailed)
    }

    /// A refusal the core decided before Start is kept while a later call
    /// fails, and shown once a Start finally succeeds, so the retry that
    /// goes through on Ask me still says why.
    func test_aRefusalOutlivesAFailedStart() {
        let refused = FirstRunFailure.grantRefused(label: "connect")
        let failed = UsesScreenLayout.afterStart(failure: .rulesFailed, pending: refused)
        XCTAssertEqual(failed.shown, .rulesFailed)
        XCTAssertEqual(failed.pending, refused)

        let retried = UsesScreenLayout.afterStart(failure: nil, pending: failed.pending)
        XCTAssertEqual(retried.shown, refused)
        XCTAssertNil(retried.pending)

        let plain = UsesScreenLayout.afterStart(failure: nil, pending: nil)
        XCTAssertNil(plain.shown)
        XCTAssertNil(plain.pending)

        // A fresh pass through the disclosures supersedes the earlier
        // verdict: granted this time, nothing stale is shown after it.
        XCTAssertNil(UsesScreenLayout.refusalToCarry(decidedNow: true, refused: nil, pending: refused))
        XCTAssertEqual(
            UsesScreenLayout.refusalToCarry(decidedNow: true, refused: refused, pending: nil), refused)
        // A plain Start (Ask me) carries the earlier one.
        XCTAssertEqual(UsesScreenLayout.refusalToCarry(decidedNow: false, refused: nil, pending: refused), refused)
        let regranted = UsesScreenLayout.afterStart(
            failure: nil, pending: UsesScreenLayout.refusalToCarry(decidedNow: true, refused: nil, pending: refused))
        XCTAssertNil(regranted.shown)
        XCTAssertNil(regranted.pending)

        // Both Start paths go through that decision.
        let source = try? Self.source()
        XCTAssertEqual(source?.components(separatedBy: "await runner.commit(.start)").count, 2)
        XCTAssertTrue(source?.contains("UsesScreenLayout.afterStart(") ?? false)
        XCTAssertEqual(source?.components(separatedBy: "UsesScreenLayout.refusalToCarry(").count, 3)
        XCTAssertFalse(source?.contains("?? pendingRefusal") ?? true)
    }

    /// An account that cannot choose Automatic reads Ask me's line and
    /// selection, whatever `sharing` holds.
    func test_watchingOnlyNeverReadsAutomatic() throws {
        let uses = try copy().uses
        let grant = try grant()
        let watching = FirstRunState(tier: .quick, step: .uses, account: .watchOnly, sharing: .automatic)
        XCTAssertEqual(UsesScreenLayout.effectiveSharing(watching), .askMe)
        XCTAssertEqual(
            UsesScreenLayout.sharingLine(uses, path: UsesScreenLayout.effectiveSharing(watching), grant: grant),
            grant.pathAskFirst)
        let joined = FirstRunState(tier: .quick, step: .uses, account: .nearAI, sharing: .automatic)
        XCTAssertEqual(UsesScreenLayout.effectiveSharing(joined), .automatic)

        let source = try Self.source()
        XCTAssertFalse(source.contains("path: runner.state.sharing"))
        XCTAssertFalse(source.contains("get: { runner.state.sharing }"))
    }

    /// The grant copy is read once; until then the line is the loading one
    /// and Start is held.
    func test_theSharingLineLoadsOnce() throws {
        let uses = try copy().uses
        XCTAssertEqual(UsesScreenLayout.sharingLine(uses, path: .askMe, grant: nil, isLoading: true), uses.sharingLoading)
        XCTAssertEqual(UsesScreenLayout.sharingLine(uses, path: .askMe, grant: nil, isLoading: false), uses.sharingUnavailable)
        let source = try Self.source()
        XCTAssertFalse(source.contains("private var grant: AutomaticGrantCopy? {"))
    }
}
