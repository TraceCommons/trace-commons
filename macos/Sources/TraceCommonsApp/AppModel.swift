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

    @Published private(set) var startup: Startup = .starting
    @Published private(set) var status: DaemonStatus = .unknown
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
    @Published private(set) var rollup: HistoryRollup?
    @Published private(set) var projects: [ProjectRow] = []
    /// The one project the daemon suggests arming, or nil. Refreshed
    /// alongside `projects`, because every reason the project list changes
    /// is also a reason this answer might have.
    @Published private(set) var armingOffer: ArmingOffer?
    @Published private(set) var consentScopes: [ConsentScope] = []
    @Published private(set) var daemonSettings: DaemonSettingsView?

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
    @Published private(set) var outcomeCounts: [String: Int] = [:]
    @Published private(set) var audit: [AuditEntry] = []
    @Published var undo: Undo?
    @Published var lastActionError: String?

    /// A one-line statement about something that DID happen, as opposed to
    /// `lastActionError`, which is about something that did not. Kept apart
    /// so the two never have to be told from each other by their wording:
    /// the Waiting screen renders this one in its own voice.
    @Published var lastActionNotice: String?

    private var daemon: TCDaemon?
    private var client: DaemonClient?
    private var subscription: TCSubscription?
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

    /// The badge counts DECISIONS OWED -- entries actually waiting for a yes
    /// or no -- not sessions found and not queue total.
    var decisionsOwed: Int {
        awaitingDecision.count
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
                sizeBytes: \.sizeBytes
            )
        )
    }

    var armedProjects: [ProjectRow] {
        projects.filter { $0.mode == .autoUpload }
    }

    var health: HealthCopy? {
        guard let label = status.health.lastErrorLabel else { return nil }
        // The budget banner says the same thing with real numbers, so the
        // bare label is suppressed when it is going to be drawn.
        if label == "daily-cap-reached" && status.dailyBudget.blocked { return nil }
        return HealthCopy.forLabel(label)
    }

    /// The spent-budget banner, when there is one.
    ///
    /// Deliberately independent of `health`. The daemon's health slot holds
    /// one label at a time and `daily-cap-reached` is last in its
    /// precedence order, so a full queue -- or any other condition -- hid
    /// the cap completely. Rendering both means the contributor sees the
    /// reason their approvals are not moving even while something else is
    /// also wrong.
    var budgetHealth: HealthCopy? {
        HealthCopy.forBudget(status.dailyBudget)
    }

    // MARK: - Lifecycle

    /// The resolved state directory, once `start()` has named one.
    ///
    /// Held so the roots screen starts the daemon against the same directory
    /// that refused, rather than re-resolving and possibly disagreeing with
    /// it.
    private(set) var configDirectory: String = ""

    func start() {
        guard case .starting = startup else { return }
        let resolved: DaemonHost.Resolution
        do {
            resolved = try DaemonHost.resolveConfigDirectory()
        } catch {
            startup = .refused("\(error)")
            return
        }
        configDirectory = resolved.path
        startDaemon(at: resolved.path, settingsJSON: nil)
    }

    /// Start (or restart) the in-process daemon against an already-resolved
    /// state directory.
    ///
    /// `settingsJSON` is the roots screen's mechanism: the C ABI persists it
    /// and only then evaluates whether both session roots are declared, so
    /// one call both records the contributor's answer and starts the watcher.
    func startDaemon(at path: String, settingsJSON: String?) {
        do {
            let daemon = try TCDaemon(configDir: path, settingsJSON: settingsJSON)
            let client = DaemonClient(daemon: daemon)
            self.daemon = daemon
            self.client = client
            startup = .running
            subscribe()
            refreshAll()
        } catch TCDaemon.TCError.rootsNotDeclared {
            // Not a dead end any more: the roots screen renders on this
            // state and calls back into `startDaemon` with the two folders
            // the contributor picked.
            startup = .needsRoots
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
            applyPendingUpdate(pending)
            publishIfChanged(\.status, status)
        case .previewReady(let result):
            applyPreviewOutcome(result)
        case .queueChanged:
            refreshQueue()
            // `queue_depth` lives on `status`, and the daemon does not
            // publish `status_changed` for a queue change, so a status
            // fetched at launch would stay at 0 forever.
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
        case .statusChanged:
            refreshStatus()
        case .digestDue(let count, let contributed, let contributedProjects, let credit, _):
            refreshQueue()
            // A digest can now be about what went out unasked, with nothing
            // waiting at all -- so this also refreshes history, which is the
            // screen those numbers came from and the one a contributor opens
            // next.
            if contributed > 0 {
                refreshHistory()
            }
            Notifier.shared.postDigest(
                pendingCount: count,
                projects: waitingByProject.map(\.label),
                contributedCount: contributed,
                contributedProjects: contributedProjects,
                creditPending: credit
            )
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
        perform("list_audit", work: { try $0.listAudit() }) { self.publishIfChanged(\.audit, $0) }
    }

    func refreshStatus() {
        perform("status", work: { try $0.status() }) { self.publishIfChanged(\.status, $0) }
    }

    func refreshQueue() {
        perform("list_pending", work: { try $0.listPending() }) { entries in
            self.applyPendingUpdate(entries)
        }
    }

    func refreshHistory() {
        perform("list_history", work: { try $0.listHistory() }) {
            self.publishIfChanged(\.history, $0)
        }
        perform("history_rollup", work: { try $0.historyRollup() }) {
            self.publishIfChanged(\.rollup, $0)
        }
    }

    func refreshProjects() {
        perform("list_projects", work: { try $0.listProjects() }) { self.publishIfChanged(\.projects, $0) }
        refreshArmingOffer()
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
    /// `ProjectIgnoreCopy.reconciliation`.
    func ignoreProject(id projectID: String, label: String, promised: Int) {
        perform(
            "set_project_mode",
            work: { try $0.setProjectMode(projectID: projectID, mode: .ignore) }
        ) { purged in
            self.lastActionNotice = ProjectIgnoreCopy.reconciliation(
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

    /// Refresh the per-tool words without touching the declaration.
    ///
    /// Called when the card appears, and only while something is declared:
    /// asking about a proxy nobody mentioned would be the probe of an
    /// undeclared local service that the declaration exists to prevent.
    func refreshRoutedTools() {
        let form = routingForm
        guard form.on, let client else { return }
        Task.detached(priority: .userInitiated) {
            let evidence = try? client.probeRoutedTools(form)
            await MainActor.run {
                // Left as it was when the call did not run: a stale answer
                // is replaced by a new one, never by a blank.
                if let evidence { self.routingEvidence = evidence }
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

    /// `isOnboardingComplete` is computed from `UserDefaults`, not from a
    /// `@Published` property, so writing the key changes nothing SwiftUI is
    /// watching. Without the explicit `objectWillChange`, pressing Done
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
    /// enrolment. Debug-only, and deliberately routed through
    /// `publishIfChanged` so a test observes exactly what the app does.
    func setStatusForTesting(_ status: DaemonStatus) {
        publishIfChanged(\.status, status)
    }
    #endif

    private static func onboardingCompleteKey(_ tenantID: String) -> String {
        "trace_commons.onboarding_complete.\(tenantID)"
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
    /// in the first place: `QueueRow.onAppear` drives `requestPreview(for:)`
    /// for whatever the viewport actually realizes, so this stays
    /// proportional to what is on screen.
    private func applyPendingUpdate(_ entries: [QueueEntry]) {
        let previousIDs = Set(pending.map(\.entryID))
        publishIfChanged(\.pending, entries)
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
    /// design: draw a pending card immediately ("Reading it locally...",
    /// see `QueueRow`) and never block waiting for the daemon's answer.
    ///
    /// Called from `QueueRow.onAppear` -- the same trigger #357 introduced
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
    func submitProject(id projectID: String, verdict: ContributorVerdict? = nil) {
        let attempted = awaitingDecision.filter { $0.projectID == projectID }.map(\.entryID)
        perform("approve", work: {
            try $0.approve(projectID: projectID, verdict: verdict)
        }) { response in
            self.refreshQueue()
            self.showToast(for: response, attempted: attempted)
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
            declaredSource: nil,
            projectID: "project_screenshot_fixture",
            projectLabel: "northwind-billing",
            sizeBytes: 1615,
            discoveredAt: Date(timeIntervalSince1970: 1_770_000_000),
            state: .pending,
            reasonLabel: nil,
            attempts: 0,
            // A single-file conversation: no delegated transcripts, none
            // dropped, so the card's extent line is absent and the capture
            // shows exactly what it showed before these fields existed.
            subagentCount: 0,
            subagentsDropped: 0
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
