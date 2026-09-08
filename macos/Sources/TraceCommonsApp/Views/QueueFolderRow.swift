import SwiftUI
import TCShellCore

/// One folder in the queue's root list.
///
/// The folder name is this row's largest text. It used to be its smallest
/// -- `TC.Font_.meta` in `inkSecondary`, beside a primary-styled
/// `Submit all` -- so the line read as a button with a caption rather than
/// as a place with actions. At 149 waiting sessions that inversion is the
/// difference between a list you can scan and one you cannot.
///
/// `Open` is the row's primary action and the only accented control on it.
/// `Submit all` is a peer, untinted, for the reason the card's `Submit` is:
/// the one-click design says Submit is not the primary action, and a filled
/// `Submit all` on the folder row was recommending exactly what the card
/// below it declines to recommend.
///
/// `Submit all` is shown at EVERY count, including one. The old rule hid it
/// at one because the row's own `Submit` was on the same screen and did the
/// same thing. Under drill-in it is a level down, so hiding it here would
/// mean opening a folder to do the thing the folder is offering. The rule
/// expired with the layout it was written for.
struct QueueFolderRow: View {
    let group: QueueGroup<QueueEntry>
    /// This project's `list_projects` row, when one has arrived. Carries
    /// the two counts the group control is drawn from; nil before the call
    /// answers, and for a project it does not list.
    let project: ProjectRow?
    /// The eligibility branch tables. Only the withheld sentence is read
    /// through them here -- the counts are the daemon's.
    let eligibilityCalls: EligibilityCalls
    let onOpen: () -> Void
    let onSubmitAll: () -> Void
    /// The opt-in bulk path: the same approval, carrying one verdict for
    /// every entry it covers. Separate from `onSubmitAll` so the plain
    /// one-click submit stays exactly one click and keeps sending no
    /// `outcome` at all.
    let onSubmitAllAs: (ContributorVerdict) -> Void
    let onIgnoreProject: () -> Void

    @State private var confirmingIgnore = false

    /// Display only, and empty against a daemon that predates the field --
    /// in which case the row shows its label alone rather than a blank line.
    private var path: String { group.entries.first?.projectPath ?? "" }

    /// What the group control offers, decided by the daemon's own counts.
    ///
    /// **A GROUP-LEVEL SUBMIT MEANS "ALL ELIGIBLE", NEVER "ALL"** -- and the
    /// daemon enforces it now, so this is presentation rather than
    /// protection. The count on the button is `contributable_count`, not
    /// `group.count`: a button promising more than it sends is the same
    /// press-then-discover shape the card's `Submit` was fixed for.
    ///
    /// `group.count` is the fallback for a project the queue is showing
    /// before `list_projects` has answered for it. The folder is on screen
    /// either way and must say something.
    private var offer: GroupSubmitOffer {
        EligibilitySurface.groupSubmit(
            pendingCount: project?.pendingCount,
            contributableCount: project?.contributableCount,
            fallbackPending: group.count,
            calls: eligibilityCalls)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            Button(action: onOpen) {
                HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
                    VStack(alignment: .leading, spacing: TC.Space.xxs) {
                        Text(group.label)
                            .font(TC.Font_.cardTitle)
                            .foregroundStyle(TC.inkPrimary)
                        if !path.isEmpty {
                            Text(path)
                                .font(TC.Font_.meta)
                                .foregroundStyle(TC.inkSecondary)
                        }
                    }
                    Spacer(minLength: TC.Space.m)
                    Text("^[\(group.count) session](inflect: true)")
                        .font(TC.Font_.ledger)
                        .monospacedDigit()
                        .foregroundStyle(TC.inkSecondary)
                    Text(Format.bytes(group.bytes))
                        .font(TC.Font_.ledger)
                        .monospacedDigit()
                        .foregroundStyle(TC.inkTertiary)
                    QueueGlyph(glyph: .chevronRight, size: 11, color: TC.inkTertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(group.label), \(group.count) waiting. Open.")

            HStack(spacing: TC.Space.s) {
                // Untinted, like the card's `Submit`: the one-click design
                // says Submit is availability, not a recommendation, and
                // the accent on this row belongs to opening the folder --
                // see `Open` at the trailing edge.
                //
                // DISABLED AT ZERO, NOT REMOVED -- a ratified deviation from
                // the queue row, where the control is simply not drawn. A
                // group header is not a row: the group still holds sessions,
                // and a folder offering no way to act on it reads as broken
                // rather than finished. Whether it may be pressed is the
                // shared table's answer, never a comparison of the count to
                // zero here.
                Button("Submit all (\(offer.count))", action: onSubmitAll)
                    .tint(.primary)
                    .disabled(!offer.offersContribute)
                    .help("""
                    Submits every session in \(group.label) that can be sent. Each is \
                    scrubbed the same way a single Submit would be, and flagged \
                    sessions are included, not held back.
                    """)
                // Beside `Submit all`, never in front of it: answering the
                // outcome question for a whole folder is a choice a
                // contributor opts into, and the common path must not grow a
                // step because this exists. Never `.tcPrimaryAction()` --
                // one primary action per row, and it is the plain button.
                //
                // Disabled on the same condition as the plain button, and it
                // has to be said separately: this is a second route to the
                // same call, and a live menu beside a disabled button is the
                // inert control #728 is about.
                Menu(VerdictCopy.submitAllAs) {
                    ForEach(ContributorVerdict.allCases, id: \.rawValue) { option in
                        Button(option.label) { onSubmitAllAs(option) }
                    }
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .disabled(!offer.offersContribute)
                .help(VerdictCopy.submitAllAsTooltip)
                Spacer(minLength: TC.Space.m)
                // Never `.tcPrimaryAction()`: it sits beside a control that
                // uploads the very traces this removes, and two adjacent
                // actions that do opposite things must not look alike.
                Button(ProjectIgnoreCopy.buttonLabel) { confirmingIgnore = true }
                    .help(ProjectIgnoreCopy.tooltip)
                // The row's primary action, in the card's position (trailing,
                // default action last). It carries the accent that `Submit
                // all` used to: looking is what this product recommends,
                // and a folder row is offering a look at what is inside.
                Button("Open", action: onOpen)
                    .tcPrimaryAction()
                    .help("Opens \(group.label) to look at each session before deciding.")
            }

            // Drawn WHEREVER the button's count is short of the folder's,
            // and never separated from it: a button that says fewer
            // sessions than the row does, with nothing explaining the gap,
            // is its own small dishonesty.
            if let withheldNote = offer.withheldLine {
                Text(withheldNote)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.inkSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
        .confirmationDialog(
            ProjectIgnoreCopy.confirmationTitle(project: group.label),
            isPresented: $confirmingIgnore,
            titleVisibility: .visible
        ) {
            Button(ProjectIgnoreCopy.buttonLabel, role: .destructive, action: onIgnoreProject)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(ProjectIgnoreCopy.confirmationBody(
                project: group.label,
                pendingCount: group.count
            ))
        }
    }
}
