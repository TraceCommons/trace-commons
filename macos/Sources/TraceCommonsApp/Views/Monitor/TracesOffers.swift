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
    /// Closed with its X, for good on this Mac (owner, 2026-10-08). It
    /// leaves by itself once History has a row anyway.
    @AppStorage("monitor.firstContributionDismissed") private var dismissed = false

    var body: some View {
        if !dismissed {
            // Drawn as the Private AI offer beside it is: the title and the
            // body at the same sizes and colour (owner, 2026-10-08).
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(copy.heading)
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                        .padding(.trailing, CardClose.clearance)
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                        Text(reviewing ? copy.review : copy.start)
                        Text(copy.followUp)
                    }
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                    GlassExpander(QueueLegacyWords.agentSetup, isOpen: $agentSetupOpen)
                    if agentSetupOpen {
                        Text(copy.agentSetup)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .overlay(alignment: .topTrailing) {
                CardClose(label: ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord) {
                    dismissed = true
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
/// only. Declining comes first and neither answer is the primary action:
/// this question opens a listener anything on the machine can use. (#1146
/// puts Turn it on first as its primary; that order is a consent change
/// left to the owner, so the card keeps it.)
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

    /// The rest of the offer's paragraphs are shown.
    @State private var learnMore = false

    var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                Text(copy.destination)
                    .glassType(Self.destinationType)
                    .foregroundStyle(GlassColor.accentText)
                    .padding(.top, GlassTokens.Space.s1)
                    .padding(.bottom, GlassTokens.Space.s3)
                Text(copy.offerTitle)
                    .glassType(GlassTokens.TypeScale.title)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                    .padding(.trailing, CardClose.clearance)
                // What it does and, in one line, what it exposes; the full
                // exposure, what it does not do and that it is asked once
                // behind Learn more (owner, 2026-10-08).
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    Text(copy.offerWhat)
                    Text(copy.offerExposureShort)
                    if learnMore {
                        Text(copy.offerExposure)
                        Text(copy.offerNoRepoint)
                        Text(copy.offerAskedOnce)
                    }
                }
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
                if !learnMore {
                    Button(copy.offerLearnMore) { learnMore = true }
                        .buttonStyle(GlassButtonStyle(.link))
                }
                HStack(spacing: GlassTokens.Space.s3) {
                    Button(copy.offerDecline, action: onDecline)
                        .buttonStyle(GlassButtonStyle(.glass))
                    Button(copy.offerAccept, action: onAccept)
                        .buttonStyle(GlassButtonStyle(.glass))
                }
            }
        }
        // The X answers as Not now does: the offer is asked once, so
        // closing it is an answer, not a deferral (owner, 2026-10-08).
        .overlay(alignment: .topTrailing) { CardClose(label: copy.offerDecline, action: onDecline) }
        .accessibilityElement(children: .contain)
    }
}

/// A card's close X (owner, 2026-10-08): the glyph alone, no container,
/// the same distance from the card's top and trailing edges; secondary,
/// primary under the pointer, with a control-sized target.
private struct CardClose: View {
    let label: String
    let action: () -> Void
    @State private var hovering = false

    /// The glyph's inset from the card's top and trailing edges.
    static let inset: CGFloat = GlassTokens.Space.s6
    /// The glyph's side.
    static let glyph: CGFloat = 12
    /// What a heading beside the X leaves free on its trailing side.
    static let clearance: CGFloat = glyph + GlassTokens.Space.s4

    var body: some View {
        Button(action: action) {
            Image(systemName: "xmark")
                .glassGlyph(Self.glyph, weight: .semibold)
                .foregroundStyle(hovering ? GlassColor.textPrimary : GlassColor.textSecondary)
                // A larger target than the glyph, centred so the glyph sits
                // exactly `inset` from both edges.
                .padding(Self.inset)
                .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .accessibilityLabel(label)
        .help(label)
    }
}

/// The offer to stop being asked about one project. Arming is a grant, so
/// it is offered only in the core's words (`tc_arming_offer_copy_json`) and
/// nothing is drawn without them. Evidence first, question second; declining
/// first and neither answer emphasised, since previews from the project stop.
/// Ron's shape (#1146 `ArmingOffer`): the eyebrow over the card, and the
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
                    HStack(spacing: GlassTokens.Space.s3) {
                        Button(copy.decline, action: onDecline)
                            .buttonStyle(GlassButtonStyle(.glass))
                        Button(copy.confirm) { confirming = true }
                            .buttonStyle(GlassButtonStyle(.glass))
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
