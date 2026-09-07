import SwiftUI
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

    /// The private-inference tone onto this shell's palette.
    ///
    /// A separate bridge from the routing and witness ones for the reason
    /// spelled out on `SettingsView.witnessTone`: the three ABI tone ranges
    /// are disjoint so a cross-wired mapper is wrong for every value.
    ///
    /// Every arm answers a distinct `TC.Tone`, and each of those carries its
    /// own glyph as well as its own colour -- so held, attention, refused
    /// and anything a later daemon grows stay distinguishable from clear in
    /// greyscale and to a colour-blind reader, which is the whole point.
    static func palette(_ tone: PrivateInferenceTone) -> TC.Tone {
        switch tone {
        case .neutral: return .neutral
        case .held: return .held
        case .clear: return .clear
        case .attention: return .attention
        case .refused: return .refused
        }
    }
}

/// Answering model calls on this computer: a destination of its own rather
/// than a card near the bottom of Settings.
///
/// Renders nothing at all if the words did not arrive, for the reason
/// `AppModel.privateInferenceCopy` gives: a screen missing the sentence
/// about what turning the switch on exposes is worse than no screen.
struct PrivateInferenceView: View {
    var body: some View {
        ScrollView {
            PrivateInferenceContent()
        }
        .tcScreen()
    }
}

/// The screen's content, split out of its `ScrollView` for the same reason
/// `SettingsContent` and `QueueContent` are: `ImageRenderer` renders a
/// `ScrollView` as blank, so the screenshot hook can only rasterize what
/// lives outside one.
struct PrivateInferenceContent: View {
    @EnvironmentObject private var model: AppModel

    /// The same narrow prose column Settings uses. This screen is three
    /// paragraphs and a switch; the full window width would set them at a
    /// measure nobody reads.
    private static let proseColumn: CGFloat = 520

    var body: some View {
        if let copy = model.privateInferenceCopy {
            content(copy)
        }
    }

    @ViewBuilder
    private func content(_ copy: PrivateInferenceCopy) -> some View {
        let state = model.privateInferenceState
        let tone = PrivateInferenceIndicator.palette(
            PrivateInferenceSurface.tone(state, calls: model.privateInferenceCalls))
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            // The list leads. Connecting a tool is the thing a contributor
            // can decide about; answering model calls at all is what happens
            // because they connected one, not a question to settle first.
            HarnessListSection(copy: copy)
            Divider().padding(.vertical, TC.Space.s)
            // The switch, below the list and unchanged: a kill switch, which
            // is what it always was.
            TCSectionHeader(title: copy.settingsTitle)
            Text(copy.offerWhat)
                .font(TC.Font_.body)
                .fixedSize(horizontal: false, vertical: true)
            // The exposure paragraph in full, on the destination as well as
            // in the offer. A contributor who declined and came back months
            // later is making the same decision and is owed the same words.
            Text(copy.offerExposure)
                .font(TC.Font_.body)
                .fixedSize(horizontal: false, vertical: true)
            Toggle(
                copy.settingsToggle,
                isOn: Binding(
                    get: { model.daemonSettings?.privateInferenceOn ?? false },
                    set: { model.applyPrivateInference($0) }
                )
            )
            .disabled(model.privateInferenceBusy || model.daemonSettings?.privateInference == nil)
            .toggleStyle(.switch)
            .tint(TC.green)
            .font(TC.Font_.body)
            // The switch above says what was asked for. This says what
            // happened, and it is drawn from the tone -- never from the
            // switch's own boolean, which stays on over a listener that
            // refused to start.
            Label(
                PrivateInferenceSurface.stateLine(
                    state, copy: copy, calls: model.privateInferenceCalls),
                systemImage: tone.symbol
            )
            .font(TC.Font_.body)
            .foregroundStyle(tone.textColor)
            .fixedSize(horizontal: false, vertical: true)
            if let serving = PrivateInferenceSurface.servingLine(
                state, calls: model.privateInferenceCalls)
            {
                Text(serving).font(TC.Font_.meta).foregroundStyle(.secondary)
            }
            Text(copy.settingsAppliesAtOnce)
                .font(TC.Font_.meta)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let error = model.lastActionError {
                ActionErrorBanner(text: error) { model.lastActionError = nil }
            }
        }
        .padding(.top, TC.Space.Content.top)
        .padding(.horizontal, TC.Space.Content.horizontal)
        .padding(.bottom, TC.Space.Content.bottom)
        .tcColumn(Self.proseColumn)
    }
}
