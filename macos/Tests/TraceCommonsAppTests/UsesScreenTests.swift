import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's Uses screen (#1030 `uses-screen.tsx`): the always-on use shown
/// ticked, locked and required, and always included in what Start sends
/// (owner, 2026-10-08, superseding 2026-09-28 point 4), nothing optional
/// ticked, the Sharing picker worded by the core, and the Private AI card
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
        ConsentScope(name: "debugging_evaluation", title: "debugging_evaluation", description: "d", alwaysOn: true, grantsDataUse: true),
        ConsentScope(name: "benchmark_only", title: "benchmark_only", description: "b", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "ranking_training", title: "ranking_training", description: "r", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "model_training", title: "model_training", description: "m", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "public_attribution", title: "public_attribution", description: "p", alwaysOn: false, grantsDataUse: false),
    ]

    private static func source(_ file: String = "UsesScreen.swift") throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/\(file)")
        return try String(contentsOf: url, encoding: .utf8)
    }

    /// Start sharing is Join's, the last step.
    private static func joinSource() throws -> String { try source("JoinScreen.swift") }

    func test_theRequiredUseIsTickedAndLockedAndNothingOptionalIs() throws {
        XCTAssertEqual(UsesScreenLayout.requiredScope(options)?.name, "debugging_evaluation")
        XCTAssertEqual(
            UsesScreenLayout.optionalScopes(options).map(\.name),
            ["benchmark_only", "ranking_training", "model_training"])
        XCTAssertEqual(UsesScreenLayout.handleScopes(options).map(\.name), ["public_attribution"])

        // The required use is included in the state; nothing optional is
        // ticked for the person.
        let fresh = FirstRunState(step: .join, account: .nearAI)
        XCTAssertTrue(fresh.scopes.isEmpty)
        let included = UsesScreenLayout.includingRequired(fresh, required: UsesScreenLayout.requiredScope(options))
        XCTAssertEqual(included.scopes, ["debugging_evaluation"])
        XCTAssertTrue(UsesScreenLayout.isTicked("debugging_evaluation", in: included))
        for optional in UsesScreenLayout.optionalScopes(options) + UsesScreenLayout.handleScopes(options) {
            XCTAssertFalse(UsesScreenLayout.isTicked(optional.name, in: included), optional.name)
        }
        // Optional choices are kept; with no required use known, nothing
        // changes.
        var chosen = fresh
        chosen.scopes = ["ranking_training"]
        XCTAssertEqual(
            UsesScreenLayout.includingRequired(chosen, required: UsesScreenLayout.requiredScope(options)).scopes,
            ["ranking_training", "debugging_evaluation"])
        XCTAssertEqual(UsesScreenLayout.includingRequired(chosen, required: nil), chosen)

        // The required use is one "Always on" line (owner, 2026-10-28),
        // never a box; the person's toggle ticks only the optional ones.
        let source = try Self.source()
        XCTAssertTrue(source.contains("Text(UsesScreenLayout.alwaysOnTitle(required, uses: copy.uses))"))
        XCTAssertFalse(source.contains("Toggle(required.title"), "the required use is a statement, not a choice")
        XCTAssertFalse(source.contains("scope(required.name)"))
        XCTAssertEqual(source.components(separatedBy: "scopes.insert(").count - 1, 1)
        XCTAssertFalse(source.contains("scopes = "))
        // The state takes the required use once it is known, and Start
        // sharing, on Join, includes it before it routes, so what is sent is
        // what Uses showed.
        XCTAssertTrue(source.contains(".onChange(of: required?.name, initial: true) { _, _ in includeRequired() }"))
        let join = try Self.joinSource()
        let start = try XCTUnwrap(join.range(of: "private func start() {"))
        let startBody = String(join[start.upperBound...].prefix(400))
        let include = try XCTUnwrap(startBody.range(of: "includeRequired()"))
        let route = try XCTUnwrap(startBody.range(of: "UsesScreenLayout.startRoute(runner.state)"))
        XCTAssertLessThan(include.lowerBound, route.lowerBound)

        // The optional group's box reads mixed while some are on; there is
        // no summary beside it (Ron, 2026-10-22: "3 optional uses, all off"
        // removed).
        var some = fresh
        some.scopes = ["ranking_training"]
        XCTAssertFalse(source.contains("optionalSummary"))
        XCTAssertEqual(UsesScreenLayout.group(some.scopes, optional: UsesScreenLayout.optionalScopes(options)), .some)
        XCTAssertEqual(UsesScreenLayout.group([], optional: UsesScreenLayout.optionalScopes(options)), .none)
    }

    /// The optional uses start unticked (R7) and two buttons of equal weight
    /// sit above them (owner, 2026-10-27): Allow all ticks every optional
    /// data use, never the handle; Only required unticks them and keeps the
    /// required use. Nothing ticks a box but the person.
    func test_theOptionalUsesStartUntickedAndTheTwoButtonsAreEqual() throws {
        let uses = try copy().uses
        XCTAssertEqual(uses.allowAll, "Allow all")
        XCTAssertEqual(uses.onlyRequired, "Only required")
        let optional = UsesScreenLayout.optionalScopes(options)
        let fresh = UsesScreenLayout.includingRequired(
            FirstRunState(step: .uses), required: UsesScreenLayout.requiredScope(options))
        XCTAssertEqual(fresh.scopes, ["debugging_evaluation"], "nothing optional is ticked for the person")
        XCTAssertEqual(UsesScreenLayout.group(fresh.scopes, optional: optional), .none)

        let all = UsesScreenLayout.pressing(.allowAll, fresh, optional: optional)
        XCTAssertEqual(
            all.scopes, ["debugging_evaluation", "benchmark_only", "ranking_training", "model_training"])
        XCTAssertFalse(all.scopes.contains("public_attribution"))
        XCTAssertEqual(UsesScreenLayout.pressing(.allowAll, all, optional: optional), all, "pressing again changes nothing")

        let required = UsesScreenLayout.pressing(.onlyRequired, all, optional: optional)
        XCTAssertEqual(required.scopes, ["debugging_evaluation"])
        XCTAssertEqual(UsesScreenLayout.pressing(.onlyRequired, required, optional: optional), required)

        // A box still toggles on its own after either press, and nothing
        // but the scopes changes.
        var one = required
        one.scopes.insert("ranking_training")
        XCTAssertEqual(UsesScreenLayout.group(one.scopes, optional: optional), .some)
        var others = all
        others.scopes = fresh.scopes
        XCTAssertEqual(others, fresh)

        // Same style, same width, side by side, in one loop: neither is
        // emphasised.
        let source = try Self.source()
        XCTAssertEqual(UsesScreenLayout.Bulk.allCases, [.allowAll, .onlyRequired])
        XCTAssertTrue(source.contains("ForEach(UsesScreenLayout.Bulk.allCases, id: \\.self) { bulk in"))
        XCTAssertTrue(source.contains("Text(bulk.title(copy.uses)).frame(maxWidth: .infinity)"))
        XCTAssertEqual(source.components(separatedBy: "GlassButtonStyle(").count - 1, 1)
        XCTAssertFalse(source.contains("uses.allowAll)"), "no button of its own for either")
        XCTAssertFalse(source.contains(".onAppear { runner.state.scopes"))
    }

    /// The required use is one Always on line under the choices, its scope
    /// the tooltip, so every box shown is a real choice (owner, 2026-10-28).
    func test_theRequiredUseIsOneAlwaysOnLine() throws {
        let uses = try copy().uses
        let required = try XCTUnwrap(UsesScreenLayout.requiredScope(options))
        XCTAssertEqual(
            UsesScreenLayout.alwaysOnTitle(required, uses: uses),
            "Always on: \(required.title) for shared traces")
        let source = try Self.source()
        XCTAssertTrue(source.contains("Text(UsesScreenLayout.alwaysOnTitle(required, uses: copy.uses))"))
        XCTAssertFalse(source.contains("GlassExpander("), "nothing to expand")
        XCTAssertFalse(source.contains("Text(copy.uses.required)"), "the line already says always on")
        let line = try XCTUnwrap(source.range(of: "Text(UsesScreenLayout.alwaysOnTitle"))
        let rows = try XCTUnwrap(source.range(of: "ForEach(UsesScreenLayout.shownOptionalScopes(options, uses: copy.uses))"))
        XCTAssertLessThan(rows.lowerBound, line.lowerBound, "the line follows the choices")
    }

    /// The Sharing radios show no choice until the person picks one, and
    /// neither is tagged or emphasised (owner, 2026-10-27).
    func test_noSharingChoiceIsPreselected() throws {
        let fresh = FirstRunState(step: .uses)
        XCTAssertNil(UsesScreenLayout.sharingSelection(fresh))
        XCTAssertEqual(UsesScreenLayout.choosingSharing(fresh, nil), fresh)
        let automatic = UsesScreenLayout.choosingSharing(fresh, .automatic)
        XCTAssertTrue(automatic.sharingChosen)
        XCTAssertEqual(UsesScreenLayout.sharingSelection(automatic), .automatic)
        let ask = UsesScreenLayout.choosingSharing(automatic, .askMe)
        XCTAssertEqual(UsesScreenLayout.sharingSelection(ask), .askMe)

        let uses = try copy().uses
        let options = UsesScreenLayout.sharingOptions(for: fresh, modes: try modes(), grant: try grant(), uses: uses)
        XCTAssertEqual(options.map(\.value), [.askMe, .automatic])
        let source = try Self.source()
        XCTAssertTrue(source.contains("get: { UsesScreenLayout.sharingSelection(runner.state) }"))
        XCTAssertFalse(source.contains("Recommended"))
        XCTAssertFalse(source.contains("GlassTag("))
    }

    /// Someone who already joined never sees the first run, and a pick on
    /// it is sent as before: Ask still stops an earlier Automatic first.
    func test_peopleWhoJoinedAreUnaffected() {
        XCTAssertFalse(OnboardingNavigation.hostsFirstRun(startup: .running, requiresOnboarding: false, entered: true))
        var state = FirstRunState(step: .join, account: .enrolled, scopes: ["debugging_evaluation"])
        state = UsesScreenLayout.choosingSharing(state, .askMe)
        XCTAssertEqual(
            Array(FirstRunPlan.calls(for: state, at: .start).prefix(2)),
            [.setAskEachTime, .setConsentScopes(["debugging_evaluation"])])
    }

    /// All optional data uses are independently visible. Attribution belongs
    /// in Public profile, so opening the data-use screen cannot grant it.
    func test_dataUsesAreVisibleAndPublicAttributionIsSeparate() throws {
        let expected = ["debugging_evaluation", "benchmark_only", "ranking_training", "model_training"]
        XCTAssertEqual(UsesScreenLayout.visibleScopes(options, optionalOpen: false).map(\.name), expected)
        XCTAssertEqual(UsesScreenLayout.visibleScopes(options, optionalOpen: true).map(\.name), expected)
        let source = try Self.source()
        XCTAssertFalse(source.contains("Toggle(sources:"), "a bulk choice obscures independent permissions")
        XCTAssertFalse(source.contains("ForEach(UsesScreenLayout.handleScopes(options))"))
        // Off the first run's Uses (owner, 2026-10-28); Settings keeps it.
        XCTAssertFalse(source.contains("Text(copy.uses.rewardOpportunities"))
        XCTAssertEqual(try copy().uses.rewardOpportunities,
                       "To earn credit for your traces, allow Train coding models. Create benchmarks and Train evaluators are extra uses that may earn more. Credit depends on acceptance and scoring.")
        let profile = try String(contentsOf: URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Settings/PublicProfileSection.swift"), encoding: .utf8)
        XCTAssertTrue(profile.contains("!$0.alwaysOn && !$0.grantsDataUse"))
        XCTAssertTrue(profile.contains("ConsentScopeToggle(scope: scope)"))
        let join = try Self.joinSource()
        XCTAssertTrue(join.contains("if runner.state.holdsEnrolment {"))
        XCTAssertTrue(join.contains("PayoutLinkView()"))
    }

    /// Start waits for no tick: the required use is always included. It
    /// still needs the required use known, the sharing words, an account it
    /// can finish, and no Start running.
    func test_startDoesNotWaitForATick() throws {
        let grant = try grant()
        let uses = try copy().uses
        let state = FirstRunState(step: .join, account: .nearAI, sharingChosen: true, enrolledInvite: "INVITE-1")
        let required = UsesScreenLayout.requiredScope(options)
        XCTAssertTrue(UsesScreenLayout.canStart(state, uses: uses, requiredScope: required, grant: grant, isCommitting: false))

        // Without the sharing words, or while a Start is running, it stays
        // disabled; with no required use known, it never enables.
        XCTAssertFalse(UsesScreenLayout.canStart(state, uses: uses, requiredScope: required, grant: nil, isCommitting: false))
        XCTAssertFalse(UsesScreenLayout.canStart(state, uses: uses, requiredScope: required, grant: grant, isCommitting: true))
        XCTAssertFalse(UsesScreenLayout.canStart(state, uses: uses, requiredScope: nil, grant: grant, isCommitting: false))

        // No footer note asks for the required use (owner, 2026-10-08).
        let source = try Self.source()
        XCTAssertFalse(source.contains("baseUseNote"))
        XCTAssertFalse(source.contains("note: "))
    }

    /// The core's grant copy with `field` removed, as a copy that lacks it
    /// would decode.
    private func grant(without field: String) throws -> AutomaticGrantCopy {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("tc-uses-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let json = try XCTUnwrap(TCCoreCopy.automaticContributionCopyJSON(configDir: dir.path))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertNotNil(object.removeValue(forKey: field), field)
        let data = try JSONSerialization.data(withJSONObject: object)
        return try XCTUnwrap(AutomaticGrantCopy.decode(fromJSON: String(data: data, encoding: .utf8)))
    }

    /// The fallback line says Starting is disabled, so whenever it shows,
    /// Start is disabled: a copy without the chosen path's line disables
    /// Start, and only on that path.
    func test_startIsDisabledWheneverTheSharingLineIsTheFallback() throws {
        let uses = try copy().uses
        let required = UsesScreenLayout.requiredScope(options)
        var state = FirstRunState(
            step: .join, account: .nearAI, scopes: ["debugging_evaluation"], sharingChosen: true,
            enrolledInvite: "INVITE-1")

        let noAskFirst = try grant(without: "path_ask_first_title")
        XCTAssertEqual(
            UsesScreenLayout.sharingLine(uses, path: .askMe, grant: noAskFirst), uses.sharingUnavailable)
        XCTAssertFalse(UsesScreenLayout.canStart(
            state, uses: uses, requiredScope: required, grant: noAskFirst, isCommitting: false))

        // A detail missing reads the fallback too: no line is shown
        // without the rest of its answer.
        let noAutomatic = try grant(without: "path_automatic_detail")
        XCTAssertTrue(UsesScreenLayout.canStart(
            state, uses: uses, requiredScope: required, grant: noAutomatic, isCommitting: false))
        state.sharing = .automatic
        XCTAssertFalse(UsesScreenLayout.canStart(
            state, uses: uses, requiredScope: required, grant: noAutomatic, isCommitting: false))
        // Automatic's line is read for whoever chose it, now that Uses
        // offers it before sign-in, so its absence holds Start on Ask's
        // words only once Ask is chosen again.
        state.sharing = .askMe
        XCTAssertTrue(UsesScreenLayout.canStart(
            state, uses: uses, requiredScope: required, grant: noAutomatic, isCommitting: false))
    }

    /// The Sharing card's line is the core's first line for the path, the
    /// rest behind an info button (owner, 2026-10-08): Ask reads
    /// `path_ask_first_title`, then `path_ask_first_detail` behind the
    /// button; Automatic reads `path_automatic_title`, then its detail and
    /// the scrub's scope and limit behind the button. No copy reads the
    /// fallbacks. Watching only offers Ask alone.
    func test_theSharingLineIsTheCores() throws {
        let uses = try copy().uses
        let grant = try grant()
        let scrub = try XCTUnwrap(grant.scrub)
        XCTAssertEqual(grant.pathAskFirstTitle, "Review each trace yourself.")
        XCTAssertEqual(grant.pathAskFirstDetail, "Nothing is contributed until you approve it.")
        XCTAssertEqual(grant.pathAutomaticTitle, "Contribute automatically from projects that first appear.")
        XCTAssertFalse(try XCTUnwrap(grant.pathAskFirst).contains("later"))
        XCTAssertEqual(UsesScreenLayout.sharingLine(uses, path: .askMe, grant: grant), grant.pathAskFirstTitle)
        XCTAssertEqual(UsesScreenLayout.sharingDetail(path: .askMe, grant: grant), grant.pathAskFirstDetail)
        XCTAssertEqual(UsesScreenLayout.sharingLine(uses, path: .automatic, grant: grant), try XCTUnwrap(uses.automaticScope))
        XCTAssertEqual(
            UsesScreenLayout.sharingDetail(path: .automatic, grant: grant, uses: uses),
            [try XCTUnwrap(uses.automaticScope), scrub.scope, scrub.limit].joined(separator: " "))
        XCTAssertEqual(UsesScreenLayout.sharingLine(uses, path: .automatic, grant: nil), uses.sharingUnavailable)
        XCTAssertNil(UsesScreenLayout.sharingDetail(path: .automatic, grant: nil))
        XCTAssertNil(UsesScreenLayout.sharingDetail(path: .askMe, grant: try self.grant(without: "path_ask_first_detail")))
        let source = try Self.source()
        XCTAssertTrue(source.contains("infoButton(title: copy.uses.sharing, text: detail)"))

        let modes = try modes()
        let joined = FirstRunState(step: .join, account: .nearAI, enrolledInvite: "INVITE-1")
        let full = UsesScreenLayout.sharingOptions(for: joined, modes: modes, grant: grant, uses: uses)
        // The canvas's order: Ask, the default, then Automatic, each with
        // the first line of the core's words for it.
        XCTAssertEqual(full.map(\.value), [.askMe, .automatic])
        XCTAssertEqual(full.map(\.title), [try XCTUnwrap(uses.askEachTime), modes.label(for: .autoUpload)])
        XCTAssertEqual(full.map(\.sub), [grant.pathAskFirstTitle, try XCTUnwrap(uses.automaticScope)])
        // Before sign-in Automatic is offered too, and says it turns on once
        // the person signs in, the last step.
        let before = UsesScreenLayout.sharingOptions(for: FirstRunState(step: .uses), modes: modes, grant: grant, uses: uses)
        XCTAssertEqual(before.map(\.value), [.askMe, .automatic])
        XCTAssertEqual(
            before.last?.sub, [try XCTUnwrap(uses.automaticScope), uses.automaticAfterSignIn].joined(separator: "\n"))
        XCTAssertEqual(UsesScreenLayout.effectiveSharing(FirstRunState(sharing: .automatic)), .automatic)
    }

    /// Private AI is its own step (Ron, 2026-10-21), worded by the Private AI
    /// copy. It has no switch any more (owner, 2026-10-27), so nothing on it
    /// can be turned on before the copy is there.
    func test_privateAIIsItsOwnStep() throws {
        XCTAssertFalse(try Self.source().contains("privateAICard"), "no Private AI card on Uses any more")
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/PrivateAIScreen.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(source.contains("model.privateInferenceCopy"))
        XCTAssertFalse(source.contains("Toggle("), "no switch on the step")
        XCTAssertTrue(source.contains("copy.privateAi.offerLead"))
    }

    private func privateAI() throws -> PrivateInferenceCopy {
        try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? ""))
    }

    /// Review Focus 3, on the screen: a refused grant comes after setup
    /// finished, so whatever the label, the notice is the first run's own
    /// line saying Automatic was not turned on and sharing is on Ask. It
    /// is never the folder-override refusal, which says nothing changed or
    /// points at a pill this screen does not have.
    @MainActor
    func test_aRefusedGrantFinishesOnAskMe() async throws {
        let uses = try copy().uses
        let privateAI = try privateAI()

        // Driven through Start's own path: both disclosures seen, the core
        // refuses (not connected), Start runs on Ask, grants nothing,
        // and the screen's notice is the refusal line.
        let daemon = StartDaemon()
        let runner = FirstRunRunner(state: onUses(.automatic), daemon: daemon)
        let pending = await UsesStart.finish(
            runner: runner, request: try readyRequest(connected: false), pending: nil)
        XCTAssertFalse(daemon.log.contains { if case .grantAutomatic = $0 { true } else { false } })
        XCTAssertEqual(daemon.log.last, .markComplete)
        XCTAssertEqual(runner.state.sharing, .askMe)
        XCTAssertFalse(runner.state.grantReady)
        XCTAssertEqual(runner.failure, .grantRefused(label: "connect"))
        XCTAssertNil(pending)
        XCTAssertEqual(UsesScreenLayout.notice(for: runner.failure, uses: uses, privateAI: privateAI), uses.sharingRefused)

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

        let join = try Self.joinSource()
        XCTAssertTrue(join.contains("UsesScreenLayout.notice(for: failure, uses: copy.uses, privateAI: privateAI)"))
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
        // Every call went through but the marker was not written: Start
        // says setup has not finished rather than doing nothing.
        XCTAssertEqual(UsesScreenLayout.notice(for: .completeFailed, uses: uses, privateAI: privateAI), uses.completeFailed)
    }

    /// A refusal the core decided before Start is kept while a later call
    /// fails, and shown once a Start finally succeeds, so the retry that
    /// goes through on Ask still says why.
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
        // A plain Start (Ask) carries the earlier one.
        XCTAssertEqual(UsesScreenLayout.refusalToCarry(decidedNow: false, refused: nil, pending: refused), refused)
        let regranted = UsesScreenLayout.afterStart(
            failure: nil, pending: UsesScreenLayout.refusalToCarry(decidedNow: true, refused: nil, pending: refused))
        XCTAssertNil(regranted.shown)
        XCTAssertNil(regranted.pending)

        // Both Start paths go through that decision.
        let source = try? Self.source()
        XCTAssertEqual(source?.components(separatedBy: "await runner.commit(.start, carrying: refusal)").count, 2)
        XCTAssertTrue(source?.contains("UsesScreenLayout.afterStart(") ?? false)
        XCTAssertEqual(source?.components(separatedBy: "UsesScreenLayout.refusalToCarry(").count, 3)
        XCTAssertFalse(source?.contains("?? pendingRefusal") ?? true)
    }

    /// Uses offers Automatic before sign-in (Ron's 2026-10-21 canvas), so
    /// the person's choice is what Uses shows and Start's line is read for;
    /// only an enrollment the daemon holds is granted it, at Start.
    func test_automaticIsShownAsChosenButOnlyAnEnrolmentIsGrantedIt() throws {
        let uses = try copy().uses
        let grant = try grant()
        let watching = FirstRunState(step: .join, account: .watchOnly, sharing: .automatic)
        XCTAssertEqual(UsesScreenLayout.effectiveSharing(watching), .automatic)
        XCTAssertEqual(
            UsesScreenLayout.sharingLine(uses, path: UsesScreenLayout.effectiveSharing(watching), grant: grant),
            try XCTUnwrap(uses.automaticScope))
        XCTAssertFalse(SharingDisclosureFlow.isNeeded(for: watching), "no grant to disclose without an enrolment")
        var ready = watching
        ready.grantReady = true
        XCTAssertFalse(FirstRunPlan.calls(for: ready, at: .start).contains { if case .grantAutomatic = $0 { return true } else { return false } })
        let joined = FirstRunState(
            step: .join, account: .nearAI, sharing: .automatic, enrolledInvite: "INVITE-1")
        XCTAssertEqual(UsesScreenLayout.effectiveSharing(joined), .automatic)
        XCTAssertTrue(SharingDisclosureFlow.isNeeded(for: joined))
    }

    /// Each use's scope is its row's plain hover tooltip (owner,
    /// 2026-10-28, replacing #1290's always-visible description).
    func test_scopeDescriptionsAreHoverTooltips() throws {
        let source = try Self.source()
        XCTAssertTrue(source.contains(".help(required.description)"))
        XCTAssertTrue(source.contains(".help(option.description)"))
        XCTAssertFalse(source.contains("Text(option.description)"))
        XCTAssertFalse(source.contains("Text(required.description)"))
        XCTAssertTrue(source.contains("ScrollView {"), "choices remain reachable in the default-height window")
    }

    /// Owner, 2026-10-08: Private AI shows the offer's first two sentences
    /// (the core's, split once) and discloses the rest in place behind the
    /// core's "Learn more"; nothing of the disclosure is dropped. It is its
    /// own step now (Ron, 2026-10-21).
    func test_thePrivateAICardLeadsWithTwoSentencesAndLearnMore() throws {
        let words: FirstRunCopy = try copy()
        XCTAssertTrue(words.privateAi.offerLead.hasPrefix("Lets your tools send AI requests through this app,"))
        XCTAssertEqual(words.privateAi.learnMore, "Learn more")
        let source = try Self.source("PrivateAIScreen.swift")
        let lead = try XCTUnwrap(source.range(of: "Text(copy.privateAi.offerLead)"))
        let more = try XCTUnwrap(source.range(of: "if more {"))
        XCTAssertLessThan(lead.lowerBound, more.lowerBound)
        XCTAssertTrue(source.contains("[copy.privateAi.offerMore, privateAI.offerExposure, privateAI.offerNoRepoint]"))
        XCTAssertTrue(source.contains("Button { more = true } label: {\n                            Text(copy.privateAi.learnMore)"))
        XCTAssertTrue(source.contains("GlassButtonStyle(.link)"))
        XCTAssertFalse(source.contains("privateAI.offerWhat"))
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
    // MARK: - Start's routing

    private func onUses(_ sharing: SharingPath) -> FirstRunState {
        FirstRunState(
            step: .join, account: .nearAI, scopes: ["debugging_evaluation"], sharing: sharing,
            daemonStarted: true, enrolledInvite: "invite")
    }

    /// The core's answer after both disclosures, the witness shown being
    /// `0xwitness`.
    private func readyRequest(connected: Bool) throws -> Flow1GrantRequest {
        var flow = SharingDisclosureFlow()
        flow.acknowledgeScrub()
        flow.acknowledgeWitness(shown: "0xwitness")
        return try XCTUnwrap(SharingDisclosureFlow.grantRequest(
            flow.progress(connected: connected, scopes: ["debugging_evaluation"])))
    }

    /// Start on Automatic goes through the disclosures; Ask, and
    /// watching only whatever `sharing` holds, commit directly.
    func test_startSendsAutomaticThroughTheDisclosures() throws {
        XCTAssertEqual(UsesScreenLayout.startRoute(onUses(.automatic)), .disclose)
        XCTAssertEqual(UsesScreenLayout.startRoute(onUses(.askMe)), .commit)
        var watching = onUses(.automatic)
        watching.account = .watchOnly
        XCTAssertEqual(UsesScreenLayout.startRoute(watching), .commit)

        // Join's Start sharing reads that route.
        let source = try Self.joinSource()
        XCTAssertTrue(source.contains("switch UsesScreenLayout.startRoute(runner.state)"))
        XCTAssertTrue(source.contains("UsesStart.finish(runner: runner"))
    }

    /// A ready answer after both disclosures reaches the daemon as the grant,
    /// with the witness shown, after the scopes are saved.
    @MainActor
    func test_aReadyAnswerAfterTheDisclosuresSendsTheGrant() async throws {
        let daemon = StartDaemon()
        let runner = FirstRunRunner(state: onUses(.automatic), daemon: daemon)
        let pending = await UsesStart.finish(
            runner: runner, request: try readyRequest(connected: true), pending: nil)
        XCTAssertEqual(daemon.log, [
            .setConsentScopes(["debugging_evaluation"]), .grantAutomatic(witness: "0xwitness"), .markComplete,
        ])
        XCTAssertEqual(runner.state.sharing, .automatic)
        XCTAssertNil(runner.failure)
        XCTAssertNil(pending)
    }

    /// Review Focus 3 when the daemon refuses the grant and the completion
    /// marker is then not written: `completeFailed` is shown first, and the
    /// daemon's refusal is kept for the retry that finishes on Ask, so
    /// setup never ends without saying Automatic was not turned on.
    @MainActor
    func test_aDaemonRefusalOutlivesAnUnfinishedStart() async throws {
        let daemon = RecordingFirstRunDaemon()
        daemon.grant = .refused(label: "automatic-grant-witness-changed")
        daemon.failing = { $0 == .markComplete }
        let runner = FirstRunRunner(state: onUses(.automatic), daemon: daemon)

        let pending = await UsesStart.finish(
            runner: runner, request: try readyRequest(connected: true), pending: nil)
        XCTAssertEqual(runner.failure, .completeFailed)
        XCTAssertEqual(runner.state.sharing, .askMe)
        XCTAssertEqual(pending, .grantRefused(label: "automatic-grant-witness-changed"))

        daemon.failing = { _ in false }
        let after = await UsesStart.plainStart(runner: runner, pending: pending)
        XCTAssertEqual(daemon.log.last, .markComplete)
        XCTAssertEqual(runner.failure, .grantRefused(label: "automatic-grant-witness-changed"))
        XCTAssertNil(after)
    }

    /// Review Focus 3 in the live app: Start writes the marker, which ends
    /// onboarding and takes the first-run host (and this screen, and the
    /// runner) off screen at the next render. So the refusal is handed to
    /// the app model in the same step as the marker, before Start returns,
    /// and the main window's notices show it after setup. Both refusals
    /// count: the core's (decided at the disclosures and carried into
    /// Start) and the daemon's (refused during Start).
    @MainActor
    func test_aRefusedGrantIsShownAfterSetupFinishes() async throws {
        let uses = try copy().uses
        let finished: (FirstRunFailure?) -> String? = { UsesScreenLayout.finishedNotice($0, uses: uses) }

        let model = AppModel()
        let core = StartDaemon(forwardingTo: model)
        let carried = FirstRunRunner(state: onUses(.automatic), daemon: core, finishedNotice: finished)
        _ = await UsesStart.finish(runner: carried, request: try readyRequest(connected: false), pending: nil)
        XCTAssertTrue(carried.completed)
        XCTAssertEqual(core.log.last, .markComplete)
        XCTAssertEqual(core.finished, [uses.sharingRefused], "handed over as the marker is written")
        XCTAssertEqual(model.firstRunNotice, uses.sharingRefused)

        let refusing = RecordingFirstRunDaemon()
        refusing.grant = .refused(label: "automatic-grant-witness-changed")
        let daemon = FirstRunRunner(state: onUses(.automatic), daemon: refusing, finishedNotice: finished)
        _ = await UsesStart.finish(runner: daemon, request: try readyRequest(connected: true), pending: nil)
        XCTAssertTrue(daemon.completed)
        XCTAssertEqual(refusing.finished, [uses.sharingRefused])

        // A Start with nothing refused hands over no notice.
        let granted = StartDaemon()
        let plain = FirstRunRunner(state: onUses(.automatic), daemon: granted, finishedNotice: finished)
        _ = await UsesStart.finish(runner: plain, request: try readyRequest(connected: true), pending: nil)
        XCTAssertEqual(granted.finished, [nil])

        // The first-run host gives its runner that mapping, and the notice
        // is drawn by the shell notices, above every section, outside the
        // first-run branch.
        let coordinator = try Self.appSource("Views/OnboardingCoordinatorView.swift")
        XCTAssertTrue(coordinator.contains("UsesScreenLayout.finishedNotice("))
        let notices = try Self.appSource("Views/ShellNotices.swift")
        let shellNotices = try XCTUnwrap(notices.range(of: "struct ShellNotices"))
        let body = notices[shellNotices.lowerBound...]
        XCTAssertTrue(body.contains("if let notice = model.firstRunNotice"))
        XCTAssertTrue(body.contains("model.firstRunNotice = nil"))
        // The Monitor, which the first-run window hands off to, draws it
        // as a popover notice over every tab (owner, 2026-10-19).
        let window = try Self.appSource("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("MonitorNotices("))
        let monitor = try Self.appSource("Views/Monitor/MonitorNotices.swift")
        XCTAssertTrue(monitor.contains("if let notice = model.firstRunNotice"))
        XCTAssertTrue(monitor.contains("onClose: { model.firstRunNotice = nil }"))
    }

    private static func appSource(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp")
            .appendingPathComponent(path)
        return try String(contentsOf: url, encoding: .utf8)
    }

    /// Start that skips the disclosures on Automatic grants nothing: without
    /// the core's ready answer there is no grant to send.
    @MainActor
    func test_aStartThatSkipsTheDisclosuresGrantsNothing() async {
        let daemon = StartDaemon()
        let runner = FirstRunRunner(state: onUses(.automatic), daemon: daemon)
        _ = await UsesStart.plainStart(runner: runner, pending: nil)
        XCTAssertEqual(daemon.log, [.setConsentScopes(["debugging_evaluation"]), .markComplete])
    }
}

/// Records Start's calls; every call succeeds and the grant is granted.
@MainActor
private final class StartDaemon: FirstRunDaemon {
    var log: [FirstRunCall] = []
    /// What each finished Start handed over, in order.
    var finished: [String?] = []
    /// The app model a finished Start's notice is passed on to, if any.
    private let model: AppModel?

    init(forwardingTo model: AppModel? = nil) {
        self.model = model
    }

    func firstRunFinished(notice: String?) {
        finished.append(notice)
        model?.firstRunFinished(notice: notice)
    }

    func startDaemon(settingsJSON: String) async -> Bool { log.append(.startDaemon(settingsJSON: settingsJSON)); return true }
    func setSourceSettings(settingsJSON: String) async -> Bool {
        log.append(.setSourceSettings(settingsJSON: settingsJSON))
        return true
    }
    func lookupInvite(_ invite: String) async -> FirstRunLookup {
        log.append(.lookupInvite(invite))
        return .refused(label: "unused")
    }
    func enrollInvite(_ invite: String) async -> Bool { log.append(.enroll(invite)); return true }
    func signInNearAI() async -> Bool { log.append(.signInNearAI); return true }
    func nearAILogin() async -> Bool { log.append(.nearAILogin); return true }
    func cancelNearAILogin() async -> Bool { true }
    func enrollNearAI() async -> FirstRunNearAIEnrolment { log.append(.enrollNearAI); return .enrolled }
    func setAskEachTime() async -> Bool { log.append(.setAskEachTime); return true }
    func saveConsentScopes(_ scopes: [String]) async -> Bool { log.append(.setConsentScopes(scopes)); return true }
    func setProjectMode(projectID: String, mode: ProjectMode) async -> Bool {
        log.append(.setProjectMode(projectID: projectID, mode))
        return true
    }
    func includePastSessions(projectID: String, sessionIDs: [String]) async -> Bool {
        log.append(.includePastSessions(projectID: projectID, sessionIDs))
        return true
    }
    func setPrivateAI(_ on: Bool) async -> Bool { log.append(.setPrivateAI(on)); return true }
    func grantAutomatic(witness: String?) async -> FirstRunGrantAnswer {
        log.append(.grantAutomatic(witness: witness))
        return .granted
    }
    func markComplete() async -> Bool { log.append(.markComplete); return true }
    func markWatchOnlyComplete() async -> Bool { log.append(.markWatchOnlyComplete); return true }
    func unenroll() async -> Bool { false }
}
