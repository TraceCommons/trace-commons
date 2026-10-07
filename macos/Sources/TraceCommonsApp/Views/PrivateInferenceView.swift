import SwiftUI
import TCDesign
import TCShellCore

/// The indicator every surface on this destination is painted from.
///
/// One place, deliberately. The sidebar row, the menu-bar section and this
/// screen's own status line all want the same answer, and the dangerous way
/// to get it is to reach for `daemonSettings?.privateInferenceOn` -- the
/// switch, which says what was ASKED FOR. A listener that refused to start
/// leaves that switch on. Everything here derives from the tone the shared
/// table answers with, so "on" and "working" can never be confused.
///
/// It holds no words. Every sentence on this destination comes from
/// `PrivateInferenceCopy`, which is composed in the Rust contributor crate.
enum PrivateInferenceIndicator {
    /// Whether an indicator may be painted as working. `Clear` alone; see
    /// `PrivateInferenceTone.readsAsWorking`.
    static func readsAsWorking(
        _ state: PrivateInferenceState, calls: PrivateInferenceCalls
    ) -> Bool {
        PrivateInferenceSurface.tone(state, calls: calls).readsAsWorking
    }

    /// The tone onto a glass status, as the window's Inference dot maps
    /// it (`MonitorWindowView.inferenceDot`): only clear is on; held,
    /// attention and refused all ask; neutral is off. A glass status is a dot,
    /// so whoever draws it draws the core's sentence beside it.
    static func status(_ tone: PrivateInferenceTone) -> GlassStatus {
        switch tone {
        case .clear: return .on
        case .held, .attention, .refused: return .ask
        case .neutral: return .off
        }
    }

    /// The Private AI dot (#1146 `flow-map.tsx` and `inference-inspector.tsx`):
    /// on while the core's tone is clear, outside (red) for every other
    /// tone, an unread one included. Drawn on the Inference tab, the map's
    /// Private AI segment and the inspector's Status row, always beside or
    /// behind the core's sentence for the same state.
    static func dotStatus(_ tone: PrivateInferenceTone) -> GlassStatus {
        tone.readsAsWorking ? .on : .outside
    }
}

/// The Private AI switch, with the core's sentence on what turning it on
/// exposes beside it, drawn by the glass Inference tab on `InferenceStore`.
///
/// The switch says what was asked for; the state line says what happened,
/// and it is drawn from the core's tone -- never from `isOn`, which stays on
/// over a listener that refused to start. `isOn` nil is a switch nobody
/// could read: drawn off and disabled, never on. A write that was not
/// confirmed is `refusal`, in the core's words, outside the expander so a
/// collapsed card still shows it.
struct PrivateAISwitchCard: View {
    let copy: PrivateInferenceCopy
    let isOn: Bool?
    let state: PrivateInferenceState
    let calls: PrivateInferenceCalls
    let busy: Bool
    let refusal: String?
    let onSet: (Bool) -> Void
    let onDismiss: () -> Void
    /// Re-reads the connection. With it the card wears #1146's panel header
    /// (the connection eyebrow and a re-read link) above its expander.
    var onRefresh: (() -> Void)?

    @State private var isOpen = false

    /// The state line and its dot, from the listener's report alone. It
    /// takes no switch: what was asked for never says what happened.
    static func stateLabel(
        state: PrivateInferenceState, copy: PrivateInferenceCopy, calls: PrivateInferenceCalls
    ) -> (line: String, status: GlassStatus) {
        (PrivateInferenceSurface.stateLine(state, copy: copy, calls: calls),
         PrivateInferenceIndicator.status(PrivateInferenceSurface.tone(state, calls: calls)))
    }

    var body: some View {
        let label = Self.stateLabel(state: state, copy: copy, calls: calls)
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                    if let onRefresh {
                        PrivateAIPanelHeader(
                            eyebrow: copy.panelConnectionEyebrow, refresh: copy.panelRefresh,
                            disabled: busy, onRefresh: onRefresh)
                    }
                    GlassExpander(copy.settingsTitle, isOpen: $isOpen)
                    GlassStatusLabel(label.line, status: label.status)
                        .fixedSize(horizontal: false, vertical: true)
                    if isOpen {
                        Text(copy.offerWhat)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                        // The exposure paragraph in full, on the destination as well as
                        // in the offer. A contributor who declined and came back months
                        // later is making the same decision and is owed the same words.
                        Text(copy.offerExposure)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                        Toggle(copy.settingsToggle, isOn: Binding(get: { isOn ?? false }, set: onSet))
                            .toggleStyle(GlassToggleStyle(.settings))
                            .disabled(busy || isOn == nil)
                        if let serving = PrivateInferenceSurface.servingLine(state, calls: calls) {
                            Text(serving)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textSecondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Text(copy.settingsAppliesAtOnce)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            if let refusal {
                GlassNotice(tone: .outside, title: refusal) {
                    Button(ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord, action: onDismiss)
                        .buttonStyle(GlassButtonStyle(.glass))
                }
            }
        }
    }
}
