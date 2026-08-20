import SwiftUI

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

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.md) {
            if let health = model.health {
                HealthBanner(health: health)
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

    private var waiting: some View {
        VStack(alignment: .leading, spacing: TC.Space.md) {
            // Left as a sentence, not compressed into a label-and-count
            // header. It is the one line on this screen written in the
            // product's voice and it says what the screen is FOR.
            Text("^[\(model.decisionsOwed) session](inflect: true) waiting for your decision")
                .font(TC.Font_.sectionTitle)
                .foregroundStyle(TC.inkPrimary)

            // Grouped by project so `Submit all` has something honest to
            // point at -- a group is exactly what `submitProject` acts on,
            // never a slice the UI made up. `waitingByProject`'s order is
            // first-seen, which is also `awaitingDecision`'s order, so this
            // reshuffles nothing a contributor has already scanned.
            VStack(spacing: TC.Space.lg) {
                ForEach(model.waitingByProject, id: \.id) { group in
                    ProjectQueueGroup(
                        group: group,
                        entries: model.awaitingDecision.filter { $0.projectID == group.id },
                        summaries: model.summaries,
                        summaryErrors: model.summaryErrors,
                        onLookInside: { previewing = $0 },
                        onSubmit: { model.approve($0) },
                        onDismiss: { model.dismiss($0) },
                        onSubmitAll: { model.submitProject(id: group.id) }
                    )
                }
            }

            // The mechanism's limits, stated once for the list rather than
            // stamped on every card -- see `ScrubbingCaveat`, which also
            // records why this sentence is NOT the design's longer standing
            // disclaimer.
            ScrubbingCaveatNote()
                .padding(.top, TC.Space.xxs)
        }
    }
}

/// One project's slice of the queue: its rows, and -- only when there is
/// more than one of them -- the `Submit all` action that approves the whole
/// group in the same gesture the design spec calls "the same gesture at the
/// project level". A single-entry group offers no second way to do what its
/// one row's own `Submit` already does.
private struct ProjectQueueGroup: View {
    let group: (id: String, label: String, count: Int, bytes: Int)
    let entries: [QueueEntry]
    let summaries: [String: PreviewSummary]
    let summaryErrors: [String: String]
    let onLookInside: (QueueEntry) -> Void
    let onSubmit: (QueueEntry) -> Void
    let onDismiss: (QueueEntry) -> Void
    let onSubmitAll: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.md) {
            HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
                Text(group.label)
                    .font(TC.Font_.meta)
                    .foregroundStyle(TC.inkSecondary)
                Spacer(minLength: TC.Space.m)
                if group.count > 1 {
                    Button("Submit all (\(group.count))", action: onSubmitAll)
                        .tcPrimaryAction()
                        .help("""
                        Submits every session waiting in \(group.label). Each is scrubbed \
                        the same way a single Submit would be, and flagged sessions are \
                        included, not held back.
                        """)
                }
            }
            rowList
        }
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
    @ViewBuilder
    private var rowList: some View {
        if DebugScreenshot.directory != nil {
            VStack(spacing: TC.Space.md) { rows }
        } else {
            LazyVStack(spacing: TC.Space.md) { rows }
        }
    }

    private var rows: some View {
        ForEach(entries) { entry in
            QueueRow(
                entry: entry,
                summary: summaries[entry.entryID],
                summaryError: summaryErrors[entry.entryID],
                onLookInside: { onLookInside(entry) },
                onSubmit: { onSubmit(entry) },
                onDismiss: { onDismiss(entry) }
            )
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
    let onLookInside: () -> Void
    let onSubmit: () -> Void
    let onDismiss: () -> Void

    private var redactionCount: Int {
        summary?.redactions.values.reduce(0, +) ?? 0
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            identity
            prompt
            footer
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard(emphasised: summary != nil && redactionCount == 0)
    }

    // MARK: - Identity

    /// project_label, never a path. The contract keeps paths off the wire
    /// and this view has nothing else to render.
    private var identity: some View {
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
            Text(entry.projectLabel)
                .font(TC.Font_.cardTitle)
                .foregroundStyle(TC.inkPrimary)
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
                                Text(Self.removedSummary(summary.redactions))
                                    .font(TC.Font_.ledger)
                                    .foregroundStyle(TC.inkPrimary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                    caption
                }
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

    /// The gold chip that replaces the removed-by-pattern figure when nothing
    /// matched. It carries the warning triangle without its dot -- the same
    /// glyph the health banner uses, quieter, because this is a thing to weigh
    /// rather than a thing to fix.
    private var nothingMatchedChip: some View {
        HStack(spacing: TC.Space.xxs) {
            QueueGlyph(glyph: .triangle, size: 11, stroke: 1.6, color: TC.gold)
            Text("nothing matched")
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
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Nothing matched a pattern.")
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

    /// Category labels and counts only; the contract guarantees the map
    /// never carries matched text. Ordered by count so the biggest number is
    /// first, which is what a person is scanning for.
    static func removedSummary(_ redactions: [String: Int]) -> String {
        redactions
            .sorted { $0.value == $1.value ? $0.key < $1.key : $0.value > $1.value }
            .map { "\($0.value) \($0.key.replacingOccurrences(of: "_", with: " "))" }
            .joined(separator: "  ·  ")
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
                .help("Skips this session only. This project will keep being offered.")
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
private struct QueueGlyph: View {
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

private struct QueueGlyphShape: Shape {
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

private enum QueueGlyphs {
    case clock
    case triangle
    case chevronRight
    case checkCircle
    case columns

    func draw(into path: inout Path) {
        switch self {
        case .clock: Self.clock(&path)
        case .triangle: Self.triangle(&path)
        case .chevronRight: Self.chevronRight(&path)
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
