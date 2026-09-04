import SwiftUI
import TCShellCore

/// The queue: one per session waiting for a decision.
///
/// Every row also carries a `Submit` action, and every project group a
/// `Submit all`: one-click submit
/// (`docs/superpowers/specs/2026-08-20-one-click-submit-design.md`) means
/// approval no longer requires opening the session first. This corrects the
/// stricter rule this comment used to state -- preview-then-approve only --
/// which was written when an approval with no prior preview silently
/// uploaded bytes nobody was shown; the daemon now builds and pins the
/// envelope itself when none exists, so a blind Submit sends exactly what a
/// preview would have shown, never something unseen.
///
/// `Look inside` stays the row's primary action, with its original
/// emphasis -- see `QueueRow.actions` for why: one click is availability,
/// not a recommendation to skip looking.
struct QueueView: View {
    @EnvironmentObject private var model: AppModel
    @State private var previewing: QueueEntry?

    var body: some View {
        ScrollView {
            QueueContent(previewing: $previewing)
        }
        .sheet(item: $previewing) { entry in
            PreviewSheet(entry: entry)
                .environmentObject(model)
        }
        .onChange(of: model.awaitingDecision.count) { _, _ in
            // Development hook: opens the first preview so the sheet can be
            // captured on a locked session. Never on by default.
            if ProcessInfo.processInfo.environment["TRACE_COMMONS_DEMO_PREVIEW"] == "1",
               previewing == nil,
               let first = model.awaitingDecision.first
            {
                previewing = first
            }
        }
    }
}

/// The queue's content, split out of its `ScrollView` so the screenshot hook
/// can rasterize the real rows: `ImageRenderer` renders a `ScrollView` as
/// blank.
struct QueueContent: View {
    @EnvironmentObject private var model: AppModel
    @Binding var previewing: QueueEntry?
    /// What this run of the view currently believes is on screen, purely so
    /// each row's `onAppear`/`onDisappear` can report a delta rather than
    /// the whole set every time. The daemon's actual idea of "visible" is
    /// `AppModel`'s, behind `setPreviewVisible` -- this is not published and
    /// nothing reads it directly.
    @State private var visibleRowIDs: Set<String> = []
    /// Which level of the queue is showing. Resolved against the live
    /// groups on every redraw (`QueueNavigation.resolve`), so a folder that
    /// empties while it is open returns to the list rather than rendering
    /// an empty detail view.
    @State private var location: QueueLocation = .root

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.md) {
            if model.history.isEmpty, let copy = model.witnessCopy?.onboarding {
                VStack(alignment: .leading, spacing: TC.Space.s) {
                    Text(copy.heading).font(TC.Font_.cardTitle)
                    Text(model.awaitingDecision.isEmpty ? copy.start : copy.review)
                    Text(copy.followUp)
                    DisclosureGroup("Agent setup") { Text(copy.agentSetup) }
                }
                .font(TC.Font_.caption)
                .padding(TC.Space.md)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(TC.surface)
            }

            if let health = model.health {
                HealthBanner(health: health)
            }
            if let budget = model.budgetHealth {
                HealthBanner(health: budget)
            }
            if let undo = model.undo {
                UndoBar(
                    undo: undo,
                    onUndo: { model.undoApproval() },
                    onKeep: { model.dismissUndo() }
                )
            }
            if let error = model.lastActionError {
                Text(error)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
            }
            if let notice = model.lastActionNotice {
                Text(notice)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
            }

            if let offer = model.armingOffer {
                ArmingOfferCard(
                    offer: offer,
                    onArm: { model.acceptArmingOffer(offer) },
                    onDecline: { model.declineArmingOffer(offer) }
                )
            }

            if model.awaitingDecision.isEmpty {
                CenteredNotice(
                    title: "Nothing is waiting.",
                    detail: """
                    When a session finishes and goes quiet, it shows up here. \
                    Nothing is sent unless you say so.
                    """
                )
                .frame(minHeight: 220)
            } else {
                waiting
            }

            NotOfferedDisclosure(counts: model.outcomeCounts)

            if let rollup = model.rollup {
                WeekBand(week: rollup.week, quarantined: rollup.quarantined)
            }
        }
        .padding(.horizontal, TC.Space.Content.horizontal)
        .padding(.top, TC.Space.Content.top)
        .padding(.bottom, TC.Space.Content.bottom)
        .tcColumn()
        .tcScreen()
    }

    /// The queue's two levels, resolved against what the queue currently
    /// holds rather than against what it held when the folder was opened.
    private var waiting: some View {
        let here = QueueNavigation.resolve(location, in: model.waitingByProject)
        return VStack(alignment: .leading, spacing: TC.Space.md) {
            switch here {
            case .root:
                folderList
            case .project(let id):
                if let group = model.waitingByProject.first(where: { $0.id == id }) {
                    folderDetail(group)
                }
            }

            // The mechanism's limits, stated once for the list rather than
            // stamped on every card -- see `ScrubbingCaveat`, which also
            // records why this sentence is NOT the design's longer standing
            // disclaimer.
            ScrubbingCaveatNote()
                .padding(.top, TC.Space.xxs)
        }
        // Writing the resolved location back is what makes a vanished
        // folder's back button unnecessary rather than broken.
        .onChange(of: model.waitingByProject.map(\.id)) { _, _ in
            location = QueueNavigation.resolve(location, in: model.waitingByProject)
        }
    }

    /// The root level: one row per folder, folder name first.
    ///
    /// Grouped by project so `Submit all` has something honest to point at
    /// -- a group is exactly what `submitProject` acts on, never a slice the
    /// UI made up. `waitingByProject`'s order is first-seen, which is also
    /// `awaitingDecision`'s order, so this reshuffles nothing a contributor
    /// has already scanned.
    private var folderList: some View {
        VStack(alignment: .leading, spacing: TC.Space.md) {
            // Left as a sentence, not compressed into a label-and-count
            // header. It is the one line on this screen written in the
            // product's voice and it says what the screen is FOR.
            Text("^[\(model.decisionsOwed) session](inflect: true) waiting for your decision")
                .font(TC.Font_.sectionTitle)
                .foregroundStyle(TC.inkPrimary)

            LazyVStack(spacing: TC.Space.md) {
                ForEach(model.waitingByProject) { group in
                    QueueFolderRow(
                        group: group,
                        onOpen: { location = .project(group.id) },
                        onSubmitAll: { model.submitProject(id: group.id) },
                        onSubmitAllAs: { model.submitProject(id: group.id, verdict: $0) },
                        onIgnoreProject: {
                            model.ignoreProject(
                                id: group.id,
                                label: group.label,
                                promised: group.count
                            )
                        }
                    )
                }
            }
        }
    }

    /// One folder's sessions, a level in from the list.
    ///
    /// The folder's own actions stay on the row one level up rather than
    /// being repeated here: two copies would be two things to keep in step.
    private func folderDetail(_ group: QueueGroup<QueueEntry>) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.md) {
            Button {
                location = .root
            } label: {
                HStack(spacing: TC.Space.xs) {
                    QueueGlyph(glyph: .chevronLeft, size: 11, color: TC.inkSecondary)
                    Text("All folders")
                        .font(TC.Font_.meta)
                        .foregroundStyle(TC.inkSecondary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                Text(group.label)
                    .font(TC.Font_.sectionTitle)
                    .foregroundStyle(TC.inkPrimary)
                if let path = group.entries.first?.projectPath, !path.isEmpty {
                    Text(path)
                        .font(TC.Font_.meta)
                        .foregroundStyle(TC.inkSecondary)
                        .textSelection(.enabled)
                }
            }

            ProjectQueueGroup(
                group: group,
                summaries: model.summaries,
                summaryErrors: model.summaryErrors,
                tooLarge: model.tooLarge,
                onLookInside: { previewing = $0 },
                onSearch: { previewing = $0 },
                onSubmit: { model.approve($0) },
                onDismiss: { model.dismiss($0) },
                onAppear: { entry in
                    model.requestPreview(for: entry)
                    visibleRowIDs.insert(entry.entryID)
                    model.setPreviewVisible(visibleRowIDs)
                },
                onDisappear: { entry in
                    visibleRowIDs.remove(entry.entryID)
                    model.setPreviewVisible(visibleRowIDs)
                }
            )
        }
    }
}

/// One folder's rows, at the queue's second level.
///
/// It used to carry the group header too -- the label, `Submit all`,
/// `Submit all as` and `Ignore`. Those moved up to `QueueFolderRow` when the
/// queue became folder-first: a folder's actions belong beside the folder,
/// and stating them in both places would be two things to keep in step.
private struct ProjectQueueGroup: View {
    let group: QueueGroup<QueueEntry>
    let summaries: [String: PreviewSummary]
    let summaryErrors: [String: String]
    let tooLarge: [String: PreviewTooLarge]
    let onLookInside: (QueueEntry) -> Void
    let onSearch: (QueueEntry) -> Void
    let onSubmit: (QueueEntry) -> Void
    let onDismiss: (QueueEntry) -> Void
    /// Called when a row actually appears on screen -- `AppModel.requestPreview(for:)`,
    /// which is where the daemon-side scheduler dedupe lives. See `rowList`
    /// for why this is what drives loading at all now.
    let onAppear: (QueueEntry) -> Void
    /// Called when a row leaves the screen. Updates visibility priority
    /// only -- never cancels the scheduled preview; a card that scrolls
    /// away keeps its place in the daemon's queue until it is dismissed or
    /// leaves the pending list for good (`AppModel.applyPendingUpdate`).
    let onDisappear: (QueueEntry) -> Void

    var body: some View {
        rowList
    }

    /// The rows themselves, split from `body` so a real queue -- a
    /// contributor with 500 sessions waiting is the case this exists for --
    /// only realizes the cards near the viewport instead of building and
    /// measuring all 500 up front.
    ///
    /// `LazyVStack` is the fix, but it carries the same hazard `QueueContent`
    /// already works around: laid out with no ancestor `ScrollView`, a lazy
    /// stack can render empty under `ImageRenderer`
    /// (`DebugScreenshot.scheduleIfRequested` renders `QueueContent` directly
    /// at a fixed size, without the real `ScrollView` `QueueView` normally
    /// wraps it in). So the screenshot hook gets the eager, always-correct
    /// `VStack` -- same spacing, same default `.center` alignment `LazyVStack`
    /// also defaults to -- and everyone else gets the lazy one.
    ///
    /// This is also what makes rows the thing that requests its own preview
    /// (`rows`'s `.onAppear`, `AppModel.requestPreview(for:)`) rather than
    /// the model asking for all of them at snapshot time: a `LazyVStack`
    /// row's `onAppear` fires exactly when
    /// SwiftUI actually realizes it, so under the real `ScrollView` a
    /// contributor with 500 sessions only ever asks for the handful near the
    /// visible area. Under the screenshot hook's eager `VStack` every row
    /// realizes immediately, so every row's `onAppear` fires immediately too
    /// -- which is correct there: a screenshot needs every card populated,
    /// and that path is a fixed-size dev tool, not the 500-session case this
    /// guards against.
    @ViewBuilder
    private var rowList: some View {
        if DebugScreenshot.directory != nil {
            VStack(spacing: TC.Space.md) { rows }
        } else {
            LazyVStack(spacing: TC.Space.md) { rows }
        }
    }

    private var rows: some View {
        ForEach(group.entries) { entry in
            QueueRow(
                entry: entry,
                summary: summaries[entry.entryID],
                summaryError: summaryErrors[entry.entryID],
                tooLarge: tooLarge[entry.entryID],
                onLookInside: { onLookInside(entry) },
                onSearch: { onSearch(entry) },
                onSubmit: { onSubmit(entry) },
                onDismiss: { onDismiss(entry) }
            )
            .onAppear { onAppear(entry) }
            .onDisappear { onDisappear(entry) }
        }
    }
}

/// One waiting session, laid out as a declaration: who it is from, what it
/// says, and a fixed manifest strip of what would actually leave the
/// machine.
///
/// The strip is in the same place on every card on purpose. Reading the
/// third card should not require reading it -- only checking whether the
/// figures in the two familiar slots look like the figures on the card
/// above.
struct QueueRow: View {
    let entry: QueueEntry
    let summary: PreviewSummary?
    let summaryError: String?
    /// Set when the daemon's preview scheduler refused this session for
    /// being over the admission cap. Renders as "too large to preview" plus
    /// the raw stat -- never a would-send estimate; see `PreviewTooLarge`.
    let tooLarge: PreviewTooLarge?
    let onLookInside: () -> Void
    /// Opens this session's search. Distinct from `onLookInside` as an
    /// intent even though the two currently coincide: the sheet opens on its
    /// Search tab already (`PreviewSheet.tab` starts at `.search`), so there
    /// is nothing to select and no `initialTab` to add. If the sheet ever
    /// opens somewhere else, this is the seam that keeps the chip pointing
    /// at the thing to do about it.
    let onSearch: () -> Void
    let onSubmit: () -> Void
    let onDismiss: () -> Void

    /// How many values scrubbing actually REMOVED.
    ///
    /// Not the sum of `redactions`: that map also carries
    /// `residual_secret_at:*`, which counts a secret the scan FOUND and did
    /// not remove. Counting those here made a session with a surviving
    /// secret look like one scrubbing had cleaned, and took it out of the
    /// gold tone that asks somebody to look. See `RedactionLabels`.
    private var redactionCount: Int {
        RedactionLabels.removedTotal(summary?.redactions ?? [:])
    }

    /// Secrets the scan found and left in what would be sent, if any.
    private var survivorLine: String? {
        guard let summary else { return nil }
        return RedactionLabels.survivorLine(summary.redactions)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            identity
            prompt
            footer
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard(emphasised: summary != nil && redactionCount == 0)
        // A second route to `Look inside`, never a replacement for it. The
        // button keeps its emphasis: one-click submit added AVAILABILITY,
        // and primary styling is a RECOMMENDATION -- see `actions`. What
        // this adds is that the obvious gesture on a card does the obvious
        // thing.
        //
        // The three footer buttons are `Button`s inside the card and consume
        // their own taps, so `Not this one`, `Submit` and `Look inside` keep
        // doing their own jobs.
        .contentShape(Rectangle())
        .onTapGesture(perform: onLookInside)
        .accessibilityElement(children: .contain)
    }

    // MARK: - Identity

    /// The project label, the agent, and -- when the session did not run at
    /// the project root -- the subdirectory it did run in.
    ///
    /// `sessionPath` is the contract's one display-path relaxation
    /// (`ipc::display_path`), for the screen only. Nothing here is logged or
    /// notified.
    private var identity: some View {
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
            VStack(alignment: .leading, spacing: TC.Space.micro) {
                Text(entry.projectLabel)
                    .font(TC.Font_.cardTitle)
                    .foregroundStyle(TC.inkPrimary)
                // Where this session actually ran, when that is not the
                // project root. A folder of sessions from one repository
                // otherwise says nothing about which of them came from
                // where. Absent when the daemon predates the field and when
                // the session ran at the root, which is the same rendering
                // either way: a line only when it says something.
                if let sessionPath = entry.sessionPath, !sessionPath.isEmpty {
                    Text(sessionPath)
                        .font(TC.Font_.meta)
                        .foregroundStyle(TC.inkTertiary)
                        .lineLimit(1)
                        .truncationMode(.head)
                }
            }
            Text(entry.agentName)
                .font(TC.Font_.meta)
                .foregroundStyle(TC.inkSecondary)
            Spacer(minLength: TC.Space.m)
            Text(Format.when(entry.discoveredAt))
                .font(TC.Font_.meta)
                .foregroundStyle(TC.inkTertiary)
        }
        .padding(.horizontal, TC.Space.l)
        .padding(.top, TC.Space.md)
        .padding(.bottom, TC.Space.s)
    }

    /// The redacted opening prompt is what identifies a session to its
    /// author; a timestamp is not. It gets the most room on the card.
    @ViewBuilder
    private var prompt: some View {
        Group {
            if let summary {
                Text(summary.openingPrompt.isEmpty ? "(no opening prompt)" : summary.openingPrompt)
                    .font(TC.Font_.body)
                    .foregroundStyle(TC.inkPrimary)
                    .lineSpacing(TC.Font_.LineHeight.spacing(for: 13, TC.Font_.LineHeight.body))
                    .lineLimit(3)
                    .textSelection(.enabled)
            } else if let tooLarge {
                // Exactly `raw_session_bytes`, a `stat`, and nothing derived
                // from it -- never a synthesized would-send figure. See the
                // scheduler design's "Admission control by size": this card
                // is a consent surface, and a plausible-looking wrong number
                // here is worse than no number.
                Text("Too large to preview (\(Format.bytes(tooLarge.rawSessionBytes))).")
                    .font(TC.Font_.body)
                    .foregroundStyle(.secondary)
            } else if let summaryError {
                Text("Couldn't read this one yet (\(summaryError)). Nothing has been sent.")
                    .font(TC.Font_.body)
                    .foregroundStyle(.secondary)
            } else {
                Text("Reading it locally…")
                    .font(TC.Font_.body)
                    .foregroundStyle(.secondary)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, TC.Space.l)
        .padding(.bottom, TC.Space.m)
    }

    // MARK: - The manifest strip

    /// The signature element, and the card's only footer.
    ///
    /// The manifest and the two buttons share one band rather than stacking
    /// into two. Stacked, they left the card with a tall empty strip under
    /// the actions and a wide empty gutter beside the figures -- the same
    /// slackness the community site avoids by banding its content across the
    /// full measure. Here the labelled figures sit at the leading edge and
    /// the decision sits at the trailing edge of the same line, which is
    /// also the shortest path from "3 KB, 4 removed" to "look inside".
    private var footer: some View {
        HStack(alignment: .bottom, spacing: TC.Space.l) {
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                if let summary {
                    HStack(alignment: .top, spacing: TC.Space.xxlSmall) {
                        cell("Would send") {
                            Text(Format.bytes(summary.wouldSendBytes))
                                .font(TC.Font_.ledger)
                                .monospacedDigit()
                                .foregroundStyle(TC.inkPrimary)
                        }
                        cell("Removed by pattern") {
                            if redactionCount == 0 {
                                nothingMatchedChip
                            } else {
                                Text(RedactionLabels.line(
                                    occurrences: summary.redactions,
                                    distinct: summary.redactionsDistinct
                                ))
                                    .font(TC.Font_.ledger)
                                    .foregroundStyle(TC.inkPrimary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                    caption
                    survivor
                }
                extent
            }
            Spacer(minLength: TC.Space.m)
            actions
        }
        .padding(.horizontal, TC.Space.l)
        .padding(.vertical, TC.Space.sm)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(TC.surfaceInset)
        .overlay(alignment: .top) {
            Rectangle().fill(TC.line).frame(height: TC.Space.hairline)
        }
    }

    /// What scrubbing did to THIS session, and what that does not prove.
    ///
    /// The sentences are `ScrubbingCaveat`'s, not the design's: that type
    /// records why the row line varies with what scrubbing actually did, and
    /// the design's fixed caption is the constant it was written against.
    /// What this pass takes from the design is the treatment -- a session
    /// where nothing matched is the one worth slowing down on, so it is the
    /// case that gets the gold rather than the strip's grey.
    private var caption: some View {
        Text(ScrubbingCaveat.rowLine(redactionCount: redactionCount))
            .font(TC.Font_.footnote)
            .foregroundStyle(ScrubbingCaveat.tone(redactionCount: redactionCount).textColor)
            .lineSpacing(TC.Font_.LineHeight.spacing(for: 10, TC.Font_.LineHeight.caption))
            .fixedSize(horizontal: false, vertical: true)
    }

    /// What this one card actually covers, and -- the half the contract makes
    /// mandatory -- whether any of it was left out to fit.
    ///
    /// Outside the `if let summary` above on purpose: both counts are
    /// load-time facts carried on the entry itself, so this line is as true
    /// while the card still reads "Reading it locally…" as it is after the
    /// preview lands. A trimmed conversation must not be able to reach a
    /// decision through a card that never got a preview.
    ///
    /// Absent entirely when there is nothing to report, so a session that
    /// delegated nothing carries no line about subagents at all.
    @ViewBuilder
    private var extent: some View {
        if let line = entry.subagentLine {
            Text(line)
                .font(TC.Font_.footnote)
                .foregroundStyle(entry.wasTrimmed ? TC.goldText : TC.inkSecondary)
                .lineSpacing(TC.Font_.LineHeight.spacing(for: 10, TC.Font_.LineHeight.caption))
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityLabel(line)
        }
    }

    /// A secret the scan FOUND and did not remove.
    ///
    /// Excluding survivors from the figures is only half the fix: filtering
    /// one out and then saying nothing would trade a wrong statement for
    /// silence about a secret still in the payload, which on a consent
    /// surface is not an improvement. So it gets its own line, in the tone
    /// the card already uses for things worth slowing down on.
    @ViewBuilder
    private var survivor: some View {
        if let survivorLine {
            HStack(spacing: TC.Space.xxs) {
                QueueGlyph(glyph: .triangle, size: 11, stroke: 1.6, color: TC.gold)
                Text(survivorLine)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.goldText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(survivorLine)
        }
    }

    /// The gold chip that replaces the removed-by-pattern figure when nothing
    /// matched. It carries the warning triangle without its dot -- the same
    /// glyph the health banner uses, quieter, because this is a thing to weigh
    /// rather than a thing to fix.
    private var nothingMatchedChip: some View {
        // A control, not a label. The gold is right and stays gold -- a
        // session where no pattern fired is the one worth slowing down on --
        // but a judgement with nothing to do about it is where a contributor
        // stops. Searching is the thing to do about it.
        Button(action: onSearch) {
            HStack(spacing: TC.Space.xxs) {
                QueueGlyph(glyph: .triangle, size: 11, stroke: 1.6, color: TC.gold)
                Text(RedactionLabels.nothingMatched)
                    .font(TC.Font_.monoChip)
                    .foregroundStyle(TC.goldText)
            }
            .padding(.horizontal, TC.Space.s)
            .padding(.vertical, 3)
            .overlay {
                Capsule().strokeBorder(
                    TC.gold.opacity(TC.Border.chipAlpha),
                    lineWidth: TC.Border.hairline
                )
            }
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .help("Opens this session's search, which is the thing to do about it.")
        .accessibilityLabel("Nothing matched a pattern. Search this session.")
    }

    private func cell<Value: View>(
        _ label: String,
        @ViewBuilder value: () -> Value
    ) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xxs) {
            TCFieldLabel(label)
            value()
        }
        .accessibilityElement(children: .combine)
    }

    // MARK: - Actions

    /// Three actions at the trailing edge, adjacent, default action last --
    /// the macOS convention, and one eye movement instead of the full width
    /// of the window.
    ///
    /// `Look inside` keeps its original emphasis as the row's primary
    /// action. One-click submit adds AVAILABILITY -- a contributor CAN
    /// decide without opening the session -- but primary styling is a
    /// RECOMMENDATION, and only the first was asked for. This product's
    /// pitch to a contributor is "that scrubbing is good and it is not
    /// perfect -- which is why you get to look first"; promoting `Submit`
    /// above `Look inside` would quietly change what the app advises on the
    /// screen where that advice matters most. (Decided 2026-08-20, after
    /// this file briefly made `Submit` the accented default; see the
    /// one-click-submit design doc.) `Submit` therefore renders as a peer of
    /// `Not this one` -- same weight class, untinted -- not demoted below
    /// them and not promoted above `Look inside`.
    private var actions: some View {
        HStack(spacing: TC.Space.s) {
            Button("Not this one", action: onDismiss)
                // Untinted on purpose. A bordered button inherits the
                // app accent, and "Not this one" rendered in the same
                // green as "Look inside" reads as a second approval.
                .tint(.primary)
                // Says "for good" because it is: a dismissal is a decision
                // about the conversation, not about the size it happened to
                // be when this card was drawn, and there is no un-dismiss.
                // The second sentence keeps the first from reading like an
                // opt-out of the whole project.
                .help("""
                Skips this session for good, even if you keep working in it. \
                This project will keep being offered.
                """)
            Button("Submit", action: onSubmit)
                // Untinted, and the same weight as "Not this one": a
                // shortcut is not a recommendation. See the note above.
                .tint(.primary)
                .help("""
                Sends this session now. Scrubbing runs the same as it always does, and \
                you'll get a moment to undo.
                """)
            // No keyboard shortcut. Return used to be bound here as the
            // default action, which meant a two-row queue registered the
            // same shortcut twice and neither row could say which one a
            // keystroke would open. In this app Return is reserved for the
            // recovery surface -- see `UndoBar` -- and is bound to nothing
            // that moves a transcript.
            Button("Look inside", action: onLookInside)
                .tcPrimaryAction()
                .help("Opens the redacted preview before deciding.")
        }
        .fixedSize()
    }
}

/// The week so far, as a band of labelled figures across the full measure.
///
/// It sits at the foot of the queue rather than the head: the screen's job
/// is decisions, and a summary above the list would push the decisions down
/// to make room for something nobody opened this window to read. At the foot
/// it uses space the list was leaving empty and answers the question a
/// person has once they are done deciding -- what has this thing actually
/// done with my work. The figures are the same three the menu bar and
/// History report, in the same words, so the three surfaces never disagree.
///
/// This is the community site's KPI band: uppercase label over a large
/// figure, ruled off, spread across the measure.
struct WeekBand: View {
    let week: HistoryCounts
    let quarantined: Int

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            TCSectionHeader(title: "This week")
            HStack(alignment: .top, spacing: TC.Space.m) {
                figure("Contributed", week.submitted, TC.greenText, .checkCircle)
                figure("Held for privacy review", quarantined, TC.blueIcon, .clock)
                figure("In the commons", week.accepted, TC.inkSecondary, .columns)
            }
        }
        .padding(.top, TC.Space.s)
    }

    private func figure(
        _ title: String,
        _ count: Int,
        _ ink: Color,
        _ glyph: QueueGlyphs
    ) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            HStack(spacing: TC.Space.xs) {
                QueueGlyph(glyph: glyph, size: 11, color: ink)
                TCFieldLabel(title)
            }
            Text("\(count)")
                .font(TC.Font_.metricValue)
                .monospacedDigit()
                .foregroundStyle(TC.inkPrimary)
        }
        .padding(TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(title) this week: \(count)")
    }
}

/// The submit toast, and -- when one is offered -- the recovery surface
/// behind it. `undo.toastLine` is `SubmitToast.line`, verbatim: see the note
/// on that type for why the wording is a contract nothing here may reword.
/// `Undo` is backed by `cancel`, which returns each entry to pending, so the
/// undo is real.
///
/// It sits at the head of the queue, which is where the decision now ends:
/// the preview sheet closes on a decision instead of loading the next session
/// into itself, so this is on screen and not behind a sheet at the moment it
/// is needed. That ordering is the whole point -- an undo rendered under a
/// modal is an undo that does not exist.
///
/// **No countdown.** See `AppModel.Undo`: the real deadline is the daemon's
/// next upload sweep and this process cannot observe it, so the bar counts up
/// from a real instant and says what it actually knows. It does not disappear
/// on a timer, because a recovery path that removes itself while recovery is
/// still possible is worse than no timer at all.
///
/// **When `undo.offerUndo` is false** -- "Nothing approved", or every
/// attempted entry was skipped -- there is nothing to recover, but the
/// sentence still needs to be seen: `SubmitToast.offerUndo` is
/// `approved > 0` and only that, so this bar renders without the Undo
/// control rather than not at all.
struct UndoBar: View {
    let undo: AppModel.Undo
    let onUndo: () -> Void
    let onKeep: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
                QueueGlyph(glyph: .clock, size: 12, color: TC.blueIcon)
                    .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 1 }
                Text(undo.toastLine)
                    .font(TC.Font_.bodyDense)
                    .foregroundStyle(TC.inkPrimary)
                if undo.offerUndo {
                    Text(held)
                        .font(TC.Font_.ledger)
                        .monospacedDigit()
                        .foregroundStyle(TC.inkSecondary)
                }
                Spacer(minLength: 0)
            }
            if undo.offerUndo {
                Text("""
                The watcher sends approved sessions on its next sweep. This app \
                cannot see when that lands, so it does not pretend to count it \
                down: undo works until the sweep starts, and says so plainly if \
                it is already too late.
                """)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
                .lineSpacing(TC.Font_.LineHeight.spacing(for: 10, TC.Font_.LineHeight.caption))
                .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: TC.Space.s) {
                if undo.offerUndo {
                    // The one Return binding in this app, and it is on the
                    // safe action: a keystroke made by a hand resting on the
                    // keyboard pulls a transcript BACK.
                    Button("Undo", action: onUndo)
                        .tcPrimaryAction()
                        .keyboardShortcut(.defaultAction)
                }
                Button(undo.offerUndo ? "Let it send" : "Dismiss", action: onKeep)
                    .tint(.primary)
                    .help(
                        undo.offerUndo
                            ? "Puts this notice away. It does not change the decision."
                            : "Puts this notice away."
                    )
                Spacer(minLength: 0)
            }
        }
        .padding(.vertical, TC.Space.md)
        .padding(.horizontal, TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    private var held: String {
        undo.heldSeconds >= AppModel.Undo.tickCeiling
            ? "held \(AppModel.Undo.tickCeiling)s+"
            : "held \(undo.heldSeconds)s"
    }
}

/// Why some entries are not waiting on a decision.
///
/// Scoped honestly: `queue_outcome_counts` covers entries that ARE on the
/// queue. It cannot explain a session the watcher discarded before an entry
/// existed, so this does not claim to.
struct NotOfferedDisclosure: View {
    let counts: [String: Int]

    @State private var expanded = false

    var body: some View {
        if !counts.isEmpty {
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                // Drawn rather than taken from `DisclosureGroup`, whose
                // triangle and label type are the system's. The design's row
                // is a plain right chevron and a 12.5pt line, and it collapses
                // to nothing more than that.
                Button {
                    expanded.toggle()
                } label: {
                    HStack(spacing: TC.Space.s) {
                        QueueGlyph(
                            glyph: .chevronRight,
                            size: 10,
                            stroke: 1.8,
                            color: TC.inkSecondary
                        )
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                        Text("Sessions no longer waiting (\(counts.values.reduce(0, +)))")
                            .font(TC.Font_.disclosure)
                            .foregroundStyle(TC.inkPrimary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(expanded ? [.isButton, .isSelected] : .isButton)

                if expanded {
                    VStack(alignment: .leading, spacing: TC.Space.xxs) {
                        ForEach(counts.sorted(by: { $0.key < $1.key }), id: \.key) { label, count in
                            Text("\(count) — \(OutcomeCopy.sentence(for: label))")
                                .font(TC.Font_.meta)
                                .foregroundStyle(TC.inkSecondary)
                        }
                        Text("""
                        This covers sessions that reached the queue. Sessions that were \
                        never queued at all are not counted here.
                        """)
                        .font(TC.Font_.footnote)
                        .foregroundStyle(TC.inkTertiary)
                        .padding(.top, TC.Space.xxs)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.leading, TC.Space.lg)
                }
            }
        }
    }
}

// MARK: - Glyphs

/// One of the design's glyphs, stated on its own 16-unit grid and stroked at
/// whatever size the call site asks for. See the note beside the same type in
/// `MainWindowView`; a later pass folds the two together.
///
/// Internal rather than file-private because `QueueFolderRow` -- the queue's
/// root list, in its own file -- draws the same disclosure chevron. Two
/// copies of one path is how two rows in one list start disagreeing.
struct QueueGlyph: View {
    let glyph: QueueGlyphs
    var size: CGFloat = 12
    /// Stroke width in grid units, converted against `size`.
    var stroke: CGFloat = 1.4
    var color: Color

    var body: some View {
        QueueGlyphShape(glyph: glyph)
            .stroke(
                color,
                style: StrokeStyle(
                    lineWidth: stroke * size / 16,
                    lineCap: .round,
                    lineJoin: .round
                )
            )
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

struct QueueGlyphShape: Shape {
    let glyph: QueueGlyphs

    func path(in rect: CGRect) -> Path {
        var path = Path()
        glyph.draw(into: &path)
        let scale = min(rect.width, rect.height) / 16
        return path
            .applying(CGAffineTransform(scaleX: scale, y: scale))
            .offsetBy(dx: rect.minX, dy: rect.minY)
    }
}

enum QueueGlyphs {
    case clock
    case triangle
    case chevronRight
    case chevronLeft
    case checkCircle
    case columns

    func draw(into path: inout Path) {
        switch self {
        case .clock: Self.clock(&path)
        case .triangle: Self.triangle(&path)
        case .chevronRight: Self.chevronRight(&path)
        case .chevronLeft: Self.chevronLeft(&path)
        case .checkCircle: Self.checkCircle(&path)
        case .columns: Self.columns(&path)
        }
    }

    /// `<circle cx=8 cy=8 r=5.7/><path d="M8 4.8V8l2.3 1.4"/>`
    static func clock(_ path: inout Path) {
        path.addEllipse(in: CGRect(x: 2.3, y: 2.3, width: 11.4, height: 11.4))
        path.move(to: CGPoint(x: 8, y: 4.8))
        path.addLine(to: CGPoint(x: 8, y: 8))
        path.addLine(to: CGPoint(x: 10.3, y: 9.4))
    }

    /// The warning triangle without its dot: `M8 2.2 14.6 13.4H1.4Z`.
    static func triangle(_ path: inout Path) {
        path.move(to: CGPoint(x: 8, y: 2.2))
        path.addLine(to: CGPoint(x: 14.6, y: 13.4))
        path.addLine(to: CGPoint(x: 1.4, y: 13.4))
        path.closeSubpath()
    }

    /// `m6 4 4 4-4 4` -- the disclosure chevron, pointing right when closed.
    static func chevronRight(_ path: inout Path) {
        path.move(to: CGPoint(x: 6, y: 4))
        path.addLine(to: CGPoint(x: 10, y: 8))
        path.addLine(to: CGPoint(x: 6, y: 12))
    }

    /// `m10 4-4 4 4 4` -- `chevronRight` mirrored, for the back control.
    static func chevronLeft(_ path: inout Path) {
        path.move(to: CGPoint(x: 10, y: 4))
        path.addLine(to: CGPoint(x: 6, y: 8))
        path.addLine(to: CGPoint(x: 10, y: 12))
    }

    /// A tick in a circle, using the design's tick path `m5.2 8.3 1.9 1.9 3.6-4.3`.
    static func checkCircle(_ path: inout Path) {
        path.addEllipse(in: CGRect(x: 1.8, y: 1.8, width: 12.4, height: 12.4))
        path.move(to: CGPoint(x: 5.2, y: 8.3))
        path.addLine(to: CGPoint(x: 7.1, y: 10.2))
        path.addLine(to: CGPoint(x: 10.7, y: 5.9))
    }

    /// `M2 13.5h12M3.5 13.5V7.5M6.5 13.5V7.5M9.5 13.5V7.5M12.5 13.5V7.5M2 6.8 8 2.6l6 4.2z`
    static func columns(_ path: inout Path) {
        path.move(to: CGPoint(x: 2, y: 13.5))
        path.addLine(to: CGPoint(x: 14, y: 13.5))
        for x in [3.5, 6.5, 9.5, 12.5] as [CGFloat] {
            path.move(to: CGPoint(x: x, y: 13.5))
            path.addLine(to: CGPoint(x: x, y: 7.5))
        }
        path.move(to: CGPoint(x: 2, y: 6.8))
        path.addLine(to: CGPoint(x: 8, y: 2.6))
        path.addLine(to: CGPoint(x: 14, y: 6.8))
        path.closeSubpath()
    }
}


/// The offer to stop being asked about one project.
///
/// It appears in the queue, above the cards it is about, once that project
/// has been contributed from several times. The placement is the argument:
/// the contributor is looking at the very thing the offer would remove, and
/// has just done it several times over.
///
/// This asks; it does not act. The daemon decides whether there is anything
/// to ask (`ProjectPolicy::arming_suggestion`) and both answers go back to
/// it, so "Not now" is remembered across relaunches and across shells rather
/// than being a dismissal this view forgets.
///
/// Nothing here is emphasised as a primary action. Arming is a real choice
/// with a real cost -- previews from this project stop -- and a card that
/// leads the eye to "yes" is not asking a question.
struct ArmingOfferCard: View {
    let offer: ArmingOffer
    var onArm: () -> Void
    var onDecline: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            // Evidence first, question second. Someone who reads only the
            // first line still learns why they are being asked.
            Text(ArmingOfferCopy.evidence(
                project: offer.projectLabel,
                count: offer.contributedCount
            ))
            .font(TC.Font_.meta)
            .foregroundStyle(.secondary)

            Text(ArmingOfferCopy.question(project: offer.projectLabel))
                .font(.callout.weight(.semibold))

            HStack(spacing: TC.Space.m) {
                // Untinted, and first: declining must not wear the accent
                // that means "yes" everywhere else in this app.
                Button(ArmingOfferCopy.decline, action: onDecline)
                    .tint(.primary)
                Button(ArmingOfferCopy.confirm, action: onArm)
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
        .accessibilityElement(children: .contain)
    }
}
