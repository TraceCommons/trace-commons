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

    /// Review Focus 3, on the screen: a refused grant is said in the core's
    /// refusal sentence, and the Sharing line reads Ask me's, never
    /// Automatic's.
    func test_aRefusedGrantFinishesOnAskMe() throws {
        let uses = try copy().uses
        let grant = try grant()
        for label in ["automatic-grant-witness-changed", "arming-terms-unavailable"] {
            let line = try XCTUnwrap(UsesScreenLayout.notice(for: .grantRefused(label: label)), label)
            XCTAssertEqual(line, TCCoreCopy.contributionOverrideRefusalLine(label: label))
        }
        XCTAssertNil(UsesScreenLayout.notice(for: nil))

        // The runner leaves the state on Ask me; the line follows it.
        var state = FirstRunState(tier: .quick, step: .uses, account: .nearAI, sharing: .automatic)
        state.sharing = .askMe
        XCTAssertEqual(UsesScreenLayout.sharingLine(uses, path: state.sharing, grant: grant), grant.pathAskFirst)

        let source = try Self.source()
        XCTAssertTrue(source.contains("UsesScreenLayout.notice(for: runner.failure)"))
    }
}
