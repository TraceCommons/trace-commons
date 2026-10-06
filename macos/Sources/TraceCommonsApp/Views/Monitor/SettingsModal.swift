import SwiftUI
import TCDesign
import TCShellCore

/// Settings as Ron's #1146 modal over the Monitor (`settings-modal.tsx`,
/// with `surfaces.tsx` `Modal` and `glass.css` `.tc-scrim` / `.tc-modal`;
/// #1241 Task 10, owner 2026-10-05).
///
/// A scrim dims the three panes behind it (the window blurs them); the modal
/// is one more pane on the opaque base, with the pane edge and the modal
/// shadow, so nothing behind it reads through. Its header is the core's
/// title and subtitle, a Watching or Paused chip from the daemon's status,
/// and a close button. Its body is two columns: the section list, which
/// scrolls the body to a section, and one scrolling body holding every
/// section in order, Compute included. Escape, the close button and a click
/// on the scrim close it. Every word is the core's or a section's own
/// heading (`ShellWordingTests`).
struct SettingsModal: View {
    let request: SettingsRequest
    let navigation: MainWindowNavigation
    /// Whether watching is paused, from the daemon's status; nil while the
    /// status is unknown, and then no chip is drawn (Ron's `core.data`).
    let paused: Bool?
    let onClose: () -> Void
    /// The Private AI pointer: closes the modal and opens Inference.
    let onPrivateAI: () -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(ComputeModel.self) private var compute

    /// Every section, in the list's order, in one body.
    static let sections: [SettingsSection] = SettingsSection.allCases

    var body: some View {
        ZStack {
            // `.tc-scrim`: dims what is behind; a click on it closes.
            GlassTokens.Color.modalScrim.color
                .contentShape(Rectangle())
                .onTapGesture(perform: onClose)
                .accessibilityHidden(true)
            dialog
                .frame(maxWidth: GlassTokens.Size.modalWidth)
                .padding(.top, GlassTokens.Space.modalInsetTop)
                .padding([.horizontal, .bottom], GlassTokens.Space.modalInset)
        }
    }

    private var words: MonitorScreensCopy? { MonitorWords.table }

    /// `.tc-modal`: a pane on the opaque base, its edge and the modal shadow.
    private var dialog: some View {
        GlassPane(padding: 0, isContent: true) {
            VStack(spacing: 0) {
                header
                hairline
                columns
            }
        }
        .glassEdge(GlassTokens.Shadow.modal, in: RoundedRectangle(cornerRadius: GlassTokens.Radius.pane, style: .continuous))
        .focusSection()
        .onExitCommand(perform: onClose)
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isModal)
        .accessibilityLabel(words?.settingsTitle ?? "")
    }

    private var header: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s6) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(words?.settingsTitle ?? "")
                    .glassType(GlassTokens.TypeScale.title)
                    .foregroundStyle(GlassColor.textPrimary)
                    .accessibilityAddTraits(.isHeader)
                Text(words?.settingsSubtitle ?? "")
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            if let chip = Self.chip(paused: paused, words: words) {
                GlassChip(chip.word, status: chip.status)
            }
            GlassRoundButton(words?.close ?? "", systemImage: "xmark", small: true, action: onClose)
                // Nothing here takes focus by default and the panes behind
                // are disabled, so Escape binds to the close button as well
                // as to the dialog's exit command.
                .keyboardShortcut(.cancelAction)
        }
        .padding(.horizontal, GlassTokens.Space.panePadding)
        .padding(.vertical, GlassTokens.Space.s5)
    }

    /// The section list beside the one body it scrolls.
    private var columns: some View {
        ScrollViewReader { proxy in
            HStack(alignment: .top, spacing: 0) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 1) {
                        ForEach(Self.sections) { item in
                            // A section whose copy has not loaded is a
                            // disabled placeholder, never a missing row.
                            let row = item.listRow(.init(model: model, compute: compute.snapshot?.title))
                            Button {
                                proxy.scrollTo(item, anchor: .top)
                            } label: {
                                Text(row.text).lineLimit(1)
                            }
                            .buttonStyle(GlassMenuRowStyle())
                            .disabled(!row.enabled)
                            .accessibilityLabel(row.enabled ? row.text : MonitorWords.unknown)
                        }
                    }
                    .padding(GlassTokens.Space.s4)
                }
                .frame(width: GlassTokens.Size.modalNavWidth)
                .accessibilityElement(children: .contain)
                .accessibilityLabel(words?.settingsSections ?? "")
                Rectangle().fill(GlassColor.hairline).frame(width: 1).accessibilityHidden(true)
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Self.sections) { item in
                            section(item).id(item)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
            }
            .onAppear { Self.scroll(to: request.section, proxy) }
            .onChange(of: request) { _, new in Self.scroll(to: new.section, proxy) }
        }
    }

    /// One section of the body. Before onboarding, no write surface outside
    /// first run (R-43): a section that writes what first run asks draws
    /// the Monitor's onboarding notice in its place, whose button opens
    /// first run. As in the Monitor's pane, it waits until the core has
    /// said, and a refused daemon is said as a refusal (`MonitorGate`).
    @ViewBuilder
    private func section(_ item: SettingsSection) -> some View {
        switch MonitorGate.of(
            startup: model.startup, onboardingKnown: model.onboardingKnown,
            requiresOnboarding: model.requiresOnboarding
        ).forSettings(availableBeforeOnboarding: item.availableBeforeOnboarding) {
        case .awaiting:
            SettingsAwaiting()
                .padding(GlassTokens.Space.panePadding)
                .frame(maxWidth: .infinity, alignment: .topLeading)
        case .down(let sentence):
            StartupRefusedBanner(sentence: sentence)
                .padding(GlassTokens.Space.panePadding)
                .frame(maxWidth: .infinity, alignment: .topLeading)
        case .signedOut:
            GlassNotice(tone: .ask, title: MonitorWords.signedOut) {
                Button(MonitorWindowView.openFirstRun) { OpenMonitor.request() }
            }
            .padding(GlassTokens.Space.panePadding)
            .frame(maxWidth: .infinity, alignment: .topLeading)
        case .open:
            switch item {
            case .compute:
                ComputeView(model: compute)
                    .padding(GlassTokens.Space.panePadding)
                    .frame(maxWidth: 560, alignment: .leading)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
            default:
                GlassSettingsContent(navigation: navigation, section: item, onPrivateAI: onPrivateAI)
            }
        }
    }

    private var hairline: some View {
        Rectangle().fill(GlassColor.hairline).frame(height: 1).accessibilityHidden(true)
    }

    /// Straight to the asked-for section; the top for none (the gear).
    static func scroll(to section: SettingsSection?, _ proxy: ScrollViewProxy) {
        guard let section else { return }
        proxy.scrollTo(section, anchor: .top)
    }

    /// The header's chip: the core's Watching or Paused, from the daemon's
    /// status, or nothing while the status is unknown.
    static func chip(paused: Bool?, words: MonitorScreensCopy?) -> (word: String, status: GlassStatus)? {
        guard let paused, let words else { return nil }
        return paused ? (words.paused, .off) : (words.watching, .on)
    }
}
