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

    /// Every section, in order, in one body.
    static let sections: [SettingsSection] = SettingsSection.allCases

    /// The sections the list names: #1146's twelve.
    static let listed: [SettingsSection] = SettingsSection.listed

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
                    .overlay(alignment: .bottom) { rule(.horizontal) }
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
                    .glassType(GlassTokens.TypeScale.title.weight(.bold))
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
        // `.tc-modal__header`: 14 16 10 18, as every `GlassModal`.
        .padding(.top, GlassTokens.Space.s7)
        .padding(.trailing, GlassTokens.Space.s8)
        .padding(.bottom, GlassTokens.Space.s5)
        .padding(.leading, Self.headerLeading)
    }

    /// The modal header's and body's leading inset (#1146 `.tc-modal__header`).
    static let headerLeading: CGFloat = 18

    /// The section list beside the one body it scrolls (#1146
    /// `settings-modal.tsx`): 180pt, 12pt secondary rows with a faint
    /// hover, and a 0.5pt rule between the two.
    private var columns: some View {
        ScrollViewReader { proxy in
            HStack(alignment: .top, spacing: 0) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 1) {
                        ForEach(Self.listed) { item in
                            // A section whose copy has not loaded is a
                            // disabled placeholder, never a missing row.
                            let row = item.listRow(words?.settingsNav)
                            Button {
                                proxy.scrollTo(item, anchor: .top)
                            } label: {
                                Text(row.text)
                            }
                            .buttonStyle(GlassSectionNavRowStyle())
                            .disabled(!row.enabled)
                            .help(row.enabled ? row.text : "")
                            .accessibilityLabel(row.enabled ? row.text : MonitorWords.unknown)
                        }
                    }
                    // #1146 `px-2 py-2.5`.
                    .padding(.horizontal, GlassTokens.Space.s4)
                    .padding(.vertical, GlassTokens.Space.s5)
                }
                .frame(width: GlassTokens.Size.modalNavWidth)
                .accessibilityElement(children: .contain)
                .accessibilityLabel(words?.settingsSections ?? "")
                rule(.vertical)
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Self.sections) { item in
                            VStack(alignment: .leading, spacing: 0) {
                                if let name = item.navName(words?.settingsNav) {
                                    GlassSectionRule(name)
                                        .padding(.horizontal, Self.bodyInset)
                                        .padding(.top, Self.sectionGap)
                                }
                                section(item)
                            }
                            .id(item)
                        }
                    }
                    .padding(.bottom, GlassTokens.Space.s10)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
            }
            .onAppear { Self.scroll(to: request.section, proxy) }
            .onChange(of: request) { _, new in Self.scroll(to: new.section, proxy) }
        }
    }

    /// The body's side inset (#1146 `px-5`).
    static let bodyInset: CGFloat = GlassTokens.Space.s9
    /// Above each section's rule (#1146 `pt-3.5` and the rule's own top).
    static let sectionGap: CGFloat = GlassTokens.Space.s5

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
                .padding(.horizontal, Self.bodyInset)
                .padding(.vertical, GlassTokens.Space.s6)
                .frame(maxWidth: .infinity, alignment: .topLeading)
        case .down(let sentence):
            StartupRefusedBanner(sentence: sentence)
                .padding(.horizontal, Self.bodyInset)
                .padding(.vertical, GlassTokens.Space.s6)
                .frame(maxWidth: .infinity, alignment: .topLeading)
        case .signedOut:
            GlassNotice(tone: .ask, title: MonitorWords.signedOut) {
                Button(MonitorWindowView.openFirstRun) { OpenMonitor.request() }
            }
            .padding(.horizontal, Self.bodyInset)
            .padding(.vertical, GlassTokens.Space.s6)
            .frame(maxWidth: .infinity, alignment: .topLeading)
        case .open:
            switch item {
            case .compute:
                ComputeView(model: compute)
                    .padding(.horizontal, Self.bodyInset)
                    .padding(.vertical, GlassTokens.Space.s6)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
            default:
                GlassSettingsContent(navigation: navigation, section: item, onPrivateAI: onPrivateAI)
            }
        }
    }

    /// The modal's 0.5pt rules: under the header (`.tc-modal__header`,
    /// the modal rule), and between the list and the body (#1146's
    /// `rgba(255,255,255,0.1)`).
    @ViewBuilder
    private func rule(_ axis: Axis) -> some View {
        switch axis {
        case .horizontal:
            Rectangle().fill(GlassTokens.Color.rule.color).frame(height: 0.5).accessibilityHidden(true)
        case .vertical:
            Rectangle().fill(GlassColor.ink(Self.navRuleInk)).frame(width: 0.5).accessibilityHidden(true)
        }
    }

    /// The list's rule (#1146 `borderRight: 0.5px solid rgba(255,255,255,0.1)`).
    static let navRuleInk: Double = 0.1

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
