#if DEBUG
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What the legacy queue draws above its list, drawn above the Traces tree:
/// the action messages, both undos, the certificate list, the two consent
/// offers, the first-contribution note and why sessions stopped waiting.
///
/// Above the tree rather than in the inspector, so hiding the inspector
/// never hides an Undo that can still take something back or an offer
/// waiting on an answer. Every sentence is the core's or `QueueLegacyWords`'.
struct TracesOffersBar: View {
    @EnvironmentObject private var model: AppModel
    let store: TracesStore

    /// The core's Dismiss, or the word the legacy banner already says.
    private var dismissWord: String { store.words?.dismiss ?? ActionMessageBanner.dismissWord }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let error = model.lastActionError {
                GlassNotice(tone: .outside, title: error) {
                    Button(dismissWord) { model.lastActionError = nil }
                        .buttonStyle(GlassButtonStyle(.glass))
                }
            }
            if let notice = model.lastActionNotice {
                GlassNotice(tone: .ask, title: notice) {
                    Button(dismissWord) { model.lastActionNotice = nil }
                        .buttonStyle(GlassButtonStyle(.glass))
                }
            }
            if let undo = model.undo {
                approvalUndo(undo)
            }
            storeUndo

            // The sessions a witness certificate is held for, drawn on every
            // render including when there are none (`CertificateSection`).
            CertificateSection(entries: model.awaitingDecision)

            if model.showsPrivateInferenceOffer, let copy = model.privateInferenceCopy {
                PrivateAIOfferGlassCard(
                    copy: copy,
                    onAccept: { model.answerPrivateInferenceOffer(accepted: true) },
                    onDecline: { model.answerPrivateInferenceOffer(accepted: false) }
                )
                .disabled(model.privateInferenceBusy)
            }
            if let offer = model.armingOffer {
                ArmingOfferGlassCard(
                    offer: offer,
                    onArm: { model.acceptArmingOffer(offer) },
                    onDecline: { model.declineArmingOffer(offer) }
                )
            }
            if model.history.isEmpty, let copy = model.witnessCopy?.onboarding {
                FirstContributionGlassNote(copy: copy, reviewing: !model.awaitingDecision.isEmpty)
            }
            NotOfferedGlassDisclosure(counts: model.outcomeCounts)
        }
    }

    /// The submit toast and, when something was approved, its Undo. No
    /// timer removes it: the deadline is the daemon's next upload sweep,
    /// which nothing here can observe (`UndoBar`).
    private func approvalUndo(_ undo: AppModel.Undo) -> some View {
        GlassNotice(tone: .ask, title: undo.toastLine) {
            if undo.offerUndo {
                Text(QueueLegacyWords.undoWillSend)
                    .fixedSize(horizontal: false, vertical: true)
                Text(QueueLegacyWords.approvedAgo(undo.heldSeconds))
                    .monospacedDigit()
            }
            HStack(spacing: GlassTokens.Space.s3) {
                if undo.offerUndo {
                    // The app's one Return binding, on the safe action: a
                    // keystroke pulls a transcript back.
                    Button(store.words?.undoContribute ?? QueueLegacyWords.undo) { model.undoApproval() }
                        .buttonStyle(GlassButtonStyle(.primary))
                        .keyboardShortcut(.defaultAction)
                }
                Button(dismissWord) { model.dismissUndo() }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .help(undo.offerUndo ? QueueLegacyWords.closeNoticeStillSends : QueueLegacyWords.closeNotice)
            }
        }
    }

    /// The contribution and the keep that can still be taken back, each
    /// with the core's words and any refusal of its undo.
    @ViewBuilder
    private var storeUndo: some View {
        if let contributed = store.lastContributed, let words = store.words {
            GlassNotice(tone: .ask, title: contributed.toast.line) {
                if contributed.toast.offerUndo {
                    Button(words.undoContribute) {
                        Task { await store.perform(.undoContribute, on: contributed.entryId) }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(store.acting.contains(contributed.entryId))
                }
            }
            TracesRefusal(store: store, entryId: contributed.entryId)
        }
        if let folder = store.lastContributedFolder, let words = store.words {
            GlassNotice(tone: .ask, title: folder.toast.line) {
                if folder.toast.offerUndo {
                    Button(words.undoContribute) { Task { await store.undoFolder(folder.projectId) } }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(store.writing.contains(folder.projectId))
                }
            }
            // A refused undo is said beside the folder (`folderNotes`).
        }
        if let kept = store.lastKept, let words = store.words {
            Button(words.undoKeep) { Task { await store.perform(.undoKeep, on: kept) } }
                .buttonStyle(GlassButtonStyle(.glass))
                .disabled(store.acting.contains(kept))
            TracesRefusal(store: store, entryId: kept)
        }
    }
}

/// The core's words for a refused action on `entryId`, if there is one.
/// The bar and the inspector both say a refusal through this.
struct TracesRefusal: View {
    let store: TracesStore
    let entryId: String

    var body: some View {
        if let refused = store.actionError, refused.entryId == entryId, let line = store.message(for: refused.error) {
            GlassNotice(tone: .outside, title: line) { EmptyView() }
        }
    }
}

/// The first-contribution note, until anything has been contributed.
private struct FirstContributionGlassNote: View {
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
private struct NotOfferedGlassDisclosure: View {
    let counts: [String: Int]

    @State private var expanded = false

    var body: some View {
        if !counts.isEmpty {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                GlassExpander(QueueLegacyWords.noLongerWaiting(counts.values.reduce(0, +)), isOpen: $expanded)
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
                        Text(QueueLegacyWords.notOfferedScope)
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
/// this question opens a listener anything on the machine can use.
private struct PrivateAIOfferGlassCard: View {
    let copy: PrivateInferenceCopy
    let onAccept: () -> Void
    let onDecline: () -> Void

    var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                Text(copy.offerTitle)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                // What it does, what it exposes, then what it does not do.
                Group {
                    Text(copy.offerWhat)
                    Text(copy.offerExposure)
                    Text(copy.offerNoRepoint)
                }
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
                Text(copy.offerAskedOnce)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: GlassTokens.Space.s3) {
                    Button(copy.offerDecline, action: onDecline)
                        .buttonStyle(GlassButtonStyle(.glass))
                    Button(copy.offerAccept, action: onAccept)
                        .buttonStyle(GlassButtonStyle(.glass))
                }
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// The offer to stop being asked about one project. Arming is a grant, so
/// it is offered only in the core's words (`tc_arming_offer_copy_json`) and
/// nothing is drawn without them. Evidence first, question second; declining
/// first and neither answer emphasised, since previews from the project stop.
private struct ArmingOfferGlassCard: View {
    let offer: ArmingOffer
    let onArm: () -> Void
    let onDecline: () -> Void

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
                    Text(copy.evidence)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(copy.question)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: GlassTokens.Space.s3) {
                        Button(copy.decline, action: onDecline)
                            .buttonStyle(GlassButtonStyle(.glass))
                        Button(copy.confirm, action: onArm)
                            .buttonStyle(GlassButtonStyle(.glass))
                    }
                }
            }
            .accessibilityElement(children: .contain)
        }
    }
}
#endif
