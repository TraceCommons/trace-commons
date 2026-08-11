import SwiftUI

/// Three groups, never one column of mixed semantics.
///
/// Quarantine reads as held, not rejected, and never states a turnaround
/// time. Credit is a record: no currency symbol, no fiat estimate, no
/// projection, no date, no streaks, no leaderboards.
struct HistoryView: View {
    @EnvironmentObject private var model: AppModel
    @State private var quarantineExpanded = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: TC.Space.xl) {
                if let rollup = model.rollup {
                    groups(rollup)
                    if rollup.quarantined > 0 {
                        quarantine(rollup)
                    }
                    credit(rollup)
                } else {
                    Text("Nothing yet.")
                        .font(TC.Font_.meta)
                        .foregroundStyle(.secondary)
                }

                if !model.history.isEmpty {
                    VStack(alignment: .leading, spacing: TC.Space.m) {
                        TCSectionHeader(
                            title: "Everything you've contributed",
                            trailing: "\(model.history.count)"
                        )
                        copyDefects
                        ForEach(model.history) { record in
                            HistoryRow(record: record)
                        }
                    }
                }
            }
            .padding(.horizontal, TC.Space.xxl)
            .padding(.vertical, TC.Space.xl)
            .tcColumn()
        }
        .tcScreen()
    }

    /// Three states, three tones, three glyphs, three words. The counts are
    /// the same shape as a queue card's manifest -- uppercase label over a
    /// monospaced figure -- so the two screens read as one system.
    private func groups(_ rollup: HistoryRollup) -> some View {
        HStack(alignment: .top, spacing: TC.Space.m) {
            tally("In the commons", rollup.allTime.accepted, .clear, "checkmark.circle")
            tally("Held for privacy review", rollup.quarantined, .held, "clock")
            tally("Waiting to be scored", rollup.allTime.submitted, .neutral, "circle.dotted")
        }
    }

    private func tally(_ title: String, _ count: Int, _ tone: TC.Tone, _ symbol: String) -> some View {
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
        .accessibilityLabel("\(title): \(count)")
    }

    private func quarantine(_ rollup: HistoryRollup) -> some View {
        DisclosureGroup(isExpanded: $quarantineExpanded) {
            VStack(alignment: .leading, spacing: 8) {
                Text("""
                A person at Trace Commons reads these before they enter the commons. \
                It happens when automated checks see something that might be personal \
                or sensitive and can't decide on its own.
                """)
                Text("""
                These have not been rejected, and they have not been shared with \
                anyone but the reviewer. They are sitting still.
                """)
                .fontWeight(.semibold)
                // Never state a turnaround time that cannot be honoured.
                Text("Typical wait: we don't have a reliable number yet.")
                    .foregroundStyle(.secondary)

                let explanations = model.history
                    .filter { $0.status == "quarantined" }
                    .flatMap(\.explanations)
                if !explanations.isEmpty {
                    // Rendered verbatim: the server's own prose beats a
                    // status word every time.
                    ForEach(Array(explanations.enumerated()), id: \.offset) { _, text in
                        Text(text).font(.callout).foregroundStyle(.secondary)
                    }
                }

                // No bulk action, and the reason is stated rather than left
                // as an absence -- see WithdrawalCopy.noBulkAction.
                Text(WithdrawalCopy.noBulkAction)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .font(.callout)
            .frame(maxWidth: 560, alignment: .leading)
            .padding(.top, 8)
        } label: {
            // Held is a state, not a colour. It says "held", it carries a
            // clock, and it is tinted -- in that order of importance.
            HStack(spacing: TC.Space.s) {
                Image(systemName: TC.Tone.held.symbol)
                    .foregroundStyle(TC.Tone.held.textColor)
                    .accessibilityHidden(true)
                Text("Held for privacy review — ^[\(rollup.quarantined) trace](inflect: true)")
                    .font(TC.Font_.cardTitle)
            }
        }
    }

    /// The withdrawal copy's own assertions, rendered where a contributor
    /// and a developer both see them. There is no Swift test target here, so
    /// the alternative was assertions nobody runs; a screen that admits its
    /// withdrawal wording has stopped being trustworthy is better than a
    /// screen that quietly keeps showing it. Empty in every healthy build.
    @ViewBuilder
    private var copyDefects: some View {
        let problems = WithdrawalCopyCheck.failures()
        if !problems.isEmpty {
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                Text("Do not trust the withdrawal wording on this screen.")
                    .font(TC.Font_.cardTitle)
                ForEach(problems, id: \.self) { problem in
                    Text(problem).font(TC.Font_.footnote)
                }
            }
            .foregroundStyle(TC.coralText)
            .padding(TC.Space.m)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
        }
    }

    private func credit(_ rollup: HistoryRollup) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            TCSectionHeader(title: "Credit")
            CreditRecordView(
                creditFinal: rollup.creditFinal,
                creditPending: rollup.creditPending,
                lastRefreshedAt: rollup.lastRefreshedAt
            )
        }
    }
}

/// The three tier confirmations side by side, on fabricated records, for the
/// screenshot hook.
///
/// It exists because the demo state directory has no enrolment and therefore
/// no history at all, so a capture of the real History screen shows an empty
/// list and proves nothing about the copy -- which is the entire substance of
/// this feature. Everything here is invented; nothing reads the queue, the
/// history cache, or the daemon.
///
/// Built from plain `Text` and `Button` on purpose: `ImageRenderer` will not
/// rasterize NSView-backed controls (`Toggle`, `TextField`, `Menu`, segmented
/// `Picker`), which come out as yellow placeholders in a capture while being
/// perfectly fine in the running app.
struct WithdrawalConfirmationCapture: View {
    private static func record(_ label: String, _ status: String) -> HistoryRecord {
        HistoryRecord(
            submissionID: "capture-\(status)",
            submittedAt: Date(timeIntervalSince1970: 1_770_000_000),
            projectLabel: label,
            source: "claude-code",
            status: status,
            consentScopes: ["debugging_evaluation"],
            creditPointsPending: 0,
            creditPointsFinal: nil,
            explanations: [],
            lastRefreshedAt: nil
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            TCSectionHeader(title: "Withdrawing, per tier")
            HistoryRow(
                record: Self.record("northwind-billing", "quarantined"),
                initiallyConfirming: true
            )
            HistoryRow(
                record: Self.record("dotfiles", "accepted"),
                initiallyConfirming: true
            )
            HistoryRow(record: Self.record("dotfiles", "withdrawn"))
        }
        .padding(TC.Space.xxl)
        .tcColumn()
        .tcScreen()
    }
}

struct HistoryRow: View {
    let record: HistoryRecord
    /// Screenshot hook only: opens the row with its confirmation already
    /// showing, since `ImageRenderer` never delivers a click.
    var initiallyConfirming: Bool = false

    @EnvironmentObject private var model: AppModel
    @State private var confirming = false

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            HStack(spacing: TC.Space.s) {
                Text(record.projectLabel).font(TC.Font_.body.weight(.semibold))
                Text(Format.when(record.submittedAt))
                    .font(TC.Font_.footnote)
                    .foregroundStyle(.tertiary)
                Spacer(minLength: TC.Space.m)
                TCTag(text: statusSentence, tone: statusTone, symbol: statusSymbol)
            }
            ForEach(Array(record.explanations.enumerated()), id: \.offset) { _, text in
                Text(text)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if let result = model.withdrawals[record.submissionID] {
                outcome(result)
            } else if confirming || initiallyConfirming {
                confirmation
            } else if isWithdrawable {
                // Plain, not filled. The filled action in this app is the
                // one a person is being invited to take; this one only
                // opens a question.
                Button("Withdraw") { confirming = true }
                    .font(TC.Font_.footnote)
            }
        }
        .padding(TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    /// Whether there is anything to withdraw. A trace already withdrawn is
    /// not offered again -- the daemon would treat it as a no-op, and an
    /// enabled button on it would suggest the first one did not take.
    private var isWithdrawable: Bool {
        record.status != "withdrawn"
    }

    /// The confirmation, shown BEFORE anything is asked of the server and
    /// worded for this trace's stage. See `WithdrawalCopy` for why the
    /// wording is keyed on the local status and why the ambiguous case
    /// states the worse outcome.
    private var confirmation: some View {
        let copy = WithdrawalCopy.confirmation(for: .init(status: record.status))
        let inFlight = model.withdrawing.contains(record.submissionID)
        return VStack(alignment: .leading, spacing: TC.Space.s) {
            Text(copy.question).font(TC.Font_.cardTitle)
            if let ambiguity = copy.ambiguity {
                Text(ambiguity)
                    .font(TC.Font_.footnote)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(Array(copy.bodies.enumerated()), id: \.offset) { index, body in
                // The body carrying the cannot-be-recalled clause is the one
                // that must not be skimmed past, so it takes the coral text
                // token and a glyph. Coral is this app's "refused / cannot
                // proceed" role; it is on type here, never as a fill, and
                // `coralText` is the darkened light-mode twin measured to
                // clear 4.5:1 on a card face.
                let gravest = index == copy.gravest
                HStack(alignment: .firstTextBaseline, spacing: TC.Space.xs) {
                    Image(systemName: gravest ? "exclamationmark.triangle" : "info.circle")
                        .imageScale(.small)
                        .accessibilityHidden(true)
                    Text(body).fixedSize(horizontal: false, vertical: true)
                }
                .font(TC.Font_.footnote)
                .foregroundStyle(gravest ? AnyShapeStyle(TC.coralText) : AnyShapeStyle(.primary))
            }
            Text(copy.credit)
                .font(TC.Font_.footnote)
                .foregroundStyle(.secondary)
            HStack(spacing: TC.Space.s) {
                // Escape backs out. Nothing binds Return: withdrawal is
                // irreversible, and this app binds Return to nothing
                // irreversible anywhere.
                Button("Keep it") { confirming = false }
                    .keyboardShortcut(.cancelAction)
                Button(inFlight ? "Withdrawing..." : copy.confirmLabel) {
                    model.withdraw(record)
                }
                .tcPrimaryAction()
                .disabled(inFlight)
            }
            .font(TC.Font_.footnote)
        }
        .frame(maxWidth: TC.Measure.prose, alignment: .leading)
        .padding(TC.Space.m)
        .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.inset))
    }

    /// What actually happened, on the row it happened to.
    @ViewBuilder
    private func outcome(_ result: AppModel.WithdrawalResult) -> some View {
        let (text, tone): (String, TC.Tone) = {
            switch result {
            case .withdrawn(let reach):
                return (WithdrawalCopy.resultSentence(reach), .refused)
            case .noAccountSession:
                return (WithdrawalCopy.accountSessionRequired, .attention)
            case .failed(let label):
                return (WithdrawalCopy.failureSentence(label: label), .attention)
            }
        }()
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.xs) {
            Image(systemName: tone.symbol)
                .imageScale(.small)
                .accessibilityHidden(true)
            Text(text).fixedSize(horizontal: false, vertical: true)
        }
        .font(TC.Font_.footnote)
        .foregroundStyle(tone.textColor)
        .frame(maxWidth: TC.Measure.prose, alignment: .leading)
    }

    private var statusSentence: String {
        switch record.status {
        case "accepted": return "In the commons"
        case "quarantined": return "Held for privacy review"
        case "submitted": return "Waiting to be scored"
        // Withdrawn is its own state, not the "something else happened"
        // bucket: a withdrawn trace stays on this list, reading as
        // withdrawn, rather than vanishing as though it had never been sent.
        case "withdrawn": return "Withdrawn by you"
        default: return "Not in the commons"
        }
    }

    private var statusTone: TC.Tone {
        switch record.status {
        case "accepted": return .clear
        case "quarantined": return .held
        case "submitted": return .neutral
        default: return .refused
        }
    }

    private var statusSymbol: String? {
        record.status == "withdrawn" ? "arrow.uturn.backward" : nil
    }
}
