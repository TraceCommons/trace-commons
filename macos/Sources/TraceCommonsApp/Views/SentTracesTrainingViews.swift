import SwiftUI
import TCDesign
import TCShellCore

/// The one-time sheet: traces already sent without Train coding models,
/// and the choice to add that use to them. Its two buttons carry equal
/// weight: one style, one size, neither the default nor prominent, and
/// neither on Return. Nothing is preselected; Not now (and Escape, the
/// scrim, the close button) closes it and sends nothing. Every sentence is
/// the core's draft copy (`SentTracesTrainingCopy`).
struct SentTracesTrainingSheetView: View {
    @ObservedObject var model: SentTracesTrainingModel
    let copy: SentTracesTrainingCopy
    let sheet: SentTracesTrainingSheet

    var body: some View {
        GlassModal(
            title: copy.title, width: .narrow,
            actions: Self.actions(
                copy, count: sheet.count, busy: model.busy,
                notNow: { model.notNow() },
                allow: { Task { await model.allowAll() } }),
            busy: model.busy, onCancel: { model.notNow() }
        ) {
            GlassModalBody(spacing: GlassTokens.Space.s4) {
                Text(copy.body(count: sheet.count))
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                Text(copy.scope)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                if model.sheetFailed {
                    GlassAlert(copy.failed)
                }
            }
        }
    }

    /// Not now, then Allow: the same role, so the same kind and size, and
    /// neither is the default or prominent, so neither is drawn as the
    /// primary or taken by Return.
    static func actions(
        _ copy: SentTracesTrainingCopy, count: UInt64, busy: Bool,
        notNow: @escaping () -> Void, allow: @escaping () -> Void
    ) -> [GlassModalAction] {
        [
            GlassModalAction(
                copy.notNowButton, role: .standard, isDefault: false, isEnabled: !busy, isProminent: false,
                accessibilityName: copy.notNowAccessibilityName, action: notNow),
            GlassModalAction(
                copy.allowButton, role: .standard, isDefault: false, isEnabled: !busy, isProminent: false,
                accessibilityName: copy.allowAccessibilityName(count: count), action: allow),
        ]
    }
}

/// Raises the sheet in the window it was asked for (`site`), and in no
/// other. Put it inside a window's `glassModalHost()`.
struct SentTracesTrainingPresenter: ViewModifier {
    let site: SentTracesTrainingSite
    @EnvironmentObject private var app: AppModel

    func body(content: Content) -> some View {
        content.modifier(SentTracesTrainingSheetLayer(site: site, model: app.sentTracesTraining))
    }
}

private struct SentTracesTrainingSheetLayer: ViewModifier {
    let site: SentTracesTrainingSite
    @ObservedObject var model: SentTracesTrainingModel

    func body(content: Content) -> some View {
        let shown = model.sheet.flatMap { $0.site == site ? $0 : nil }
        content.glassModal(isPresented: Binding(
            get: { shown != nil && model.copy != nil },
            set: { if !$0 { model.notNow() } })
        ) {
            if let shown, let copy = model.copy {
                SentTracesTrainingSheetView(model: model, copy: copy, sheet: shown)
            }
        }
        // A window closed under its sheet is Not now: nothing was chosen,
        // and the sheet is not left waiting for a window that is gone.
        .onDisappear {
            if model.sheet?.site == site { model.notNow() }
        }
    }
}

/// The quiet line under Home's Insights card while sent traces lack Train
/// coding models, with Review, which raises the same sheet. Re-read when
/// Home's History is, and hidden on any refusal. It sits beside the
/// Insights card, not inside it: the card is one button, and a button
/// inside it would lose its own accessibility name.
struct SentTracesTrainingLine: View {
    @ObservedObject var model: SentTracesTrainingModel
    /// Home's History, whose every re-read re-reads the count.
    let history: [DaemonData.HistoryRow]?

    var body: some View {
        // Always present, so `.task` runs even while nothing is drawn.
        VStack(alignment: .leading, spacing: 0) {
            if let copy = model.copy, let count = model.state.count {
                GlassCard(quiet: true) {
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                        Text(copy.line(count: count))
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 0)
                        Button(copy.reviewButton) { model.review() }
                            .buttonStyle(GlassButtonStyle(.link))
                            .accessibilityLabel(copy.reviewAccessibilityName)
                    }
                }
            }
        }
        .task { await model.refresh() }
        .onChange(of: history) {
            Task { await model.refresh() }
        }
    }
}

/// In a sent trace's inspector: the line and Allow while the trace lacks
/// Train coding models, Credit pending once it was allowed until its
/// History row shows credit, and the failed line beside Allow after a
/// refused grant.
struct SentTraceTrainingSection: View {
    @ObservedObject var model: SentTracesTrainingModel
    let row: DaemonData.HistoryRow

    var body: some View {
        let id = row.submissionId
        let mark = model.state.mark(
            for: id,
            credited: SentTracesTrainingState.credited(final: row.creditPointsFinal))
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            if let copy = model.copy {
                if mark == .pending {
                    GlassTag(copy.pending)
                        .fixedSize()
                } else if model.state.offers(id) {
                    Text(copy.traceLine)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if mark == .failed {
                        GlassAlert(copy.failed)
                    }
                    Button(copy.traceButton) { Task { await model.allow(id) } }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .accessibilityLabel(copy.traceAccessibilityName)
                        .disabled(model.busy)
                }
            }
        }
        .task {
            if model.state.eligibility == .unknown { await model.refresh() }
        }
    }
}
