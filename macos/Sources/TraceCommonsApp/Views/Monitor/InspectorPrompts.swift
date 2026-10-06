import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// What the contributor must be able to see whenever it is live, at the top
/// of the inspector on Home, Traces and History (Ron's #1146
/// `WaitingPrompts`): the action messages, the undos (the approval's, a
/// session's Contribute and a folder's Submit all, in Ron's `UndoBar` shape,
/// and Undo keep), the arming offer, the Private AI offer and the
/// first-contribution note.
///
/// The inspector opens itself when one of these appears (`InspectorDemand`),
/// so none runs out of sight in a window that started with it closed. Every
/// sentence is the core's or `QueueLegacyWords`'.
struct InspectorPrompts: View {
    @EnvironmentObject private var model: AppModel
    let store: TracesStore

    /// The core's Dismiss, or the word the legacy banner already says.
    private var dismissWord: String {
        store.words?.dismissAction ?? ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord
    }

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
                    .buttonStyle(GlassButtonStyle(.primary))
                    .keyboardShortcut(.defaultAction)
            }
            Button(dismissWord) { model.dismissUndo() }
                .buttonStyle(GlassButtonStyle(.glass))
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
            UndoBarCard(eyebrow: words.undo.approvalSaved, title: contributed.toast.line) {
                EmptyView()
            } actions: {
                if contributed.toast.offerUndo {
                    Button(busy ? words.undo.undoing : words.undoContribute) {
                        Task { await store.perform(.undoContribute, on: contributed.entryId) }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(busy)
                }
                Button(dismissWord) { store.dismissContributed() }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(busy)
            }
            TracesRefusal(store: store, entryId: contributed.entryId)
        }
        if let folder = store.lastContributedFolder, let words = store.words {
            let busy = store.writing.contains(folder.projectId)
            UndoBarCard(eyebrow: words.undo.approvalSaved, title: folder.toast.line) {
                EmptyView()
            } actions: {
                if folder.toast.offerUndo {
                    Button(busy ? words.undo.undoing : words.undoContribute) {
                        Task { await store.undoFolder(folder.projectId) }
                    }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .disabled(busy)
                }
                Button(dismissWord) { store.dismissContributedFolder() }
                    .buttonStyle(GlassButtonStyle(.glass))
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
}

/// Ron's `UndoBar` card: an eyebrow, the line that says what was approved,
/// the lines under it, and its buttons. Every word is the caller's, from
/// the core.
private struct UndoBarCard<Detail: View, Actions: View>: View {
    let eyebrow: String?
    let title: String
    @ViewBuilder let detail: () -> Detail
    @ViewBuilder let actions: () -> Actions

    var body: some View {
        GlassCard {
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
                HStack(spacing: GlassTokens.Space.s3) { actions() }
                    .padding(.top, GlassTokens.Space.s2)
            }
        }
        .accessibilityElement(children: .contain)
    }
}
