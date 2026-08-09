import SwiftUI

/// The queue: one per session waiting for a decision.
///
/// The row's only forward action is "Look inside". Approve deliberately does
/// NOT live here -- preview-then-approve only, because blind approval of a
/// real transcript is the unrecoverable misclick. (The shared spec sketches
/// a `Contribute` button on the row; this shell follows the stricter rule
/// stated alongside it, "the tray's only forward action is Review", and puts
/// the single Contribute button inside the preview sheet.)
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
        VStack(alignment: .leading, spacing: TC.Space.l) {
            if let health = model.health {
                HealthBanner(health: health)
            }
            if let undo = model.undo {
                UndoBar(undo: undo) { model.undoApproval() }
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
        .padding(.horizontal, TC.Space.xl)
        .padding(.vertical, TC.Space.l)
        .tcColumn()
        .tcScreen()
    }

    private var waiting: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            // Left as a sentence, not compressed into a label-and-count
            // header. It is the one line on this screen written in the
            // product's voice and it says what the screen is FOR.
            Text("^[\(model.decisionsOwed) session](inflect: true) waiting for your decision")
                .font(TC.Font_.screenTitle)

            VStack(spacing: TC.Space.s) {
                ForEach(model.awaitingDecision) { entry in
                    QueueRow(
                        entry: entry,
                        summary: model.summaries[entry.entryID],
                        summaryError: model.summaryErrors[entry.entryID],
                        onLookInside: { previewing = entry },
                        onDismiss: { model.dismiss(entry) }
                    )
                }
            }

            // The mechanism's limits, stated once for the list rather than
            // stamped on every card -- see `ScrubbingCaveat`.
            ScrubbingCaveatNote()
                .padding(.top, TC.Space.xxs)
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
            Text(entry.agentName)
                .font(TC.Font_.footnote)
                .foregroundStyle(.secondary)
            Spacer(minLength: TC.Space.m)
            Text(Format.when(entry.discoveredAt))
                .font(TC.Font_.footnote)
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, TC.Space.l)
        .padding(.top, TC.Space.m)
        .padding(.bottom, TC.Space.xs)
    }

    /// The redacted opening prompt is what identifies a session to its
    /// author; a timestamp is not. It gets the most room on the card.
    @ViewBuilder
    private var prompt: some View {
        Group {
            if let summary {
                Text(summary.openingPrompt.isEmpty ? "(no opening prompt)" : summary.openingPrompt)
                    .font(TC.Font_.body)
                    .foregroundStyle(.primary)
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
                    HStack(alignment: .top, spacing: TC.Space.xl) {
                        cell("Would send") {
                            Text(Format.bytes(summary.wouldSendBytes))
                                .font(TC.Font_.ledger)
                                .monospacedDigit()
                        }
                        cell("Removed by pattern") {
                            if redactionCount == 0 {
                                TCTag(text: "nothing matched", tone: .attention)
                            } else {
                                Text(Self.removedSummary(summary.redactions))
                                    .font(TC.Font_.ledger)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                    Text(ScrubbingCaveat.rowLine(redactionCount: redactionCount))
                        .font(TC.Font_.footnote)
                        .foregroundStyle(
                            redactionCount == 0
                                ? AnyShapeStyle(TC.Tone.attention.textColor)
                                : AnyShapeStyle(.secondary)
                        )
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: TC.Space.m)
            actions
        }
        .padding(.horizontal, TC.Space.l)
        .padding(.vertical, TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(TC.surfaceInset)
        .overlay(alignment: .top) {
            Rectangle().fill(TC.line).frame(height: TC.Space.hairline)
        }
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

    /// Both actions at the trailing edge, adjacent, default action last --
    /// the macOS convention, and one eye movement instead of the full width
    /// of the window.
    private var actions: some View {
        HStack(spacing: TC.Space.s) {
            Button("Not this one", action: onDismiss)
                // Untinted on purpose. A bordered button inherits the
                // app accent, and "Not this one" rendered in the same
                // green as "Look inside" reads as a second approval.
                .tint(.primary)
                .help("Skips this session only. This project will keep being offered.")
            Button("Look inside", action: onLookInside)
                .tcPrimaryAction()
                .keyboardShortcut(.defaultAction)
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
                figure("Contributed", week.submitted, .clear, TC.Tone.clear.symbol)
                figure("Held for privacy review", quarantined, .held, TC.Tone.held.symbol)
                figure("In the commons", week.accepted, .neutral, "building.columns")
            }
        }
        .padding(.top, TC.Space.s)
    }

    private func figure(_ title: String, _ count: Int, _ tone: TC.Tone, _ symbol: String) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            HStack(spacing: TC.Space.xs) {
                Image(systemName: symbol)
                    .imageScale(.small)
                    .foregroundStyle(tone.textColor)
                    .accessibilityHidden(true)
                TCFieldLabel(title)
            }
            Text("\(count)")
                .font(.title2.weight(.bold))
                .monospacedDigit()
        }
        .padding(TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(title) this week: \(count)")
    }
}

/// "Sending… [ Undo ] (4)". Backed by `cancel`, which returns the entry to
/// pending, so the undo is real.
struct UndoBar: View {
    let undo: AppModel.Undo
    let onUndo: () -> Void

    var body: some View {
        HStack(spacing: TC.Space.m) {
            ProgressView().controlSize(.small)
            Text("Sending \(undo.projectLabel)…").font(TC.Font_.meta)
            Button("Undo", action: onUndo)
            Text("(\(undo.secondsRemaining))")
                .font(TC.Font_.ledger)
                .monospacedDigit()
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, TC.Space.l)
        .padding(.vertical, TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }
}

/// Why some entries are not waiting on a decision.
///
/// Scoped honestly: `queue_outcome_counts` covers entries that ARE on the
/// queue. It cannot explain a session the watcher discarded before an entry
/// existed, so this does not claim to.
struct NotOfferedDisclosure: View {
    let counts: [String: Int]

    var body: some View {
        if !counts.isEmpty {
            DisclosureGroup("Sessions no longer waiting (\(counts.values.reduce(0, +)))") {
                VStack(alignment: .leading, spacing: TC.Space.xxs) {
                    ForEach(counts.sorted(by: { $0.key < $1.key }), id: \.key) { label, count in
                        Text("\(count) — \(OutcomeCopy.sentence(for: label))")
                            .font(TC.Font_.meta)
                            .foregroundStyle(.secondary)
                    }
                    Text("""
                    This covers sessions that reached the queue. Sessions that were \
                    never queued at all are not counted here.
                    """)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(.tertiary)
                    .padding(.top, TC.Space.xxs)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, TC.Space.xs)
            }
            .font(TC.Font_.meta)
        }
    }
}
