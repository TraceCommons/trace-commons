import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

// The pieces the inspector's prompts draw (`InspectorPrompts`): the two
// consent offers, the first-contribution note, why sessions stopped
// waiting, and a refused action in the core's words. Ron's #1146 puts them
// at the top of the inspector; on macOS they stay above the Traces tree
// (owner, 2026-10-07, an accepted difference), and the inspector still
// opens beside them when one appears (`InspectorDemand`). Every sentence is
// the core's or `QueueLegacyWords`'.

/// The core's words for a refused action on `entryId`, if there is one.
/// The prompts and the session card both say a refusal through this.
struct TracesRefusal: View {
    let store: TracesStore
    let entryId: String

    var body: some View {
        if let refused = store.actionError, refused.entryId == entryId, let line = store.message(for: refused.error) {
            GlassAlert(line)
        }
    }
}

/// The first-contribution note, until anything has been contributed.
struct FirstContributionGlassNote: View {
    let copy: FirstContributionCopy
    /// Whether a session is already waiting, which picks the line that
    /// points at it over the one that says how to start.
    let reviewing: Bool

    @State private var agentSetupOpen = false

    var body: some View {
        GlassCard(quiet: true) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(copy.heading)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                Group {
                    Text(reviewing ? copy.review : copy.start)
                    Text(copy.followUp)
                }
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
                GlassExpander(QueueLegacyWords.agentSetup, isOpen: $agentSetupOpen)
                if agentSetupOpen {
                    Text(copy.agentSetup)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }
}

/// Why some entries are not waiting on a decision. Scoped as the legacy
/// disclosure is: `queue_outcome_counts` covers entries that reached the
/// queue, and the scope note says so.
struct NotOfferedGlassDisclosure: View {
    let counts: [String: Int]
    /// The Summary's words from the core (`summary_panel`); nil keeps the
    /// legacy queue's.
    var words: MonitorSummaryCopy? = nil

    @State private var expanded = false

    /// The expander's title: the core's summary line when it is in hand.
    static func title(_ count: Int, words: MonitorSummaryCopy?) -> String {
        guard let words else { return QueueLegacyWords.noLongerWaiting(count) }
        return FirstRunCopy.fill(words.noLongerWaiting, ["count": String(count)])
    }

    /// What the counts cover: the sessions that reached the queue.
    static func scope(words: MonitorSummaryCopy?) -> String {
        words?.noLongerWaitingScope ?? QueueLegacyWords.notOfferedScope
    }

    var body: some View {
        if !counts.isEmpty {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                GlassExpander(Self.title(counts.values.reduce(0, +), words: words), isOpen: $expanded)
                if expanded {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        ForEach(counts.sorted(by: { $0.key < $1.key }), id: \.key) { label, count in
                            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s2) {
                                Text(String(count)).monospacedDigit()
                                Text(TCOutcome.line(label: label))
                            }
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                        }
                        Text(Self.scope(words: words))
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.leading, GlassTokens.Space.s10)
                }
            }
        }
    }
}

/// The offer to answer model calls on this computer, in the core's words
/// only. The accept comes first as a glass button and the decline after it
/// as a link, as #1146 orders it: the order the owner ruled for every card
/// (Ron, 2026-10-09).
///
/// #1146's `PrivateInferenceOffer` head and body: the destination as a
/// mono accent eyebrow, the title as an h2, then the four paragraphs drawn
/// alike, 8 apart.
struct PrivateAIOfferGlassCard: View {
    let copy: PrivateInferenceCopy
    let onAccept: () -> Void
    let onDecline: () -> Void

    /// #1146's destination eyebrow: mono 10, extra bold, uppercase, tracked
    /// .16em.
    static let destinationType = GlassTypeStyle(
        textStyle: .caption2, size: 10, weight: .heavy, lineHeight: 10, tracking: 1.6,
        design: .monospaced, uppercase: true, tabular: false)

    var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                Text(copy.destination)
                    .glassType(Self.destinationType)
                    .foregroundStyle(GlassColor.accentText)
                    .padding(.bottom, GlassTokens.Space.s3)
                Text(copy.offerTitle)
                    .glassType(GlassTokens.TypeScale.title)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                // What it does, what it exposes, what it does not do, and
                // that it is asked once: four paragraphs drawn alike.
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    Text(copy.offerWhat)
                    Text(copy.offerExposure)
                    Text(copy.offerNoRepoint)
                    Text(copy.offerAskedOnce)
                }
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: GlassTokens.Space.s4) {
                    Button(copy.offerAccept, action: onAccept)
                        .buttonStyle(GlassButtonStyle(.glass))
                    Button(copy.offerDecline, action: onDecline)
                        .buttonStyle(GlassButtonStyle(.link))
                }
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// The offer to stop being asked about one project. Arming is a grant, so
/// it is offered only in the core's words (`tc_arming_offer_copy_json`) and
/// nothing is drawn without them. Evidence first, question second; the
/// confirm first as a glass button and the decline after it as a link, the
/// order the owner ruled for every card (Ron, 2026-10-09). Ron's shape (#1146 `ArmingOffer`): the eyebrow over the card, and the
/// card's confirm opens a confirmation with the core's body before anything
/// is armed.
struct ArmingOfferGlassCard: View {
    let offer: ArmingOffer
    /// The core's "OPTIONAL AUTOMATION"; nil draws no eyebrow.
    let eyebrow: String?
    let onArm: () -> Void
    let onDecline: () -> Void

    /// The confirmation is open: nothing arms until it is answered.
    @State private var confirming = false

    private var copy: ProjectArmingCopy? {
        ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(
            project: offer.projectLabel,
            count: offer.contributedCount
        ))
    }

    var body: some View {
        if let copy {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    if let eyebrow {
                        Text(eyebrow)
                            .glassType(GlassTokens.TypeScale.eyebrow)
                            .foregroundStyle(GlassColor.textTertiary)
                    }
                    Text(copy.evidence)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(copy.question)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                    // #1146 `arming-offer.tsx`: what arming does, on the
                    // card itself, before either button (and again in the
                    // confirmation).
                    Text(copy.body)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: GlassTokens.Space.s4) {
                        Button(copy.confirm) { confirming = true }
                            .buttonStyle(GlassButtonStyle(.glass))
                        Button(copy.decline, action: onDecline)
                            .buttonStyle(GlassButtonStyle(.link))
                    }
                }
            }
            .accessibilityElement(children: .contain)
            // A whole-window confirmation. Declining here closes the
            // confirmation only; the offer stays until it is answered on
            // the card.
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
