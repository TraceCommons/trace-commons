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
        ConsentScope(name: "debugging_evaluation", title: "debugging_evaluation", description: "d", alwaysOn: true, grantsDataUse: true),
        ConsentScope(name: "benchmark_only", title: "benchmark_only", description: "b", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "ranking_training", title: "ranking_training", description: "r", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "model_training", title: "model_training", description: "m", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "public_attribution", title: "public_attribution", description: "p", alwaysOn: false, grantsDataUse: false),
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

        // The row carries Ron's inline "required", and nothing in the screen
        // ticks a scope except the person's own toggle.
        let source = try Self.source()
        XCTAssertTrue(source.contains("Text(copy.uses.required)"))
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

    /// Ron's review of #1235, item 2: "List my handle publicly as a
    /// contributor" is its own row after the optional group, shown whether
    /// or not the group is open.
    func test_theHandleRowIsAlwaysShownAfterTheOptionalGroup() throws {
        XCTAssertEqual(
            UsesScreenLayout.expandedScopes(options).map(\.name),
            ["benchmark_only", "ranking_training", "model_training"])
        XCTAssertEqual(
            UsesScreenLayout.visibleScopes(options, optionalOpen: false).map(\.name),
            ["debugging_evaluation", "public_attribution"])
        XCTAssertEqual(
            UsesScreenLayout.visibleScopes(options, optionalOpen: true).map(\.name),
            ["debugging_evaluation", "benchmark_only", "ranking_training", "model_training", "public_attribution"])
        let source = try Self.source()
        XCTAssertTrue(source.contains("UsesScreenLayout.expandedScopes(options)"))
        XCTAssertFalse(source.contains("optional + UsesScreenLayout.handleScopes(options)"))
    }

    func test_startIsDisabledUntilTheRequiredUseIsTicked() throws {
        let grant = try grant()
        let uses = try copy().uses
        var state = FirstRunState(tier: .quick, step: .uses, account: .nearAI, enrolledInvite: "INVITE-1")
        let required = UsesScreenLayout.requiredScope(options)
        XCTAssertFalse(UsesScreenLayout.canStart(state, uses: uses, requiredScope: required, grant: grant, isCommitting: false))

        // Every optional use ticked is still not the required one.
        state.scopes = Set(UsesScreenLayout.optionalScopes(options).map(\.name))
        XCTAssertFalse(UsesScreenLayout.canStart(state, uses: uses, requiredScope: required, grant: grant, isCommitting: false))

        state.scopes.insert("debugging_evaluation")
        XCTAssertTrue(UsesScreenLayout.canStart(state, uses: uses, requiredScope: required, grant: grant, isCommitting: false))

        // Without the sharing words, or while a Start is running, it stays
        // disabled; with no required use known, it never enables.
        XCTAssertFalse(UsesScreenLayout.canStart(state, uses: uses, requiredScope: required, grant: nil, isCommitting: false))
        XCTAssertFalse(UsesScreenLayout.canStart(state, uses: uses, requiredScope: required, grant: grant, isCommitting: true))
        XCTAssertFalse(UsesScreenLayout.canStart(state, uses: uses, requiredScope: nil, grant: grant, isCommitting: false))

        // The footer note is Ron's, until the box is ticked.
        XCTAssertNil(UsesScreenLayout.footerNote(uses, state: state, requiredScope: required))
        state.scopes.remove("debugging_evaluation")
        XCTAssertEqual(UsesScreenLayout.footerNote(uses, state: state, requiredScope: required), uses.baseUseNote)
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
            tier: .quick, step: .uses, account: .nearAI, scopes: ["debugging_evaluation"], enrolledInvite: "INVITE-1")

        let noAskFirst = try grant(without: "path_ask_first")
        XCTAssertEqual(
            UsesScreenLayout.sharingLine(uses, path: .askMe, grant: noAskFirst), uses.sharingUnavailable)
        XCTAssertFalse(UsesScreenLayout.canStart(
            state, uses: uses, requiredScope: required, grant: noAskFirst, isCommitting: false))

        let noAutomatic = try grant(without: "path_automatic")
        XCTAssertTrue(UsesScreenLayout.canStart(
            state, uses: uses, requiredScope: required, grant: noAutomatic, isCommitting: false))
        state.sharing = .automatic
        XCTAssertFalse(UsesScreenLayout.canStart(
            state, uses: uses, requiredScope: required, grant: noAutomatic, isCommitting: false))
        // Watching only reads Ask me's line, so Automatic's absence does
        // not hold it. (Watching only with nothing enrolled: beside an
        // enrolment the daemon holds, Start is not offered at all.)
        state.account = .watchOnly
        state.enrolledInvite = nil
        XCTAssertTrue(UsesScreenLayout.canStart(
            state, uses: uses, requiredScope: required, grant: noAutomatic, isCommitting: false))
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
        let joined = FirstRunState(tier: .quick, step: .uses, account: .nearAI, enrolledInvite: "INVITE-1")
        let full = UsesScreenLayout.sharingOptions(for: joined, modes: modes)
        // Ron's #1030 order: Automatic first, then Ask me.
        XCTAssertEqual(full.map(\.value), [.automatic, .askMe])
        XCTAssertEqual(full.map(\.title), [modes.label(for: .autoUpload), modes.label(for: .ask)])
        XCTAssertEqual(UsesScreenLayout.sharingOptions(for: FirstRunState(account: .watchOnly), modes: modes).map(\.value), [.askMe])
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
    @MainActor
    func test_aRefusedGrantFinishesOnAskMe() async throws {
        let uses = try copy().uses
        let privateAI = try privateAI()

        // Driven through Start's own path: both disclosures seen, the core
        // refuses (not connected), Start runs on Ask me, grants nothing,
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
        // Every call went through but the marker was not written: Start
        // says setup has not finished rather than doing nothing.
        XCTAssertEqual(UsesScreenLayout.notice(for: .completeFailed, uses: uses, privateAI: privateAI), uses.completeFailed)
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
        XCTAssertEqual(source?.components(separatedBy: "await runner.commit(.start, carrying: refusal)").count, 2)
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
        let joined = FirstRunState(
            tier: .quick, step: .uses, account: .nearAI, sharing: .automatic, enrolledInvite: "INVITE-1")
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
    // MARK: - Start's routing

    private func onUses(_ sharing: SharingPath) -> FirstRunState {
        FirstRunState(
            tier: .quick, step: .uses, account: .nearAI, scopes: ["debugging_evaluation"], sharing: sharing,
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

    /// Start on Automatic goes through the disclosures; Ask me, and
    /// watching only whatever `sharing` holds, commit directly.
    func test_startSendsAutomaticThroughTheDisclosures() throws {
        XCTAssertEqual(UsesScreenLayout.startRoute(onUses(.automatic)), .disclose)
        XCTAssertEqual(UsesScreenLayout.startRoute(onUses(.askMe)), .commit)
        var watching = onUses(.automatic)
        watching.account = .watchOnly
        XCTAssertEqual(UsesScreenLayout.startRoute(watching), .commit)

        // The screen's Start reads that route.
        let source = try Self.source()
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
    /// daemon's refusal is kept for the retry that finishes on Ask me, so
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
        // The Monitor, which the first-run window hands off to, draws them.
        let window = try Self.appSource("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("ShellNotices()"))
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
}
