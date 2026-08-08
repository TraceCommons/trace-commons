import SwiftUI

/// The queue: one row per session waiting for a decision.
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
        VStack(alignment: .leading, spacing: 14) {
                if let health = model.health {
                    HealthBanner(health: health)
                }
                if let undo = model.undo {
                    UndoBar(undo: undo) { model.undoApproval() }
                }
                if let error = model.lastActionError {
                    Text(error)
                        .font(.callout)
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
                    Text("^[\(model.decisionsOwed) session](inflect: true) waiting for your decision")
                        .font(.title3.weight(.semibold))
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

                NotOfferedDisclosure(counts: model.outcomeCounts)
            }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct QueueRow: View {
    let entry: QueueEntry
    let summary: PreviewSummary?
    let summaryError: String?
    let onLookInside: () -> Void
    let onDismiss: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            // project_label, never a path. The contract keeps paths off the
            // wire and this view has nothing else to render.
            HStack(spacing: 8) {
                Text(entry.projectLabel).font(.headline)
                Text(entry.agentName).foregroundStyle(.secondary)
                Text("·").foregroundStyle(.tertiary)
                Text(Format.when(entry.discoveredAt)).foregroundStyle(.secondary)
            }
            .font(.callout)

            // The redacted opening prompt is what identifies a session to
            // its author; a timestamp is not.
            if let summary {
                Text(summary.openingPrompt.isEmpty ? "(no opening prompt)" : summary.openingPrompt)
                    .font(.callout)
                    .foregroundStyle(.primary)
                    .lineLimit(3)
                    .textSelection(.enabled)
                Text("Would send \(Format.bytes(summary.wouldSendBytes))  ·  "
                    + summary.redactionReceipt)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else if let summaryError {
                Text("Couldn't read this one yet (\(summaryError)). Nothing has been sent.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else {
                Text("Reading it locally…").font(.callout).foregroundStyle(.secondary)
            }

            // Always shown, never hidden.
            Text("Scrubbing is pattern-based. It misses things it hasn't seen before.")
                .font(.caption)
                .foregroundStyle(.secondary)

            HStack {
                Button("Look inside", action: onLookInside)
                    .keyboardShortcut(.defaultAction)
                Spacer()
                Button("Not this one", action: onDismiss)
                    .help("Skips this session only. This project will keep being offered.")
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 10))
    }
}

/// "Sending… [ Undo ] (4)". Backed by `cancel`, which returns the entry to
/// pending, so the undo is real.
struct UndoBar: View {
    let undo: AppModel.Undo
    let onUndo: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            ProgressView().controlSize(.small)
            Text("Sending \(undo.projectLabel)…")
            Button("Undo", action: onUndo)
            Text("(\(undo.secondsRemaining))").foregroundStyle(.secondary)
            Spacer()
        }
        .padding(12)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
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
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(counts.sorted(by: { $0.key < $1.key }), id: \.key) { label, count in
                        Text("\(count) — \(OutcomeCopy.sentence(for: label))")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    Text("""
                    This covers sessions that reached the queue. Sessions that were \
                    never queued at all are not counted here.
                    """)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .padding(.top, 4)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 6)
            }
            .font(.callout)
        }
    }
}
