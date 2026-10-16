import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What the contributor must be able to see whenever it is live, above the
/// Traces tree under its health banners: the action messages, the undos
/// (the approval's, a session's Contribute and a folder's Submit all, in
/// Ron's `UndoBar` shape, and Undo keep), the arming offer, the Private AI
/// offer and the first-contribution note. Ron's #1146 mounts these as
/// `WaitingPrompts` at the top of the inspector; offers, undo and health
/// above the tree is an accepted difference (owner, 2026-10-07), so on
/// Traces none of them depends on the inspector being shown. Off Traces the
/// window draws them at the top of the inspector, as Ron's shell does
/// (`MonitorWindowView.promptsInInspector`), and an undo or offer appearing
/// there opens it (`InspectorDemand`). Every sentence is the core's or
/// `QueueLegacyWords`'.
struct InspectorPrompts: View {
    @EnvironmentObject private var model: AppModel
    let store: TracesStore

    /// The core's Dismiss, or the word the legacy banner already says.
    private var dismissWord: String {
        store.words?.dismissAction ?? ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            // A failed action: the line, unboxed, and its Dismiss a link
            // after it (Ron, 2026-10-09).
            if let error = model.lastActionError {
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                    GlassAlert(error)
                    Button(dismissWord) { model.lastActionError = nil }
                        .buttonStyle(GlassButtonStyle(.link))
                        .fixedSize()
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

            if let offer = model.armingOffer {
                ArmingOfferGlassCard(
                    offer: offer,
                    eyebrow: store.words?.optionalAutomation,
                    onArm: { model.acceptArmingOffer(offer) },
                    onDecline: { model.declineArmingOffer(offer) }
                )
            }
            if model.showsPrivateInferenceOffer, let copy = model.privateInferenceCopy {
                PrivateAIOfferGlassCard(
                    copy: copy,
                    onAccept: { model.answerPrivateInferenceOffer(accepted: true) },
                    onDecline: { model.answerPrivateInferenceOffer(accepted: false) }
                )
                .disabled(model.privateInferenceBusy)
            }
            // Only on an answered, empty history; whether anything is under
            // review reads the answered queue.
            if model.historyAnswered, model.queueAnswered, model.history.isEmpty,
               let copy = model.witnessCopy?.onboarding {
                FirstContributionGlassNote(copy: copy, reviewing: !model.awaitingDecision.isEmpty)
            }
        }
    }

    /// Ron's `UndoBar` over the approval the app model holds: the core's
    /// "APPROVAL SAVED", always, over the core's toast for it, then how long ago. No
    /// timer removes it, and no countdown is drawn: the deadline is the
    /// daemon's next upload sweep, which nothing here can observe.
    private func approvalUndo(_ undo: AppModel.Undo) -> some View {
        UndoBarCard(eyebrow: store.words?.undo.approvalSaved, title: undo.toastLine) {
            if undo.offerUndo {
                Text(QueueLegacyWords.undoWillSend)
                    .fixedSize(horizontal: false, vertical: true)
                Text(QueueLegacyWords.approvedAgo(undo.heldSeconds))
                    .monospacedDigit()
            }
        } actions: {
            if undo.offerUndo {
                // The app's one Return binding, on the safe action: a
                // keystroke pulls a transcript back.
                Button(store.words?.undoContribute ?? QueueLegacyWords.undo) { model.undoApproval() }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .keyboardShortcut(.defaultAction)
            }
            // Ron's `TertiaryLink`: closing the card is the lesser action.
            Button(dismissWord) { model.dismissUndo() }
                .buttonStyle(GlassButtonStyle(.link))
                .help(undo.offerUndo ? QueueLegacyWords.closeNoticeStillSends : QueueLegacyWords.closeNotice)
        }
    }

    /// The contribution, the folder's Submit all and the keep that can
    /// still be taken back, each with the core's words and any refusal of
    /// its undo. "Undoing…" while the undo is in flight. The two cards end
    /// with the core's Dismiss, as Ron's `UndoBar` does: it closes the card
    /// and the contribution stands.
    @ViewBuilder
    private var storeUndo: some View {
        if let contributed = store.lastContributed, let words = store.words {
            let busy = store.acting.contains(contributed.entryId)
            UndoBarCard(eyebrow: words.undo.approvalSaved, title: Self.title(contributed.label, toast: contributed.toast, words: words)) {
                UndoDetail(toast: contributed.toast, label: contributed.label, at: contributed.at, words: words)
            } actions: {
                if contributed.toast.offerUndo {
                    Button(busy ? words.undo.undoing : words.undoContribute) {
                        Task { await store.perform(.undoContribute, on: contributed.entryId) }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(busy)
                }
                Button(dismissWord) { store.dismissContributed() }
                    .buttonStyle(GlassButtonStyle(.link))
                    .disabled(busy)
            } refusal: {
                // A refused undo, in the card under its buttons.
                TracesRefusal(store: store, entryId: contributed.entryId)
            }
        }
        if let folder = store.lastContributedFolder, let words = store.words {
            let busy = store.writing.contains(folder.projectId)
            UndoBarCard(eyebrow: words.undo.approvalSaved, title: Self.title(folder.label, toast: folder.toast, words: words)) {
                UndoDetail(toast: folder.toast, label: folder.label, at: folder.at, words: words)
            } actions: {
                if folder.toast.offerUndo {
                    Button(busy ? words.undo.undoing : words.undoContribute) {
                        Task { await store.undoFolder(folder.projectId) }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(busy)
                }
                Button(dismissWord) { store.dismissContributedFolder() }
                    .buttonStyle(GlassButtonStyle(.link))
                    .disabled(busy)
            }
            // A refused undo is said beside the folder (`folderNotes`).
        }
        if let kept = store.lastKept, let words = store.words {
            let busy = store.acting.contains(kept)
            Button(busy ? words.undo.undoing : words.undoKeep) { Task { await store.perform(.undoKeep, on: kept) } }
                .buttonStyle(GlassButtonStyle(.glass))
                .disabled(busy)
            TracesRefusal(store: store, entryId: kept)
        }
    }

    /// #1146's undo title, "{label} approved", for the folder the
    /// contribution came from; the core's toast when no folder is named.
    static func title(_ label: String?, toast: SubmitToast, words: MonitorTracesCopy) -> String {
        guard let label, !label.isEmpty, toast.offerUndo else { return toast.line }
        return words.undo.approved.replacingOccurrences(of: "{label}", with: label)
    }
}

/// The line under a store undo's title: while Undo is offered, how long
/// ago it was approved (the accepted count-up), else #1146's "Upload may
/// already have started."; then the core's toast, when the title did not
/// already say it, so its redaction and flag counts are never lost.
private struct UndoDetail: View {
    let toast: SubmitToast
    let label: String?
    let at: Date
    let words: MonitorTracesCopy

    var body: some View {
        if toast.offerUndo {
            TimelineView(.periodic(from: at, by: 1)) { context in
                Text(QueueLegacyWords.approvedAgo(max(0, Int(context.date.timeIntervalSince(at)))))
                    .monospacedDigit()
            }
        } else {
            Text(words.undo.mayHaveStarted)
        }
        if InspectorPrompts.title(label, toast: toast, words: words) != toast.line {
            Text(toast.line).fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// Ron's `UndoBar` card (`undo-bar.tsx`): the eyebrow, the line that says
/// what was approved and the lines under it on the left, its buttons on the
/// right, Undo then the Dismiss link. A refused undo is said in the card,
/// under the row that holds its buttons. Every word is the caller's, from
/// the core.
private struct UndoBarCard<Detail: View, Actions: View, Refusal: View>: View {
    let eyebrow: String?
    let title: String
    @ViewBuilder let detail: () -> Detail
    @ViewBuilder let actions: () -> Actions
    @ViewBuilder let refusal: () -> Refusal

    var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                row
                refusal()
            }
        }
        .accessibilityElement(children: .contain)
    }

    private var row: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s8) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                if let eyebrow {
                    Text(eyebrow)
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                }
                Text(title)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) { detail() }
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .layoutPriority(1)
            HStack(spacing: GlassTokens.Space.s4) { actions() }
                .lineLimit(1)
                .fixedSize()
        }
    }
}

extension UndoBarCard where Refusal == EmptyView {
    init(
        eyebrow: String?, title: String,
        @ViewBuilder detail: @escaping () -> Detail, @ViewBuilder actions: @escaping () -> Actions
    ) {
        self.init(eyebrow: eyebrow, title: title, detail: detail, actions: actions, refusal: { EmptyView() })
    }
}
