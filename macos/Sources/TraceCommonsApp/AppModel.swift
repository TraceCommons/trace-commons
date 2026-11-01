import Combine
import CryptoKit
import Foundation
import SwiftUI
import TCBridge
import TCShellCore

/// Everything the UI reads, and the only thing that talks to the daemon.
///
/// `@MainActor` throughout: `tc_subscribe` callbacks arrive on a Rust
/// background thread, and `handle(event:)` is the single place that hops
/// back before touching any published property.
@MainActor
final class AppModel: ObservableObject {
    @Published var managedSnapshot: ManagedSnapshot? {
        didSet { if let copy = managedSnapshot?.copy { managedCopy = copy } }
    }
    @Published var managedCLIOptionsDrafts = ManagedCLIOptionsDrafts()
    @Published var managedBusy = false
    /// The copy key of the last failed managed action (`ManagedSurface.errorKey`),
    /// never the daemon's code.
    @Published var managedErrorKey: String?
    @Published var managedErrorTool: String?
    /// The request that failed, so its failure is said where it was asked:
    /// a launch on the NEAR AI card, the rest on the sessions card.
    @Published var managedErrorMethod: String?
    /// The last copy table the daemon sent, kept when a later read fails so
    /// the connecting and failure states still have their words. Before the
    /// first read it is the core's own table, so the connecting card is
    /// never drawn without them (owner, 2026-10-22: it showed a bare icon).
    private var managedCopy: [String: String] = AppModel.bundledManagedCopy

    /// The core's managed-sessions table (`tc_managed_copy_json`), read
    /// without the daemon; empty if it could not be read.
    static let bundledManagedCopy: [String: String] = {
        guard let json = TCDiscovery.managedCopyJSON(),
              let table = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
        else { return [:] }
        return table.compactMapValues { $0 as? String }
    }()

    func managedText(_ key: String) -> String { managedCopy[key] ?? Self.bundledManagedCopy[key] ?? "" }

    func refreshManagedSessions() {
        perform("managed_snapshot", work: { try $0.managedSnapshot() }) { snapshot in
            if snapshot.revision >= (self.managedSnapshot?.revision ?? 0), self.managedSnapshot != snapshot { self.managedSnapshot = snapshot }
        }
    }

    /// NEAR AI's chat models for `tool`'s Launch sheet picker; nil when
    /// they could not be read, which leaves only the default to choose.
    /// No error is shown: launching on the default needs no catalogue.
    /// With `cacheOnly` the daemon answers from its cache and never reads
    /// NEAR AI: for a session row's display name.
    func managedModelCatalogue(tool: String, cacheOnly: Bool = false, done: @escaping @MainActor (ManagedModelCatalogue?) -> Void) {
        guard let client else { return done(nil) }
        Task.detached(priority: .userInitiated) {
            let catalogue = try? client.managedModelCatalogue(tool: tool, cacheOnly: cacheOnly)
            await MainActor.run { done(catalogue) }
        }
    }

    /// Read or reset one folder/agent profile without choosing an account.
    /// A nil `cwd` is the home folder, which the daemon resolves.
    func managedFolderSettings(tool: String, cwd: String?, reset: Bool = false,
                               done: @escaping @MainActor (ManagedFolderSettings?) -> Void) {
        guard let client else { return done(nil) }
        var params: [String: Any] = ["tool": tool]
        if let cwd { params["cwd"] = cwd }
        let sent = params
        Task.detached(priority: .userInitiated) {
            let settings = try? { () throws -> ManagedFolderSettings in
                let response = try client.managedAction(
                    reset ? "managed_folder_settings_reset" : "managed_folder_settings_get",
                    params: sent)
                return try JSONDecoder().decode(ManagedFolderSettings.self,
                    from: JSONSerialization.data(withJSONObject: response))
            }()
            await MainActor.run { done(settings) }
        }
    }

    /// Star or unstar one model for the Launch sheet's picker. `done` gets
    /// the daemon's stored list, or nil when it refused (a model NEAR AI no
    /// longer lists, or NEAR AI could not be read).
    func setFavoriteModel(_ model: String, favorite: Bool,
                          done: @escaping @MainActor (_ favorites: [String]?, _ errorLabel: String?) -> Void) {
        guard let client else { return done(nil, nil) }
        let params = ManagedSurface.favoriteParams(model: model, favorite: favorite)
        Task.detached(priority: .userInitiated) {
            let (favorites, label): ([String]?, String?) = {
                do {
                    let answer = try client.managedAction("managed_model_favorite_set", params: params)
                    return (answer["favorites"] as? [String] ?? [], nil)
                } catch let failure as DaemonClient.Failure {
                    return (nil, failure.message)
                } catch {
                    return (nil, nil)
                }
            }()
            await MainActor.run {
                // Every window's settings, the Settings card's list among
                // them, follow the daemon's `status_changed`, which it
                // publishes when the stored list moved.
                done(favorites, label)
            }
        }
    }

    /// The model `options`, a session's CLI options for `tool`, name with
    /// the tool's own model flag, as the core reads them
    /// (`managed_cli_model`); none when they name none or the read fails.
    func cliModel(tool: String, options: String, done: @escaping @MainActor (_ model: String?) -> Void) {
        guard let client else { return done(nil) }
        let params: [String: Any] = ["tool": tool, "cli_options": options]
        Task.detached(priority: .userInitiated) {
            let model = (try? client.managedAction("managed_cli_model", params: params))?["model"] as? String
            await MainActor.run { done(model) }
        }
    }

    /// The terminal app managed sessions open in, by the id the daemon
    /// offered in `capabilities.terminal_choices`. The daemon refuses any
    /// other value; the snapshot is re-read after the write.
    func setManagedTerminal(_ id: String) {
        managedAction("set_managed_terminal", params: ["terminal": id])
    }

    /// Only a named tool and setup action cross the RPC boundary. Setup needs
    /// no sign-in, managed account, profile, session, or working directory.
    func setupManagedTool(_ tool: String, action: String) {
        guard managedSnapshot?.capabilities.toolSetup == true,
              ManagedSurface.supportedTools.contains(tool), ["install", "update"].contains(action) else { return }
        managedAction("managed_tool_setup", params: ["tool": tool, "action": action])
    }

    func saveManagedCLIOptions(tool: String) {
        guard managedCLIOptionsDrafts.isChanged(tool: tool, saved: managedSnapshot?.cliOptions) else { return }
        managedAction("managed_cli_options_set", params: [
            "tool": tool,
            "options": managedCLIOptionsDrafts.value(tool: tool, saved: managedSnapshot?.cliOptions)
        ])
    }

    /// `terminal`, when set, is saved as the terminal choice before the
    /// request, in the same task, so a launch dialog that asked for one
    /// opens in it.
    func managedAction(_ method: String, params: [String: Any], openTerminal: Bool = false, newAccountKey: String? = nil,
                       terminal: String? = nil) {
        guard let client, !managedBusy else { return }
        managedBusy = true
        managedErrorKey = nil
        managedErrorTool = nil
        managedErrorMethod = nil
        let optionsBeingSaved = method == "managed_cli_options_set" ? params["options"] as? String : nil
        let attemptedTool = params["tool"] as? String
            ?? managedSnapshot?.accounts.first(where: { $0.id == (params["account_id"] as? String) })?.tool
        // Snapshot the untyped parameters before crossing to the worker.
        let encodedParams = Result {
            guard JSONSerialization.isValidJSONObject(params) else { throw CocoaError(.coderInvalidValue) }
            return try JSONSerialization.data(withJSONObject: params)
        }
        Task.detached(priority: .userInitiated) {
            let result = Result {
                guard let params = try JSONSerialization.jsonObject(with: encodedParams.get()) as? [String: Any] else {
                    throw CocoaError(.coderInvalidValue)
                }
                if let terminal {
                    _ = try client.managedAction("set_managed_terminal", params: ["terminal": terminal])
                }
                var response = try client.managedAction(method, params: params)
                if method == "managed_tool_setup", response["opened"] as? Bool != true {
                    throw DaemonClient.Failure(code: "unavailable", message: "managed-tool-setup-unconfirmed")
                }
                if let optionsBeingSaved,
                   response["tool"] as? String != attemptedTool || response["options"] as? String != optionsBeingSaved || response["revision"] as? UInt64 == nil {
                    throw DaemonClient.Failure(code: "unavailable", message: "managed-cli-options-unconfirmed")
                }
                if method == "managed_account_add", let accountID = response["id"] as? String {
                    if let key = newAccountKey {
                        response = try client.managedAction("managed_account_set_key", params: ["account_id": accountID, "key": key])
                    } else {
                        response = try client.managedAction("managed_account_reconnect", params: ["account_id": accountID])
                    }
                }
                if openTerminal {
                    guard let session = response["session_id"] as? String,
                          let ticket = response["ticket"] as? String else {
                        throw DaemonClient.Failure(code: "launch-unknown", message: "managed-launch-unknown")
                    }
                    _ = try client.managedAction("managed_terminal_launch", params: ["session_id": session, "ticket": ticket])
                }
                return response["revision"] as? UInt64
            }
            await MainActor.run {
                self.managedBusy = false
                if case .success(let revision) = result, let revision, let optionsBeingSaved, let tool = attemptedTool {
                    if revision >= (self.managedSnapshot?.revision ?? 0) {
                        self.managedSnapshot?.cliOptions?[tool] = optionsBeingSaved
                        self.managedSnapshot?.revision = revision
                    }
                    self.managedCLIOptionsDrafts.saved(tool: tool, options: optionsBeingSaved)
                }
                if case .failure(let error) = result {
                    let failure = error as? DaemonClient.Failure
                    self.managedErrorKey = ManagedSurface.errorKey(code: failure?.code, message: failure?.message)
                    self.managedErrorTool = attemptedTool
                    self.managedErrorMethod = method
                }
                self.refreshManagedSessions()
            }
        }
    }

    enum Startup: Equatable {
        case starting
        /// The daemon is running in-process.
        case running
        /// Refused to start, with a sentence a person can act on.
        case refused(String)
        /// Refused because nobody has said which session folders to watch.
        ///
        /// Separate from `.refused` because it is the one refusal with a way
        /// out: the roots screen collects two folders and starts the daemon
        /// with them. Before this case existed the refusal rendered as a
        /// static notice and every screen that could clear it lived behind
        /// the daemon it was blocking, so a fresh install could never
        /// finish onboarding.
        case needsRoots
    }

    /// The recovery hold that follows an approval.
    ///
    /// It deliberately carries no countdown. The real deadline is the
    /// daemon's next upload sweep -- `drain_approved` claims everything in
    /// `Approved` on a poll tick -- and this process cannot see when that
    /// tick lands: the socket's `status` and `get_settings` views expose the
    /// digest interval and the queue TTL but not the poll interval, and
    /// `list_pending` returns only `Pending` entries, so an approved entry
    /// disappears from everything the app can observe the moment it is
    /// approved.
    ///
    /// The old five-second counter was a number this app made up. Counting
    /// down to zero and vanishing told a contributor the window had closed
    /// when it usually had not, and told them nothing at all about the case
    /// that actually matters -- the sweep that fires one second after they
    /// clicked. So this counts UP, from a time that is real, and the
    /// affordance stays until the contributor puts it away or until the
    /// daemon refuses the cancel (`undoApproval` says so plainly when it
    /// does).
    ///
    /// One-click submit widened this from a single entry to a set: the row
    /// action approves one id, the project action can approve many, and
    /// `cancel` has no bulk form -- `undoApproval` below drives it once per
    /// id in `entryIDs`. `toastLine` and `offerUndo` are `SubmitToast`'s,
    /// carried here rather than re-derived: this is the one sentence the
    /// contributor sees for what just happened, whether or not Undo is
    /// offered alongside it (see `ApproveResponse.toast`).
    struct Undo: Equatable {
        let entryIDs: [String]
        let toastLine: String
        /// Whether the Undo control itself is shown. False for "Nothing
        /// approved" and fully-skipped responses -- the toast still needs to
        /// be seen, but there is nothing to undo. See `SubmitToast.offerUndo`.
        let offerUndo: Bool
        /// When the approval was made, on this machine's clock.
        let approvedAt: Date
        /// Seconds since `approvedAt`, ticked for display. Stops advancing
        /// after `Undo.tickCeiling`; the affordance does not.
        var heldSeconds: Int

        /// The display counter stops here. Past a couple of minutes the exact
        /// figure has stopped meaning anything, and a ticker that runs for the
        /// life of the process to redraw a number nobody is reading is waste.
        static let tickCeiling = 120
    }

    @Published private(set) var contributionLine = PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? "")?.accountContributionRefresh ?? ""
    @Published private(set) var contributionBusy = false
    private var inviteAttempt: (code: String, key: String)?
    func updateContributionAccount(inviteCode: String? = nil) async -> Bool {
        guard let client, let scope = status.accountScope, !contributionBusy else { return false }
        contributionBusy = true
        contributionLine = privateInferenceCopy?.accountContributionChecking ?? ""
        if let inviteCode, inviteAttempt?.code != inviteCode {
            inviteAttempt = (inviteCode, UUID().uuidString)
        }
        let attempt = inviteAttempt
        let outcome = await Task.detached(priority: .userInitiated) {
            Result {
                if let inviteCode, let attempt {
                    return try client.redeemInvite(code: inviteCode, idempotencyKey: attempt.key, scope: scope)
                }
                return try client.contributionStatus(scope: scope)
            }
        }.value
        contributionBusy = false
        guard self.client === client, self.status.accountScope == scope, !Task.isCancelled else { return false }
        switch outcome {
        case .success(let account):
            guard account.accountScope == scope else { return false }
            contributionLine = account.line
            if inviteCode != nil { inviteAttempt = nil }
            return true
        case .failure:
            contributionLine = privateInferenceCopy?.accountContributionUnavailable ?? ""
            return false
        }
    }

    @Published private(set) var startup: Startup = .starting
    @Published private(set) var isStartingDaemon = false
    private let daemonStartup: DaemonStartup
    /// The opt-in events to declare now (`Notifier.acceptedEvents`).
    /// Replaced only by tests.
    private let reengageAccepts: () async -> [String]

    init(
        daemonStartup: DaemonStartup? = nil,
        reengageAccepts: @escaping () async -> [String] = { await Notifier.shared.acceptedEvents() }
    ) {
        self.daemonStartup = daemonStartup ?? DaemonStartup()
        self.reengageAccepts = reengageAccepts
        // `@Published` emits in `willSet`, so the emitted value is read,
        // not `PendingInvite.shared.value`. Delivered on the main actor:
        // `PendingInvite` is main-actor isolated.
        inviteLinks = PendingInvite.shared.$value.sink { [weak self] invite in
            guard let invite else { return }
            MainActor.assumeIsolated { self?.inviteLinkArrived(invite) }
        }
        sentTracesTraining.client = { [weak self] in self?.client }
    }
    private var inviteLinks: AnyCancellable?
    /// Sent traces without Train coding models: their count, the one-time
    /// sheet a turn-on raises, and the marks a grant leaves.
    let sentTracesTraining = SentTracesTrainingModel()
    /// An invite link that arrived before the config directory was known,
    /// so the watch-only marker it takes back is cleared once it is.
    private var inviteAwaitingConfigDirectory: String?
    @Published private(set) var status: DaemonStatus = .unknown {
        didSet {
            if status.accountScope != oldValue.accountScope {
                contributionLine = privateInferenceCopy?.accountContributionRefresh ?? ""
                inviteAttempt = nil
            }
            // Worded across the ABI once per notice the daemon sends, not on
            // every re-render of the card.
            if status.legacyInviteMigration != oldValue.legacyInviteMigration {
                legacyMigrationNotice = status.legacyInviteMigration.noticeJSON
                    .flatMap(legacyMigrationWording)
                    .flatMap(LegacyMigrationNotice.decode(fromJSON:))
            }
        }
    }
    /// The notice after a legacy invite identity moved to a NEAR AI account,
    /// in the Rust's words, or nil when there is none or it cannot be read.
    @Published private(set) var legacyMigrationNotice: LegacyMigrationNotice?
    /// Words `status.legacy_invite_migration.notice`. The ABI in the app;
    /// replaced only by tests.
    var legacyMigrationWording: (String) -> String? = TCConsentCopy.legacyMigrationNoticeJSON(
        forNotice:)
    @Published private(set) var pending: [QueueEntry] = [] {
        didSet { recomputeWaiting() }
    }
    @Published private(set) var summaries: [String: PreviewSummary] = [:]
    @Published private(set) var summaryErrors: [String: String] = [:]
    /// A session the daemon's preview scheduler refused to parse for being
    /// over the admission cap. Carries only what `PreviewTooLarge` carries
    /// -- a raw stat and the cap -- never a would-send estimate.
    @Published private(set) var tooLarge: [String: PreviewTooLarge] = [:]
    @Published private(set) var history: [HistoryRecord] = []
    /// Whether the last `status` read failed. Until a status answers, a
    /// failed read is the gates' answer: `LaunchRouting.onboardingKnown`
    /// takes it as known, and the unanswered status requires onboarding, so
    /// every write surface stays closed (fail closed). The launch does not
    /// open first run on it (`LaunchRouting.launchOpening`): the next status
    /// event re-reads, and an onboarded install is not sent to Welcome by a
    /// transient failure.
    @Published private(set) var statusReadFailed = false
    /// Whether the daemon has answered `list_pending` (or sent a snapshot)
    /// and `list_history`. Until then `pending` and `history` are
    /// placeholders, and an empty one is not "none"; a failed read leaves
    /// them false.
    @Published private(set) var queueAnswered = false
    @Published private(set) var historyAnswered = false
    @Published private(set) var rollup: HistoryRollup?
    @Published private(set) var projects: [ProjectRow] = []
    /// The one project the daemon suggests arming, or nil. Refreshed
    /// alongside `projects`, because every reason the project list changes
    /// is also a reason this answer might have.
    @Published private(set) var armingOffer: ArmingOffer?
    /// The standing Automatic grant, for Settings' sharing choice (#1753
    /// A2). Nil until `automatic_grant` has answered, which the choice draws
    /// as nothing picked. Refreshed alongside `projects`.
    @Published private(set) var automaticGrant: AutomaticGrant?
    @Published private(set) var consentScopes: [ConsentScope] = []
    /// Whether missions are shown anywhere (`missions_enabled`); off until
    /// the daemon says so, and off before its settings have arrived.
    var missionsShown: Bool { daemonSettings?.missionsShown ?? false }

    @Published private(set) var daemonSettings: DaemonSettingsView? {
        // A settings write can change what leaves this machine -- inference
        // evidence adds or removes the prompt-and-reply line, a filter change
        // moves the local route's -- so the disclosure moves with it.
        didSet { if daemonSettings != oldValue { refreshRouteDisclosure() } }
    }

    // MARK: - The local proxy

    /// The routing surface's fixed words, decoded once from the Rust.
    ///
    /// Nil only if the export or the decode failed, and the card renders
    /// nothing at all in that case. A screen with blanks beside tool names
    /// would be worse, and a screen with Swift-authored words worse still.
    @Published private(set) var routingCopy: RoutingCopy? = RoutingCopy.decode(
        fromJSON: TCRoutingCopy.copyJSON() ?? ""
    )
    /// What IronWire last answered about which tools point at it, or nil
    /// for nothing held. Nil is not a fault; it is the absence of evidence,
    /// and every tool reads as not known while it stands.
    @Published private(set) var routingEvidence: RoutingEvidence?
    /// The sentence the last probe produced, shown under the Apply button.
    @Published private(set) var routingProbeLine: String?
    /// A probe is in flight. Drives the button's own label, which is a
    /// shared word like every other on this card.
    @Published private(set) var routingChecking = false

    /// What a running IronWire published about itself, as far as this app
    /// has asked.
    ///
    /// Starts as nothing found rather than as nil, because that is the
    /// state of a machine nobody has asked about yet AND the state of a
    /// machine without IronWire, and the card says the same thing about
    /// both: here are the fields, say which port. It becomes a found port
    /// only when `discover_routing` says so.
    @Published private(set) var routingDiscovery = RoutingDiscovery.none

    /// Everything on this surface that is decided in the Rust: the sentences
    /// that interpolate, and the two branch tables that pick a word and a
    /// state line. This shell fills in no holes and owns no `switch`; see
    /// `TCRoutingCopy`.
    let routingCalls = RoutingCalls(
        tokenLine: { TCRoutingCopy.tokenLine(path: $0) },
        unreachableLine: { TCRoutingCopy.unreachableLine(port: $0) },
        discoveryLine: { TCRoutingCopy.discoveryLine(port: $0) },
        toolWord: { TCRoutingCopy.toolWord(sourceMode: $0, wiring: $1) },
        toolTone: { TCRoutingCopy.toolTone(sourceMode: $0, wiring: $1) },
        stateLine: { TCRoutingCopy.stateLine(state: $0) },
        stateTone: { TCRoutingCopy.stateTone(state: $0) }
    )
    // MARK: - Answering model calls on this computer

    /// The offer's and the settings card's fixed words, decoded once from
    /// the Rust.
    ///
    /// Nil only if the export or the decode failed, and both surfaces render
    /// nothing at all in that case. An offer with a missing sentence would be
    /// an offer with the exposure paragraph missing, which is worse than no
    /// offer.
    @Published private(set) var privateInferenceCopy: PrivateInferenceCopy? =
        PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? "")

    /// The two branch tables and the interpolated sentence, all decided in
    /// the Rust. This shell owns no `switch` on this surface.
    let privateInferenceCalls = PrivateInferenceCalls(
        stateLine: { TCPrivateInference.stateLine(state: $0) },
        stateTone: { TCPrivateInference.stateTone(state: $0) },
        servingLine: { TCPrivateInference.servingLine(port: $0) },
        shouldOffer: { TCPrivateInference.shouldOffer(answered: $0, on: $1) },
        quitNeedsNotice: { TCPrivateInference.quitNeedsNotice(on: $0, state: $1) },
        menuWord: { TCPrivateInference.menuWord(state: $0) },
        menuTone: { TCPrivateInference.menuTone(state: $0) }
    )

    /// What the listener is doing, from the daemon's own report.
    ///
    /// Never nil: a daemon that has never heard of the field reads as the
    /// empty label, which the shared table answers as unreported.
    var privateInferenceState: PrivateInferenceState {
        daemonSettings?.privateInferenceState?.surfaceState
            ?? PrivateInferenceState(label: "", port: nil)
    }

    var privateInferenceQuitDetail: String? {
        PrivateInferenceSurface.quitDetail(on: daemonSettings?.privateInferenceOn ?? false,
            state: privateInferenceState, copy: privateInferenceCopy, calls: privateInferenceCalls)
    }

    /// Whether to put the offer in front of the contributor right now.
    ///
    /// Asked of the shared table on every settings read rather than latched
    /// at launch: the answer lives in the daemon, so a contributor who
    /// answered in another window has answered.
    var showsPrivateInferenceOffer: Bool {
        guard let settings = daemonSettings, privateInferenceCopy != nil else { return false }
        return PrivateInferenceSurface.shouldOffer(
            answered: settings.privateInferenceAnswered,
            on: settings.privateInferenceOn,
            calls: privateInferenceCalls
        )
    }

    @Published private(set) var privateInferenceBusy = false

    /// Both answers are written to the daemon and remembered across shells.
    func answerPrivateInferenceOffer(accepted: Bool) {
        submitPrivateInference { try $0.answerPrivateInferenceOffer(accepted: accepted) }
    }

    /// Keep the last confirmed switch while one write is in flight.
    func applyPrivateInference(_ on: Bool) {
        submitPrivateInference { try $0.setPrivateInference(on) }
    }

    private func submitPrivateInference(_ work: @escaping @Sendable (DaemonClient) throws -> DaemonSettingsView) {
        guard !privateInferenceBusy else {
            objectWillChange.send()
            return
        }
        guard let client else {
            lastActionError = privateInferenceCopy?.writeUnconfirmed
            objectWillChange.send()
            return
        }
        privateInferenceBusy = true
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try work(client) }
            await MainActor.run {
                self.privateInferenceBusy = false
                switch outcome {
                case .success(let settings):
                    if self.lastActionError == self.privateInferenceCopy?.writeUnconfirmed { self.lastActionError = nil }
                    self.publishIfChanged(\.daemonSettings, settings)
                case .failure:
                    self.lastActionError = self.privateInferenceCopy?.writeUnconfirmed
                }
                // Repaint the derived binding even when a failed write left
                // its confirmed value unchanged.
                self.objectWillChange.send()
            }
        }
    }

    // MARK: - The tools on this computer

    /// The branch tables and the sentences the harness list turns on, all
    /// decided in the Rust. This shell owns no `switch` over a state or an
    /// outcome, and picks none of the words either.
    let harnessCalls = HarnessCalls(
        stateCode: { TCHarness.stateCode(state: $0) },
        planOutcomeCode: { TCHarness.planOutcomeCode(outcome: $0) },
        actionAvailable: { TCHarness.actionAvailable(action: $0, installed: $1, connected: $2) },
        stateLine: { TCHarness.stateLine(state: $0) },
        lastCallLine: { TCHarness.lastCallLine(secondsAgo: $0) },
        outcomeLine: { TCHarness.outcomeLine(outcome: $0) },
        spendLine: { TCHarness.spendLine(micros: $0) }
    )

    /// The tools, as of the last read. `.none` is "nothing is known", which
    /// is what an unreadable payload and a daemon that has never heard of
    /// the call both honestly say.
    @Published private(set) var harnesses: HarnessList = .none

    /// One action at a time, and never a second while a write is in flight.
    @Published private(set) var harnessBusy = false

    /// The worked-out change, waiting to be shown. Nothing is written while
    /// this is nil, and nothing is written without it having been non-nil
    /// first: the preview is the only route to a commit.
    @Published private(set) var harnessPreview: HarnessPlan?

    /// One tool and one action, held while the exposure question is put.
    struct HarnessRequest: Equatable {
        let id: String
        let action: HarnessAction
    }

    /// The connect that is waiting on the exposure question, if any.
    @Published private(set) var harnessExposureRequest: HarnessRequest?

    /// Re-read, never cached. `connected` is a fact about a file this app
    /// does not own, and the state above it turns on a ledger that moves on
    /// its own.
    func refreshHarnesses() {
        perform("harness_list", work: { try $0.harnessList() }) {
            self.publishIfChanged(\.harnesses, $0)
        }
    }

    /// The contributor pressed the one button on a row.
    ///
    /// A connect made while nothing here answers model calls stops at the
    /// exposure question first: starting the listener opens it to everything
    /// on this machine, and that does not follow from connecting one tool.
    /// A disconnect never asks -- it only ever removes something.
    func beginHarnessAction(id: String, action: HarnessAction) {
        guard !harnessBusy, harnessPreview == nil, harnessExposureRequest == nil else { return }
        if action == .connect,
            HarnessSurface.connectNeedsExposure(
                listenerOn: daemonSettings?.privateInferenceOn ?? false)
        {
            harnessExposureRequest = HarnessRequest(id: id, action: action)
            return
        }
        planHarness(id: id, action: action)
    }

    /// The answer to that question.
    ///
    /// Declining writes the marker alone and connects nothing. Accepting
    /// turns the destination on and records the answer in one write, and
    /// only then works out the change -- which is still shown before it is
    /// made.
    func answerHarnessExposure(accepted: Bool) {
        guard let request = harnessExposureRequest, !harnessBusy else { return }
        guard accepted else {
            harnessExposureRequest = nil
            answerPrivateInferenceOffer(accepted: false)
            return
        }
        guard let client else {
            harnessExposureRequest = nil
            return
        }
        // The sheet stays up, busy, until the write answers: a scrim click
        // or Escape meanwhile cannot abandon it (#1280 review).
        harnessBusy = true
        Task.detached(priority: .userInitiated) {
            let outcome = Result { () -> (DaemonSettingsView, HarnessPlan?) in
                let settings = try client.setPrivateInference(true)
                return (settings, try client.harnessPlan(id: request.id, action: request.action))
            }
            await MainActor.run {
                self.harnessBusy = false
                self.harnessExposureRequest = nil
                switch outcome {
                case .success(let (settings, plan)):
                    self.publishIfChanged(\.daemonSettings, settings)
                    self.harnessPreview = plan
                case .failure:
                    self.reportHarnessFailure()
                }
                self.refreshHarnesses()
            }
        }
    }

    /// Works the change out and shows it. Writes nothing.
    private func planHarness(id: String, action: HarnessAction) {
        guard let client else {
            reportHarnessFailure()
            return
        }
        harnessBusy = true
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try client.harnessPlan(id: id, action: action) }
            await MainActor.run {
                self.harnessBusy = false
                switch outcome {
                case .success(let plan):
                    self.harnessPreview = plan
                case .failure(let error):
                    // A connect the daemon refused for want of somewhere to
                    // point the tool is the exposure question, not an error
                    // to show: the listener is off, and turning it on is a
                    // decision with words of its own.
                    if let failure = error as? DaemonClient.Failure,
                        HarnessSurface.isNoDestination(failure.message)
                    {
                        self.harnessExposureRequest = HarnessRequest(id: id, action: action)
                    } else {
                        self.reportHarnessFailure()
                    }
                }
            }
        }
    }

    /// The contributor said no to the change. The file keeps every value it
    /// has, and the plan is simply dropped -- it expires on the far side.
    func cancelHarnessPreview() {
        guard !harnessBusy else { return }
        harnessPreview = nil
    }

    /// The contributor said yes.
    ///
    /// The only thing sent is the id the daemon minted, so what is written
    /// can only be what was shown. A plan the daemon no longer holds --
    /// expired, already committed, or a file that moved underneath it -- is
    /// reported as a change that did not happen and is NOT retried: the
    /// contributor is shown a fresh list, and a fresh preview if they ask
    /// again.
    func confirmHarnessPreview() {
        guard let plan = harnessPreview,
            HarnessSurface.canCommit(plan, calls: harnessCalls),
            let planID = plan.planID,
            let client, !harnessBusy
        else { return }
        // The preview stays up, busy, until the commit answers (#1280 review).
        harnessBusy = true
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try client.harnessCommit(planID: planID) }
            await MainActor.run {
                self.harnessBusy = false
                self.harnessPreview = nil
                if case .failure = outcome { self.reportHarnessFailure() }
                self.refreshHarnesses()
            }
        }
    }

    /// One sentence for every way a change can fail to happen, and it is the
    /// payload's: the change was not confirmed, look and try again.
    private func reportHarnessFailure() {
        lastActionError = privateInferenceCopy?.writeUnconfirmed
    }

    // MARK: - The key this destination answers with

    /// The three branch tables the credential card turns on, all decided in
    /// the Rust. This shell owns no `switch` on this surface either.
    /// The four branch tables the queue's eligibility rows turn on, all
    /// decided in the Rust. This shell owns no `switch` on this surface
    /// either -- see `EligibilitySurface`.
    let eligibilityCalls = EligibilityCalls(
        stateLine: { TCContributionEligibility.stateLine(state: $0) },
        stateTone: { TCContributionEligibility.stateTone(state: $0) },
        control: { TCContributionEligibility.control(state: $0) },
        reasonLine: { TCContributionEligibility.reasonLine(reason: $0) },
        withheldLine: { TCContributionEligibility.withheldLine(withheld: $0) },
        groupControl: { TCContributionEligibility.groupControl(pending: $0, contributable: $1) }
    )

    /// The three branch tables the queue's attestation mark turns on, all
    /// decided in the Rust -- see `AttestationSurface`.
    ///
    /// Three and not four. There is no control table here, because the mark
    /// describes the trace and offers nothing to press; whether a session
    /// may be sent stays `eligibilityCalls.control`'s question.
    ///
    /// The reason closure is `TCAttestation`'s, never
    /// `TCContributionEligibility`'s. Both take the same thirteen labels and
    /// answer different sentences, so the wrong one here would compile,
    /// render, and tell a contributor their session had been refused.
    let attestationCalls = AttestationCalls(
        markLine: { TCAttestation.markLine(mark: $0) },
        markTone: { TCAttestation.markTone(mark: $0) },
        reasonLine: { TCAttestation.reasonLine(reason: $0) }
    )

    let credentialCalls = CredentialCalls(
        stateLine: { TCNearAiCredential.stateLine(state: $0) },
        stateTone: { TCNearAiCredential.stateTone(state: $0) },
        action: { TCNearAiCredential.action(state: $0) },
        harnessNotice: { TCNearAiCredential.harnessNotice(credentialed: $0) ?? "" }
    )

    let balanceCalls = BalanceCalls(
        stateLine: { TCNearAiBalance.stateLine(state: $0) },
        stateTone: { TCNearAiBalance.stateTone(state: $0) },
        action: { TCNearAiBalance.action(state: $0) },
        amount: { TCNearAiBalance.amount(present: $0, nanos: $1, scale: $2) },
        remainingLine: { TCNearAiBalance.remainingLine(present: $0, nanos: $1, scale: $2) },
        limitLine: { TCNearAiBalance.limitLine(present: $0, nanos: $1, scale: $2) },
        spentLine: { TCNearAiBalance.spentLine(present: $0, nanos: $1, scale: $2) },
        observedLine: { TCNearAiBalance.observedLine(secondsAgo: $0) }
    )

    /// What this machine holds, from the daemon's own report.
    ///
    /// Seeded as unreported rather than absent: before the first poll
    /// answers, this shell has read nothing, and "no key is kept here" is a
    /// claim about the machine that would invite a second sign-in.
    @Published private(set) var credentialStatus: CredentialStatus = .unreported

    /// What is left in the account, from the daemon's own report.
    ///
    /// Seeded as unreported for `credentialStatus`'s reason, and it matters
    /// more here: every other seed value would be a claim about somebody's
    /// money made before anything was read.
    @Published private(set) var balanceStatus: BalanceStatus = .unreported
    @Published private(set) var balanceRefreshing = false
    private var balanceReadGeneration = 0

    /// The ceremony this shell started, while it is still going.
    ///
    /// Kept because `browser_url` is served ONCE, by start, and no poll
    /// re-serves it -- and so a cancel can name the attempt this shell
    /// started. Not required for one: the daemon accepts an unnamed cancel
    /// and stops whatever sign-in it is running. Dropped when the ceremony
    /// leaves the state that has something to cancel.
    @Published private(set) var credentialAttempt: CredentialAttempt?

    /// The attempt awaited by first run or account recovery, kept apart
    /// from Settings' `credentialAttempt` so cancellation names its own.
    @Published private var firstRunLoginAttemptID: String?
    @Published private(set) var nearAiBrowserSignInWaiting = false
    @Published private(set) var nearAiBrowserRestarting = false
    private var nearAiBrowserGeneration = 0

    /// A second Sign in again replaces the browser attempt, without
    /// starting a second account flow or trusting the old poll's result.
    var canRestartNearAiBrowserSignIn: Bool {
        nearAiBrowserSignInWaiting && firstRunLoginAttemptID != nil
            && !nearAiBrowserRestarting && (accountSigningIn || nearAiSigningIn)
    }

    @Published private(set) var credentialBusy = false

    /// How often a sign-in in flight is re-read. Tests shorten it.
    var credentialPollInterval: Duration = .seconds(2)

    /// The loop that re-reads a sign-in while it is in flight.
    ///
    /// The ceremony finishes in a browser this app does not own and the
    /// daemon publishes no event when it does, so the model asks. It lives
    /// here and not in a view because the sign-in starts from the Private AI
    /// notice and from Home as well as from Settings, and closing any of them
    /// must not stop it. At most one runs. It ends when the shared table stops
    /// offering a cancel, which is also where `refreshNearAiCredential` lets
    /// go of the attempt, so cancel, failure and timeout all stop it.
    private(set) var credentialPoll: Task<Void, Never>?

    /// The shared credential poll owns notifications for its ceremony,
    /// including one recovered from the daemon. Account/first-run browser
    /// flows publish their own completion. An epoch invalidates reads that
    /// were already in flight when a ceremony was cancelled or replaced.
    private var credentialSignInNotifiesAccount = false
    private var credentialSignInNotificationEpoch = 0

    /// A sign-in is pending while this shell holds its attempt or the shared
    /// table still offers a cancel for the status last read. The second half
    /// covers an app restarted while the daemon kept its ceremony running.
    private var credentialSignInPending: Bool {
        credentialAttempt != nil
            || CredentialSurface.action(credentialStatus, calls: credentialCalls) == .cancel
    }

    private func startCredentialPollIfNeeded() {
        guard credentialPoll == nil, credentialSignInPending else { return }
        credentialPoll = Task { @MainActor [weak self] in
            while let self, !Task.isCancelled {
                try? await Task.sleep(for: self.credentialPollInterval)
                guard !Task.isCancelled, self.credentialSignInPending else { break }
                self.refreshNearAiCredential()
            }
            self?.credentialPoll = nil
        }
    }

    /// Re-read on every settings refresh, and again while a ceremony is in
    /// flight: the ceremony finishes in a browser this app does not own, so
    /// nothing here is told when it does.
    func refreshNearAiCredential() {
        guard let client else { return }
        let attempt = credentialAttempt?.attemptID
        let notificationEpoch = credentialSignInNotificationEpoch
        let followsAccountFlow = nearAiBrowserSignInWaiting || accountSigningIn || nearAiSigningIn
        perform(
            CredentialSurface.statusMethod,
            work: { try $0.nearAiCredentialStatus(attemptID: attempt) }
        ) { status in
            guard self.client === client,
                  notificationEpoch == self.credentialSignInNotificationEpoch else { return }
            let wasPending = self.credentialSignInPending
            let previous = self.credentialStatus
            self.publishIfChanged(\.credentialStatus, status)
            // The key came or went, by whichever path (the browser poll, the
            // Sign in button's own flow, Forget, an expiry): Private AI's
            // state is read again now, and every Private AI dot with it
            // (owner, 2026-10-29: "the color should update if I login to
            // near.ai"). The windows' stores follow this read.
            if Self.credentialPresenceMoved(from: previous, to: status) {
                self.refreshSettings()
            }
            // The attempt is let go the moment the shared table stops
            // offering a cancel for it. Holding a finished attempt's id
            // would keep polling a ceremony nobody is waiting on.
            if CredentialSurface.action(status, calls: self.credentialCalls) == .cancel {
                if !followsAccountFlow, !self.nearAiBrowserSignInWaiting,
                   !self.accountSigningIn, !self.nearAiSigningIn {
                    self.credentialSignInNotifiesAccount = true
                }
                self.startCredentialPollIfNeeded()
            } else {
                // Finish this observation epoch before publishing completion:
                // a slower waiting reply must not resurrect the ceremony.
                if wasPending { self.credentialSignInNotificationEpoch &+= 1 }
                let notifiesAccount = self.credentialSignInNotifiesAccount
                self.credentialSignInNotifiesAccount = false
                let wasHeld = self.credentialAttempt != nil
                self.credentialAttempt = nil
                // The sign-in landed: everything the key is behind is read
                // again, as after a write (`submitNearAiCredential`), so the
                // notices and cards that wait on it go by themselves.
                if wasHeld || notifiesAccount, status.state == CredentialSurface.statePresent {
                    self.refreshSettings()
                    self.refreshHarnesses()
                    self.refreshManagedSessions()
                }
                // An inference key can outlive its Cloud session. A named
                // failed/cancelled attempt can also leave an older session,
                // so trust its lifecycle when the daemon reports one.
                let verdict = status.attemptStatus == nil
                    ? NearAILoginPoll.verdict(status) : NearAILoginPoll.freshVerdict(status)
                if notifiesAccount, status.sessionState == CredentialSurface.statePresent,
                   verdict == .signedIn {
                    self.accountSessionGeneration &+= 1
                }
            }
        }
        refreshNearAiBalance()
    }

    /// Whether a credential read says the NEAR AI key came or went. Only
    /// between two answered reads: the first answer after launch is not a
    /// sign-in, and the launch already reads settings.
    nonisolated static func credentialPresenceMoved(from previous: CredentialStatus, to current: CredentialStatus) -> Bool {
        guard !previous.state.isEmpty, !current.state.isEmpty else { return false }
        return (previous.state == CredentialSurface.statePresent) != (current.state == CredentialSurface.statePresent)
    }

    /// Re-read whenever the key is, because it is the key the balance is
    /// read WITH: signing in, signing in again and forgetting all move this
    /// row, and a card whose two halves disagreed about whether a session
    /// exists would be worse than either half alone.
    ///
    /// Lifecycle reads supersede earlier reads, including a pending button
    /// refresh: the credential or spending organization may have changed.
    func refreshNearAiBalance(fresh: Bool = false) {
        readNearAiBalance(fresh: fresh || balanceRefreshing, showingProgress: balanceRefreshing)
    }

    /// Coalesce repeated clicks, while keeping lifecycle reads independent.
    func refreshNearAiBalanceFromButton() {
        guard !balanceRefreshing else { return }
        readNearAiBalance(fresh: true, showingProgress: true)
    }

    private func readNearAiBalance(fresh: Bool, showingProgress: Bool) {
        balanceReadGeneration &+= 1
        let generation = balanceReadGeneration
        guard let client else {
            balanceRefreshing = false
            if showingProgress { publishIfChanged(\.balanceStatus, BalanceStatus(state: "unavailable")) }
            return
        }
        balanceRefreshing = showingProgress
        Task {
            let outcome = await DaemonWork.run { Result { try client.nearAiBalance(fresh: fresh) } }
            guard generation == self.balanceReadGeneration else { return }
            self.balanceRefreshing = false
            guard self.client === client else { return }
            switch outcome {
            case .success(let status):
                self.recordRead(BalanceSurface.statusMethod, answered: true)
                self.publishIfChanged(\.balanceStatus, status)
            case .failure:
                self.recordRead(BalanceSurface.statusMethod, answered: false)
                // An old amount must not look like a successful refresh.
                self.publishIfChanged(\.balanceStatus, BalanceStatus(state: "unavailable"))
            }
        }
    }

    // MARK: - The organization Private AI spends from

    /// The account's organizations, as the daemon last listed them. Seeded
    /// unreported, which draws nothing: most accounts have one organization,
    /// and an unknown count is never "several".
    @Published private(set) var spendOrganizations: SpendOrganizations = .unreported

    /// A switch is in flight. It takes about ten seconds: the daemon waits
    /// for the new key to become usable before moving Private AI onto it.
    @Published private(set) var spendOrganizationSwitching = false

    /// The last switch's refusal, in the daemon's words. Every refusal
    /// changed nothing.
    @Published private(set) var spendOrganizationFailure: String?

    /// Read lazily -- when the Private AI tab or the Private AI section of
    /// Settings appears, and after a switch -- never on the sign-in or the
    /// key hand-off. The daemon caches it with the balance.
    ///
    /// Not `perform`: a daemon too old to know the method answers
    /// `unknown_method`, which is "nothing to show", not an error to raise.
    func refreshSpendOrganizations(fresh: Bool = false) {
        guard let client else { return }
        Task.detached(priority: .utility) {
            let status = (try? client.nearAiSpendOrganizations(fresh: fresh)) ?? .unreported
            await MainActor.run {
                guard self.client === client else { return }
                self.publishIfChanged(\.spendOrganizations, status)
            }
        }
    }

    func selectSpendOrganization(_ organizationID: String) async {
        guard let client, !spendOrganizationSwitching, !credentialBusy else { return }
        spendOrganizationSwitching = true
        spendOrganizationFailure = nil
        let outcome = await Task.detached(priority: .userInitiated) {
            Result { try client.nearAiSelectSpendOrganization(organizationID) }
        }.value
        guard self.client === client else { return }
        spendOrganizationSwitching = false
        switch outcome {
        case .success:
            // The choice is recorded, so the prompt ends now, not on the
            // re-read below.
            publishIfChanged(\.spendOrganizations, spendOrganizations.withPromptEnded())
        case .failure(let error):
            let label = (error as? DaemonClient.Failure)?.message ?? ""
            spendOrganizationFailure = spendOrganizations.failureLine(for: label)
        }
        // The key moved, so the balance and billing it is read with did too.
        refreshNearAiBalance(fresh: true)
        refreshSpendOrganizations(fresh: true)
    }

    /// The prompt's "Not now": record the organization already in use as the
    /// choice, so the prompt ends without a switch. The select method with
    /// the current organization mints nothing and only records it. A failure
    /// keeps the prompt and says nothing -- no Settings failure line, which is
    /// for switches -- and the next re-read shows the truth.
    func dismissSpendOrganizationPrompt() async {
        guard let client, !spendOrganizationSwitching, !credentialBusy,
            let current = spendOrganizations.currentOrganizationID
        else { return }
        let outcome = await Task.detached(priority: .userInitiated) {
            Result { try client.nearAiSelectSpendOrganization(current) }
        }.value
        guard self.client === client else { return }
        if case .success = outcome {
            publishIfChanged(\.spendOrganizations, spendOrganizations.withPromptEnded())
        }
        refreshSpendOrganizations(fresh: true)
    }

    /// Whether billing has opened in the browser during this run.
    ///
    /// Set only when the browser accepted the page. A payment made there
    /// reaches the account through the service's own webhook, which nothing
    /// here hears about, so from then on the app coming forward is the cue to
    /// read the balance again. The daemon stops serving a figure from before
    /// the click, so the read is a fresh one.
    private(set) var billingOpened = false

    func noteBillingOpened() {
        billingOpened = true
    }

    /// Re-read the balance when the app comes forward, once billing has been
    /// opened. Answers whether a read was asked for.
    @discardableResult
    func refreshBalanceOnActivation() -> Bool {
        guard billingOpened else { return false }
        refreshNearAiBalance()
        return true
    }

    /// Re-read the sign-in when the app comes forward while an attempt is
    /// held: the browser is in front during sign-in, so coming back is the
    /// cue that it may have finished. Answers whether a read was asked for.
    @discardableResult
    func refreshCredentialOnActivation() -> Bool {
        guard credentialAttempt != nil else { return false }
        refreshNearAiCredential()
        return true
    }

    func nearAiFunding(expected: FundingDestination?) async -> FundingStatus? {
        guard let client, !credentialBusy else { return nil }
        let result = await Task.detached(priority: .userInitiated) {
            try? client.nearAiFunding(expected: expected)
        }.value
        guard self.client === client, !Task.isCancelled, !credentialBusy else { return nil }
        return result
    }

    /// Begins the ceremony and hands back the one URL that was served.
    ///
    /// The URL is returned rather than opened here: opening a browser is the
    /// view's `openURL` environment, and this model has no window. A `nil`
    /// means the ceremony did not begin in a way this shell can carry
    /// through, and nothing is left half-started -- the daemon's own attempt
    /// times out on the browser.
    func startNearAiCredential(provider: String? = nil) async -> URL? {
        guard let client, !credentialBusy else { return nil }
        credentialBusy = true
        let outcome = await Task.detached(priority: .userInitiated) {
            Result { try client.nearAiCredentialStart(provider: provider) }
        }.value
        credentialBusy = false
        guard case .success(let attempt) = outcome, let attempt else {
            lastActionError = privateInferenceCopy?.writeUnconfirmed
            return nil
        }
        credentialSignInNotificationEpoch &+= 1
        credentialSignInNotifiesAccount = true
        credentialAttempt = attempt
        startCredentialPollIfNeeded()
        refreshNearAiCredential()
        return URL(string: attempt.browserURL)
    }

    /// Stops waiting on the browser.
    ///
    /// Sent with no attempt id when this shell holds none: the daemon cancels
    /// whatever sign-in it is running, so an app restarted mid-ceremony can
    /// still stop the one it never started.
    func cancelNearAiCredential() {
        credentialSignInNotificationEpoch &+= 1
        credentialSignInNotifiesAccount = false
        let attemptID = credentialAttempt?.attemptID
        submitNearAiCredential { try $0.nearAiCredentialCancel(attemptID: attemptID) }
    }

    /// Removes the stored key from this machine.
    func forgetNearAiCredential() {
        submitNearAiCredential { try $0.nearAiCredentialForget() }
    }

    func migrateNearAiCredential() {
        submitNearAiCredential { try $0.nearAiCredentialMigrate() }
    }

    /// One write, then a re-read of everything the key is behind.
    ///
    /// The listener and the tool list are re-read as well as the card: the
    /// key is what this destination answers with, so forgetting it moves the
    /// sentence under the switch and the connect controls too, and a card
    /// that updated alone would leave both claiming otherwise.
    ///
    /// The busy flag is cleared on BOTH outcomes. `perform` runs its
    /// continuation on success only, which would leave a failed cancel with
    /// the button disabled and no way back.
    private func submitNearAiCredential(_ work: @escaping @Sendable (DaemonClient) throws -> Void) {
        guard !credentialBusy else { return }
        guard let client else {
            lastActionError = privateInferenceCopy?.writeUnconfirmed
            return
        }
        credentialBusy = true
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try work(client) }
            await MainActor.run {
                self.credentialBusy = false
                if case .failure = outcome {
                    self.lastActionError = self.privateInferenceCopy?.writeUnconfirmed
                }
                self.refreshNearAiCredential()
                self.refreshSettings()
                self.refreshHarnesses()
            }
        }
    }

    // MARK: - The redaction witness

    /// The witness surface's fixed words, decoded once from the Rust.
    ///
    /// Nil only if the export or the decode failed, and the card renders
    /// nothing at all in that case, for the reason `routingCopy` gives.
    @Published private(set) var witnessCopy: WitnessCopy? = WitnessCopy.decode(
        fromJSON: TCWitness.copyJSON() ?? ""
    )

    /// What the witness is doing, as `TC_WITNESS_STATE_*`.
    ///
    /// **Nil is "nobody has asked yet", not "absent".** The config directory
    /// is not resolved until `start()` runs, and seeding this with a state
    /// would be this shell asserting something about a file it has not read.
    /// The card renders no witness sentence while it stands.
    @Published private(set) var witnessStateCode: Int32?

    /// The configuration behind that state, when it could be read.
    ///
    /// Nil is NOT "no witness" -- that is state `absent` on a successful
    /// read. It is an unenrolled device or a config that could not be read,
    /// and `witnessStateCode` is what says which.
    @Published private(set) var witnessStatus: WitnessStatus?

    /// The ABI's fixed label from the last witness read or write that
    /// refused, or nil.
    ///
    /// An operator string like `witness-pin-required`, never wording, and no
    /// sentence is built around it: that sentence would exist in this shell
    /// alone. It carries no path, no token and no trace content.
    @Published private(set) var witnessLabel: String?

    /// A witness read or write is in flight.
    @Published private(set) var witnessBusy = false

    /// The sentences and tones that are decided in Rust. This shell fills in
    /// no holes and owns no `switch`; see `TCWitness`.
    let witnessCalls = WitnessCalls(
        stateLine: { TCWitness.stateLine(state: $0) },
        stateTone: { TCWitness.stateTone(state: $0) },
        lastResultLine: { TCWitness.lastResultLine() },
        lastResultTone: { TCWitness.lastResultTone() }
    )

    /// The witness state as a case, or nil while nothing has been asked.
    ///
    /// Derived from the state code and from nothing else -- never from
    /// `witnessStatus?.url` being non-nil, which is the boolean this surface
    /// refuses to hand a shell, spelled differently.
    var witnessState: WitnessTrustState? {
        witnessStateCode.map(WitnessTrustState.fromABI)
    }

    /// Ask what the witness is doing and publish the answer.
    ///
    /// Two calls, deliberately: `tc_witness_trust_state` answers for every
    /// input, including the unenrolled and unreadable cases where the status
    /// JSON refuses. A card driven off the status alone would have nothing
    /// to say in exactly the states that matter most.
    func refreshWitness() {
        let dir = configDirectory
        guard !dir.isEmpty else { return }
        Task.detached(priority: .userInitiated) {
            let code = TCWitness.trustState(configDir: dir)
            let read = TCWitness.statusJSON(configDir: dir)
            await MainActor.run { self.publishWitness(code: code, read: read, wrote: nil) }
        }
    }

    /// Write the configuration, then re-read and publish what came back.
    ///
    /// `canConfigure` is checked here as well as in the card: an empty pin
    /// list produces a client that refuses every submission from the moment
    /// it is saved, and this shell does not make that call.
    func configureWitness(_ form: WitnessForm) {
        guard form.canConfigure, let pins = form.measurementsJSON else { return }
        let url = form.url.trimmingCharacters(in: .whitespacesAndNewlines)
        let address = form.signingAddress.trimmingCharacters(in: .whitespacesAndNewlines)
        writeWitness { dir in
            TCWitness.configure(
                configDir: dir, url: url, signingAddress: address, measurementsJSON: pins)
        }
    }

    /// Stop using a witness, then re-read and publish what came back.
    ///
    /// This is the way out of a refusal, and it is a real change rather than
    /// a setting being switched off: later submissions carry this app's own
    /// judgement of what was left rather than a certificate. The card says
    /// so, in the Rust's words.
    func clearWitness() {
        writeWitness(clearsDraft: true) { TCWitness.clear(configDir: $0) }
    }

    /// Nothing is applied optimistically. The write's own answer decides
    /// only whether a refusal label is shown; what the card renders is the
    /// state and status read back afterwards.
    ///
    /// `clearsDraft`: a clear that succeeds leaves nothing for an edited
    /// field to describe, so the fields read the daemon again.
    private func writeWitness(clearsDraft: Bool = false, _ work: @escaping @Sendable (String) -> TCWitness.Outcome) {
        let dir = configDirectory
        guard !dir.isEmpty else { return }
        witnessBusy = true
        witnessLabel = nil
        Task.detached(priority: .userInitiated) {
            let wrote = work(dir)
            let code = TCWitness.trustState(configDir: dir)
            let read = TCWitness.statusJSON(configDir: dir)
            await MainActor.run {
                self.witnessBusy = false
                if clearsDraft, case .done = wrote { self.witnessDraft = nil }
                self.publishWitness(code: code, read: read, wrote: wrote)
            }
        }
    }

    private func publishWitness(
        code: Int32, read: TCWitness.StatusRead, wrote: TCWitness.Outcome?
    ) {
        publishIfChanged(\.witnessStateCode, code)
        var label: String?
        switch read {
        case .status(let json):
            publishIfChanged(\.witnessStatus, WitnessStatus.decode(fromJSON: json))
        case .refused(let refusalLabel):
            publishIfChanged(\.witnessStatus, nil)
            label = refusalLabel
        }
        // A refused write is the more specific answer, and the one somebody
        // just asked for, so it wins over a refused read.
        if case .refused(let writeLabel) = wrote { label = writeLabel }
        publishIfChanged(\.witnessLabel, label)
        // The disclosure names the witness, so it moves with it.
        refreshRouteDisclosure()
    }

    // MARK: - What leaves this machine (K11)

    /// The daemon's facts in the shared crate's words, or where the panel
    /// stands without them: loading before the first answer, unreadable when
    /// there is no daemon to ask or its answer did not decode. Never a blank
    /// panel, which would read as nothing to disclose.
    @Published private(set) var routeDisclosureState: RouteDisclosureState = .loading
    var routeDisclosure: RouteDisclosure? {
        if case .shown(let disclosure) = routeDisclosureState { return disclosure }
        return nil
    }
    /// Which read is the latest asked for. Each read is its own detached
    /// task, so answers can land out of order; only the latest one's is
    /// published, and an older answer arriving after it is dropped.
    private var routeDisclosureGeneration: UInt64 = 0
    /// The Rust's words for that case, read once: they do not change.
    let routeDisclosureUnreadableCopy: RouteDisclosureUnreadable? =
        TCConsentCopy.routeDisclosureUnreadableJSON().flatMap {
            RouteDisclosureUnreadable.decode(fromJSON: $0)
        }
    /// Held certificates' claims, by entry id, for the review sheet.
    @Published private(set) var certificateDetails: [String: CertificateDetail] = [:]

    /// Re-read what leaves this machine. Called wherever a fact it states
    /// can change: the witness, enrollment, and any settings write (through
    /// `daemonSettings`), as well as when a disclosure surface appears.
    func refreshRouteDisclosure() {
        routeDisclosureGeneration &+= 1
        let generation = routeDisclosureGeneration
        guard let client else {
            publishIfChanged(\.routeDisclosureState, .unreadable)
            return
        }
        Task.detached(priority: .userInitiated) {
            let facts = try? client.routeDisclosureFactsJSON()
            let disclosure = facts
                .flatMap { TCConsentCopy.routeDisclosureJSON(forFacts: $0) }
                .flatMap { RouteDisclosure.decode(fromJSON: $0) }
            await MainActor.run {
                guard generation == self.routeDisclosureGeneration else { return }
                self.publishIfChanged(
                    \.routeDisclosureState, disclosure.map { .shown($0) } ?? .unreadable)
            }
        }
    }

    /// Ask for the certificate one entry holds. Only for an entry whose
    /// `holdsCertificate` is true; anything unreadable leaves nothing shown.
    func loadCertificateDetail(entryID: String) {
        guard let client else { return }
        Task.detached(priority: .userInitiated) {
            let detail = (try? client.certificateDetailJSON(entryID: entryID)).flatMap { json in
                TCConsentCopy.certificateDetailCopyJSON().flatMap {
                    CertificateDetail.decode(detailJSON: json, copyJSON: $0)
                }
            }
            await MainActor.run { self.certificateDetails[entryID] = detail }
        }
    }

    @Published private(set) var outcomeCounts: [String: Int] = [:]
    @Published private(set) var audit: [AuditEntry] = []
    @Published var undo: Undo?
    @Published var lastActionError: String?

    /// A one-line statement about something that DID happen, as opposed to
    /// `lastActionError`, which is about something that did not. Kept apart
    /// so the two never have to be told from each other by their wording.
    ///
    /// Nothing here clears it on the way to somewhere else -- only two
    /// actions ever assign it, so unlike `lastActionError` it is not
    /// overwritten by the next thing that goes wrong. Its dismiss control on
    /// the Traces tab (`TracesOffersBar`'s notice) is therefore the only way
    /// out of it, which is why it has one.
    @Published var lastActionNotice: String?
    /// What a finished first run must still say -- Automatic was refused, so
    /// sharing is on Ask -- shown above every section once the first-run
    /// host has gone (`FirstRunDaemon.firstRunFinished`). The core's
    /// sentence; dismissed by the person.
    @Published var firstRunNotice: String?
    /// A tool whose launch dialog the menu bar's popover asked for (owner,
    /// 2026-10-19); the Private AI tab opens the dialog and clears it.
    @Published var pendingManagedLaunch: String?

    private var daemon: TCDaemon?
    private var client: DaemonClient?
    var skillLearningClient: DaemonClient? { client }
    /// The client the NEAR AI payout section calls through. Nil while no
    /// daemon is running.
    var payoutLinkDaemonClient: DaemonClient? { client }

    /// Uses fresh cloud membership, then prepares inference and payment for
    /// a sole owned organization. An account change invalidates every await.
    func prepareSingleOrganizationOnboarding() async -> SingleOrganizationOnboardingOutcome {
        guard let client else { return .unavailable }
        let generation = accountSessionGeneration
        let current: @MainActor @Sendable () -> Bool = { [weak self] in
            self?.client === client && self?.accountSessionGeneration == generation
        }
        let preparation = SingleOrganizationOnboarding(
            organizations: {
                try await DaemonWork.run { Result { try client.nearAiSpendOrganizations(fresh: true) } }.get()
            },
            payoutOrganizations: {
                try await DaemonWork.run { Result { try client.payoutLink.organizations() } }.get()
            },
            status: {
                try await DaemonWork.run { Result { try client.payoutLink.status() } }.get()
            },
            selectSpend: { id in
                try await DaemonWork.run { Result { try client.nearAiSelectSpendOrganization(id) } }.get()
            },
            startLink: { id, revision, version in
                try await DaemonWork.run { Result {
                    try client.payoutLink.startLink(
                        railId: PayoutLinkSurface.nearAiRailId, organizationId: id,
                        orgsRevision: revision, expectedVersion: version)
                } }.get()
            },
            confirmLink: { id in
                try await DaemonWork.run { Result {
                    try client.payoutLink.confirmLink(
                        railId: PayoutLinkSurface.nearAiRailId, organizationId: id)
                } }.get()
            },
            isCurrent: current)
        let result = await preparation.prepare()
        guard current() else { return .unavailable }
        return result
    }

    /// The one subscription and what it declares (`ReengageDeclaration`):
    /// `reengage_due` only while a notification can be posted.
    private var reengage: ReengageDeclaration<TCSubscription>?
    private var subscription: TCSubscription? { reengage?.token }
    /// Whether the daemon currently counts this app as a renderer.
    var declaresReengagement: Bool { reengage?.isDeclared ?? false }
    /// The C1 data contract's live client (K1 of #1173), for screens that
    /// read through `DaemonDataClient`. Created with the daemon and fed by
    /// the same `tc_subscribe` callback as `handle(event:)`, so there is
    /// one subscription and both sides see the same frames. `nil` while no
    /// daemon is running. Published, so a screen holding `daemonData` sees
    /// a daemon restart replace the client rather than keep a finished one.
    @Published private(set) var liveData: LiveDaemonClient? {
        didSet {
            subscriptionUsageModel = liveData.map { client in
                SubscriptionUsageModel(reader: { refresh, interactive, provider, login in
                    try await client.subscriptionUsage(refresh: refresh, interactive: interactive, provider: provider, login: login)
                }, writer: { provider, on in
                    try await client.setSubscriptionUsage(provider: provider, on: on)
                })
            }
        }
    }
    /// Settings and Insights share permission and current readings. Only
    /// Overview observes it; Settings reads metadata and saves consent.
    @Published private(set) var subscriptionUsageModel: SubscriptionUsageModel?
    var daemonData: (any DaemonDataClient)? { liveData }
    private var undoTask: Task<Void, Never>?

    /// Client-side bookkeeping for the daemon's bounded preview scheduler --
    /// see `PreviewRequestTracker`'s doc. Not published: nothing renders off
    /// it directly, `summaries`/`tooLarge`/`summaryErrors` are what views
    /// read.
    private var previewTracker = PreviewRequestTracker()
    /// Coalesces `preview_visible` sends against a scroll settling -- see
    /// `PreviewVisibilityCoalescer`'s doc. `visibleDebounce` is the real
    /// timer; the coalescer holds only the pure bookkeeping of what to send
    /// once it fires.
    private var visibilityCoalescer = PreviewVisibilityCoalescer()
    private var visibleDebounce: Task<Void, Never>?

    // MARK: - Derived state the shell renders

    /// What is waiting for a yes or no, and how that splits by project.
    ///
    /// Both are *stored*, recomputed only when `pending` actually changes
    /// (see `recomputeWaiting`), because both used to be computed
    /// properties that a SwiftUI body evaluated afresh every single time.
    /// `QueueContent` alone read `awaitingDecision` to test emptiness, read
    /// `decisionsOwed` for its headline, read `waitingByProject` for its
    /// group headers -- which walked `awaitingDecision` again -- and then
    /// filtered `awaitingDecision` once more *per group* to find that
    /// group's rows. At the 500-entry cap the queue runs at, that was tens
    /// of thousands of entry visits and a dozen freshly allocated arrays on
    /// the main thread for every redraw, however small the thing that
    /// prompted it. See #388.
    ///
    /// A fresh array every time also denied SwiftUI any chance of deciding
    /// a group had not changed; a stored one at least holds still.
    @Published private(set) var awaitingDecision: [QueueEntry] = []

    /// What is waiting, per project, with sizes, its own entries, and the
    /// id `submitProject` takes.
    ///
    /// Grouped by `projectID`, not `projectLabel`: a label is a display
    /// name only, not guaranteed unique across two different projects, and
    /// grouping by it here would silently merge them into one bucket with
    /// one Submit button that could approve the wrong project's entries.
    /// Order is first-seen, which is also `awaitingDecision`'s order, so
    /// this reshuffles nothing a contributor has already scanned.
    @Published private(set) var waitingByProject: [QueueGroup<QueueEntry>] = []

    /// Sessions waiting whose preview reported that no pattern fired.
    /// Drives `QueueShieldState` only, and never the badge: the count is
    /// what a contributor with 149 sessions is reading, and this is a state
    /// the count cannot carry.
    ///
    /// Stored rather than computed for the reason `awaitingDecision` is:
    /// a SwiftUI body would otherwise walk the whole waiting list on every
    /// redraw. It depends on `summaries`, which arrive asynchronously long
    /// after the queue settles, so it is recomputed from both sides --
    /// `recomputeWaiting` and `applyPreviewOutcome`.
    @Published private(set) var nothingMatchedCount: Int = 0

    /// Only the daemon decides which sessions owe a decision (K6). The
    /// review list can include sessions held by other gates. Missing on an
    /// older daemon means unavailable, never an inferred count or zero.
    var decisionsOwed: Int? {
        status.decisionsOwed
    }

    /// The single place the two derived queue views are rebuilt. Called
    /// from `pending`'s `didSet`, so it runs when the queue moved and never
    /// when a view merely redrew.
    private func recomputeWaiting() {
        let waiting = pending.filter { $0.state == .pending }
        publishIfChanged(\.awaitingDecision, waiting)
        publishIfChanged(
            \.waitingByProject,
            QueueGrouping.groups(
                waiting,
                projectID: \.projectID,
                projectLabel: \.projectLabel,
                // Every decoded queue entry carries its size; only a
                // carried-over preview entry can lack one, and none is
                // ever in `pending`.
                sizeBytes: { $0.sizeBytes ?? 0 }
            )
        )
        recomputeNothingMatched()
    }

    /// How many waiting sessions have a preview that removed nothing.
    ///
    /// A session with no preview yet is NOT counted: nothing is known about
    /// it, and "nothing matched" is a report, not a default. It starts
    /// counting the moment its preview lands.
    private func recomputeNothingMatched() {
        let count = awaitingDecision.reduce(into: 0) { total, entry in
            guard let summary = summaries[entry.entryID] else { return }
            if RedactionLabels.removedTotal(summary.redactions) == 0 { total += 1 }
        }
        publishIfChanged(\.nothingMatchedCount, count)
    }

    var armedProjects: [ProjectRow] {
        projects.filter { $0.mode == .autoUpload }
    }

    var health: HealthCopy? {
        guard let label = status.health.lastErrorLabel else { return nil }
        // The witness banner says the same thing with the count, so the
        // bare label is suppressed when it is going to be drawn.
        if label == "witness-saturated" && witnessCapacityHealth != nil { return nil }
        // And the held-folder notice, which names the folders and says why --
        // again only when it is going to be drawn.
        if label == GateHeld.label && gateHeldNotice != nil { return nil }
        return HealthCopy.core(label: label, maxQueueEntries: daemonSettings?.maxQueueEntries)
    }

    /// The notice for armed folders the automatic-contribution gate is
    /// holding, in the Rust's words, when there are any. Independent of
    /// `health` for the reason `witnessCapacityHealth` is. Nil when nothing
    /// is held or the notice cannot be read; the label, if it holds the
    /// slot, then falls back to the core's on-hold line.
    var gateHeldNotice: GateHeldNotice? {
        guard status.gateHeld.held else { return nil }
        return TCConsentCopy.gateHeldNoticeJSON(forHeld: status.gateHeld.json)
            .flatMap(GateHeldNotice.decode(fromJSON:))
    }

    /// The banner for approved sessions held on a busy privacy witness, when
    /// there are any. Independent of `health` because another label can
    /// hold the daemon's one health slot while these sessions are still
    /// waiting.
    var witnessCapacityHealth: HealthCopy? {
        HealthCopy.forWitnessCapacity(status.witnessCapacity)
    }

    // MARK: - Lifecycle

    /// The resolved state directory, once `start()` has named one.
    ///
    /// Held so the roots screen starts the daemon against the same directory
    /// that refused, rather than re-resolving and possibly disagreeing with
    /// it.
    private(set) var configDirectory: String = "" {
        didSet {
            guard let invite = inviteAwaitingConfigDirectory, !configDirectory.isEmpty else { return }
            inviteAwaitingConfigDirectory = nil
            inviteLinkArrived(invite)
        }
    }

    /// Whether the watcher this shell is driving belongs to another
    /// process.
    ///
    /// Worth surfacing rather than hiding: an attached daemon cannot be
    /// stopped from here and cannot open a redacted body preview, and a
    /// contributor who is not told that meets it as a dead control.
    var isAttachedDaemon: Bool { daemon?.isAttached ?? false }

    /// The quit prompt that is true for this process, from the core
    /// (`tc_quit_prompt_json`): the ABI reads off the daemon handle whether
    /// this app hosts the watcher, is attached to one, or has none, and
    /// chooses the sentence. Nil only on a caught panic.
    var quitPrompt: QuitPrompt? {
        QuitPrompt.decode(fromJSON: daemon?.quitPromptJSON()
            ?? TCCoreCopy.quitPromptWithoutWatcherJSON())
    }

    var traceNavigationReady: Bool {
        guard case .running = startup else { return false }
        return !requiresOnboarding
    }

    func start() {
        start(plistChannel: BuildChannelGate.plistChannel)
    }

    /// `start()` with the bundle's `TCChannel` given, so a test can show a
    /// mismatch refusing without a bundle. A bundle that disagrees with the
    /// dylib, or names no channel, refuses here with the core's sentence and
    /// resolves no directory and starts no daemon (staging design, 4.3).
    /// `start(configDirectory:)` below is a test seam that skips this check.
    func start(plistChannel: String?) {
        guard case .starting = startup else { return }
        let channel: BuildChannel
        switch BuildChannelGate.verdict(plistChannel: plistChannel) {
        case .agreed(let agreed):
            channel = agreed
        case .refused(let sentence):
            startup = .refused(sentence)
            return
        }
        let resolved: DaemonHost.Resolution
        do {
            resolved = try DaemonHost.resolveConfigDirectory(channel: channel)
        } catch {
            startup = .refused("\(error)")
            return
        }
        start(configDirectory: resolved.path)
    }

    /// Explicit directory seam also exercises first-install startup without
    /// touching the developer's state or altering process-global environment.
    func start(configDirectory path: String) {
        guard case .starting = startup else { return }
        configDirectory = path
        startDaemon(at: path, settingsJSON: nil)
    }

    /// Start (or restart) the in-process daemon against an already-resolved
    /// state directory.
    ///
    /// `settingsJSON` is the roots screen's mechanism: the C ABI persists it
    /// and only then evaluates whether both session roots are declared, so
    /// one call both records the contributor's answer and starts the watcher.
    func startDaemon(
        at path: String,
        settingsJSON: String?,
        completion: (@MainActor @Sendable (Startup) -> Void)? = nil
    ) {
        guard daemon == nil, !daemonStartup.isStarting else { return }
        isStartingDaemon = true
        daemonStartup.start(configDirectory: path, settingsJSON: settingsJSON) { [weak self] result in
            guard let self else { return false }
            self.isStartingDaemon = false
            switch result {
            case .success(let daemon):
                self.daemon = daemon
                self.client = DaemonClient(daemon: daemon)
                self.liveData = DaemonDataWiring.live(daemon)
                self.startup = .running
                self.subscribe()
                self.refreshAll()
                #if DEBUG
                // K2 (#1173): console-only, so a developer can confirm the
                // dry run took before trusting the screens. Asked of the
                // daemon, not read from the environment, so it matches the
                // daemon's own mode. A label, not a sentence
                // (`ShellWordingTests`).
                if let client = self.client {
                    Task.detached {
                        if client.devDryRunActive() {
                            NSLog("TraceCommons: status.dev_dry_run=true")
                        }
                    }
                }
                #endif
            case .failure(TCDaemon.TCError.rootsNotDeclared):
                self.startup = .needsRoots
            case .failure(let error):
                self.startup = .refused("\(error)")
            }
            completion?(self.startup)
            return true
        }
    }

    private func subscribe() {
        guard let daemon else { return }
        // Captured, not read through `self`: the callback runs on a Rust
        // thread, and `deliver` is lock-guarded and never calls back in.
        let liveData = self.liveData
        // Declares nothing yet: `reengage_due` is declared on this same
        // subscription once the system's answer is known, and only while a
        // notification can be posted (`refreshReengageDeclaration`).
        let subscription = daemon.subscribe { [weak self] json in
            liveData?.deliver(eventJSON: json)
            // Rust background thread. Nothing observable may be touched
            // here; hop first, always.
            let event = DaemonEventParser.parse(json)
            Task { @MainActor in
                self?.handle(event: event)
            }
        }
        // This app renders the re-engagement notifications itself
        // (`Notifier.postReengage`), so the daemon may send them and count
        // it as a renderer -- but only while the system lets it post.
        reengage = subscription.map { token in
            ReengageDeclaration(token: token) { [weak daemon] token, accepts in
                daemon?.redeclare(token, accepts: accepts)
            }
        }
        Task { await refreshReengageDeclaration() }
        // No `subscribe` call follows: the contract's `snapshot`-on-subscribe
        // is a property of the SOCKET connection loop, which sends it to the
        // client that just connected. `tc_subscribe` attaches to the event
        // bus directly and gets no such courtesy frame, so the first paint
        // comes from the explicit `list_pending` + `status` in refreshAll()
        // rather than from waiting on a snapshot that will never arrive.
    }

    /// Declares `reengage_due` when a notification can be posted now, and
    /// withdraws it when one cannot. Called at start, when the permission
    /// prompt is answered, and when the app comes forward (the contributor
    /// may have changed it in System Settings).
    func refreshReengageDeclaration() async {
        let accepts = await reengageAccepts()
        reengage?.update(accepts: accepts)
    }

    /// The reads a `queue_changed` asks for, at most once per
    /// `queueChangeRefreshes` window however many events arrive in it.
    private func refreshAfterQueueChange() {
        refreshQueue()
        // The daemon count lives on `status`, not this local review
        // list. Fetch it too, including for older event publishers
        // that don't accompany queue changes with `status_changed`.
        refreshStatus()
        // A queue change is when a project can first become visible:
        // `list_projects` reports discovered projects from the queue, and
        // a session for a project nobody has ruled on is exactly what a
        // queue change delivers. Projects were fetched only by
        // `refreshAll()` at launch and after `setProjectMode`, so a
        // project discovered while the app was open stayed invisible in
        // both Settings and onboarding screen 5 until a relaunch.
        refreshProjects()
        // An entry leaving the queue -- accepted, quarantined, or
        // otherwise resolved -- is exactly the moment a new row appears
        // in history and the rollup tallies move. `refreshHistory()` was
        // reachable only from `refreshAll()` at launch, which is why the
        // History screen kept showing the counts from the moment the app
        // started no matter how many uploads finished after that: this
        // is the daemon's own signal that one just did. Both calls are
        // cheap daemon-side reads of state it already holds (no
        // recomputation, no network fan-out), so firing them on every
        // `queue_changed` costs the same as the queue/status/projects
        // refreshes right above, which already do this on every event
        // without a debounce.
        refreshHistory()
    }

    /// Batches `queue_changed` events: the first in a burst waits this
    /// long, and the rest land in the same refresh.
    private lazy var queueChangeRefreshes = RefreshCoalescer(delay: .milliseconds(100)) { [weak self] in
        self?.refreshAfterQueueChange()
    }

    private func handle(event: DaemonEvent) {
        switch event {
        case .snapshot(let pending, let status):
            applyPendingUpdate(pending)
            recordRead("status", answered: true)
            publishIfChanged(\.status, status)
        case .previewReady(let result):
            applyPreviewOutcome(result)
        case .queueChanged:
            // Coalesced: one Submit all over a hundred traces is hundreds of
            // `queue_changed`, and five reads per event, each redrawing on
            // the main thread, pegged it for as long as they kept coming.
            queueChangeRefreshes.request()
        case .statusChanged:
            refreshStatus()
            // The daemon says Private AI's listener state moves under this
            // too (the proxy learning NEAR AI's sign-in on its tick): the
            // menu bar's dot and the windows' stores follow the settings read.
            refreshSettings()
            // The listener's own state moves under this, and with it whether
            // any tool has answered yet. Re-read rather than left to age.
            refreshHarnesses()
        case .reengageDue(let due):
            Notifier.shared.postReengage(due)
        case .digestDue(let count, let contributed, let contributedProjects, let credit, let text):
            refreshQueue()
            // A digest can now be about what went out unasked, with nothing
            // waiting at all -- so this also refreshes history, which is the
            // screen those numbers came from and the one a contributor opens
            // next.
            if contributed > 0 {
                refreshHistory()
            }
            Notifier.shared.postDigest(
                text: text,
                pendingCount: count,
                projects: waitingByProject.map(\.label),
                contributedCount: contributed,
                contributedProjects: contributedProjects,
                creditPending: credit
            )
        case .resyncRequired, .lagged:
            refreshManagedSessions()
            refreshQueue()
            refreshStatus()
        case .nearAiBalanceChanged:
            // The daemon counted credit arriving: the balance it reads next
            // is the one the Balance tile marks.
            refreshNearAiBalance()
        case .unknown(let name):
            if name == "managed_changed" { refreshManagedSessions() }
        }
    }

    /// Teardown. Every user action here runs its daemon call on a detached
    /// task, so at the moment a contributor quits there can be a preview, an
    /// enrollment or a refresh sitting inside the C ABI with the raw handle.
    /// This method must not free that handle until those have left.
    ///
    /// It does not try to track those tasks itself. Tracking them here would
    /// mean tracking Swift Tasks, which can be cancelled and resumed at
    /// suspension points that have nothing to do with when the C call
    /// actually returns. The only place that knows a C call is in progress
    /// is the wrapper that makes it, so `TCDaemon.shutdown` owns the
    /// drain: it refuses new calls, waits for outstanding ones, and frees
    /// only if it can prove the handle is idle. If it cannot prove that, it
    /// leaks the handle on purpose -- see the note on `TCDaemon`.
    ///
    /// Called on the main thread (willTerminate), which is a plain thread
    /// with no tokio context, as the ABI requires. It blocks there for up to
    /// a few seconds in the bad case; that is the correct trade against
    /// freeing memory another thread is reading.
    func shutdown() {
        daemonStartup.cancel()
        isStartingDaemon = false
        undoTask?.cancel()
        credentialPoll?.cancel()
        credentialPoll = nil
        let subscription = self.subscription
        let daemon = self.daemon
        // Dropped first so no new work can be started from this side while
        // teardown runs; `perform`, `enroll` and the rest all guard on
        // `client`.
        reengage = nil
        self.daemon = nil
        self.client = nil
        // Screens' `for await` loops end here rather than waiting on a
        // subscription that is about to be cancelled.
        self.liveData?.finishEvents()
        self.liveData = nil
        guard let daemon else { return }
        if case .leaked(let reason) = daemon.shutdown(unsubscribing: subscription) {
            // A fixed label, no path or token, per this repo's logging rule.
            // The handle stayed allocated on purpose; the process is exiting.
            lastActionError = "shutdown: handle-leaked-\(reason)"
        }
    }

    // MARK: - Publishing

    /// Assign to a `@Published` property only when the value actually
    /// differs.
    ///
    /// Every refresher below re-fetches from the daemon and writes the
    /// decoded answer straight back. A freshly decoded array is a *new*
    /// value even when it is byte-identical to the one already held, so a
    /// plain assignment fires `objectWillChange` regardless -- and one
    /// `objectWillChange` invalidates every view observing this model,
    /// which at 500 queue rows is a full view-graph rebuild on the main
    /// thread. A single `queue_changed` event runs four refreshers and so
    /// used to publish five or six of them back to back for a queue that
    /// had not moved.
    ///
    /// Comparing first turns "the daemon answered" into "the answer
    /// changed", which is the thing a view actually needs to redraw for.
    /// Every model written through here is `Equatable`, and the compare is
    /// over a few hundred small structs -- orders of magnitude cheaper than
    /// the rebuild it avoids.
    private func publishIfChanged<T: Equatable>(
        _ keyPath: ReferenceWritableKeyPath<AppModel, T>,
        _ value: T
    ) {
        guard self[keyPath: keyPath] != value else { return }
        self[keyPath: keyPath] = value
    }

    // MARK: - Refresh

    func refreshAll() {
        refreshManagedSessions()
        refreshStatus()
        refreshQueue()
        refreshHistory()
        refreshProjects()
        refreshSettings()
        refreshConsentOptions()
        refreshOutcomeCounts()
        refreshAudit()
        refreshPublicProfile()
        refreshHarnesses()
        refreshNearAiCredential()
    }

    /// The local change log. Refreshed alongside everything else at launch,
    /// and again after each call that APPENDS to it -- arming a project,
    /// changing consent scopes, acknowledging the NEAR AI notice -- because
    /// the daemon publishes no event for an audit append, so a list fetched
    /// once would show a contributor everything except the change they just
    /// made.
    func refreshAudit() {
        perform("list_audit", work: { try $0.listAudit() }) { self.publishIfChanged(\.audit, $0) }
    }

    func refreshStatus() {
        perform("status", work: { try $0.status() }, onFailure: { self.publishIfChanged(\.statusReadFailed, true) }) {
            self.publishIfChanged(\.status, $0)
            self.publishIfChanged(\.statusReadFailed, false)
        }
    }

    func refreshQueue() {
        perform("list_pending", work: { try $0.listPending() }) { entries in
            self.applyPendingUpdate(entries)
        }
    }

    func refreshHistory() {
        perform("list_history", work: { try $0.listHistory() }) {
            self.publishIfChanged(\.history, $0)
            self.publishIfChanged(\.historyAnswered, true)
        }
        perform("history_rollup", work: { try $0.historyRollup() }) {
            self.publishIfChanged(\.rollup, $0)
        }
    }

    func refreshProjects() {
        perform("list_projects", work: { try $0.listProjects() }) { self.publishIfChanged(\.projects, $0) }
        refreshArmingOffer()
        perform("automatic_grant", work: { try $0.automaticGrant() }) { self.publishIfChanged(\.automaticGrant, $0) }
    }

    /// The daemon decides whether there is an offer and what it says; this
    /// only carries the answer. The rule -- how many contributions, which
    /// modes qualify, how long "Not now" lasts -- is
    /// `ProjectPolicy::arming_suggestion`, in one place, so the three shells
    /// cannot drift into offering different things.
    func refreshArmingOffer() {
        perform("arming_suggestion", work: { try $0.armingSuggestion() }) {
            self.publishIfChanged(\.armingOffer, $0)
        }
    }

    /// Arms the offered project. The offer clears because the daemon's next
    /// answer will not include an armed project, but it is cleared here too
    /// so the card does not linger for a round trip.
    func acceptArmingOffer(_ offer: ArmingOffer) {
        perform(
            "set_project_mode",
            work: { try $0.setProjectMode(projectID: offer.projectId, mode: .autoUpload) }
        ) { _ in
            self.armingOffer = nil
            self.refreshProjects()
            self.refreshAudit()
        }
    }

    /// "Not now". Silenced for thirty days by the daemon, not forgotten --
    /// and persisted there rather than here, so it survives a relaunch and
    /// applies to whichever shell asks next.
    func declineArmingOffer(_ offer: ArmingOffer) {
        perform("decline_arming", work: { try $0.declineArming(projectID: offer.projectId) }) { _ in
            self.armingOffer = nil
        }
    }

    /// Sets `project`'s mode via the daemon and refreshes `projects` from
    /// the daemon's own answer on success. Deliberately does not flip
    /// `project.mode` optimistically: the whole reason this method exists
    /// is that a UI that assumes a choice landed, when it did not, is worse
    /// than a UI that offers no choice at all. A failure lands in
    /// `lastActionError`, same as every other action here, and the caller
    /// must leave its own state alone until this succeeds.
    ///
    /// Named by `project.projectId`, the opaque id `list_projects` mints for
    /// every row. This used to send `projectLabel` as a `project_key`, which
    /// is a final path segment rather than a key and was refused with
    /// `project-key-unrecognized`.
    func setProjectMode(_ project: ProjectRow, mode: ProjectMode) {
        perform(
            "set_project_mode",
            work: { try $0.setProjectMode(projectID: project.projectId, mode: mode) }
        ) { _ in
            self.refreshProjects()
            // Arming or disarming a project is one of the changes the daemon
            // records; see `refreshAudit`.
            self.refreshAudit()
        }
    }

    /// Decline a whole project from the Waiting screen.
    ///
    /// The daemon clears what that project has waiting as part of setting the
    /// mode, so this refreshes the queue as well as the project list -- the
    /// cards are expected to disappear in the same round trip.
    ///
    /// `promised` is the count the confirmation named, which had to be read
    /// off this shell's own queue before the call. The daemon's `purged` is
    /// the authority: the queue is live, and a poll or an approval between
    /// the render and the click moves it. When the two disagree the
    /// contributor is told rather than left to notice -- see
    /// `tc_project_ignore_reconciled_text`.
    func ignoreProject(id projectID: String, label: String, promised: Int) {
        perform(
            "set_project_mode",
            work: { try $0.setProjectMode(projectID: projectID, mode: .ignore) }
        ) { purged in
            self.lastActionNotice = TCCoreCopy.projectIgnoreReconciled(
                project: label,
                promised: promised,
                purged: purged
            )
            self.refreshQueue()
            self.refreshProjects()
            self.refreshAudit()
        }
    }

    func refreshSettings() {
        perform("get_settings", work: { try $0.settings() }) { self.publishIfChanged(\.daemonSettings, $0) }
    }

    /// The declaration the daemon is holding, as the card's three controls.
    ///
    /// The port shows the conventional number when nothing is declared.
    /// That is display only: `RoutingSurface.settingsParams` writes nothing
    /// while the switch is off, so a default nobody chose never becomes an
    /// announcement that a local service is in use.
    ///
    /// A discovered port fills the field only where nothing is declared.
    /// The contributor's own port always wins -- see
    /// `RoutingForm.fromDeclaration`.
    var routingForm: RoutingForm {
        RoutingForm.fromDeclaration(
            mode: daemonSettings?.ironwire?.mode,
            port: daemonSettings?.ironwire?.port,
            tokenDir: daemonSettings?.ironwire?.tokenDir,
            discoveredPort: routingDiscovery.port
        )
    }

    /// Ask what the machine already knows, and show it.
    ///
    /// **This writes nothing and reads nothing of the contributor's.** It
    /// reads one file IronWire left, learns a port from it, and puts that
    /// port in a field. Declaring is still the switch and the button; a
    /// discovery that declared on its own would be this window announcing a
    /// local service nobody mentioned, which is the whole thing the
    /// declaration exists to stop.
    ///
    /// A machine without IronWire is not a failure and produces no error
    /// state: the answer is `found: false`, and a call that did not run at
    /// all degrades to the same thing, because both mean there is nothing
    /// to offer.
    func discoverRouting() {
        guard let client else { return }
        Task.detached(priority: .userInitiated) {
            let discovery = (try? client.discoverRouting()) ?? .none
            await MainActor.run { self.routingDiscovery = discovery }
        }
    }

    /// Changes one source's declaration on the running daemon -- Settings'
    /// "Watched folders", the after-first-run counterpart of the roots
    /// screen's start-with-settings.
    ///
    /// Nothing is applied optimistically: the rows read the daemon's
    /// answer, which `set_settings` returns in full, and an undecided
    /// choice is never sent. A failed confirmation requests fresh settings.
    func setSourceRoot(_ kind: SourceKind, _ choice: SourceChoice) async -> Bool {
        guard let params = choice.settingsParams(for: kind) else { return false }
        return await setSettingsDeclaration(params)
    }

    /// Write `params` through `set_settings` and keep what the daemon
    /// answers. A lost response does not prove the write failed, so the
    /// modes are read back; a requested path is never retained.
    func setSettingsDeclaration(_ params: [String: Any]) async -> Bool {
        guard let client else { return false }
        let encodedParams = Result {
            try JSONSerialization.data(withJSONObject: DaemonClient.settingsParams(params))
        }
        let result = await Task.detached(priority: .userInitiated) {
            Result {
                guard let params = try JSONSerialization.jsonObject(with: encodedParams.get()) as? [String: Any] else {
                    throw CocoaError(.coderInvalidValue)
                }
                return try client.setSettings(params)
            }
        }.value
        switch result {
        case .success(let settings):
            daemonSettings = settings
            return true
        case .failure:
            // A lost response does not prove the write failed. Read back the
            // authoritative modes, and never retain the requested path.
            if let confirmed = await Task.detached(operation: { try? client.settings() }).value {
                daemonSettings = confirmed
            }
            return false
        }
    }

    /// Write the declaration, then -- when it is on -- ask what was found.
    ///
    /// The evidence is dropped before the write, not after the answer: the
    /// words have to stop asserting the moment the declaration changes, not
    /// once a replacement arrives. Nothing here asks anybody to restart the
    /// app; the daemon rebuilds its reader in the same call.
    ///
    /// The probes run only from here -- a contributor pressing the switch or
    /// the button. Nothing on the submission path calls them.
    func applyIronWire(_ form: RoutingForm) {
        routingEvidence = nil
        routingProbeLine = nil
        routingChecking = form.on
        perform("set_settings", work: { try $0.setIronWire(form) }) { view in
            self.publishIfChanged(\.daemonSettings, view)
            // The daemon now holds this form, so the card reads it again --
            // unless the person has edited past it while the write was out.
            if self.routingDraft == form { self.routingDraft = nil }
            guard form.on else {
                self.routingChecking = false
                return
            }
            self.checkRouting(form)
        }
        if !form.on { routingChecking = false }
    }

    /// Ask whether the proxy answers, and say what it answered.
    private func checkRouting(_ form: RoutingForm) {
        guard let client else { return }
        Task.detached(priority: .userInitiated) {
            let outcome = try? client.probeRouting(form)
            let evidence = try? client.probeRoutedTools(form)
            await MainActor.run {
                self.routingChecking = false
                // A call that did not run is not a fact about the proxy.
                // `.unknown` is the outcome that claims nothing, and it is
                // what a refused call degrades to here.
                guard let copy = self.routingCopy else { return }
                self.routingProbeLine = RoutingSurface.probeLine(
                    outcome ?? .unknown, copy: copy, calls: self.routingCalls
                )
                self.routingEvidence = evidence
            }
        }
    }

    /// Refresh the per-tool words without touching the declaration, and say
    /// what the answer was.
    ///
    /// Called when the card appears, and only while something is declared:
    /// asking about a proxy nobody mentioned would be the probe of an
    /// undeclared local service that the declaration exists to prevent.
    ///
    /// The sentence is set from the same answer, which is what the Windows
    /// shell does when its settings card loads with a proxy declared.
    /// Without it, opening Settings against a declared proxy that is not
    /// running painted four "not known" rows and no sentence: the reason
    /// was in this answer's outcome and was thrown away, and only a button
    /// press could put it on screen. No second call is made for it -- the
    /// tool-list answer this already asks for carries the outcome.
    func refreshRoutedTools() {
        let form = routingForm
        guard form.on, let client else { return }
        Task.detached(priority: .userInitiated) {
            let evidence = try? client.probeRoutedTools(form)
            await MainActor.run {
                // Left as it was when the call did not run: a stale answer
                // is replaced by a new one, never by a blank -- and a
                // sentence about a call that did not happen is not a fact
                // about the proxy, so none is written either.
                guard let evidence else { return }
                self.routingEvidence = evidence
                guard let copy = self.routingCopy else { return }
                self.routingProbeLine = RoutingSurface.probeLine(
                    evidence.outcome, copy: copy, calls: self.routingCalls
                )
            }
        }
    }

    // MARK: - Enrollment

    enum EnrollOutcome {
        case succeeded(EnrollResult)
        /// Deliberately carries no message. The daemon's `enroll` only ever
        /// reports the generic `unavailable` / `enroll-failed` for this
        /// path -- see `DaemonClient.enroll` -- so there is nothing more
        /// specific a caller could show even if this case carried a string.
        case failed
    }

    /// What a preparation did, and the words the daemon chose for it.
    ///
    /// This used to return `AdmissionPreparation?` through `try?`, which threw
    /// the refusal away: the bridge now carries the daemon's sentence on the
    /// `Failure` it raises, and `try?` discarded the whole error to get a
    /// `nil` the view could only answer with one generic line. Sixteen causes
    /// arrived here and left as the same sentence.
    ///
    /// The success sentence comes from the same `view`, so both outcomes are
    /// the daemon's words rather than this shell's.
    func prepareAdmissionSession(entryID: String, backend: String) async -> AdmissionOutcome {
        guard let client else { return AdmissionOutcome(succeeded: false, sentence: nil) }
        return await Task.detached(priority: .userInitiated) {
            AppModel.admissionOutcome(from: client, entryID: entryID, backend: backend)
        }.value
    }

    /// The whole decision, so the detached task above holds none of it.
    ///
    /// Split out because `client` is private and a test cannot reach this
    /// arm through the model: without it the only reachable path is the
    /// no-client guard, and dropping the daemon's sentence again would stay
    /// green. Takes the client so a test can hand it one over a recording
    /// transport and cross the real bridge and the real decode.
    nonisolated static func admissionOutcome(
        from client: DaemonClient,
        entryID: String,
        backend: String
    ) -> AdmissionOutcome {
        do {
            let prepared = try client.prepareAdmissionSession(entryID: entryID, backend: backend)
            return AdmissionOutcome(
                succeeded: prepared.view?.ready == true,
                sentence: prepared.view?.message
            )
        } catch {
            return AdmissionOutcome(
                succeeded: false,
                sentence: DaemonClient.refusalSentence(from: error)
            )
        }
    }

    func nativeWalletFlow(action: String, flowID: String, commons: String, account: String) async -> NativeWalletView? {
        guard let client else { return nil }
        return await Task.detached { try? client.nativeWalletFlow(action: action, flowID: flowID, commons: commons, account: account) }.value
    }
    /// Join with the NEAR AI login, reporting the daemon's own control name.
    ///
    /// The label is passed back untouched for `TCNearAiEnroll` to turn into a
    /// sentence. This model does not know which of the ten refusals it is and
    /// must not guess: a shell-side table would be an eleventh that agrees
    /// with the shared one until it does not.
    func nearAiAccountEnroll(commons: String) async -> NearAiEnrollOutcome {
        guard let client else { return .refused("near_ai_enroll_unavailable") }
        return await Task.detached(priority: .userInitiated) { () -> NearAiEnrollOutcome in
            do {
                return .joined(try client.nearAiAccountEnroll(commons: commons))
            } catch let failure as DaemonClient.Failure {
                // `message` is the daemon's control name -- the enrollment
                // handler answers a label and nothing else, because the
                // errors underneath can quote a remote body or a URL. Empty
                // falls back to the generic label rather than to a blank.
                return .refused(
                    failure.message.isEmpty ? "near_ai_enroll_unavailable" : failure.message)
            } catch {
                return .refused("near_ai_enroll_unavailable")
            }
        }.value
    }

    /// Redeems `invite` for enrollment. Bypasses the `perform` helper (and
    /// its `lastActionError` label) on purpose: that helper renders
    /// `failure.message`, and `enroll`'s failure message must never reach a
    /// screen -- the first run's Folders and Tools steps show the core's
    /// `enroll_refused` for every failure of this call instead
    /// (`FirstRunFailure.enrollFailed`, `FoldersScreenLayout.notice`).
    func enroll(invite: String, scopes: [String] = []) async -> EnrollOutcome {
        guard let client else { return .failed }
        let outcome = await Task.detached(priority: .userInitiated) { () -> EnrollOutcome in
            do {
                return .succeeded(try client.enroll(invite: invite, scopes: scopes))
            } catch {
                return .failed
            }
        }.value
        // Enrolling moves the route off `not_enrolled`; a failure may still
        // have landed, so re-read either way.
        refreshRouteDisclosure()
        return outcome
    }

    /// Records that the NEAR AI first-use notice was shown, and clears the
    /// health label that otherwise keeps the daemon refusing that filter.
    /// Refreshes settings and status afterward so `nearAIConfigured` /
    /// `health` reflect the daemon's own post-acknowledgment state rather
    /// than an assumption made here.
    /// Records that one void notice was shown, then re-reads status so the
    /// daemon's own list, not an assumption made here, decides what stays.
    func acknowledgeGrantVoid(id: UInt64) {
        perform(
            "acknowledge_grant_voids",
            work: { try $0.acknowledgeGrantVoids(ids: [id]) }
        ) { _ in
            self.refreshStatus()
            self.refreshAudit()
        }
    }

    /// Records that one rewording notice was shown (K5), then re-reads
    /// status so the daemon's own list decides what stays.
    func acknowledgeArmingRewording(id: UInt64) {
        perform(
            "acknowledge_arming_rewordings",
            work: { try $0.acknowledgeArmingRewordings(ids: [id]) }
        ) { _ in
            self.refreshStatus()
            self.refreshAudit()
        }
    }

    /// "Dismiss all": records every notice above the tabs that has its own
    /// acknowledgement as shown -- grant voids, arming rewordings and the
    /// legacy-migration notice -- in one round trip each, then re-reads
    /// status so the daemon's own lists decide what stays. It re-arms and
    /// changes nothing else, exactly as each card's own acknowledgement.
    /// The offer to move is put away as its own Dismiss does, and moves
    /// nothing.
    func acknowledgeAllNotices() {
        let voids = status.grantVoids.map(\.id)
        let rewordings = status.armingRewordings.map(\.id)
        let legacy = legacyMigrationNotice != nil
        firstRunNotice = nil
        if showsLegacyMigrationOfferCard { dismissLegacyMigrationOffer() }
        perform(
            "acknowledge_all_notices",
            work: { client in
                if !voids.isEmpty { try client.acknowledgeGrantVoids(ids: voids) }
                if !rewordings.isEmpty { try client.acknowledgeArmingRewordings(ids: rewordings) }
                if legacy { try client.acknowledgeLegacyInviteMigration() }
            }
        ) { _ in
            self.refreshStatus()
            self.refreshAudit()
        }
    }

    /// How many notices "Dismiss all" would acknowledge.
    var acknowledgeableNoticeCount: Int {
        status.grantVoids.count + status.armingRewordings.count
            + (legacyMigrationNotice != nil ? 1 : 0) + (firstRunNotice != nil ? 1 : 0)
            + (showsLegacyMigrationOfferCard ? 1 : 0)
    }

    /// Projects whose "Ask" the daemon refused, by project id, so
    /// the notice can show the Rust's refusal line. Cleared on a retry.
    @Published private(set) var askFirstRefused: Set<String> = []

    /// "Ask" on a rewording or held-folder notice. The same call as
    /// Settings -- `set_project_mode` with the project's id and
    /// `notify_only` -- which also answers a rewording notice. A refusal
    /// changes nothing; the notice stays and says so.
    func askFirst(projectID: String) {
        guard let client else { return }
        askFirstRefused.remove(projectID)
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try client.setProjectMode(projectID: projectID, mode: .ask) }
            await MainActor.run {
                if case .failure = outcome {
                    self.askFirstRefused.insert(projectID)
                }
                self.refreshStatus()
                self.refreshProjects()
                self.refreshAudit()
            }
        }
    }

    /// Records that the legacy invite migration notice was shown, then
    /// re-reads status so the daemon, not this shell, decides it is gone.
    func acknowledgeLegacyInviteMigration() {
        perform(
            "acknowledge_legacy_invite_migration",
            work: { try $0.acknowledgeLegacyInviteMigration() }
        ) { _ in
            self.refreshStatus()
        }
    }

    // MARK: - The offer to move a legacy invite identity

    /// The offer's words, read across the ABI once
    /// (`TCCoreCopy.legacyMigrationOfferJSON`). Nil when the export cannot
    /// be read, and then no offer is drawn anywhere.
    static let coreLegacyMigrationOffer: LegacyMigrationOffer? =
        TCCoreCopy.legacyMigrationOfferJSON().flatMap(LegacyMigrationOffer.decode(fromJSON:))
    var legacyMigrationOffer: LegacyMigrationOffer? { Self.coreLegacyMigrationOffer }
    /// Words a refusal label (`TCCoreCopy.legacyMigrationRefusalLine`). The
    /// ABI in the app; replaced only by tests.
    var legacyMigrationRefusalWording: (String) -> String? = { TCCoreCopy.legacyMigrationRefusalLine(label: $0) }

    /// Whether the daemon offers the move: a legacy invite identity, not yet
    /// moved (`status.legacy_invite_migration.offered`). Settings draws the
    /// offer while this holds; the card above the tabs also needs it not
    /// dismissed (`showsLegacyMigrationOfferCard`).
    var isLegacyMigrationOffered: Bool {
        statusRead == .answered && status.legacyInviteMigration.offered && legacyMigrationOffer != nil
    }

    /// Whether the offer is drawn: while the daemon offers the move, and
    /// after it stops because it refused this identity for good (a pooled
    /// invite), for as long as that refusal is held, so the refusal is read
    /// rather than vanishing with the offer. Only for the identity that was
    /// refused.
    var isLegacyMigrationOfferDrawn: Bool {
        if isLegacyMigrationOffered { return true }
        guard statusRead == .answered, legacyMigrationOffer != nil, legacyMigrationRefusal != nil,
            let refused = legacyMigrationRefusalTenantID
        else { return false }
        return status.tenantID == refused
    }

    /// Move is drawn only while the daemon offers the move.
    var offersLegacyMigrationMove: Bool { isLegacyMigrationOffered }

    var showsLegacyMigrationOfferCard: Bool {
        isLegacyMigrationOfferDrawn && !isLegacyMigrationOfferDismissed
    }

    /// Whether the card was dismissed on this Mac for this identity. A local
    /// marker: no daemon record fits (`acknowledge_legacy_invite_migration`
    /// is the notice after a move), and the card is this shell's alone.
    /// Keyed by a digest of the tenant, so a different invite identity
    /// enrolled later is offered the move afresh.
    var isLegacyMigrationOfferDismissed: Bool {
        guard let key = Self.legacyMigrationOfferDismissedKey(status.tenantID) else { return false }
        return UserDefaults.standard.bool(forKey: key)
    }

    /// Puts the card away for good on this Mac. The offer stays in Settings.
    /// Computed from `UserDefaults`, so the write is announced, as
    /// `markOnboardingComplete` does.
    func dismissLegacyMigrationOffer() {
        guard let key = Self.legacyMigrationOfferDismissedKey(status.tenantID) else {
            refreshStatus()
            return
        }
        objectWillChange.send()
        UserDefaults.standard.set(true, forKey: key)
    }

    static func legacyMigrationOfferDismissedKey(_ tenantID: String?) -> String? {
        guard let tenantID, !tenantID.isEmpty else { return nil }
        let digest = SHA256.hash(data: Data(tenantID.utf8))
        return "trace_commons.legacy_migration_offer_dismissed."
            + digest.map { String(format: "%02x", $0) }.joined()
    }

    /// The move is running. The offer shows its working line in place of
    /// the button meanwhile.
    @Published private(set) var legacyMigrationBusy = false
    /// The core's sentence for the last refused move, or nil.
    @Published private(set) var legacyMigrationRefusal: String?
    /// The identity the held refusal was for, so another identity enrolled
    /// on this Mac does not inherit it.
    private var legacyMigrationRefusalTenantID: String?
    /// The daemon could not tell which invite this device joined with
    /// (`legacy_migration_invite_needed`): the offer asks for it, and the
    /// next Move sends it. Stays until a move succeeds.
    @Published private(set) var legacyMigrationInviteNeeded = false
    /// The move could not be started at all (no daemon answer).
    @Published private(set) var legacyMigrationStartFailed = false
    /// The move is waiting on the NEAR AI login step in the browser.
    @Published private(set) var legacyMigrationAwaitingNearAILogin = false
    /// The NEAR AI login step ended without finishing; nothing was moved.
    @Published private(set) var legacyMigrationNearAILoginFailed = false
    /// The NEAR AI login step: the first run's browser step (`nearAILogin`),
    /// where the person signs in to NEAR AI and the daemon keeps the
    /// session the move provisions the account with. True once it finished.
    /// Replaced only by tests.
    var legacyMigrationNearAILoginStep: (() async -> Bool)?
    /// Ends the browser step the NEAR AI login step began (`cancelNearAILogin`).
    /// Replaced only by tests.
    var legacyMigrationNearAILoginCancel: (() async -> Void)?
    /// The running NEAR AI login step, so Cancel can stop it.
    private var legacyMigrationNearAILoginTask: Task<Bool, Never>?

    /// Cancel while waiting on the NEAR AI login: stop waiting and end the
    /// browser step. Nothing is moved, and the offer stays.
    func cancelLegacyMigrationNearAILogin() {
        guard let task = legacyMigrationNearAILoginTask else { return }
        task.cancel()
        let cancel = legacyMigrationNearAILoginCancel ?? { [weak self] in _ = await self?.cancelNearAILogin() }
        Task { await cancel() }
    }

    /// Runs the NEAR AI login step once, as its own task so Cancel can stop it.
    /// True when it finished; false when it failed, ran out of time or was
    /// cancelled (then `legacyMigrationNearAILoginFailed` stays false).
    private func runLegacyMigrationNearAILoginStep() async -> Bool {
        let step = legacyMigrationNearAILoginStep ?? { [weak self] in await self?.nearAILogin() ?? false }
        legacyMigrationAwaitingNearAILogin = true
        let task = Task { await step() }
        legacyMigrationNearAILoginTask = task
        let finished = await task.value
        legacyMigrationNearAILoginTask = nil
        legacyMigrationAwaitingNearAILogin = false
        if !finished && !task.isCancelled { legacyMigrationNearAILoginFailed = true }
        return finished && !task.isCancelled
    }

    /// Move: `legacy_invite_migrate`, which the contributor chose. A refusal
    /// changes nothing and is said in the core's words; on success status is
    /// re-read, the daemon stops offering the move and its notice takes
    /// over. `invite` is sent only once the daemon asked for it.
    ///
    /// The move leads with the NEAR AI login step (owner, 2026-10-11): when the
    /// daemon has no session to provision the account with
    /// (`legacy_migration_no_near_ai_session`), this runs the NEAR AI login step
    /// and then the move again, once, without a second press. A step that
    /// does not finish moves nothing.
    func migrateLegacyInvite(invite: String? = nil) async {
        guard let client, !legacyMigrationBusy else { return }
        let pasted = invite?.trimmingCharacters(in: .whitespacesAndNewlines)
        let sent = legacyMigrationInviteNeeded && !(pasted ?? "").isEmpty ? pasted : nil
        legacyMigrationBusy = true
        legacyMigrationRefusal = nil
        legacyMigrationStartFailed = false
        legacyMigrationNearAILoginFailed = false
        let move = {
            await Task.detached(priority: .userInitiated) {
                Result { try client.migrateLegacyInvite(invite: sent) }
            }.value
        }
        var outcome = await move()
        if case .failure(let failure as DaemonClient.Failure) = outcome,
            failure.message == LegacyMigrationOffer.nearAILoginNeededLabel, self.client === client
        {
            guard await runLegacyMigrationNearAILoginStep(), self.client === client else {
                legacyMigrationBusy = false
                refreshStatus()
                return
            }
            outcome = await move()
        }
        legacyMigrationBusy = false
        guard self.client === client else { return }
        switch outcome {
        case .success:
            recordRead("legacy_invite_migrate", answered: true)
            legacyMigrationInviteNeeded = false
        case .failure(let failure as DaemonClient.Failure):
            recordRead("legacy_invite_migrate", answered: false)
            // `message` is a fixed `legacy_migration_*` label by contract.
            if failure.message == LegacyMigrationOffer.inviteNeededLabel {
                legacyMigrationInviteNeeded = true
            }
            legacyMigrationRefusal = legacyMigrationRefusalWording(failure.message)
            legacyMigrationRefusalTenantID = status.tenantID
            if legacyMigrationRefusal == nil { legacyMigrationStartFailed = true }
        case .failure:
            recordRead("legacy_invite_migrate", answered: false)
            legacyMigrationStartFailed = true
        }
        refreshStatus()
    }

    /// Void notices whose "Turn back on" the daemon refused, by notice id,
    /// so the card can show the Rust's refusal line. Cleared on a retry.
    @Published private(set) var grantVoidRearmRefused: Set<UInt64> = []

    /// "Turn back on" on a project's void notice. The same call as arming a
    /// project in Settings -- `set_project_mode` with the project's id and
    /// `auto_upload` -- so the daemon applies the same refusals, writes the
    /// same `armed-auto-upload` row, and clears the notice itself. A refusal
    /// changes nothing; the notice stays and says so.
    func rearmGrantVoid(id: UInt64, projectID: String) {
        guard let client else { return }
        grantVoidRearmRefused.remove(id)
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try client.setProjectMode(projectID: projectID, mode: .autoUpload) }
            await MainActor.run {
                if case .failure = outcome {
                    self.grantVoidRearmRefused.insert(id)
                }
                self.refreshStatus()
                self.refreshProjects()
                self.refreshAudit()
            }
        }
    }

    func acknowledgeNearAINotice() {
        perform(
            "acknowledge_near_ai_notice",
            work: { try $0.acknowledgeNearAINotice() }
        ) { _ in
            self.refreshSettings()
            self.refreshStatus()
            self.refreshAudit()
        }
    }

    func refreshConsentOptions() {
        perform("consent_options", work: { try $0.consentOptions() }) {
            self.publishIfChanged(\.consentScopes, $0)
        }
    }

    enum SetScopesOutcome: Equatable {
        case succeeded([String])
        /// Deliberately carries no message, matching `EnrollOutcome.failed`:
        /// `set_consent_scopes` only reports `not-logged-in` (this call
        /// only ever runs after `enroll` already succeeded, so that should
        /// not be reachable) or a local config-write failure, neither of
        /// which is more actionable to a contributor than a flat retry.
        case failed
    }

    /// Applies the consent scopes chosen on the first run's Uses screen
    /// (`FirstRunCall.setConsentScopes`, the first call of Start). Bypasses
    /// `perform` (like `enroll`) so the first run's runner can await the
    /// outcome and stop Start with `scopesFailed` unless the daemon actually
    /// recorded the choice. This call, not `enroll`, applies scopes in this
    /// app's flow: `FirstRunPlan` enrolls with none.
    func setConsentScopes(_ scopes: [String]) async -> SetScopesOutcome {
        guard let client else { return .failed }
        let outcome: SetScopesOutcome = await Task.detached(priority: .userInitiated) {
            do {
                return .succeeded(try client.setConsentScopes(scopes))
            } catch {
                return .failed
            }
        }.value
        if case .succeeded(let confirmed) = outcome {
            status.consentScopes = confirmed
            refreshStatus()
            refreshAudit()
        } else if let confirmed = await Task.detached(operation: { try? client.status() }).value {
            recordRead("status", answered: true)
            publishIfChanged(\.status, confirmed)
        }
        return outcome
    }

    // MARK: - Settings section state
    //
    // The Settings window draws a fresh section view per section
    // (`.id(section)`), so a section's `@State` is thrown away by switching
    // section. What must survive that -- a write in flight, its refusal, a
    // half-typed draft -- lives here, where every section view reads it and
    // none owns it (G8 of #1229).

    /// A Settings consent write is in flight. While it is, every consent row
    /// is disabled, wherever it is drawn.
    @Published private(set) var consentWriteBusy = false
    /// The last Settings consent write was refused. The words are the core's,
    /// chosen when drawn (`ConsentScopeRows.refusalLine`).
    @Published private(set) var consentWriteRefused = false

    /// Adds or removes one optional scope from Settings, sending the daemon's
    /// own list with this one changed. The list is built when the write
    /// starts, from what the daemon reported, and no second write starts
    /// until the first has answered: a list built while one is in flight
    /// would still hold the scope that write withdraws.
    func toggleConsentScope(_ scope: ConsentScope, granted: Bool, options: [ConsentScope]) async {
        guard !consentWriteBusy, status.loggedIn, !scope.alwaysOn else { return }
        let before = status.consentScopes
        let scopes = ConsentScopeRows.nextScopes(
            reported: before, options: options, toggling: scope, granted: granted)
        consentWriteRefused = false
        consentWriteBusy = true
        let outcome = await setConsentScopes(Array(scopes))
        if case .failed = outcome {
            consentWriteRefused = true
        }
        consentWriteBusy = false
        offerSentTracesTraining(outcome, before: before, at: .settings)
    }

    /// Train coding models was just turned on: offer it, once, on the
    /// traces already sent without it (`SentTracesTrainingModel`). Only
    /// after the daemon confirmed the write.
    private func offerSentTracesTraining(
        _ outcome: SetScopesOutcome, before: [String], at site: SentTracesTrainingSite
    ) {
        guard case .succeeded(let confirmed) = outcome,
            SentTracesTrainingState.turnsOn(before: before, after: confirmed)
        else { return }
        Task { await sentTracesTraining.offerAfterTurnOn(at: site) }
    }

    /// A Watched folders write is in flight; the card is disabled until it
    /// answers.
    @Published private(set) var sourceRootBusy = false
    /// The last Watched folders write was refused.
    @Published private(set) var sourceRootSaveFailed = false

    func saveSourceRoot(_ kind: SourceKind, _ choice: SourceChoice) async {
        guard let params = choice.settingsParams(for: kind) else {
            // Nothing to write is a refusal, as it always was.
            if !sourceRootBusy { sourceRootSaveFailed = true }
            return
        }
        await saveSettings(params)
    }

    /// One Watched folders declaration, written through `set_settings` with
    /// the section's in-flight flag and refusal: a source row's, or the
    /// exported-traces row's off switch.
    func saveSettings(_ params: [String: Any]) async {
        guard !sourceRootBusy else { return }
        sourceRootBusy = true
        sourceRootSaveFailed = false
        sourceRootSaveFailed = !(await setSettingsDeclaration(params))
        sourceRootBusy = false
    }

    /// The login item's last refusal, in the words it was drawn with.
    @Published var loginItemActionError: String?
    /// The routing card's edited form; `nil` means nothing has been edited
    /// and the card reads the daemon's answer.
    @Published var routingDraft: RoutingForm?
    /// The witness card's edited fields; `nil` means nothing has been edited.
    @Published var witnessDraft: WitnessForm?
    /// The public profile's edited handle and bio; `nil` means that field
    /// has not been edited and reads the daemon's answer. On the model so a
    /// section switch keeps an edit, and so a background refresh of the
    /// profile cannot rewrite what is being typed.
    @Published var profileHandleDraft: String?
    @Published var profileBioDraft: String?

    @Published private(set) var inferenceEvidenceBusy = false
    @Published private(set) var inferenceEvidenceSaveFailed = false

    func setInferenceEvidence(_ enabled: Bool, disclosureConfirmed: Bool = false) async {
        guard !inferenceEvidenceBusy else { return }
        inferenceEvidenceSaveFailed = false
        guard let client else {
            daemonSettings?.ironwireAttestedBodies = nil
            inferenceEvidenceSaveFailed = true
            return
        }
        inferenceEvidenceBusy = true
        defer { inferenceEvidenceBusy = false }
        let result = await Task.detached(priority: .userInitiated) {
            Result { try client.setInferenceEvidence(enabled, disclosureConfirmed: disclosureConfirmed) }
        }.value
        switch result {
        case .success(let settings):
            daemonSettings = settings
            refreshAudit()
        case .failure:
            if let confirmed = await Task.detached(operation: { try? client.settings() }).value {
                daemonSettings = confirmed
            }
            inferenceEvidenceSaveFailed = true
        }
        // A lost answer does not prove the write failed, and an unchanged
        // settings view triggers no re-read of its own: ask the daemon.
        refreshRouteDisclosure()
    }

    @Published private(set) var tokenStorageNotice = ""
    func setLocalTokenCapture(_ enabled: Bool) async {
        guard !tokenContributionBusy, let client else { return }
        tokenContributionBusy = true
        tokenStorageNotice = ""
        defer { tokenContributionBusy = false }
        let result = await Task.detached(priority: .userInitiated) {
            Result { try client.setSettings(["token_capture_enabled": enabled]) }
        }.value
        switch result {
        case .success(let settings): daemonSettings = settings
        case .failure: tokenStorageNotice = daemonSettings?.tokenStorage?.failureLine ?? ""
        }
    }
    func cleanTokenStorage(discard: Bool) async {
        guard !tokenContributionBusy, let client else { return }
        tokenContributionBusy = true
        tokenStorageNotice = ""
        defer { tokenContributionBusy = false }
        let result = await Task.detached(priority: .userInitiated) {
            Result { try client.tokenStorageAction(discard: discard) }
        }.value
        switch result {
        case .success(let status): daemonSettings?.tokenStorage = status
        case .failure: tokenStorageNotice = daemonSettings?.tokenStorage?.failureLine ?? ""
        }
    }

    /// The Insights switches that are daemon settings, by their key.
    enum InsightsDaemonSwitch: String, CaseIterable, Hashable, Sendable {
        /// Count Private AI tokens (`insights_ledger_feed`, on by default).
        case ledgerFeed = "insights_ledger_feed"
        /// Count tokens in watched folders (`insights_counter_pass`, on by
        /// default and always on; product owner, 2026-10-29). Settings draws
        /// no switch for it.
        case counterPass = "insights_counter_pass"
        /// The weekly summary card (`insights_recap_card_enabled`).
        case recapCard = "insights_recap_card_enabled"

        /// The daemon's value; `nil` from a daemon that does not report it.
        func value(_ settings: DaemonSettingsView) -> Bool? {
            switch self {
            case .ledgerFeed: settings.insightsLedgerFeed
            case .counterPass: settings.insightsCounterPass
            case .recapCard: settings.insightsRecapCardEnabled
            }
        }
    }

    /// The Insights switches with a write in flight; each takes no other.
    @Published private(set) var insightsSwitchBusy: Set<InsightsDaemonSwitch> = []
    /// The core's words for a write the daemon refused or did not confirm
    /// (`MonitorScreensCopy.requestFailed`), by switch; cleared by that
    /// switch's next confirmed write.
    @Published private(set) var insightsSwitchRefusal: [InsightsDaemonSwitch: String] = [:]

    /// A ledger-feed write is in flight; the switch takes no other.
    var insightsLedgerFeedBusy: Bool { insightsSwitchBusy.contains(.ledgerFeed) }
    /// The refusal said under the ledger-feed switch.
    var insightsLedgerFeedRefusal: String? { insightsSwitchRefusal[.ledgerFeed] }

    /// Turns the Insights ledger feed on or off (`insights_ledger_feed`).
    func setInsightsLedgerFeed(_ on: Bool) async {
        await setInsightsSwitch(.ledgerFeed, on)
    }

    /// Turns one Insights daemon switch on or off, sending only its key.
    /// Nothing is applied optimistically: the switch reads
    /// `daemonSettings`, and only a reply that echoes what was asked counts
    /// as confirmed. A reply carrying another value is where the daemon
    /// stands, kept and refused; a failed write reads the settings back and
    /// never claims the asked value.
    func setInsightsSwitch(_ which: InsightsDaemonSwitch, _ on: Bool) async {
        guard !insightsSwitchBusy.contains(which) else { return }
        guard let client else {
            insightsSwitchRefusal[which] = MonitorWords.table?.requestFailed
            return
        }
        insightsSwitchBusy.insert(which)
        defer { insightsSwitchBusy.remove(which) }
        let key = which.rawValue
        let result = await Task.detached(priority: .userInitiated) {
            Result { try client.setSettings([key: on]) }
        }.value
        switch result {
        case .success(let settings) where which.value(settings) == on:
            daemonSettings = settings
            insightsSwitchRefusal[which] = nil
        case .success(let settings):
            if which.value(settings) != nil { daemonSettings = settings }
            insightsSwitchRefusal[which] = MonitorWords.table?.requestFailed
        case .failure:
            if let confirmed = await Task.detached(operation: { try? client.settings() }).value {
                daemonSettings = confirmed
            }
            insightsSwitchRefusal[which] = MonitorWords.table?.requestFailed
        }
    }

    @Published private(set) var tokenContributionBusy = false
    @Published private(set) var tokenContributionSaveFailed = false

    func setTokenContribution(_ enabled: Bool, disclosureConfirmed: Bool = false) async {
        guard !tokenContributionBusy else { return }
        tokenContributionSaveFailed = false
        guard let client else {
            daemonSettings?.tokenDistributionsContribution = nil
            tokenContributionSaveFailed = true
            return
        }
        tokenContributionBusy = true
        defer { tokenContributionBusy = false }
        let result = await Task.detached(priority: .userInitiated) {
            Result { try client.setTokenContribution(enabled, disclosureConfirmed: disclosureConfirmed) }
        }.value
        switch result {
        case .success(let settings):
            daemonSettings = settings
            refreshAudit()
        case .failure:
            if let confirmed = await Task.detached(operation: { try? client.settings() }).value {
                daemonSettings = confirmed
            }
            tokenContributionSaveFailed = true
        }
    }

    // MARK: - Onboarding resume

    /// Whether the core has said enough to know if onboarding is required
    /// (`LaunchRouting.onboardingKnown`), in one spelling for the launch,
    /// the Monitor's gates and every request's routing.
    var onboardingKnown: Bool {
        LaunchRouting.onboardingKnown(startup: startup, statusAnswered: status.answered, statusFailed: statusReadFailed)
    }

    /// What the launch's own request opens (`LaunchRouting.launchOpening`).
    var launchOpening: LaunchRouting.LaunchOpening {
        LaunchRouting.launchOpening(startup: startup, statusAnswered: status.answered, requiresOnboarding: requiresOnboarding,
                                    background: LaunchRouting.isBackgroundLaunch(CommandLine.arguments))
    }

    /// Whether the first run has been finished (Start on the Uses screen)
    /// for the *currently enrolled* device. Keyed off `status.tenantID`
    /// rather than a single global flag: `enroll` alone flips
    /// `status.loggedIn` to true (it happens when Folders or Tools commits,
    /// before the data uses are chosen on Uses), so `loggedIn` cannot by
    /// itself distinguish "fully
    /// onboarded" from "enrolled but consent was never confirmed." A
    /// contributor who quit mid-flow must come back to the rest of
    /// onboarding, not straight to the main window with whatever scopes
    /// `enroll`'s floor-only default happened to leave in place -- see the
    /// coordinator's atomicity note.
    ///
    /// The app needs a NEAR AI login (owner, 2026-10-12: sign-in
    /// required): while the daemon holds no enrollment the first run is
    /// required, whatever an earlier watch-only run wrote
    /// (`isWatchOnlyComplete` no longer opens the app), so a sign-out sends
    /// the person back to sign in. Before the first status arrives
    /// `loggedIn` reads false, so onboarding reads required until then;
    /// the hosts wait on `onboardingKnown` before acting on it.
    var requiresOnboarding: Bool {
        if startup == .needsRoots { return true }
        if Self.reviewSkipsSignIn { return false }
        return !status.loggedIn || !isOnboardingComplete
    }

    /// Debug builds only: `TC_REVIEW_SKIP_SIGN_IN=1` opens the main window
    /// without the NEAR AI sign-in, so a design review can reach the screens
    /// behind it (owner, 2026-10-12). A release build never reads it.
    static var reviewSkipsSignIn: Bool {
        #if DEBUG
        ProcessInfo.processInfo.environment["TC_REVIEW_SKIP_SIGN_IN"] == "1"
        #else
        false
        #endif
    }

    /// Whether a watch-only first run was finished against this config
    /// directory. Keyed by a digest of the directory, never the path itself.
    var isWatchOnlyComplete: Bool {
        guard let key = Self.watchOnlyCompleteKey(configDirectory) else { return false }
        return UserDefaults.standard.bool(forKey: key)
    }

    var isOnboardingComplete: Bool {
        guard let tenantID = status.tenantID, !tenantID.isEmpty else { return false }
        // A daemon-backed CLI completion is additional proof for the current
        // enrollment, not a preference to copy into a tenant's legacy marker.
        // Keep GUI completions and older-daemon behavior intact.
        return status.setupComplete == true
            || UserDefaults.standard.bool(forKey: Self.onboardingCompleteKey(tenantID))
    }

    /// The GUI's legacy completion marker is computed from `UserDefaults`,
    /// so writing that key changes nothing SwiftUI is watching. Shared CLI
    /// completion arrives through the published status instead.
    /// Without the explicit `objectWillChange`, pressing Done
    /// updated the marker and left the contributor sitting on the Done
    /// screen until some *unrelated* published value happened to change --
    /// and `publishIfChanged` exists precisely to stop that from happening,
    /// so on a quiet daemon it never did. Do not remove this send without
    /// making the marker itself observable.
    func markOnboardingComplete() {
        guard let tenantID = status.tenantID else {
            // No tenant to key the marker to yet: the button must not be
            // inert. Re-ask the daemon so the next press has one.
            refreshStatus()
            return
        }
        objectWillChange.send()
        UserDefaults.standard.set(true, forKey: Self.onboardingCompleteKey(tenantID))
    }

    #if DEBUG
    /// Test seam: `status` is `private(set)` and otherwise only ever set
    /// from a live daemon reply, so there is no other way to exercise the
    /// tenant-keyed onboarding marker without a running daemon and a real
    /// enrollment. Debug-only, and deliberately routed through
    /// `publishIfChanged` so a test observes exactly what the app does.
    func setClientForTesting(_ client: DaemonClient) {
        self.client = client
    }
    func setDaemonSettingsForTesting(_ settings: DaemonSettingsView) { publishIfChanged(\.daemonSettings, settings) }
    func setStartupForTesting(_ startup: Startup) { self.startup = startup }
    func setHarnessesForTesting(_ harnesses: HarnessList) { publishIfChanged(\.harnesses, harnesses) }
    func setArmingOfferForTesting(_ offer: ArmingOffer?) { publishIfChanged(\.armingOffer, offer) }
    func setConfigDirectoryForTesting(_ path: String) { configDirectory = path }
    func clearWatchOnlyMarkerForTesting() {
        guard let key = Self.watchOnlyCompleteKey(configDirectory) else { return }
        UserDefaults.standard.removeObject(forKey: key)
    }
    func setHistoryForTesting(_ history: [HistoryRecord]) { publishIfChanged(\.history, history) }

    func setStatusForTesting(_ status: DaemonStatus) {
        recordRead("status", answered: true)
        publishIfChanged(\.status, status)
    }
    #endif

    private static func onboardingCompleteKey(_ tenantID: String) -> String {
        "trace_commons.onboarding_complete.\(tenantID)"
    }

    private static func watchOnlyCompleteKey(_ configDirectory: String) -> String? {
        guard !configDirectory.isEmpty else { return nil }
        let digest = SHA256.hash(data: Data(configDirectory.utf8))
        return "trace_commons.watch_only_complete." + digest.map { String(format: "%02x", $0) }.joined()
    }

    func refreshOutcomeCounts() {
        perform("queue_outcome_counts", work: { try $0.queueOutcomeCounts() }) {
            self.publishIfChanged(\.outcomeCounts, $0)
        }
    }

    // MARK: - Preview scheduling

    /// Reconciles `pending` against a fresh list from the daemon (a
    /// `snapshot` event, `refreshQueue`, or any other refetch), and does the
    /// one thing a change in that list requires beyond updating `pending`
    /// itself: cancel the scheduled preview for anything that left it for
    /// good (approved, dismissed, expired, superseded -- `dismiss` also
    /// cancels its own preview server-side, but a cancel for an id the
    /// daemon already dropped is a defined no-op, not an error).
    ///
    /// Deliberately does **not** loop over the fresh list requesting a
    /// preview for everything newly waiting -- that was this method's shape
    /// before #353/#357 made the queue's row list a `LazyVStack`. Doing so
    /// here would mean asking the daemon about all 500 entries the instant
    /// a snapshot arrives, which defeats the point of realizing rows lazily
    /// in the first place: the legacy queue row drove `requestPreview(for:)`
    /// from `onAppear` for whatever the viewport realized, so this stayed
    /// proportional to what was on screen. Since R15 the glass Traces tab
    /// reads previews through `TracesStore`, and only `SelfTest` asks here.
    ///
    /// Internal rather than private so a test can land a snapshot and watch
    /// what a view holding this model would see. `pending` is
    /// `@Published private(set)` and there is no other way in; the
    /// eligibility gate on an open preview sheet reads `awaitingDecision`,
    /// which only a snapshot moves, so a test that cannot deliver one
    /// cannot prove the sheet re-reads it.
    func applyPendingUpdate(_ entries: [QueueEntry]) {
        let previousIDs = Set(pending.map(\.entryID))
        publishIfChanged(\.pending, entries)
        publishIfChanged(\.queueAnswered, true)
        let currentIDs = Set(entries.map(\.entryID))
        let vanished = previousIDs.subtracting(currentIDs)
        if !vanished.isEmpty {
            previewTracker.forget(vanished)
            for id in vanished {
                summaries[id] = nil
                summaryErrors[id] = nil
                tooLarge[id] = nil
                cancelPreview(id)
            }
        }
    }

    /// One `preview_request` per card, requirement 1 of the scheduler
    /// design: draw a pending card immediately and never block waiting for
    /// the daemon's answer.
    ///
    /// Called by `SelfTest` since R15; before it, from the legacy queue
    /// row's `onAppear` -- the same trigger #357 introduced
    /// as `requestSummary(for:)`, kept here under the scheduler's name
    /// because what changed is not when a row asks, only what happens once
    /// it does: this goes through the daemon's bounded preview scheduler
    /// (two workers, dedup, a cache, an admission cap) instead of the
    /// client-side `ConcurrencyLimiter` #357 added. Only one bound should
    /// own this work -- the daemon is the one that can see the total across
    /// all three shells (and, later, the approve and upload paths too), so
    /// `ConcurrencyLimiter` is not used here; see its own doc for whether it
    /// still has a reason to exist.
    ///
    /// `previewTracker` is #357's `requestingSummaries` in-flight set,
    /// generalized to the scheduler's five states rather than a plain
    /// "is a call running" flag -- it is what keeps a `LazyVStack` row
    /// recycled during a fast scroll from resending `preview_request` while
    /// the daemon still has the job `queued`/`running`.
    func requestPreview(for entry: QueueEntry) {
        guard let client else { return }
        let id = entry.entryID
        guard summaries[id] == nil,
            summaryErrors[id] == nil,
            tooLarge[id] == nil,
            previewTracker.shouldRequest(id)
        else { return }
        previewTracker.markRequested(id)
        Task.detached(priority: .utility) {
            let outcome = Result { try client.requestPreview(entryID: id) }
            await MainActor.run {
                switch outcome {
                case .success(let result):
                    self.applyPreviewOutcome(result)
                case .failure(let error):
                    self.previewTracker.apply(state: .failed, to: id)
                    self.summaryErrors[id] = (error as? DaemonClient.Failure)?.message
                        ?? "preview-request-failed"
                }
            }
        }
    }

    /// Applies one preview outcome, wherever it arrived from: the immediate
    /// response to `preview_request` (a cache hit, a refusal, or "it's
    /// queued/running now") or the later `preview_ready` event (requirement
    /// 2 of the scheduler design). `queued`/`running` leave every dictionary
    /// alone -- the card keeps reading "Reading it locally..." -- and only
    /// update `previewTracker` so a redundant request is not sent while one
    /// is already in flight.
    private func applyPreviewOutcome(_ result: PreviewRequestResult) {
        previewTracker.apply(state: result.state, to: result.entryID)
        switch result.state {
        case .queued, .running:
            break
        case .ready:
            if let summary = result.summary, summaries[result.entryID] != summary {
                summaries[result.entryID] = summary
                recomputeNothingMatched()
            }
        case .tooLarge:
            let refusal = PreviewTooLarge(
                rawSessionBytes: result.rawSessionBytes ?? 0,
                limitBytes: result.limitBytes ?? 0
            )
            if tooLarge[result.entryID] != refusal {
                tooLarge[result.entryID] = refusal
            }
        case .failed:
            let label = result.label ?? "preview-failed"
            if summaryErrors[result.entryID] != label {
                summaryErrors[result.entryID] = label
            }
        }
    }

    /// Drops a scheduled preview -- requirement 4: sent when a card is
    /// dismissed or leaves the list for good (`applyPendingUpdate` above),
    /// never on every scroll. Fire-and-forget: a `dropped: false` reply is a
    /// defined no-op by contract, and there is nothing actionable to do with
    /// a failure here either way.
    private func cancelPreview(_ entryID: String) {
        guard let client else { return }
        Task.detached(priority: .utility) {
            _ = try? client.cancelPreview(entryID: entryID)
        }
    }

    /// Called by a row's `onAppear`/`onDisappear` with what is currently on
    /// screen. Requirement 3: `preview_visible` decides preview *order*, is
    /// cheap and idempotent, but is meant to be sent once a scroll settles,
    /// not once per frame -- so this only records the change with
    /// `visibilityCoalescer` and (re)starts a debounce timer, cancelling
    /// whatever timer was already waiting. A fast scroll through many rows
    /// therefore produces one send, of whatever was on screen when it
    /// stopped, not one send per row that crossed the viewport.
    func setPreviewVisible(_ entryIDs: Set<String>) {
        visibilityCoalescer.setVisible(entryIDs)
        visibleDebounce?.cancel()
        visibleDebounce = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 250_000_000)
            guard !Task.isCancelled else { return }
            self?.flushVisiblePreviews()
        }
    }

    private func flushVisiblePreviews() {
        guard let client, let ids = visibilityCoalescer.takePendingSend() else { return }
        let idList = Array(ids)
        Task.detached(priority: .utility) {
            _ = try? client.setVisiblePreviews(entryIDs: idList)
        }
    }

    // MARK: - Decisions

    /// One click, one session. Builds and pins the envelope if it was never
    /// previewed, approves, then raises the toast -- see
    /// `docs/superpowers/specs/2026-08-20-one-click-submit-design.md`. This
    /// is also what the preview sheet's `Contribute` button calls: a preview
    /// only means the pin already exists, not a different daemon call.
    ///
    /// `verdict` is the contributor's optional answer to the outcome
    /// question. It defaults to none, and none is sent as an absent
    /// parameter rather than an empty one -- see
    /// `DaemonClient.approveParams`.
    /// `correction` is what the contributor wrote in the correction box.
    /// Blank or absent sends no key, and the call is then exactly the one
    /// this model made before the box existed.
    ///
    /// `completion` reports whether the daemon refused the submission
    /// because the correction contains something credential-shaped. That
    /// refusal gets no toast: the sheet is still on screen, still holding
    /// the text, and shows its own message instead -- see
    /// `CorrectionCopy.credentialHeadline`. Every other outcome toasts as
    /// before and reports `false`.
    func approve(
        _ entry: QueueEntry,
        verdict: ContributorVerdict? = nil,
        correction: String? = nil,
        completion: ((Bool) -> Void)? = nil
    ) {
        // THE PRESS DECIDES WHAT IS SENT. The queue card draws its Submit
        // only for a row the shared table offers one for, and its rows come
        // from `awaitingDecision` so they are never stale -- but the tap
        // still lands after the render that drew the button, and a snapshot
        // can arrive in between. Asked here rather than in the view so
        // every route to a single approval goes through it.
        //
        // A refusal is silent on purpose: the row is about to repaint
        // without its button and with the sentence saying why, which is the
        // answer. An error banner would name a failure that did not happen.
        guard EligibilitySurface.mayProceed(
            entry, in: awaitingDecision, id: \.entryID,
            eligibility: { $0.contributionEligibility }, calls: eligibilityCalls)
        else {
            completion?(false)
            return
        }
        perform("approve", work: {
            try $0.approve(entryID: entry.entryID, verdict: verdict, correction: correction)
        }) { response in
            self.refreshQueue()
            if response.wasRefusedForACorrectionCredential {
                completion?(true)
                return
            }
            self.showToast(for: response, attempted: [entry.entryID])
            completion?(false)
        }
    }

    /// One click, one project: approves every entry `waitingByProject` is
    /// currently showing for `projectID`, which must be the id `entry_value`
    /// publishes (`QueueEntry.projectID`) -- the daemon refuses a label
    /// here. An id naming no project the daemon knows throws a `Failure`
    /// (`bad_params` / `project-id-unrecognized`) that `perform` reports as
    /// `lastActionError`, never as a skip.
    ///
    /// `verdict` applies to every entry the approval covers. The plain
    /// `Submit all` passes none; `Submit all as...` is the opt-in path that
    /// passes one.
    ///
    /// **A GROUP-LEVEL SUBMIT MEANS "ALL ELIGIBLE", NEVER "ALL", AND THE
    /// DAEMON ENFORCES THAT.** Both group selectors act only on entries whose
    /// eligibility is `eligible`, and report how many they left out as
    /// `excluded_ineligible`. This shell used to fan the call out into one
    /// `entry_id` approval per eligible entry and merge the responses; it
    /// does not any more, because a per-project approve has no row to check
    /// and no shell can close that gap from outside. One call, one approval
    /// instant, one hold that covers every entry it took.
    func submitProject(id projectID: String, verdict: ContributorVerdict? = nil) {
        // The ids this shell believes the call covers, for the undo path
        // only. The daemon decides what it actually takes, and its
        // `excluded_ineligible` says how many it did not -- neither is
        // recovered by filtering here.
        let attempted = EligibilitySurface.contributable(
            awaitingDecision.filter { $0.projectID == projectID },
            eligibility: { $0.contributionEligibility }, calls: eligibilityCalls
        ).map(\.entryID)
        perform("approve", work: {
            try $0.approve(projectID: projectID, verdict: verdict)
        }) { response in
            self.refreshQueue()
            self.showToast(for: response, attempted: attempted)
            // What became of the rest, from the daemon's own count and the
            // shared sentence. NEVER BRANCHED ON HERE: the table answers the
            // empty string for zero and for the absence, so a filter that
            // ran and took everything, and one that never ran, both draw
            // nothing without this code knowing which it was.
            // `clamping`, not `Int64(...)`: the field is unsigned on the
            // wire and a plain conversion TRAPS above `Int64.max` rather
            // than wrapping. No honest daemon sends that, which is exactly
            // why it would be a crash nobody had thought about.
            let withheld = Int64(clamping: response.excludedIneligible ?? 0)
            let sentence = self.eligibilityCalls.withheldLine(withheld) ?? ""
            self.lastActionNotice = sentence.isEmpty ? nil : sentence
        }
    }

    /// Renders `response` as the toast and, when it offers one, starts the
    /// recovery affordance behind it.
    ///
    /// `attempted` is the set of ids the caller asked the daemon to approve
    /// -- `response.approved` is only a count, so `ApproveResponse
    /// .approvedEntryIDs` is what recovers which of `attempted` actually
    /// went through, and that recovered set is what Undo drives `cancel`
    /// with.
    private func showToast(for response: ApproveResponse, attempted: [String]) {
        undoTask?.cancel()
        let toast = response.toast
        let approvedIDs = response.approvedEntryIDs(attempted: attempted)
        let startedAt = Date()
        undo = Undo(
            entryIDs: approvedIDs,
            toastLine: toast.line,
            offerUndo: toast.offerUndo,
            approvedAt: startedAt,
            heldSeconds: 0
        )
        guard toast.offerUndo else {
            // Nothing to count up toward -- the toast still needs to be
            // seen, but there is no recovery window behind it.
            return
        }
        undoTask = Task { @MainActor in
            for second in 1...Undo.tickCeiling {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                if Task.isCancelled { return }
                guard var current = undo, current.approvedAt == startedAt else { return }
                current.heldSeconds = second
                undo = current
            }
            // The counter stops; the affordance stays. Only `undoApproval`
            // and `dismissUndo` clear it.
        }
    }

    /// Put the recovery affordance away without cancelling anything. The
    /// contributor is saying "yes, send it", which is the choice they already
    /// made -- so this touches the daemon not at all.
    func dismissUndo() {
        undoTask?.cancel()
        undo = nil
    }

    /// Undo, and be honest when it is too late.
    ///
    /// `cancel` takes one entry at a time and only works while that entry is
    /// still `approved` -- the daemon's uploader can pick an approved entry
    /// up immediately, observed in the self-test, where the entry had
    /// already moved to `failed` before the five seconds elapsed -- so any
    /// one of `undo.entryIDs` can lose the race independently of the rest.
    /// This drives `cancel` once per id and reports honestly if any of them
    /// were too late, rather than claiming a clean undo that some entries
    /// did not get.
    func undoApproval() {
        guard let undo, undo.offerUndo else { return }
        undoTask?.cancel()
        let ids = undo.entryIDs
        self.undo = nil
        guard let client else { return }
        Task.detached(priority: .userInitiated) {
            let tooLate = ids.reduce(into: 0) { count, id in
                let outcome = Result { try client.cancel(entryID: id) }
                if case .failure = outcome { count += 1 }
            }
            await MainActor.run {
                if tooLate == ids.count, ids.count == 1 {
                    self.lastActionError = "Too late to undo -- this one had already left "
                        + "the waiting list. History shows what happened to it."
                } else if tooLate > 0 {
                    self.lastActionError = "Too late to undo \(tooLate) of \(ids.count) -- "
                        + "they had already left the waiting list. History shows what "
                        + "happened to them."
                }
                self.refreshQueue()
                self.refreshHistory()
            }
        }
    }

    func dismiss(_ entry: QueueEntry) {
        perform("dismiss", work: { try $0.dismiss(entryID: entry.entryID) }) { _ in
            self.refreshQueue()
            self.refreshOutcomeCounts()
        }
    }

    // MARK: - Public profile

    /// What the last claim or withdrawal did.
    ///
    /// `published` and `left` both carry `cached`, which is the daemon's
    /// `handle_persisted` and is **not** whether the call worked. By the
    /// time that flag exists the server has already accepted the change, so
    /// both of those cases are successes; `cached == false` only means this
    /// device failed to write its local copy, and the sentence for it says
    /// so without retracting the public fact. Reporting that as `refused`
    /// would tell a contributor their handle is private when it is public.
    enum ProfileOutcome: Equatable {
        case published(cached: Bool)
        case left(cached: Bool)
        /// The daemon or the server refused. Carries the daemon's fixed
        /// label, which by contract is never a path, a token, or a response
        /// body.
        case refused(String)
        /// A refused withdrawal, which needs its own sentence: after one,
        /// the handle is still published.
        case leaveRefused(String)
    }

    /// The cached profile, or `nil` for "not on the roster". `nil` is also
    /// what an unenrolled device gets, which is correct: it has claimed
    /// nothing.
    @Published private(set) var publicProfile: DaemonClient.PublicProfile?
    @Published private(set) var profileOutcome: ProfileOutcome?
    @Published private(set) var profileBusy = false

    func refreshPublicProfile() {
        guard let client else { return }
        Task.detached(priority: .userInitiated) {
            let outcome = try? client.publicProfile()
            await MainActor.run {
                // A failure -- `not-logged-in` above all -- is the
                // off-the-roster state, not an error worth a banner. Whether
                // it may be drawn as one is `publicProfileRead`'s call.
                if let outcome {
                    self.publicProfile = outcome.onRoster ? outcome : nil
                } else if self.statusRead == .answered, !self.status.loggedIn {
                    // Signed out by the daemon's own answer: nothing is
                    // claimed, so the cache goes.
                    self.publicProfile = nil
                }
                // Otherwise a refused refresh keeps the cached profile: an
                // answered read stays answered, and dropping the cache here
                // would draw an on-roster contributor the opt-in card.
                self.recordRead("get_public_profile", answered: outcome != nil)
            }
        }
    }

    /// Claims or updates the public handle.
    ///
    /// Bypasses `perform` for the same reason `withdraw` does: that helper
    /// funnels every failure into `lastActionError` as a bare label, and a
    /// label is not a sentence a contributor can act on when the thing that
    /// was refused is the handle they just typed.
    ///
    /// The profile is taken from the daemon's own answer rather than from
    /// what was sent: the handle it stored is the validated display form,
    /// which is trimmed, and the roster date is the server's.
    func claimHandle(_ handle: String, bio: String) {
        guard let client, !profileBusy else { return }
        profileBusy = true
        profileOutcome = nil
        let trimmedBio = bio.trimmingCharacters(in: .whitespacesAndNewlines)
        // An empty box means "no bio", explicitly. The PUT replaces the
        // whole profile, so there is no "leave it alone" to express.
        let bioParam: String? = trimmedBio.isEmpty ? nil : trimmedBio
        Task.detached(priority: .userInitiated) {
            let result = Result { try client.setPublicProfile(handle: handle, bio: bioParam) }
            await MainActor.run {
                self.profileBusy = false
                switch result {
                case .success(let profile):
                    self.publicProfile = profile.onRoster ? profile : nil
                    // A build that did not report the flag is treated as
                    // having persisted: the alternative is warning about a
                    // cache miss that may not have happened, on a profile
                    // that is public either way.
                    self.profileOutcome = .published(cached: profile.handlePersisted ?? true)
                    // The daemon now holds this profile, in its stored form,
                    // so the fields read it again -- unless the person has
                    // edited past what was sent while the write was out.
                    if (self.profileHandleDraft ?? handle) == handle, (self.profileBioDraft ?? bio) == bio {
                        self.profileHandleDraft = nil
                        self.profileBioDraft = nil
                    }
                    self.refreshStatus()
                    self.refreshAudit()
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? "profile-update-failed"
                    self.profileOutcome = .refused(label)
                }
            }
        }
    }

    /// Withdraws the public handle from the roster.
    func leaveRoster() {
        guard let client, !profileBusy else { return }
        profileBusy = true
        profileOutcome = nil
        Task.detached(priority: .userInitiated) {
            let result = Result { try client.clearPublicProfile() }
            await MainActor.run {
                self.profileBusy = false
                switch result {
                case .success(let profile):
                    self.publicProfile = profile.onRoster ? profile : nil
                    self.profileOutcome = .left(cached: profile.handlePersisted ?? true)
                    // Off the roster there is no profile for an edit to be of.
                    self.profileHandleDraft = nil
                    self.profileBioDraft = nil
                    self.refreshStatus()
                    self.refreshAudit()
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message
                        ?? "profile-withdraw-failed"
                    self.profileOutcome = .leaveRefused(label)
                }
            }
        }
    }

    func clearProfileOutcome() {
        profileOutcome = nil
    }

    // MARK: - Withdrawal

    /// What a withdrawal attempt did, kept per submission so the row that
    /// was acted on can say it rather than a screen-level banner saying it
    /// about nothing in particular.
    enum WithdrawalResult: Equatable {
        /// The server withdrew it, and reported this tier. `nil` reach means
        /// the daemon sent a label this build does not know -- which is
        /// reported as not-knowable, never smoothed into the mild answer.
        case withdrawn(WithdrawalReach?, String? = nil)
        /// The daemon has no account session, so the request was never made.
        case noAccountSession
        /// Anything else. Carries the daemon's fixed label, which by
        /// contract is never a path, a token, or a response body.
        case failed(String)
    }

    @Published private(set) var withdrawals: [String: WithdrawalResult] = [:]
    @Published private(set) var withdrawing: Set<String> = []
    @Published private(set) var sessionDetails: [String: SessionDetail] = [:]
    @Published private(set) var sessionDetailErrors: [String: String] = [:]
    @Published private(set) var loadingSessionDetails: Set<String> = []
    @Published private(set) var publicRunErrors: [String: String] = [:]
    @Published private(set) var publicRunWorking: Set<String> = []
    @Published var skillLearningStore = SkillLearningStore()
    private var accountOwnedContentScope: String?
    /// Stamps each detail read in the order it started. Any number of rows
    /// may be read at once, and each read lands for its own row; one row is
    /// never read twice at once (`loadingSessionDetails`).
    private var sessionDetailRequestSequence: UInt64 = 0
    /// The stamp of the newest read that decided the account content belongs
    /// to: the one that set `accountOwnedContentScope`, or cleared it. A read
    /// that started before it and answers for another account is stale
    /// (#869): it neither replaces nor clears what that newer read decided.
    private var sessionDetailScopeSequence: UInt64 = 0
    @Published private(set) var skillLearningCopy: SkillLearningCopy? = SkillLearningCopy.decode(
        fromJSON: TCSkillLearning.copyJSON() ?? ""
    )
    @Published private(set) var publicRunCopy: PublicRunCopy? = PublicRunCopy.decode(
        fromJSON: TCPublicRun.copyJSON() ?? ""
    )

    /// Withdraws one trace.
    ///
    /// Bypasses `perform` deliberately, like `enroll` does. That helper
    /// funnels every failure into `lastActionError` as `"withdraw:
    /// account-session-required"`, and the two things wrong with that here
    /// are that the label is not a sentence anybody can act on, and that a
    /// screen-level error next to a row that still says "In the commons"
    /// leaves it genuinely ambiguous whether the trace was withdrawn. Both
    /// outcomes are recorded against the submission instead, and the view
    /// states them in words on that row.
    ///
    /// On success the daemon has already updated its own history cache, so
    /// `refreshHistory` is what turns the row over to `withdrawn` -- the
    /// status is re-read rather than assumed here.
    func withdraw(_ record: HistoryRecord) {
        guard let client else { return }
        let id = record.submissionID
        guard !withdrawing.contains(id) else { return }
        withdrawing.insert(id)
        withdrawals[id] = nil
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try client.withdraw(submissionID: id) }
            await MainActor.run {
                self.withdrawing.remove(id)
                switch outcome {
                case .success(let value):
                    self.withdrawals[id] = .withdrawn(value.distributionReach, value.tokenDeletionNote)
                    self.refreshHistory()
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? "withdraw-failed"
                    self.withdrawals[id] = label == "account-session-required"
                        ? .noAccountSession
                        : .failed(label)
                    // The commons rejected the stored session: History's
                    // rows ask for sign-in rather than offer a Retry that
                    // would be refused again.
                    if label == "account-session-required" { self.accountSession = .signedOut }
                }
            }
        }
    }

    /// What the app knows of the account session Withdraw needs (Ron's
    /// #1146 `accountSignedIn`). Only `signedIn` lets a History row offer
    /// Withdraw; anything else offers sign-in, and `checking` says so while
    /// the session is first read.
    enum AccountSessionRead: Equatable {
        case unread
        case checking
        case signedIn
        case signedOut
    }

    /// Why the last sign-in from History did not leave an active session.
    enum AccountSignInFailure: Equatable {
        /// Sign-in returned, and the session read again is not active.
        case inactive
        /// Sign-in returned, and the session could not be read again.
        case unverified
        /// Sign-in did not finish.
        case failed
    }

    /// Where History's refresh request (Ron's #1146 refresh control) stands.
    enum HistoryRefreshState: Equatable {
        case idle
        case requesting
        case requested
        case failed
    }

    @Published private(set) var accountSession: AccountSessionRead = .unread
    /// The last status read's `sign_in`: `near_ai` when Sign in again is the
    /// NEAR AI path (`near_ai_account_sign_in`), nil for an `account_sign_in` account
    /// or before a read.
    @Published private(set) var accountSignInPath: String?
    @Published private(set) var accountSigningIn = false
    /// The last status read's `renewal_failure`: `near_ai_sign_in_required`
    /// when NEAR AI refused the stored login, so only a fresh browser
    /// sign-in helps. Nil when the last read named none, and before a read.
    @Published private(set) var accountRenewalFailure: String?
    /// The Private AI tab's Sign in is running (`signInToNearAI`), the
    /// browser wait included.
    @Published private(set) var nearAiSigningIn = false
    /// The last `signInToNearAI` did not leave a usable login.
    @Published private(set) var nearAiSignInFailed = false
    @Published private(set) var accountSignInFailure: AccountSignInFailure?
    @Published private(set) var historyRefresh: HistoryRefreshState = .idle

    /// Reads the account session (`account_session_status`). An answer
    /// that does not say signed in, or no answer, is not signed in.
    func refreshAccountSession() {
        if accountSession == .unread { accountSession = .checking }
        Task {
            let read = await firstRunCall { try $0.accountSessionStatus() }
            if case .success(let session) = read {
                accountSignInPath = session.signIn
                accountRenewalFailure = session.renewalFailure
            }
            if case .success(let session) = read, session.signedIn == true {
                accountSession = .signedIn
            } else {
                accountSession = .signedOut
            }
        }
    }

    /// Opens a URL in the person's browser. A seam so a test can sign in
    /// without a browser.
    var openBrowser: (URL) -> Void = { NSWorkspace.shared.open($0) }

    /// Sign in again on a NEAR AI account: the daemon runs the ceremony
    /// with the NEAR AI login it holds (`near_ai_account_sign_in`). When
    /// NEAR AI no longer accepts that login (`near_ai_sign_in_required`),
    /// this signs in to NEAR AI in the browser first and asks once more.
    /// The session is then read again, and only that read is trusted.
    func signInAgain() async {
        if nearAiBrowserSignInWaiting {
            await restartNearAIBrowserSignIn()
            return
        }
        guard !accountSigningIn else { return }
        accountSigningIn = true
        accountSignInFailure = nil
        defer { accountSigningIn = false }
        var answer = await firstRunCall { try $0.nearAiAccountSignIn() }
        if case .failure(let failure as DaemonClient.Failure) = answer,
           failure.message == "near_ai_sign_in_required" {
            guard await nearAILogin(fresh: true) else {
                accountSignInFailure = .failed
                return
            }
            answer = await firstRunCall { try $0.nearAiAccountSignIn() }
        }
        guard case .success = answer else {
            accountSignInFailure = .failed
            return
        }
        await rereadAccountSessionAfterSignIn()
    }

    /// Refresh the NEAR AI login explicitly, even when the separate Commons
    /// account session is still valid. Once the browser completes, restore or
    /// reread the Commons session when this account uses NEAR AI sign-in.
    func signInToNearAI() async {
        if nearAiBrowserSignInWaiting {
            await restartNearAIBrowserSignIn()
            return
        }
        guard !nearAiSigningIn, !accountSigningIn else { return }
        nearAiSigningIn = true
        nearAiSignInFailed = false
        defer { nearAiSigningIn = false }
        if accountSignInPath == nil,
           case .success(let session) = await firstRunCall({ try $0.accountSessionStatus() }) {
            accountSignInPath = session.signIn
            accountRenewalFailure = session.renewalFailure
        }
        guard await nearAILogin(fresh: true) else {
            nearAiSignInFailed = true
            return
        }
        if accountSignInPath == "near_ai" {
            await signInAgain()
            nearAiSignInFailed = accountSignInFailure != nil
        } else {
            accountSessionGeneration &+= 1
        }
        refreshNearAiCredential()
        refreshNearAiBalance(fresh: true)
    }

    /// After a sign-in: the session read again, and only that read trusted.
    private func rereadAccountSessionAfterSignIn() async {
        switch await firstRunCall({ try $0.accountSessionStatus() }) {
        case .success(let session) where session.signedIn == true:
            accountSession = .signedIn
            accountSignInPath = session.signIn
            accountRenewalFailure = session.renewalFailure
            // The refusals sign-in answers are forgotten: those rows offer
            // Withdraw again, which asks first.
            withdrawals = withdrawals.filter { $0.value != .noAccountSession }
            accountSessionGeneration &+= 1
        case .success:
            accountSession = .signedOut
            accountSignInFailure = .inactive
        case .failure, nil:
            accountSession = .signedOut
            accountSignInFailure = .unverified
        }
    }

    /// Moves each time a sign-in lands, so a section that read "Sign in
    /// again" (the payout section, Home's Credit tile) reads again.
    @Published private(set) var accountSessionGeneration = 0

    /// Sign in again by whichever path this account has: the NEAR AI one
    /// for a NEAR AI account, `account_sign_in` otherwise. The path comes
    /// from a status read, taken first when none has been.
    func signInToAccount() async {
        if accountSignInPath == nil,
           case .success(let session) = await firstRunCall({ try $0.accountSessionStatus() }) {
            accountSignInPath = session.signIn
        }
        await signInToWithdraw()
    }

    /// Signs in through the native identity path (`account_sign_in`, which
    /// opens the browser), then reads the session again and trusts only
    /// that read, as Ron's History does.
    func signInToWithdraw() async {
        // A NEAR AI account signs in through NEAR AI.
        if accountSignInPath == "near_ai" { return await signInAgain() }
        guard !accountSigningIn else { return }
        accountSigningIn = true
        accountSignInFailure = nil
        defer { accountSigningIn = false }
        guard case .success = await firstRunCall({ try $0.accountSignIn() }) else {
            accountSignInFailure = .failed
            return
        }
        await rereadAccountSessionAfterSignIn()
    }

    /// The refresh control (Ron's #1146 HistoryRefreshControl): asks the daemon's poller to check
    /// the server sooner (`refresh_history`), then reads History again.
    /// True only when the daemon said the refresh was requested.
    @discardableResult
    func requestHistoryRefresh() async -> Bool {
        guard historyRefresh != .requesting else { return false }
        historyRefresh = .requesting
        guard case .success(true) = await firstRunCall({ try $0.refreshHistory() }) else {
            historyRefresh = .failed
            return false
        }
        historyRefresh = .requested
        refreshHistory()
        return true
    }

    /// Forgets the last refresh request's outcome, so a later visit to
    /// History does not show it. A request in flight is left alone.
    func clearHistoryRefresh() {
        if historyRefresh != .requesting { historyRefresh = .idle }
    }

    /// Reads a session's detail. The detail already held stays until the
    /// daemon answers: replaced when it does, kept beside the error when it
    /// fails, so a reload (one runs on every app switch) never unmounts the
    /// public-run editor drawn from it, or the draft typed there. An account
    /// change still clears it (`clearAccountOwnedContent`).
    func loadSessionDetail(_ record: HistoryRecord) {
        let id = record.submissionID
        guard let client else {
            // The core is down: say the read could not be made, with the
            // core's line and Retry, rather than leave the detail empty.
            sessionDetailErrors[id] = TCPublicRun.sessionDetailErrorLine(label: "daemon-unavailable")
            return
        }
        guard !loadingSessionDetails.contains(id) else { return }
        guard !publicRunWorking.contains(id) else { return }
        sessionDetailRequestSequence &+= 1
        let requestSequence = sessionDetailRequestSequence
        loadingSessionDetails.insert(id)
        sessionDetailErrors[id] = nil
        Task.detached(priority: .userInitiated) {
            let result = Result { try client.sessionDetail(submissionID: id) }
            await MainActor.run {
                self.loadingSessionDetails.remove(id)
                let superseded = requestSequence < self.sessionDetailScopeSequence
                switch result {
                case .success(let detail):
                    if superseded, let scope = detail.ownerScopeSHA256, scope != self.accountOwnedContentScope {
                        // Read under an account a newer read has replaced:
                        // not shown, and the row says so, with Retry,
                        // rather than drawing nothing.
                        self.sessionDetailErrors[id] = TCPublicRun.sessionDetailErrorLine(label: "session-owner-changed")
                        return
                    }
                    if detail.ownerScopeSHA256 != nil {
                        self.sessionDetailScopeSequence = max(self.sessionDetailScopeSequence, requestSequence)
                    }
                    self.reconcileAccountOwnedContent(scope: detail.ownerScopeSHA256)
                    self.sessionDetails[id] = detail
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    if !superseded,
                        label == "account-session-required"
                            || label == "session-detail-not-found"
                            || label == "session-owner-changed"
                    {
                        self.sessionDetailScopeSequence = requestSequence
                        self.clearAccountOwnedContent()
                    }
                    self.sessionDetailErrors[id] = TCPublicRun.sessionDetailErrorLine(label: label)
                }
            }
        }
    }

    private func reconcileAccountOwnedContent(scope: String?) {
        guard let scope else { return }
        if let previous = accountOwnedContentScope, previous != scope {
            clearAccountOwnedContent()
        }
        accountOwnedContentScope = scope
    }

    private func clearAccountOwnedContent() {
        accountOwnedContentScope = nil
        sessionDetails.removeAll()
        sessionDetailErrors.removeAll()
        publicRunErrors.removeAll()
        skillLearningStore = SkillLearningStore()
    }

    func publishPublicRun(_ record: HistoryRecord, draft: PublicRunDraftInput) {
        guard let client else { return }
        let id = record.submissionID
        guard let detail = sessionDetails[id],
              let taskSuccess = detail.taskSuccess
        else { return }
        guard !publicRunWorking.contains(id) else { return }
        guard !loadingSessionDetails.contains(id) else { return }
        publicRunWorking.insert(id)
        publicRunErrors[id] = nil
        Task.detached(priority: .userInitiated) {
            let result = Result {
                try client.publishPublicRun(
                    submissionID: id,
                    draft: draft,
                    taskSuccess: taskSuccess,
                    contributedVersion: detail.contributedVersion,
                    expectedPublicationVersion: detail.publicationVersion
                )
            }
            await MainActor.run {
                self.publicRunWorking.remove(id)
                switch result {
                case .success(let page):
                    if var detail = self.sessionDetails[id] {
                        detail.publication = page
                        detail.publicationVersion = page.version
                        self.sessionDetails[id] = detail
                    }
                    if let warning = page.credentialWarning {
                        self.publicRunErrors[id] = TCPublicRun.publicationErrorLine(label: warning)
                    }
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    self.publicRunErrors[id] = TCPublicRun.publicationErrorLine(label: label)
                }
            }
        }
    }

    func unpublishPublicRun(_ record: HistoryRecord) {
        guard let client else { return }
        let id = record.submissionID
        guard !publicRunWorking.contains(id) else { return }
        guard !loadingSessionDetails.contains(id) else { return }
        publicRunWorking.insert(id)
        publicRunErrors[id] = nil
        Task.detached(priority: .userInitiated) {
            let result = Result { try client.unpublishPublicRun(submissionID: id) }
            await MainActor.run {
                self.publicRunWorking.remove(id)
                switch result {
                case .success(let outcome):
                    if var detail = self.sessionDetails[id] {
                        detail.publication = nil
                        detail.publicationVersion = outcome.expectedPublicationVersion
                        self.sessionDetails[id] = detail
                    }
                    if let warning = outcome.credentialWarning {
                        self.publicRunErrors[id] = TCPublicRun.publicationErrorLine(label: warning)
                    }
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    self.publicRunErrors[id] = TCPublicRun.publicationErrorLine(label: label)
                }
            }
        }
    }

    // MARK: - Pause

    func pause(until: Date?) {
        perform("pause", work: { try $0.pause(until: until) }) { _ in self.refreshStatus() }
    }

    func resume() {
        perform("resume", work: { try $0.resume() }) { _ in self.refreshStatus() }
    }

    // MARK: - Preview body

    /// How many times `needle` appears in an entry's pre-redaction session,
    /// or nil when that could not be checked.
    ///
    /// Synchronous: the ABI call scans an already-parsed session and returns
    /// a count, with no redaction pass to block on.
    func searchOriginal(entryID: String, needle: String) -> Int? {
        client?.searchOriginal(entryID: entryID, needle: needle)
    }

    /// The core's turn index over an open preview's body, anchored to that
    /// body's digest (`LookInside.bodyDigest`). Off the main actor: the
    /// core re-resolves the preview to check the anchor.
    func previewTurns(entryID: String, bodyDigest: String) async -> PreviewTurns? {
        guard let client else { return nil }
        return await Task.detached(priority: .userInitiated) {
            client.previewTurns(entryID: entryID, bodyDigest: bodyDigest)
        }.value
    }

    /// Opens the in-process preview off the main actor -- the redaction pass
    /// blocks -- and hands the open handle back on the main actor.
    func supportsWitnessReview() async -> Bool {
        guard let client else { return false }
        return await Task.detached { (try? client.supportsWitnessReview()) == true }.value
    }

    func requestWitnessReview(entryID: String) async -> Bool {
        await witnessReviewOutcome(entryID: entryID).succeeded
    }

    /// A refused review and the sentence the daemon chose for it.
    ///
    /// The `Bool` above is kept for callers that only need to know whether to
    /// reload. This one is for the sheet, which has to say *why*: before it,
    /// every refusal -- a receipt the reviewer declined, a reviewer that was
    /// simply down, one that could not prove itself -- rendered the same
    /// single sentence.
    func witnessReviewOutcome(entryID: String) async -> WitnessReviewOutcome {
        guard let client else { return WitnessReviewOutcome(succeeded: false, sentence: nil) }
        return await Task.detached(priority: .userInitiated) {
            do {
                try client.requestWitnessReview(entryID: entryID)
                return WitnessReviewOutcome(succeeded: true, sentence: nil)
            } catch {
                return WitnessReviewOutcome(
                    succeeded: false,
                    sentence: DaemonClient.refusalSentence(from: error),
                    retryLine: DaemonClient.busyRetryLine(from: error)
                )
            }
        }.value
    }

    func openPreview(entryID: String) async -> PreviewOutcome {
        guard let client else { return .failed("the watcher isn't running") }
        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                // The blocking redaction pass creates a fresh handle here.
                // Resume transfers it once to the main-actor caller; this
                // worker keeps no reference to use after that handoff.
                let outcome: PreviewOutcome
                do {
                    outcome = .opened(try client.openPreview(entryID: entryID))
                } catch {
                    outcome = .failed("\(error)")
                }
                continuation.resume(returning: outcome)
            }
        }
    }

    /// Opens a real preview for the first waiting entry, runs a real search
    /// over the redacted body, and hands back everything the sheet needs to
    /// be rendered without its own async load. Used by the screenshot hook.
    /// A wholly synthetic preview for the screenshot hook.
    ///
    /// This used to open a REAL queued entry and hand its redacted body to
    /// `PreviewSheet`, which was then rasterized to a PNG in a directory the
    /// caller named. That put trace content in a durable file outside the
    /// protected state directory. The preview exemption covers showing
    /// redacted content to the contributor who owns the entry -- it does not
    /// cover writing it to an arbitrary path, and "we only ever point this at
    /// fixtures" is a property of how it is invoked, not of the code.
    ///
    /// The screenshots exist to show what the UI looks like, and a fabricated
    /// transcript does that just as well. Nothing here reads the queue.
    func loadCaptureSample(needle: String) async -> (QueueEntry, PreviewSheet.Preloaded)? {
        let transcript = """
            user: Add a retry to the Northwind billing sync -- it drops the \
            batch when the upstream 503s.

            assistant: I will wrap the call in a bounded retry. The credential \
            was scrubbed from this transcript: [REDACTED:aws_secret_key]

            tool: edit billing/sync.rs
            """
        let summary = PreviewSummary(
            wouldSendBytes: 4160,
            rawSessionBytes: 1615,
            eventCount: 3,
            openingPrompt: "Add a retry to the Northwind billing sync",
            redactions: ["aws_secret_key": 1, "local_path": 3],
            redactionsDistinct: ["aws_secret_key": 1, "local_path": 2],
            piiLabelsPresent: ["email"],
            consentScopes: ["debugging_evaluation"],
            residualRisk: "pattern-based"
        )
        let entry = QueueEntry(
            entryID: "entry_screenshot_fixture",
            sessionHash: "sha256:0000000000000000",
            source: "claude-code",
            declaredSource: nil,
            projectID: "project_screenshot_fixture",
            projectLabel: "northwind-billing",
            projectPath: "~/code/northwind-billing",
            sessionPath: nil,
            sizeBytes: 1615,
            discoveredAt: Date(timeIntervalSince1970: 1_770_000_000),
            state: .pending,
            reasonLabel: nil,
            attempts: 0,
            // A single-file conversation: no delegated transcripts, none
            // dropped, so the card's extent line is absent and the capture
            // shows exactly what it showed before these fields existed.
            subagentCount: 0,
            subagentsDropped: 0,
            // The screenshot fixture stands for an invited contributor: no
            // eligibility field, so the capture shows the card exactly as it
            // looked before this surface existed.
            eligibility: nil,
            eligibilityReason: nil,
            // The same invited contributor still gets a mark: an attested
            // session, which is the state the capture is meant to show.
            attestation: "attested",
            attestationReason: nil,
            // The capture shows a session a certificate is held for, which
            // is the state the certificate-held list is drawn from.
            holdsCertificateRaw: true
        )
        var offsets: [Int] = []
        if !needle.isEmpty {
            var searchRange = transcript.startIndex..<transcript.endIndex
            while let found = transcript.range(of: needle, range: searchRange) {
                offsets.append(transcript.distance(from: transcript.startIndex, to: found.lowerBound))
                searchRange = found.upperBound..<transcript.endIndex
            }
        }
        return (
            entry,
            PreviewSheet.Preloaded(
                summary: summary,
                transcript: transcript,
                needle: needle,
                offsets: offsets
            )
        )
    }

    /// What a preparation did, and the words for it either way.
    ///
    /// The mirror of [`WitnessReviewOutcome`], for the same reason: a `Bool`
    /// cannot carry why.
    struct AdmissionOutcome: Sendable {
        let succeeded: Bool
        /// The daemon's classified sentence, or `nil` when it sent none and
        /// the caller should keep its own fallback.
        let sentence: String?
    }

    /// What a witness review did, and the words for it when it refused.
    struct WitnessReviewOutcome: Sendable {
        let succeeded: Bool
        /// The daemon's classified sentence, or `nil` when it sent none and
        /// the caller should keep its own fallback.
        let sentence: String?
        /// Set only when the witness was busy: when the person may try the
        /// review again. A busy witness judged nothing, so it is not a
        /// refusal.
        var retryLine: String? = nil
    }

    enum PreviewOutcome {
        case opened(TCPreview)
        /// A fixed label from the ABI, safe to show: it never carries a
        /// path, a token, or trace content.
        case failed(String)
    }

    // MARK: - Where each Settings read stands

    /// Labels of the reads that have answered at least once. Kept apart from
    /// the data: a list that answered empty and one that never answered are
    /// both `[]`, and only this tells them apart.
    @Published private(set) var answeredReads: Set<String> = []
    /// Labels of the reads whose last call failed. A later success clears it.
    @Published private(set) var failedReads: Set<String> = []

    private func recordRead(_ label: String, answered: Bool) {
        if answered {
            if !answeredReads.contains(label) { answeredReads.insert(label) }
            if failedReads.contains(label) { failedReads.remove(label) }
        } else if !failedReads.contains(label) {
            failedReads.insert(label)
        }
    }

    private func read(_ label: String, answered: Bool = false) -> SettingsRead {
        SettingsRead.resolve(
            answered: answered || answeredReads.contains(label),
            failed: failedReads.contains(label),
            startup: startup)
    }

    /// `status`. The placeholder comparison stays for answers that reach the
    /// model some other way; the recorded answer covers a signed-out status
    /// that decodes equal to the placeholder.
    var statusRead: SettingsRead { read("status", answered: status.answered) }
    /// Managed-session settings, including the saved CLI options.
    var managedCLIOptionsRead: SettingsRead { read("managed_snapshot", answered: managedSnapshot != nil) }
    /// `get_settings`, or any write that handed back the settings.
    var settingsRead: SettingsRead { read("get_settings", answered: daemonSettings != nil) }
    /// `list_audit`: what "Nothing has been changed." waits on.
    var auditRead: SettingsRead { read("list_audit") }
    /// `list_projects`: what "No projects seen yet." waits on.
    var projectsRead: SettingsRead { read("list_projects") }
    /// The cached public profile. A failed read while signed in may be
    /// hiding a roster entry, so it is a failure; signed out (by the
    /// daemon's own answer) it is the daemon saying nothing is claimed.
    var publicProfileRead: SettingsRead {
        let signedOutRefusal = failedReads.contains("get_public_profile")
            && statusRead == .answered && !status.loggedIn
        return read("get_public_profile", answered: publicProfile != nil || signedOutRefusal)
    }
    /// The witness state. It is read from the config directory, not the
    /// daemon, so it answers even when the daemon refused -- unless the
    /// directory never resolved, when no read can ever run.
    var witnessRead: SettingsRead {
        if witnessStateCode != nil { return .answered }
        if configDirectory.isEmpty, read("witness") == .coreDown { return .coreDown }
        return .awaiting
    }

    // MARK: - Plumbing

    private func perform<T: Sendable>(
        _ label: String,
        work: @escaping @Sendable (DaemonClient) throws -> T,
        onFailure: (@MainActor @Sendable () -> Void)? = nil,
        onSuccess: @escaping @MainActor @Sendable (T) -> Void
    ) {
        guard let client else { return }
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try work(client) }
            await MainActor.run {
                switch outcome {
                case .success(let value):
                    self.recordRead(label, answered: true)
                    onSuccess(value)
                case .failure(let error):
                    self.recordRead(label, answered: false)
                    // `error.message` is a fixed label by contract, never a
                    // path, a token, or a server response body.
                    if let failure = error as? DaemonClient.Failure {
                        self.lastActionError = "\(label): \(failure.message)"
                    } else {
                        self.lastActionError = "\(label): failed"
                    }
                    onFailure?()
                }
            }
        }
    }
}

// MARK: - First run

/// The live first-run calls. Here rather than beside `FirstRunRunner` because
/// they need `client` and `daemon`, which stay private. Every call answers an
/// outcome the runner can stop on, so the fire-and-forget paths
/// (`setProjectMode(_:mode:)`, `applyPrivateInference(_:)`) are not used:
/// one drops a call while another is in flight, and neither reports back.
extension AppModel: FirstRunDaemon {
    func startDaemon(settingsJSON: String) async -> Bool {
        // Already running (a returning person whose roots were declared):
        // the declaration goes through `set_settings` instead.
        if daemon != nil { return await setSourceSettings(settingsJSON: settingsJSON) }
        // `startDaemon(at:...)` returns without calling back in these cases,
        // so awaiting it would never end.
        guard !daemonStartup.isStarting, !configDirectory.isEmpty else { return false }
        let path = configDirectory
        return await withCheckedContinuation { continuation in
            startDaemon(at: path, settingsJSON: settingsJSON) { startup in
                continuation.resume(returning: startup == .running)
            }
        }
    }

    func setSourceSettings(settingsJSON: String) async -> Bool {
        let result = await firstRunCall { client in
            // Only the serialized input crosses to the worker. The untyped
            // dictionary stays local to the call that validates and sends it.
            let object = try JSONSerialization.jsonObject(with: Data(settingsJSON.utf8))
            guard let declarations = object as? [String: Any] else {
                throw CocoaError(.coderInvalidValue)
            }
            return try client.setSettings(declarations)
        }
        guard case .success(let settings) = result else { return false }
        publishIfChanged(\.daemonSettings, settings)
        return true
    }

    func firstRunEstimate() async -> DaemonData.FirstRunEstimate? {
        guard case .success(let estimate) = await firstRunCall({ try $0.firstRunEstimate() }) else { return nil }
        return estimate
    }

    func lookupInvite(_ invite: String) async -> FirstRunLookup {
        switch await firstRunCall({ try $0.inviteLookup(invite) }) {
        case .success(let lookup) where lookup.valid:
            return .found(lookup)
        case .success(let lookup):
            return .refused(label: lookup.reasonLabel ?? "invite-invalid")
        case .failure(let failure as DaemonClient.Failure)
        where !failure.message.isEmpty && !Self.transientLookupLabels.contains(failure.message):
            // Includes `invite-host-not-allowed`, which the daemon sends with
            // the `unavailable` code but which no retry can change.
            return .refused(label: failure.message)
        case .failure, nil:
            return .unavailable
        }
    }

    /// Lookup failures that say nothing about the invite: the daemon could
    /// not reach the issuer, or its reply could not be read.
    private static let transientLookupLabels: Set<String> = [
        "invite-lookup-unavailable", "unparseable-response", "missing-result",
    ]

    func enrollInvite(_ invite: String) async -> Bool {
        if case .succeeded = await enroll(invite: invite, scopes: []) { return true }
        return false
    }

    func signInNearAI() async -> Bool {
        guard case .success(let session) = await firstRunCall({ try $0.accountSignIn() }) else { return false }
        return session.signedIn == true
    }

    /// The near.ai login for a first run without an invite: done at once if
    /// the daemon keeps a session, else the browser sign-in, waited on
    /// until it ends (`NearAILoginPoll`). Called directly rather than
    /// through `startNearAiCredential`, whose failure writes the Settings
    /// notice; the first run says its own (`folders.sign_in_failed`).
    func nearAILogin() async -> Bool { await nearAILogin(fresh: false) }

    /// `fresh`: sign in even when a NEAR AI login is stored, because NEAR AI
    /// refused it (Sign in again's `near_ai_sign_in_required`).
    func nearAILogin(fresh: Bool) async -> Bool {
        guard let client, !nearAiBrowserSignInWaiting else { return false }
        if !fresh {
            let first = await Task.detached { try? client.nearAiCredentialStatus(attemptID: nil) }.value
            if NearAILoginPoll.verdict(first) == .signedIn { return true }
        }
        // A Cancel before the browser opens: no attempt is begun, and none
        // that began meanwhile opens the browser.
        if Task.isCancelled { return false }
        credentialSignInNotificationEpoch &+= 1
        credentialSignInNotifiesAccount = false
        nearAiBrowserGeneration &+= 1
        nearAiBrowserSignInWaiting = true
        defer {
            nearAiBrowserSignInWaiting = false
            firstRunLoginAttemptID = nil
        }
        let started = await Task.detached { try? client.nearAiCredentialStart() }.value
        guard let attempt = started, let url = URL(string: attempt.browserURL) else { return false }
        if Task.isCancelled || self.client !== client {
            _ = await Task.detached { try? client.nearAiCredentialCancel(attemptID: attempt.attemptID) }.value
            return false
        }
        firstRunLoginAttemptID = attempt.attemptID
        openBrowser(url)
        var deadline = Date().addingTimeInterval(NearAILoginPoll.limit)
        var deadlineAttemptID = attempt.attemptID
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(1))
            // Cancelled (the first run's Cancel): stop at once, without
            // another poll that could report a sign-in.
            if Task.isCancelled { return false }
            guard self.client === client else { return false }
            // Cancellation and replacement are one operation. In particular,
            // an old poll must not finish the flow while cancel is in flight.
            if nearAiBrowserRestarting { continue }
            guard let attemptID = firstRunLoginAttemptID else { return false }
            if deadlineAttemptID != attemptID {
                deadlineAttemptID = attemptID
                deadline = Date().addingTimeInterval(NearAILoginPoll.limit)
            }
            guard Date() < deadline else { return false }
            let generation = nearAiBrowserGeneration
            let status = await Task.detached { try? client.nearAiCredentialStatus(attemptID: attemptID) }.value
            guard !Task.isCancelled, self.client === client else { return false }
            guard !nearAiBrowserRestarting, generation == nearAiBrowserGeneration,
                  firstRunLoginAttemptID == attemptID else { continue }
            switch fresh ? NearAILoginPoll.freshVerdict(status) : NearAILoginPoll.verdict(status) {
            case .signedIn:
                refreshNearAiCredential()
                return true
            case .ended:
                refreshNearAiCredential()
                return false
            case .waiting:
                continue
            }
        }
        return false
    }

    /// Cancel the named attempt before requesting a new one. The original
    /// waiter follows the replacement, so only it completes the account flow.
    private func restartNearAIBrowserSignIn() async {
        guard canRestartNearAiBrowserSignIn, let client,
              let attemptID = firstRunLoginAttemptID else { return }
        nearAiBrowserRestarting = true
        nearAiBrowserGeneration &+= 1
        let generation = nearAiBrowserGeneration
        defer { nearAiBrowserRestarting = false }
        let cancelled = await DaemonWork.run {
            Result { try client.nearAiCredentialCancel(attemptID: attemptID) }
        }
        guard self.client === client, nearAiBrowserSignInWaiting,
              generation == nearAiBrowserGeneration else { return }
        guard case .success = cancelled else {
            // A failed cancel keeps the original wait and permits retry.
            lastActionError = privateInferenceCopy?.credentialFailed
            return
        }
        firstRunLoginAttemptID = nil
        let started = await DaemonWork.run { try? client.nearAiCredentialStart() }
        guard self.client === client, nearAiBrowserSignInWaiting, !Task.isCancelled,
              generation == nearAiBrowserGeneration else {
            if let attempt = started {
                _ = await DaemonWork.run { try? client.nearAiCredentialCancel(attemptID: attempt.attemptID) }
            }
            return
        }
        guard let attempt = started, let url = URL(string: attempt.browserURL) else {
            lastActionError = privateInferenceCopy?.credentialFailed
            return
        }
        firstRunLoginAttemptID = attempt.attemptID
        openBrowser(url)
    }

    /// Cancel on Folders or Tools: end the browser sign-in `nearAILogin`
    /// started, named by its attempt when one began, else whatever sign-in
    /// the daemon runs. Called directly rather than through
    /// `cancelNearAiCredential`, which writes the Settings notice and shares
    /// its busy flag; the first run says its own line.
    func cancelNearAILogin() async -> Bool {
        let attemptID = firstRunLoginAttemptID
        guard case .success = await firstRunCall({ try $0.nearAiCredentialCancel(attemptID: attemptID) })
        else { return false }
        refreshNearAiCredential()
        return true
    }

    /// Enroll through the near.ai login with no invite. The daemon's label
    /// is passed back for the core's line, never shown.
    func enrollNearAI(referralCode: String?) async -> FirstRunNearAIEnrolment {
        guard let client else { return .refused(label: "near_ai_enroll_unavailable") }
        let outcome = await Task.detached(priority: .userInitiated) { () -> FirstRunNearAIEnrolment in
            do {
                return try client.nearAiAccountEnroll(referralCode: referralCode).enrolled
                    ? .enrolled : .refused(label: "near_ai_enroll_unavailable")
            } catch let failure as DaemonClient.Failure {
                return .refused(label: failure.message.isEmpty ? "near_ai_enroll_unavailable" : failure.message)
            } catch {
                return .refused(label: "near_ai_enroll_unavailable")
            }
        }.value
        if outcome == .enrolled { refreshStatus() }
        return outcome
    }

    func setAskEachTime() async -> Bool {
        guard case .success = await firstRunCall({ try $0.setAskEachTime() }) else { return false }
        refreshStatus()
        refreshProjects()
        return true
    }

    func saveConsentScopes(_ scopes: [String]) async -> Bool {
        let before = status.consentScopes
        let outcome = await setConsentScopes(scopes)
        // The first run's Start: a person who already has sent traces sees
        // the sheet on the Monitor, which the first run hands off to, not
        // over the Start that is still running.
        offerSentTracesTraining(outcome, before: before, at: .monitor)
        if case .succeeded = outcome { return true }
        return false
    }

    func setProjectMode(projectID: String, mode: ProjectMode) async -> Bool {
        guard case .success = await firstRunCall({ try $0.setProjectMode(projectID: projectID, mode: mode) })
        else { return false }
        refreshProjects()
        return true
    }

    func includePastSessions(projectID: String, sessionIDs: [String]) async -> Bool {
        guard case .success = await firstRunCall({
            try $0.includePastSessions(projectID: projectID, sessionIDs: sessionIDs)
        }) else { return false }
        return true
    }

    /// The Rules screen's folders. Nil without a daemon or on a refusal, so
    /// the screen keeps waiting rather than showing an empty list nobody
    /// reported.
    func rulesProjects() async -> [ProjectRow]? {
        guard case .success(let projects) = await firstRunCall({ try $0.listProjects(includeFound: true) })
        else { return nil }
        publishIfChanged(\.projects, projects)
        return projects
    }

    /// One folder's past sessions for the picker. Nil on a refusal: the
    /// folder then lists nothing to tick.
    func pastSessions(projectID: String) async -> PastSessionList? {
        guard case .success(let list) = await firstRunCall({ try $0.listPastSessions(projectID: projectID) })
        else { return nil }
        return list
    }

    func setPrivateAI(_ on: Bool) async -> Bool {
        guard case .success(let settings) = await firstRunCall({ try $0.setPrivateInference(on) })
        else { return false }
        publishIfChanged(\.daemonSettings, settings)
        return true
    }

    /// The first run's grant: a folder set to Ask keeps asking (#1753's
    /// ruling that a folder's own rule always wins).
    func grantAutomatic(witness: String?) async -> FirstRunGrantAnswer {
        await grantAutomatic(witness: witness, keepFolderAsks: true)
    }

    /// The grant, from the first run or Settings' sharing choice.
    func grantAutomatic(witness: String?, keepFolderAsks: Bool) async -> FirstRunGrantAnswer {
        switch await firstRunCall({ try $0.grantAutomatic(witnessSigningAddress: witness, keepFolderAsks: keepFolderAsks) }) {
        case .success(let grant) where grant.granted:
            refreshStatus()
            refreshProjects()
            return .granted
        case .failure(let failure as DaemonClient.Failure) where !failure.message.isEmpty:
            return .refused(label: failure.message)
        case .success, .failure, nil:
            // Not granted, and no daemon label to say why.
            return .refused(label: "automatic-grant-unavailable")
        }
    }

    /// Reads status first: right after enrolling, the tenant the marker is
    /// keyed by may not have reached `status` yet. Without a tenant nothing
    /// is marked, and that is reported rather than passed off as done.
    func markComplete() async -> Bool {
        if case .success(let fresh) = await firstRunCall({ try $0.status() }) {
            publishIfChanged(\.status, fresh)
        }
        guard status.tenantID != nil else { return false }
        markOnboardingComplete()
        return isOnboardingComplete
    }

    /// An invite link (`PendingInvite`) takes back a finished watch-only
    /// run while the daemon holds no enrollment: the marker is cleared, so
    /// `requiresOnboarding` turns true, a window hosts the first run again,
    /// and the coordinator applies the parked link to Join. Without this a
    /// finished watcher could never join, since the first run is the only
    /// place a link is applied. An enrolled daemon's marker is the tenant's
    /// and is left alone. Announced, as `markWatchOnlyComplete` is.
    ///
    /// Only an invite does this: a link carries any string, and the
    /// coordinator discards one the core does not accept as an invite
    /// (`OnboardingNavigation.receive`), so the same `TCInvite.issuerHost`
    /// check runs here, and a finished watcher is not sent back to Join
    /// with nothing to apply.
    func inviteLinkArrived(_ invite: String) {
        guard !status.loggedIn else { return }
        guard TCInvite.issuerHost(invite) != nil else { return }
        guard let key = Self.watchOnlyCompleteKey(configDirectory) else {
            inviteAwaitingConfigDirectory = invite
            return
        }
        guard UserDefaults.standard.bool(forKey: key) else { return }
        objectWillChange.send()
        UserDefaults.standard.removeObject(forKey: key)
    }

    /// The first run finished; keep what it must still say for the main
    /// window (`ShellNotices`), which outlives the first-run host.
    func firstRunFinished(notice: String?) {
        firstRunNotice = notice
    }

    /// Watching only: the marker is keyed by the config directory, since
    /// there is no tenant. Status is read first, and an enrolled daemon is
    /// not marked: its Start is the tenant's (`markComplete`). As with that
    /// marker, the write is announced, because `requiresOnboarding` is
    /// computed from `UserDefaults` and nothing else would tell the hosts.
    func markWatchOnlyComplete() async -> Bool {
        if case .success(let fresh) = await firstRunCall({ try $0.status() }) {
            publishIfChanged(\.status, fresh)
        }
        guard !status.loggedIn, let key = Self.watchOnlyCompleteKey(configDirectory) else { return false }
        objectWillChange.send()
        UserDefaults.standard.set(true, forKey: key)
        return isWatchOnlyComplete
    }

    /// The daemon drops this Mac's enrollment (`unenroll`). Status is read
    /// again, so `status.loggedIn` is false before watch only is marked.
    func unenroll() async -> Bool {
        guard case .success = await firstRunCall({ try $0.unenroll() }) else { return false }
        if case .success(let fresh) = await firstRunCall({ try $0.status() }) {
            publishIfChanged(\.status, fresh)
        }
        return true
    }

    /// One blocking client call off the main actor. Nil without a daemon.
    private func firstRunCall<T: Sendable>(_ work: @escaping @Sendable (DaemonClient) throws -> T) async -> Result<T, Error>? {
        guard let client else { return nil }
        // On the daemon-call queue, not the cooperative pool: a call that
        // waits on the network (a renewal, a ceremony) must not hold one of
        // the few threads every other task in the app runs on.
        return await DaemonWork.run { Result { try work(client) } }
    }
}
