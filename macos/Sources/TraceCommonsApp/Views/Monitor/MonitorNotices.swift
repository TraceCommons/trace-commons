import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Every notice the window shows, as popover notices (owner, 2026-10-19):
/// one stack floating over the top of the tab it belongs to, under the tab
/// bar, pushing nothing down (`GlassPopoverStack`). It was cards inline in
/// each tab's list and a column of them above the tabs.
///
/// Where each one goes:
/// - Every tab: what is about this app or this account, not a tab's
///   subject -- a daemon that refused to start, onboarding owed, the
///   attached-daemon limits, first run's last word, the legacy invite move
///   and its notice -- and, in front, a failed or finished action, which
///   may come from any tab.
/// - Traces (the tree and History): the traces moving now, the undos, the
///   core's health, the consent notices about folders (grant voids,
///   arming rewordings, a gate hold), the arming offer, the backlog and
///   idle suggestions and the idle notifications offer, and the
///   first-contribution note; on History also its verdict news and the
///   verdict notifications offer.
/// - Private AI: the offer to route AI requests through NEAR AI, and the
///   NEAR AI sign-in while there is none (owner, 2026-10-19: the sign-in
///   card is a popover notice).
///
/// Each card's X closes it. Where a card already had a button whose only
/// effect was to put it away (Dismiss, Got it), the X does that and the
/// button is gone; where answering is its own decision (No thanks, Not
/// now), the button stays and the X only closes the card until the window
/// is opened again (`closed`). The four acknowledged safeguards instead
/// stay closed for the same incident across windows and restarts. A notice
/// that goes only when its cause does (a down core, a consent gate hold, the
/// attached daemon) has no X.
///
/// Every word is the core's, the daemon's, or the cards' own as before;
/// this view lays them out and writes none.
struct MonitorNotices: View {
    enum Place: Equatable {
        case home, privateAI, traces, history, insights
        /// Home's History page, restored from before History moved to
        /// Traces: its own offers only, since its inspector still draws the
        /// Traces prompts (`MonitorWindowView.promptsInInspector`).
        case homeHistory
    }

    let place: Place
    let traces: TracesStore
    let home: HomeStore
    /// The traces being contributed, kept or dismissed, oldest first
    /// (`MapTransferPlayer.cards`): drawn on Traces only.
    var transferCards: [MapTransfer] = []
    var onCloseTransfer: (String) -> Void = { _ in }
    /// The cards closed with their X until the window is opened again, by
    /// item id.
    @Binding var closed: Set<String>

    @EnvironmentObject private var model: AppModel
    @Environment(\.openURL) private var openURL
    /// The one-time notification offers' store, as Settings reads it.
    @State private var offers = NudgeSettingsStore(client: nil)
    @State private var dismissals = MonitorNoticeDismissals.shared
    /// Closed with its X, for good on this Mac (owner, 2026-10-08).
    @AppStorage("monitor.firstContributionDismissed") private var firstContributionDismissed = false

    var body: some View {
        let shown = Self.shown(items, closed: closed.union(persistentClosed))
        VStack(spacing: 0) {
            if !shown.isEmpty {
                // Not in a glass group: one would blend the stacked cards'
                // glass into a single surface, and the earlier cards would
                // no longer read as peeking out below the front one.
                GlassPopoverStack(shown, anchor: .top)
                    .environment(\.glassLayer, .floating)
                    .transition(.opacity)
            }
        }
        .frame(maxWidth: .infinity, alignment: .top)
        .onChange(of: currentHealthIDs) { _, current in
            closed = Self.prunedClosed(closed, currentHealth: current)
        }
        .onChange(of: dismissalSnapshot, initial: true) { previous, snapshot in
            closed = Self.closedAfterScopeChange(closed, from: previous.scope, to: snapshot.scope)
            dismissals.reconcile(snapshot.conditions, scope: snapshot.scope)
        }
        .task(id: model.liveData.map(ObjectIdentifier.init)) {
            offers.attach(model.daemonData)
            await offers.load()
        }
    }

    /// The items not closed for now, in the stack's order.
    static func shown(_ items: [GlassPopoverItem], closed: Set<String>) -> [GlassPopoverItem] {
        items.filter { !closed.contains($0.id) }
    }

    /// The close id of a health banner, or nil when it may not be closed.
    /// Durable safeguards use the daemon's condition code; other waiting
    /// notices keep their window-local title identity. A down or unread core
    /// is not closable: dismissing it would leave the tab reading as healthy.
    static func healthCloseID(for banner: TracesHealth.Banner) -> String? {
        guard banner.tone == .ask else { return nil }
        if let code = banner.condition, let kind = MonitorNoticeDismissals.Kind(rawValue: code) {
            return kind.closeID
        }
        return "health:" + banner.title
    }

    /// `closed` without the closed health banners that are no longer shown,
    /// so one whose cause has gone comes back when the cause recurs (the next
    /// day's upload limit, say). Other closed cards are left as they are.
    static func prunedClosed(_ closed: Set<String>, currentHealth: Set<String>) -> Set<String> {
        closed.filter { !$0.hasPrefix("health:") || currentHealth.contains($0) }
    }

    /// A close made before the account was known is window-local. It must
    /// not hide the same condition for a subsequently identified account.
    static func closedAfterScopeChange(
        _ closed: Set<String>, from previous: MonitorNoticeDismissals.Scope?, to current: MonitorNoticeDismissals.Scope?
    ) -> Set<String> {
        guard previous != current else { return closed }
        return closed.subtracting(MonitorNoticeDismissals.Kind.allCases.map(\.closeID))
    }

    /// Where a tab's notices go: History is the Traces tab's other page.
    static func place(
        _ tab: MonitorWindowView.Tab, tracesPage: TracesPage, homePage: HomeTabView.Page = .overview
    ) -> Place {
        switch tab {
        case .home: homePage == .history ? .homeHistory : .home
        case .inference: .privateAI
        case .traces: tracesPage == .history ? .history : .traces
        case .insights: .insights
        }
    }

    /// Back first, front last: what waits on an answer behind what is
    /// happening now.
    private var items: [GlassPopoverItem] {
        var items = appItems
        switch place {
        case .traces:
            items += tracesOffers + tracesNow
        case .history:
            items += tracesOffers + historyOffers + tracesNow
        case .privateAI:
            items += privateAIItems
        case .homeHistory:
            items += historyOffers
        case .home, .insights:
            break
        }
        items += actionItems
        if place == .traces || place == .history {
            items += TraceTransferCard.items(
                transferCards,
                folderLabel: { transfer in traces.tree.allFolders.first { $0.id == transfer.folder }?.label },
                onClose: onCloseTransfer)
        }
        return items
    }

    private func close(_ id: String) -> () -> Void {
        { closed.insert(id) }
    }

    private var closeWord: String { MonitorWords.close }
    private var details: String { MonitorWords.noticeDetails }

    /// The core's "Dismiss all", on each card it would put away, while
    /// there is more than one (owner feedback, 2026-10-11).
    @ViewBuilder
    private var dismissAll: some View {
        if model.acknowledgeableNoticeCount > 1, let word = ActionNoticeWords.coreDismissAllWord {
            Button(word) { model.acknowledgeAllNotices() }
                .buttonStyle(GlassButtonStyle(.link))
                .lineLimit(1)
        }
    }

    // MARK: Every tab

    private var appItems: [GlassPopoverItem] {
        var items: [GlassPopoverItem] = []
        // The window's gate, which goes only when its cause does: a daemon
        // that refused to start, in the core's down title over the
        // refusal's sentence, and onboarding owed, whose button opens first
        // run (`MonitorGate`).
        switch MonitorGate.of(
            startup: model.startup, onboardingKnown: model.onboardingKnown,
            requiresOnboarding: model.requiresOnboarding)
        {
        case .down(let sentence):
            // The one view first run and Settings say a refusal with.
            items.append(GlassPopoverItem(id: "startup-refused") {
                StartupRefusedBanner(sentence: sentence, popover: true)
            })
        case .signedOut:
            items.append(GlassPopoverItem(id: "signed-out") {
                GlassPopoverNotice(title: MonitorWords.signedOut) {
                    GlassNoticeMark(.ask)
                } summary: { EmptyView() } details: { EmptyView() } actions: {
                    Button(MonitorWindowView.openFirstRun) { OpenMonitor.request() }
                        .buttonStyle(GlassButtonStyle(.glass))
                }
            })
        case .awaiting, .open:
            break
        }
        if model.isAttachedDaemon, let copy = TCAttach.copy() {
            items.append(GlassPopoverItem(id: "attached") {
                GlassPopoverNotice(title: copy.attachedTitle) {
                    GlassNoticeMark(.off)
                } summary: {
                    Text(copy.attachedDetail)
                } details: { EmptyView() } actions: { EmptyView() }
            })
        }
        if let notice = model.firstRunNotice {
            items.append(GlassPopoverItem(id: "first-run") {
                GlassPopoverNotice(title: notice, closeLabel: closeWord, onClose: { model.firstRunNotice = nil }) {
                    GlassNoticeMark(.ask)
                } summary: { EmptyView() } details: { EmptyView() } actions: { dismissAll }
            })
        }
        if let notice = model.legacyMigrationNotice {
            items.append(GlassPopoverItem(id: "legacy-notice") {
                GlassPopoverNotice(
                    title: notice.title, detailsLabel: details, closeLabel: closeWord,
                    onClose: { model.acknowledgeLegacyInviteMigration() }
                ) {
                    GlassNoticeMark(systemImage: "person.crop.circle.badge.checkmark", tint: GlassColor.accentText)
                } summary: {
                    Text(notice.body)
                } details: {
                    Text(notice.folders)
                } actions: { dismissAll }
            })
        }
        if model.showsLegacyMigrationOfferCard, let offer = model.legacyMigrationOffer {
            items.append(GlassPopoverItem(id: "legacy-offer") {
                GlassPopoverNotice(
                    title: offer.title, detailsLabel: details, closeLabel: offer.dismiss,
                    onClose: { model.dismissLegacyMigrationOffer() }
                ) {
                    GlassNoticeMark(systemImage: "person.crop.circle.badge.checkmark", tint: GlassColor.accentText)
                } summary: {
                    Text(offer.body)
                } details: {
                    Text(offer.later)
                } actions: {
                    LegacyMigrationMoveControls(offer: offer, placement: .notice)
                }
            })
        }
        return items
    }

    /// A failed or finished action, from any tab, in front of the rest.
    private var actionItems: [GlassPopoverItem] {
        var items: [GlassPopoverItem] = []
        if let notice = model.lastActionNotice {
            items.append(GlassPopoverItem(id: "action-notice") {
                GlassPopoverNotice(title: notice, closeLabel: closeWord, onClose: { model.lastActionNotice = nil }) {
                    GlassNoticeMark(.ask)
                } summary: { EmptyView() } details: { EmptyView() } actions: { EmptyView() }
            })
        }
        if let error = model.lastActionError {
            items.append(GlassPopoverItem(id: "action-error") {
                GlassPopoverNotice(title: error, closeLabel: closeWord, onClose: { model.lastActionError = nil }) {
                    GlassNoticeMark(.outside)
                } summary: { EmptyView() } details: { EmptyView() } actions: { EmptyView() }
            })
        }
        return items
    }

    // MARK: Traces

    /// What waits on an answer about the traces: the first-contribution
    /// note, the idle notifications offer, the backlog or idle suggestion,
    /// and the arming offer.
    private var tracesOffers: [GlassPopoverItem] {
        var items: [GlassPopoverItem] = []
        // Only on an answered, empty history; whether anything is under
        // review reads the answered queue.
        if !firstContributionDismissed, model.historyAnswered, model.queueAnswered, model.history.isEmpty,
           let copy = model.witnessCopy?.onboarding
        {
            let reviewing = !model.awaitingDecision.isEmpty
            items.append(GlassPopoverItem(id: "first-contribution") {
                GlassPopoverNotice(
                    title: copy.heading, detailsLabel: details,
                    closeLabel: ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord,
                    onClose: { firstContributionDismissed = true }
                ) {
                    GlassNoticeMark(systemImage: "lightbulb", tint: GlassColor.accentText)
                } summary: {
                    Text(reviewing ? copy.review : copy.start)
                } details: {
                    Text(copy.followUp)
                    Text(QueueLegacyWords.agentSetup)
                        .fontWeight(.semibold)
                        .foregroundStyle(GlassColor.textPrimary)
                    Text(copy.agentSetup)
                } actions: { EmptyView() }
            })
        }
        items += offerItems(.traces)
        if let card = traces.nudgeCard {
            items.append(nudgeItem(
                card, busy: traces.nudgeBusy,
                refusal: traces.nudgeError.flatMap { traces.words?.line(for: $0) }
            ) { intent in Task { await traces.perform(intent) } })
        }
        if let offer = model.armingOffer {
            items.append(GlassPopoverItem(id: "arming-offer:" + offer.projectId) {
                ArmingOfferPopover(
                    offer: offer, eyebrow: traces.words?.optionalAutomation,
                    closeLabel: closeWord, onClose: close("arming-offer:" + offer.projectId),
                    onArm: { model.acceptArmingOffer(offer) },
                    onDecline: { model.declineArmingOffer(offer) })
            })
        }
        return items
    }

    /// History's own: its verdict news, and the verdict notifications offer.
    private var historyOffers: [GlassPopoverItem] {
        var items = offerItems(.history)
        if let card = home.verdictsCard {
            items.append(nudgeItem(
                card, busy: home.nudgeBusy,
                refusal: home.nudgeError.flatMap { MonitorWords.table?.line(for: $0) }
            ) { intent in Task { await home.perform(intent) } })
        }
        return items
    }

    /// What is happening to the traces now, in front of the offers: the
    /// consent notices about folders, a folder change's result, the core's
    /// health, and the undos.
    private var tracesNow: [GlassPopoverItem] {
        var items = consentItems
        if let notice = traces.folderNotice {
            let id = "folder-notice:" + notice
            items.append(GlassPopoverItem(id: id) {
                GlassPopoverNotice(title: notice, closeLabel: closeWord, onClose: close(id)) {
                    GlassNoticeMark(.ask)
                } summary: { EmptyView() } details: { EmptyView() } actions: { EmptyView() }
            })
        }
        // Why approved traces are not moving, or that the core is not
        // answering: the list never reads as current over a last good
        // tree. A waiting safeguard can be closed and comes back when its
        // cause recurs; a down or unread core has no X.
        for banner in TracesHealth.banners(
            phase: traces.phase, status: traces.status, words: traces.words, coreDown: TracesHealth.coreDownLine,
            maxQueueEntries: model.daemonSettings?.maxQueueEntries)
        where !banner.title.isEmpty {
            let closeID = Self.healthCloseID(for: banner)
            items.append(GlassPopoverItem(id: closeID ?? "health:" + banner.id) {
                GlassPopoverNotice(
                    title: banner.title, closeLabel: closeID == nil ? nil : closeWord,
                    onClose: closeID.map { closeHealth(banner, id: $0) }
                ) {
                    GlassNoticeMark(banner.tone)
                } summary: {
                    if let detail = banner.detail { Text(detail) }
                } details: { EmptyView() } actions: { EmptyView() }
            })
        }
        items += undoItems
        return items
    }

    /// Read the scope from the same status as the safeguards. An unread
    /// identity never borrows the previous account's presentation choices.
    private var dismissalSnapshot: MonitorNoticeDismissals.Snapshot {
        let status = traces.status
        let account = status?.accountScope ?? status?.tenantId ?? ""
        let scope = MonitorNoticeDismissals.Scope(
            configDirectory: model.configDirectory,
            channel: BuildChannelGate.dylibChannel?.channel ?? "", account: account)
        return .init(scope: scope, conditions: MonitorNoticeDismissals.conditions(
            status: status, loaded: traces.phase == .loaded))
    }

    private var persistentClosed: Set<String> {
        let snapshot = dismissalSnapshot
        return Set(snapshot.conditions.compactMap { kind, condition in
            guard case .active(let incident) = condition,
                  dismissals.isDismissed(incident, scope: snapshot.scope) else { return nil }
            return kind.closeID
        })
    }

    private func closeHealth(_ banner: TracesHealth.Banner, id: String) -> () -> Void {
        let snapshot = dismissalSnapshot
        if let code = banner.condition, let kind = MonitorNoticeDismissals.Kind(rawValue: code),
           let scope = snapshot.scope, case .active(let incident) = snapshot.conditions[kind] {
            return { dismissals.dismiss(incident, scope: scope) }
        }
        return close(id)
    }

    /// The close ids of the closable health banners shown now, read the
    /// way `tracesNow` reads them.
    private var currentHealthIDs: Set<String> {
        Set(TracesHealth.banners(
            phase: traces.phase, status: traces.status, words: traces.words, coreDown: nil,
            maxQueueEntries: model.daemonSettings?.maxQueueEntries
        ).filter { !$0.title.isEmpty }.compactMap(Self.healthCloseID(for:)))
    }

    /// The consent notices about folders: every grant the daemon voided,
    /// every arming rewording, and a gate hold.
    private var consentItems: [GlassPopoverItem] {
        var items: [GlassPopoverItem] = []
        for void in model.status.grantVoids {
            // The ABI words every element; nil is a caught panic or a
            // payload this build cannot decode, and nothing is drawn.
            guard let notice = TCConsentCopy.voidNoticeJSON(forVoid: void.json).flatMap(GrantVoidNotice.decode(fromJSON:))
            else { continue }
            let rearm = notice.rearmTarget(for: void)
            items.append(GlassPopoverItem(id: "void:\(void.id)") {
                GlassPopoverNotice(
                    title: notice.title, detailsLabel: details, closeLabel: notice.acknowledge,
                    onClose: { model.acknowledgeGrantVoid(id: void.id) },
                    refusal: model.grantVoidRearmRefused.contains(void.id) ? notice.rearmFailed : nil
                ) {
                    GlassNoticeMark(systemImage: "lock.shield", tint: GlassStatus.ask.color)
                } summary: {
                    Text(notice.body)
                } details: {
                    NoticeLines(heading: notice.reasonsHeading, bullets: notice.reasons)
                    Text(notice.rearm)
                } actions: {
                    // "Turn back on" sits beside the sentence saying that
                    // doing so agrees to the new settings.
                    if let rearm, let action = notice.rearmAction {
                        Button(action) { model.rearmGrantVoid(id: void.id, projectID: rearm) }
                            .buttonStyle(GlassButtonStyle(.glass))
                            .lineLimit(1)
                    }
                    dismissAll
                }
            })
        }
        for rewording in model.status.armingRewordings {
            guard let notice = TCConsentCopy.armingRewordedNoticeJSON(forRewording: rewording.json)
                .flatMap(ArmingRewordedNotice.decode(fromJSON:))
            else { continue }
            let target = notice.askFirstTarget(for: rewording)
            items.append(GlassPopoverItem(id: "reworded:\(rewording.id)") {
                GlassPopoverNotice(
                    title: notice.title, detailsLabel: details, closeLabel: notice.acknowledge,
                    onClose: { model.acknowledgeArmingRewording(id: rewording.id) },
                    refusal: target.map(model.askFirstRefused.contains) == true ? notice.askFirstFailed : nil
                ) {
                    GlassNoticeMark(systemImage: "lock.shield", tint: GlassStatus.ask.color)
                } summary: {
                    Text(notice.body)
                } details: {
                    NoticeLines(heading: notice.nowHeading, bullets: [])
                    ForEach([notice.scope, notice.limit, notice.noReview], id: \.self) { Text($0) }
                } actions: {
                    if let target, let action = notice.askFirstAction {
                        Button(action) { model.askFirst(projectID: target) }
                            .buttonStyle(GlassButtonStyle(.glass))
                            .lineLimit(1)
                    }
                    dismissAll
                }
            })
        }
        if let held = model.gateHeldNotice {
            // No X: it goes when the hold does.
            items.append(GlassPopoverItem(id: "gate-held") {
                GlassPopoverNotice(title: held.title, detailsLabel: details) {
                    GlassNoticeMark(systemImage: "lock.shield", tint: GlassStatus.ask.color)
                } summary: {
                    Text(held.body)
                } details: {
                    NoticeLines(heading: nil, bullets: held.reasons)
                    Text(held.release)
                    if !held.projects.isEmpty {
                        Text(held.askFirst)
                        ForEach(held.projects, id: \.line) { project in
                            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                                Text(project.line).foregroundStyle(GlassColor.textPrimary)
                                if let projectID = project.projectId, let action = project.askFirstAction {
                                    Button(action) { model.askFirst(projectID: projectID) }
                                        .buttonStyle(GlassButtonStyle(.glass))
                                        .lineLimit(1)
                                }
                                if let projectID = project.projectId, model.askFirstRefused.contains(projectID),
                                   let failed = project.askFirstFailed
                                {
                                    GlassAlert(failed)
                                }
                            }
                        }
                    }
                } actions: { EmptyView() }
            })
        }
        return items
    }

    /// The undos: a keep, a folder's Submit all, a trace's Contribute, and
    /// the approval the app model holds, each while it can still be taken
    /// back. Dismiss is the X: it closes the card and the contribution
    /// stands, as Ron's `UndoBar` does.
    private var undoItems: [GlassPopoverItem] {
        var items: [GlassPopoverItem] = []
        guard let words = traces.words else { return approvalUndo.map { [$0] } ?? [] }
        if let kept = traces.lastKept, let title = FlowMapScene.words?.keptHereOne {
            let busy = traces.acting.contains(kept)
            let id = "kept:" + kept
            let label = traces.tree.allSessions.first { $0.entryId == kept }?.projectLabel
            items.append(GlassPopoverItem(id: id) {
                GlassPopoverNotice(
                    title: title, closeLabel: closeWord, onClose: close(id),
                    refusal: Self.refusal(traces, entryId: kept)
                ) {
                    GlassNoticeMark(systemImage: "tray.and.arrow.down", tint: GlassColor.textSecondary)
                } summary: {
                    if let label { Text(label) }
                } details: { EmptyView() } actions: {
                    Button(busy ? words.undo.undoing : words.undoKeep) {
                        Task { await traces.perform(.undoKeep, on: kept) }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(busy)
                }
            })
        }
        if let folder = traces.lastContributedFolder {
            let busy = traces.writing.contains(folder.projectId)
            items.append(GlassPopoverItem(id: "folder-undo:" + folder.projectId) {
                GlassPopoverNotice(
                    title: InspectorPrompts.title(folder.label, toast: folder.toast, words: words),
                    tag: words.undo.approvalSaved, tagTone: .on,
                    closeLabel: closeWord, onClose: { traces.dismissContributedFolder() }
                ) {
                    GlassNoticeMark(systemImage: "arrow.uturn.backward", tint: GlassStatus.on.color)
                } summary: {
                    UndoDetail(toast: folder.toast, label: folder.label, at: folder.at, words: words)
                } details: { EmptyView() } actions: {
                    if folder.toast.offerUndo {
                        Button(busy ? words.undo.undoing : words.undoContribute) {
                            Task { await traces.undoFolder(folder.projectId) }
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(busy)
                    }
                }
            })
        }
        if let contributed = traces.lastContributed {
            let busy = traces.acting.contains(contributed.entryId)
            items.append(GlassPopoverItem(id: "undo:" + contributed.entryId) {
                GlassPopoverNotice(
                    title: InspectorPrompts.title(contributed.label, toast: contributed.toast, words: words),
                    tag: words.undo.approvalSaved, tagTone: .on,
                    closeLabel: closeWord, onClose: { traces.dismissContributed() },
                    refusal: Self.refusal(traces, entryId: contributed.entryId)
                ) {
                    GlassNoticeMark(systemImage: "arrow.uturn.backward", tint: GlassStatus.on.color)
                } summary: {
                    UndoDetail(toast: contributed.toast, label: contributed.label, at: contributed.at, words: words)
                } details: { EmptyView() } actions: {
                    if contributed.toast.offerUndo {
                        Button(busy ? words.undo.undoing : words.undoContribute) {
                            Task { await traces.perform(.undoContribute, on: contributed.entryId) }
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(busy)
                    }
                }
            })
        }
        if let approval = approvalUndo { items.append(approval) }
        return items
    }

    /// Ron's `UndoBar` over the approval the app model holds: the core's
    /// "APPROVAL SAVED", the core's toast, then how long ago. No timer
    /// removes it: the deadline is the daemon's next upload sweep, which
    /// nothing here can observe.
    private var approvalUndo: GlassPopoverItem? {
        guard let undo = model.undo else { return nil }
        let words = traces.words
        return GlassPopoverItem(id: "approval-undo:" + undo.entryIDs.joined(separator: ",")) {
            GlassPopoverNotice(
                title: undo.toastLine, tag: words?.undo.approvalSaved, tagTone: .on,
                closeLabel: undo.offerUndo ? QueueLegacyWords.closeNoticeStillSends : QueueLegacyWords.closeNotice,
                onClose: { model.dismissUndo() }
            ) {
                GlassNoticeMark(systemImage: "arrow.uturn.backward", tint: GlassStatus.on.color)
            } summary: {
                if undo.offerUndo {
                    Text(QueueLegacyWords.undoWillSend)
                    Text(QueueLegacyWords.approvedAgo(undo.heldSeconds)).monospacedDigit()
                }
            } details: { EmptyView() } actions: {
                if undo.offerUndo {
                    // The app's one Return binding, on the safe action: a
                    // keystroke pulls a transcript back.
                    Button(words?.undoContribute ?? QueueLegacyWords.undo) { model.undoApproval() }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .keyboardShortcut(.defaultAction)
                }
            }
        }
    }

    /// The core's words for a refused review action on `entryId`, if one is
    /// held (`TracesRefusal`).
    static func refusal(_ store: TracesStore, entryId: String) -> String? {
        guard let refused = store.actionError, refused.entryId == entryId else { return nil }
        return store.message(for: refused.error)
    }

    /// The one-time offer for a page's own kind -- the verdicts offer on
    /// History, the idle one on Traces -- in the core's words, as Settings
    /// draws it. Not now closes it until the window opens again; No thanks
    /// answers it.
    private func offerItems(_ place: NudgeSurface.Place) -> [GlassPopoverItem] {
        offers.offers(on: place).map { offer in
            let id = "notify-offer:" + offer.kind
            return GlassPopoverItem(id: id) {
                GlassPopoverNotice(
                    title: offer.text, closeLabel: closeWord, onClose: close(id),
                    refusal: offers.refusedOfferKind == offer.kind
                        ? offers.writeError.flatMap { MonitorWords.table?.line(for: $0) } : nil
                ) {
                    GlassNoticeMark(systemImage: "bell", tint: GlassColor.accentText)
                } summary: { EmptyView() } details: { EmptyView() } actions: {
                    // The accept first as a button, the decline after it as
                    // a link: every card's order (Ron, 2026-10-09).
                    Button(offer.accept) {
                        Task {
                            await offers.accept(offer) { _ = await Notifier.shared.requestAuthorizationIfNeverAsked() }
                        }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(offers.writing)
                    Button(offer.decline) { Task { await offers.answer(offer, accept: false) } }
                        .buttonStyle(GlassButtonStyle(.link))
                        .disabled(offers.writing)
                }
            }
        }
    }

    /// A nudge card in the daemon's words: its title, its body, and its
    /// buttons in the daemon's order, Not now as the link after the action.
    private func nudgeItem(
        _ card: NudgeSurface.Card, busy: Bool, refusal: String?, act: @escaping (NudgeSurface.Intent) -> Void
    ) -> GlassPopoverItem {
        let id = "nudge:\(card.kind)"
        return GlassPopoverItem(id: id) {
            GlassPopoverNotice(title: card.title, closeLabel: closeWord, onClose: close(id), refusal: refusal) {
                GlassNoticeMark(systemImage: "clock", tint: GlassColor.accentText)
            } summary: {
                if let body = card.body { Text(body) }
            } details: { EmptyView() } actions: {
                ForEach(Array(card.actions.enumerated()), id: \.offset) { _, action in
                    Button(action.label) { act(action.intent) }
                        .buttonStyle(GlassButtonStyle(NudgeGlassCard.style(action.intent)))
                        .disabled(busy)
                }
            }
        }
    }

    // MARK: Private AI

    /// The offer to route AI requests through NEAR AI, and the NEAR AI
    /// sign-in while there is none.
    private var privateAIItems: [GlassPopoverItem] {
        var items: [GlassPopoverItem] = []
        if model.showsPrivateInferenceOffer, let copy = model.privateInferenceCopy {
            items.append(GlassPopoverItem(id: "private-ai-offer") {
                // The X answers as Not now does: the offer is asked once, so
                // closing it is an answer, not a deferral (owner,
                // 2026-10-08). What it does and, in one line, what it
                // exposes; the rest behind Details.
                GlassPopoverNotice(
                    title: copy.offerTitle, tag: copy.destination, tagTone: .accent,
                    detailsLabel: details, closeLabel: copy.offerDecline,
                    onClose: { model.answerPrivateInferenceOffer(accepted: false) }
                ) {
                    GlassNoticeMark(systemImage: "shield.lefthalf.filled", tint: GlassColor.accentText)
                } summary: {
                    Text(copy.offerWhat)
                    Text(copy.offerExposureShort)
                } details: {
                    Text(copy.offerExposure)
                    Text(copy.offerNoRepoint)
                    Text(copy.offerAskedOnce)
                } actions: {
                    Button(copy.offerAccept) { model.answerPrivateInferenceOffer(accepted: true) }
                        .buttonStyle(GlassButtonStyle(.glass))
                    Button(copy.offerDecline) { model.answerPrivateInferenceOffer(accepted: false) }
                        .buttonStyle(GlassButtonStyle(.link))
                }
                .disabled(model.privateInferenceBusy)
            })
        }
        if let snapshot = model.managedSnapshot, !snapshot.nearAiSignedIn {
            items.append(GlassPopoverItem(id: "near-ai-sign-in") {
                GlassPopoverNotice(
                    title: model.managedText("near_ai"), closeLabel: closeWord, onClose: close("near-ai-sign-in")
                ) {
                    GlassNoticeMark(systemImage: "sparkles", tint: GlassColor.accentText)
                } summary: {
                    Text(model.managedText("sign_in_needed"))
                } details: { EmptyView() } actions: {
                    Button(model.managedText("sign_in_near_ai")) { NearAiSignIn.begin(model, openURL: openURL) }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .lineLimit(1)
                        .accessibilityLabel(model.managedText("sign_in_near_ai_accessibility"))
                        .disabled(model.credentialBusy)
                }
            })
        }
        return items
    }
}

/// The NEAR AI sign-in, begun as Settings > Private AI begins it: the
/// daemon starts the ceremony and the browser finishes it. A browser that
/// never opened leaves a ceremony nobody can finish, so it is stopped
/// rather than left to time out. It names no provider, so the daemon starts
/// this build's default (`channel::NEAR_AI_DEFAULT_SIGN_IN_PROVIDER`): the
/// NEAR wallet in production, NEAR AI's Google login in staging.
@MainActor
enum NearAiSignIn {
    static func begin(_ model: AppModel, openURL: OpenURLAction, then: (() -> Void)? = nil) {
        Task {
            guard let url = await model.startNearAiCredential() else { return }
            let accepted = await openURL.opened(url)
            if !accepted { model.cancelNearAiCredential() }
            then?()
        }
    }
}

/// A notice's heading line and its reasons, as bullets. The dot is
/// decoration: each reason's own words are the element.
private struct NoticeLines: View {
    let heading: String?
    let bullets: [String]

    var body: some View {
        if let heading {
            Text(heading)
                .fontWeight(.semibold)
                .foregroundStyle(GlassColor.textPrimary)
        }
        ForEach(bullets, id: \.self) { reason in
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s2) {
                Image(systemName: "circle.fill")
                    .imageScale(.small)
                    .scaleEffect(0.4)
                    .accessibilityHidden(true)
                Text(reason)
            }
        }
    }
}

/// The offer to stop being asked about one project, as a popover notice
/// (`ArmingOfferGlassCard`'s words and order): the evidence under the
/// question, what arming does behind Details, and the confirm, which opens
/// a whole-window confirmation with the core's body before anything is
/// armed. Arming is a grant, so nothing is drawn without the core's words.
private struct ArmingOfferPopover: View {
    let offer: ArmingOffer
    /// The core's "OPTIONAL AUTOMATION"; nil draws no tag.
    let eyebrow: String?
    let closeLabel: String
    let onClose: () -> Void
    let onArm: () -> Void
    let onDecline: () -> Void
    @State private var confirming = false

    private var copy: ProjectArmingCopy? {
        ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(
            project: offer.projectLabel, count: offer.contributedCount))
    }

    var body: some View {
        if let copy {
            GlassPopoverNotice(
                title: copy.question, tag: eyebrow, tagTone: .neutral,
                detailsLabel: MonitorWords.noticeDetails, closeLabel: closeLabel, onClose: onClose
            ) {
                GlassNoticeMark(systemImage: "bolt", tint: GlassColor.accentText)
            } summary: {
                Text(copy.evidence)
            } details: {
                Text(copy.body)
            } actions: {
                Button(copy.confirm) { confirming = true }
                    .buttonStyle(GlassButtonStyle(.glass))
                Button(copy.decline, action: onDecline)
                    .buttonStyle(GlassButtonStyle(.link))
            }
            // Declining here closes the confirmation only; the offer stays
            // until it is answered on the card.
            .glassModal(isPresented: $confirming) {
                GlassConfirmation(
                    title: copy.question, message: copy.body,
                    actions: [
                        .cancel(copy.decline) { confirming = false },
                        GlassModalAction(copy.confirm, isDefault: true) {
                            confirming = false
                            onArm()
                        },
                    ],
                    onCancel: { confirming = false })
            }
        }
    }
}
