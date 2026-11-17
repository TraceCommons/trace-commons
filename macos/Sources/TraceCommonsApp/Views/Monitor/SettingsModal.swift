import SwiftUI
import TCDesign
import TCShellCore

/// Settings, drawn as the body of its own window (owner, 2026-10-08:
/// `SettingsWindowView`), no longer as #1146's modal over the Monitor. The
/// name is kept from the modal it was (#1241 Task 10).
///
/// One pane on the opaque base, so nothing behind it reads through. Its
/// header is the core's title and a Watching or Paused chip
/// from the daemon's status; the window has its own close button. Its body
/// is two columns: the section list, which scrolls the body to a section,
/// and one scrolling body holding every section in order, Compute included.
/// Escape closes the window. Every word is the core's or a section's own
/// heading (`ShellWordingTests`).
struct SettingsModal: View {
    let request: SettingsRequest
    let navigation: MainWindowNavigation
    /// Whether watching is paused, from the daemon's status; nil while the
    /// status is unknown, and then no chip is drawn (Ron's `core.data`).
    let paused: Bool?
    /// Closes the window.
    let onClose: () -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(ComputeModel.self) private var compute

    /// Every section, in order, in one body.
    static let sections: [SettingsSection] = SettingsSection.allCases

    /// The sections the list can name, in order (the Settings IA review's
    /// groups, owner 2026-11-04).
    static let listed: [SettingsSection] = SettingsSection.listed

    /// Whether anything in Payouts applies; its row is listed only then (D2).
    @State private var payoutsShown = false

    /// The rows the list draws now: every listed section but Payouts while
    /// nothing in it applies.
    static func rows(payoutsShown: Bool) -> [SettingsSection] {
        listed.filter { $0 != .payouts || payoutsShown }
    }

    var body: some View {
        // The window's own surface, edge to edge on the opaque pane base:
        // no pane inside it, whose rounded edge read as a window framed
        // within the window (owner, 2026-10-09).
        VStack(spacing: 0) {
            header
                .overlay(alignment: .bottom) { rule(.horizontal) }
            columns
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(GlassTokens.Color.settingsGround.color)
        .environment(\.glassPaneIsContent, true)
        .focusSection()
        .onExitCommand(perform: onClose)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(words?.settingsTitle ?? "")
    }

    private var words: MonitorScreensCopy? { MonitorWords.table }

    private var header: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s6) {
            Text(words?.settingsTitle ?? "")
                .glassType(GlassTokens.TypeScale.title.weight(.bold))
                .foregroundStyle(GlassColor.textPrimary)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 0)
            if let chip = Self.chip(paused: paused, words: words) {
                GlassChip(chip.word, status: chip.status)
            }
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
                        let rows = Self.rows(payoutsShown: payoutsShown)
                        ForEach(rows) { item in
                            // A hairline between groups, and no group
                            // headings (D17).
                            if SettingsSection.startsGroup(item, in: rows) {
                                GlassHairline(GlassColor.ink(Self.navRuleInk))
                                    .padding(.vertical, GlassTokens.Space.s3)
                                    .accessibilityHidden(true)
                            }
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
                    // Not lazy: `scrollTo` lands by the height of what is
                    // drawn, and a lazy stack guesses the sections it has
                    // not drawn, so a jump to a far section stopped short
                    // in an earlier one. Each section keeps its own lists
                    // lazy.
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Self.sections) { item in
                            VStack(alignment: .leading, spacing: 0) {
                                if item != .payouts || payoutsShown,
                                   let name = item.navName(words?.settingsNav) {
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
        // General is the app's own (the appearance): it never waits on the
        // core or on first run.
        if item == .general {
            GlassSettingsContent(navigation: navigation, section: item)
        } else if item == .payouts {
            // Payouts says nothing until it applies: no onboarding notice
            // and no rule over an empty body (D2).
            if gate(for: item) == .open {
                GlassSettingsContent(navigation: navigation, section: item,
                                     onPayoutsPresence: { payoutsShown = $0 })
            }
        } else {
            gatedSection(item)
        }
    }

    private func gate(for item: SettingsSection) -> MonitorGate {
        MonitorGate.of(
            startup: model.startup, onboardingKnown: model.onboardingKnown,
            requiresOnboarding: model.requiresOnboarding
        ).forSettings(availableBeforeOnboarding: item.availableBeforeOnboarding)
    }

    @ViewBuilder
    private func gatedSection(_ item: SettingsSection) -> some View {
        switch gate(for: item) {
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
                GlassSettingsContent(navigation: navigation, section: item)
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
            GlassHairline(GlassTokens.Color.rule.color)
        case .vertical:
            GlassHairline(GlassColor.ink(Self.navRuleInk), axis: .vertical)
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
