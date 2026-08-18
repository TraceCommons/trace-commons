import Foundation
import SwiftUI
import TCBridge

/// Everything the UI reads, and the only thing that talks to the daemon.
///
/// `@MainActor` throughout: `tc_subscribe` callbacks arrive on a Rust
/// background thread, and `handle(event:)` is the single place that hops
/// back before touching any published property.
@MainActor
final class AppModel: ObservableObject {
    enum Startup: Equatable {
        case starting
        /// The daemon is running in-process.
        case running
        /// Refused to start, with a sentence a person can act on.
        case refused(String)
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
    struct Undo: Equatable {
        let entryID: String
        let projectLabel: String
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

    @Published private(set) var startup: Startup = .starting
    @Published private(set) var status: DaemonStatus = .unknown
    @Published private(set) var pending: [QueueEntry] = []
    @Published private(set) var summaries: [String: PreviewSummary] = [:]
    @Published private(set) var summaryErrors: [String: String] = [:]
    @Published private(set) var history: [HistoryRecord] = []
    @Published private(set) var rollup: HistoryRollup?
    @Published private(set) var projects: [ProjectRow] = []
    @Published private(set) var consentScopes: [ConsentScope] = []
    @Published private(set) var daemonSettings: DaemonSettingsView?
    @Published private(set) var outcomeCounts: [String: Int] = [:]
    @Published private(set) var audit: [AuditEntry] = []
    @Published var undo: Undo?
    @Published var lastActionError: String?

    private var daemon: TCDaemon?
    private var client: DaemonClient?
    private var subscription: TCSubscription?
    private var undoTask: Task<Void, Never>?

    // MARK: - Derived state the shell renders

    /// The badge counts DECISIONS OWED -- entries actually waiting for a yes
    /// or no -- not sessions found and not queue total.
    var decisionsOwed: Int {
        pending.filter { $0.state == .pending }.count
    }

    var awaitingDecision: [QueueEntry] {
        pending.filter { $0.state == .pending }
    }

    var armedProjects: [ProjectRow] {
        projects.filter { $0.mode == .autoUpload }
    }

    var health: HealthCopy? {
        guard let label = status.health.lastErrorLabel else { return nil }
        return HealthCopy.forLabel(label)
    }

    /// What is waiting, per project, with sizes. These are not buttons.
    var waitingByProject: [(label: String, count: Int, bytes: Int)] {
        var order: [String] = []
        var counts: [String: (Int, Int)] = [:]
        for entry in awaitingDecision {
            if counts[entry.projectLabel] == nil {
                order.append(entry.projectLabel)
                counts[entry.projectLabel] = (0, 0)
            }
            let current = counts[entry.projectLabel]!
            counts[entry.projectLabel] = (current.0 + 1, current.1 + entry.sizeBytes)
        }
        return order.map { ($0, counts[$0]!.0, counts[$0]!.1) }
    }

    // MARK: - Lifecycle

    func start() {
        guard case .starting = startup else { return }
        let resolved: DaemonHost.Resolution
        do {
            resolved = try DaemonHost.resolveConfigDirectory()
        } catch {
            startup = .refused("\(error)")
            return
        }
        do {
            let daemon = try TCDaemon(configDir: resolved.path)
            let client = DaemonClient(daemon: daemon)
            self.daemon = daemon
            self.client = client
            startup = .running
            subscribe()
            refreshAll()
        } catch {
            startup = .refused("\(error)")
        }
    }

    private func subscribe() {
        guard let daemon else { return }
        subscription = daemon.subscribe { [weak self] json in
            // Rust background thread. Nothing observable may be touched
            // here; hop first, always.
            let event = DaemonEventParser.parse(json)
            Task { @MainActor in
                self?.handle(event: event)
            }
        }
        // No `subscribe` call follows: the contract's `snapshot`-on-subscribe
        // is a property of the SOCKET connection loop, which sends it to the
        // client that just connected. `tc_subscribe` attaches to the event
        // bus directly and gets no such courtesy frame, so the first paint
        // comes from the explicit `list_pending` + `status` in refreshAll()
        // rather than from waiting on a snapshot that will never arrive.
    }

    private func handle(event: DaemonEvent) {
        switch event {
        case .snapshot(let pending, let status):
            self.pending = pending
            self.status = status
            loadMissingSummaries()
        case .queueChanged:
            refreshQueue()
            // `queue_depth` lives on `status`, and the daemon does not
            // publish `status_changed` for a queue change, so a status
            // fetched at launch would stay at 0 forever.
            refreshStatus()
        case .statusChanged:
            refreshStatus()
        case .digestDue(let count, _):
            refreshQueue()
            Notifier.shared.postDigest(pendingCount: count, projects: waitingByProject.map(\.label))
        case .resyncRequired, .lagged:
            refreshQueue()
            refreshStatus()
        case .unknown:
            break
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
        undoTask?.cancel()
        let subscription = self.subscription
        let daemon = self.daemon
        // Dropped first so no new work can be started from this side while
        // teardown runs; `perform`, `enroll` and the rest all guard on
        // `client`.
        self.subscription = nil
        self.daemon = nil
        self.client = nil
        guard let daemon else { return }
        if case .leaked(let reason) = daemon.shutdown(unsubscribing: subscription) {
            // A fixed label, no path or token, per this repo's logging rule.
            // The handle stayed allocated on purpose; the process is exiting.
            lastActionError = "shutdown: handle-leaked-\(reason)"
        }
    }

    // MARK: - Refresh

    func refreshAll() {
        refreshStatus()
        refreshQueue()
        refreshHistory()
        refreshProjects()
        refreshSettings()
        refreshConsentOptions()
        refreshOutcomeCounts()
        refreshAudit()
        refreshPublicProfile()
    }

    /// The local change log. Refreshed alongside everything else at launch,
    /// and again after each call that APPENDS to it -- arming a project,
    /// changing consent scopes, acknowledging the NEAR AI notice -- because
    /// the daemon publishes no event for an audit append, so a list fetched
    /// once would show a contributor everything except the change they just
    /// made.
    func refreshAudit() {
        perform("list_audit", work: { try $0.listAudit() }, onSuccess: { self.audit = $0 })
    }

    func refreshStatus() {
        perform("status", work: { try $0.status() }, onSuccess: { self.status = $0 })
    }

    func refreshQueue() {
        perform("list_pending", work: { try $0.listPending() }) { entries in
            self.pending = entries
            self.loadMissingSummaries()
        }
    }

    func refreshHistory() {
        perform("list_history", work: { try $0.listHistory() }, onSuccess: { self.history = $0 })
        perform("history_rollup", work: { try $0.historyRollup() }, onSuccess: { self.rollup = $0 })
    }

    func refreshProjects() {
        perform("list_projects", work: { try $0.listProjects() }, onSuccess: { self.projects = $0 })
    }

    /// Sets `project`'s mode via the daemon and refreshes `projects` from
    /// the daemon's own answer on success. Deliberately does not flip
    /// `project.mode` optimistically: the whole reason this method exists
    /// is that a UI that assumes a choice landed, when it did not, is worse
    /// than a UI that offers no choice at all. A failure lands in
    /// `lastActionError`, same as every other action here, and the caller
    /// must leave its own state alone until this succeeds.
    ///
    /// `project.projectLabel` is the only identifier `ProjectRow` carries.
    /// It is not guaranteed to be a `project_key` the daemon will accept --
    /// see the comment on `DaemonClient.setProjectMode` -- so callers
    /// should expect `project-key-unrecognized` today and must surface it
    /// rather than assume success.
    func setProjectMode(_ project: ProjectRow, mode: ProjectMode) {
        perform(
            "set_project_mode",
            work: { try $0.setProjectMode(projectKey: project.projectLabel, mode: mode) }
        ) { _ in
            self.refreshProjects()
            // Arming or disarming a project is one of the changes the daemon
            // records; see `refreshAudit`.
            self.refreshAudit()
        }
    }

    func refreshSettings() {
        perform("get_settings", work: { try $0.settings() }, onSuccess: { self.daemonSettings = $0 })
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

    /// Redeems `invite` for enrollment. Bypasses the `perform` helper (and
    /// its `lastActionError` label) on purpose: that helper renders
    /// `failure.message`, and `enroll`'s failure message must never reach a
    /// screen -- `OnboardingConnectView` renders one fixed sentence for
    /// every failure of this call instead.
    func enroll(invite: String, scopes: [String] = []) async -> EnrollOutcome {
        guard let client else { return .failed }
        return await Task.detached(priority: .userInitiated) { () -> EnrollOutcome in
            do {
                return .succeeded(try client.enroll(invite: invite, scopes: scopes))
            } catch {
                return .failed
            }
        }.value
    }

    /// Records that the NEAR AI first-use notice was shown, and clears the
    /// health label that otherwise keeps the daemon refusing that filter.
    /// Refreshes settings and status afterward so `nearAIConfigured` /
    /// `health` reflect the daemon's own post-acknowledgment state rather
    /// than an assumption made here.
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
        perform("consent_options", work: { try $0.consentOptions() }, onSuccess: {
            self.consentScopes = $0
        })
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

    /// Applies the consent scopes chosen on `ConsentScopesView`. Bypasses
    /// `perform` (like `enroll`) so the onboarding coordinator can await the
    /// outcome and only advance past the consent screen once the daemon has
    /// actually recorded the choice -- see the coordinator's ordering note
    /// on why this call, not `enroll`, is what applies scopes in this app's
    /// flow.
    func setConsentScopes(_ scopes: [String]) async -> SetScopesOutcome {
        guard let client else { return .failed }
        let outcome: SetScopesOutcome = await Task.detached(priority: .userInitiated) {
            do {
                return .succeeded(try client.setConsentScopes(scopes))
            } catch {
                return .failed
            }
        }.value
        if case .succeeded = outcome {
            refreshStatus()
            refreshAudit()
        }
        return outcome
    }

    // MARK: - Onboarding resume

    /// Whether onboarding has been walked to the end (the Done screen) for
    /// the *currently enrolled* device. Keyed off `status.tenantID` rather
    /// than a single global flag: `enroll` alone flips `status.loggedIn` to
    /// true (it happens on screen 2, before consent is even chosen on
    /// screen 3), so `loggedIn` cannot by itself distinguish "fully
    /// onboarded" from "enrolled but consent was never confirmed." A
    /// contributor who quit mid-flow must come back to the rest of
    /// onboarding, not straight to the main window with whatever scopes
    /// `enroll`'s floor-only default happened to leave in place -- see the
    /// coordinator's atomicity note.
    var isOnboardingComplete: Bool {
        guard let tenantID = status.tenantID else { return false }
        return UserDefaults.standard.bool(forKey: Self.onboardingCompleteKey(tenantID))
    }

    func markOnboardingComplete() {
        guard let tenantID = status.tenantID else { return }
        UserDefaults.standard.set(true, forKey: Self.onboardingCompleteKey(tenantID))
    }

    private static func onboardingCompleteKey(_ tenantID: String) -> String {
        "trace_commons.onboarding_complete.\(tenantID)"
    }

    func refreshOutcomeCounts() {
        perform("queue_outcome_counts", work: { try $0.queueOutcomeCounts() }, onSuccess: {
            self.outcomeCounts = $0
        })
    }

    /// Row summaries carry the redacted opening prompt, the would-send size
    /// and the redaction receipt -- all three come from the preview pass, so
    /// each row loads its own once.
    private func loadMissingSummaries() {
        guard let client else { return }
        for entry in awaitingDecision where summaries[entry.entryID] == nil
            && summaryErrors[entry.entryID] == nil
        {
            let id = entry.entryID
            Task.detached(priority: .utility) {
                let outcome = Result { try client.previewSummary(entryID: id) }
                await MainActor.run {
                    switch outcome {
                    case .success(let summary): self.summaries[id] = summary
                    case .failure(let error):
                        self.summaryErrors[id] = (error as? DaemonClient.Failure)?.message
                            ?? "preview-failed"
                    }
                }
            }
        }
    }

    // MARK: - Decisions

    /// Approve, then raise the recovery affordance. `cancel` returns the
    /// entry to `pending`, so the undo is real rather than cosmetic.
    func approve(_ entry: QueueEntry) {
        perform("approve", work: { try $0.approve(entryID: entry.entryID) }) { _ in
            self.refreshQueue()
            self.startUndo(for: entry)
        }
    }

    private func startUndo(for entry: QueueEntry) {
        undoTask?.cancel()
        undo = Undo(
            entryID: entry.entryID,
            projectLabel: entry.projectLabel,
            approvedAt: Date(),
            heldSeconds: 0
        )
        undoTask = Task { @MainActor in
            for second in 1...Undo.tickCeiling {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                if Task.isCancelled { return }
                guard var current = undo, current.entryID == entry.entryID else { return }
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
    /// `cancel` only works while the entry is still `approved`. The daemon's
    /// uploader can pick an approved entry up immediately -- observed in the
    /// self-test, where the entry had already moved to `failed` before the
    /// five seconds elapsed -- so the undo can lose the race. When it does,
    /// this says so plainly instead of showing a raw label.
    func undoApproval() {
        guard let undo else { return }
        undoTask?.cancel()
        let id = undo.entryID
        self.undo = nil
        guard let client else { return }
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try client.cancel(entryID: id) }
            await MainActor.run {
                if case .failure = outcome {
                    self.lastActionError = "Too late to undo -- this one had already left "
                        + "the waiting list. History shows what happened to it."
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
                // off-the-roster state, not an error worth a banner.
                self.publicProfile = (outcome?.onRoster ?? false) ? outcome : nil
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
        case withdrawn(WithdrawalReach?)
        /// The daemon has no account session, so the request was never made.
        case noAccountSession
        /// Anything else. Carries the daemon's fixed label, which by
        /// contract is never a path, a token, or a response body.
        case failed(String)
    }

    @Published private(set) var withdrawals: [String: WithdrawalResult] = [:]
    @Published private(set) var withdrawing: Set<String> = []

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
                    self.withdrawals[id] = .withdrawn(value.distributionReach)
                    self.refreshHistory()
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? "withdraw-failed"
                    self.withdrawals[id] = label == "account-session-required"
                        ? .noAccountSession
                        : .failed(label)
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

    /// Opens the in-process preview off the main actor -- the redaction pass
    /// blocks -- and hands the open handle back on the main actor.
    func openPreview(entryID: String) async -> PreviewOutcome {
        guard let client else { return .failed("the watcher isn't running") }
        return await Task.detached(priority: .userInitiated) { () -> PreviewOutcome in
            do {
                return .opened(try client.openPreview(entryID: entryID))
            } catch {
                return .failed("\(error)")
            }
        }.value
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
            piiLabelsPresent: ["email"],
            consentScopes: ["debugging_evaluation"],
            residualRisk: "pattern-based"
        )
        let entry = QueueEntry(
            entryID: "entry_screenshot_fixture",
            sessionHash: "sha256:0000000000000000",
            source: "claude-code",
            projectLabel: "northwind-billing",
            sizeBytes: 1615,
            discoveredAt: Date(timeIntervalSince1970: 1_770_000_000),
            state: .pending,
            reasonLabel: nil,
            attempts: 0
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

    enum PreviewOutcome {
        case opened(TCPreview)
        /// A fixed label from the ABI, safe to show: it never carries a
        /// path, a token, or trace content.
        case failed(String)
    }

    // MARK: - Plumbing

    private func perform<T>(
        _ label: String,
        work: @escaping (DaemonClient) throws -> T,
        onSuccess: @escaping (T) -> Void
    ) {
        guard let client else { return }
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try work(client) }
            await MainActor.run {
                switch outcome {
                case .success(let value):
                    onSuccess(value)
                case .failure(let error):
                    // `error.message` is a fixed label by contract, never a
                    // path, a token, or a server response body.
                    if let failure = error as? DaemonClient.Failure {
                        self.lastActionError = "\(label): \(failure.message)"
                    } else {
                        self.lastActionError = "\(label): failed"
                    }
                }
            }
        }
    }
}
