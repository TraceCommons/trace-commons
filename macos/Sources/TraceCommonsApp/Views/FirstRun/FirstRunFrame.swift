import SwiftUI
import TCDesign
import TCShellCore

/// The step's primary action and the caption beside it (Ron's
/// `ScreenFooter`). Every string is the core's; the screen picks which.
struct FirstRunFooter {
    let title: String
    let isEnabled: Bool
    /// The caption on the footer's leading side, when the screen has one.
    let note: String?
    let action: () -> Void

    init(title: String, isEnabled: Bool, note: String? = nil, action: @escaping () -> Void) {
        self.title = title
        self.isEnabled = isEnabled
        self.note = note
        self.action = action
    }
}

extension FirstRunCopy.Frame {
    /// The tier's name, shown as the pane's eyebrow.
    func eyebrow(for tier: FirstRunTier) -> String {
        switch tier {
        case .quick: return quickSetup
        case .custom: return customSetup
        }
    }

    /// The progress labels for a tier, in `FirstRunNavigation`'s order: the
    /// order lives there, this only names each step.
    func steps(for tier: FirstRunTier) -> [String] {
        FirstRunNavigation.steps(for: tier).map(label(for:))
    }

    func label(for step: FirstRunStep) -> String {
        switch step {
        case .join: return stepJoin
        case .folders: return stepFolders
        case .tools: return stepTools
        case .rules: return stepRules
        case .uses: return stepUses
        }
    }
}

/// The frame's decisions, apart from the view so they can be tested.
enum FirstRunFrameLayout {
    /// The current step's index within its tier's steps.
    static func current(_ state: FirstRunState) -> Int {
        FirstRunNavigation.steps(for: state.tier).firstIndex(of: state.step) ?? 0
    }

    /// "Custom setup instead" sits on Quick's Folders only (#1030
    /// `tool-screens.tsx`): it is the one place Quick asks something Custom
    /// asks differently.
    static func offersCustomSetupInstead(_ state: FirstRunState) -> Bool {
        state.tier == .quick && state.step == .folders
    }

    /// The disabled Continue explains itself only on the tool screens, where
    /// the reason is an unanswered tool; elsewhere the reason differs.
    static func showsAnswerEveryTool(_ state: FirstRunState, footer: FirstRunFooter) -> Bool {
        guard !footer.isEnabled else { return false }
        return state.step == .folders || state.step == .tools
    }
}

/// Ron's first-run frame (#1030 `ftux-frame.tsx`) in glass: one pane with
/// the tier as eyebrow, the step progress, an optional Back, the runner's
/// failure as a notice, the screen, and its footer.
///
/// `notice` is the sentence a screen maps the runner's failure to; the
/// frame shows it and decides nothing about it.
struct FirstRunFrame<Content: View>: View {
    private let copy: FirstRunCopy
    @Binding private var state: FirstRunState
    private let onBack: (() -> Void)?
    private let notice: String?
    private let footer: FirstRunFooter
    private let content: Content

    init(
        copy: FirstRunCopy,
        state: Binding<FirstRunState>,
        onBack: (() -> Void)?,
        notice: String? = nil,
        footer: FirstRunFooter,
        @ViewBuilder content: () -> Content
    ) {
        self.copy = copy
        self._state = state
        self.onBack = onBack
        self.notice = notice
        self.footer = footer
        self.content = content()
    }

    var body: some View {
        GlassPane {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s8) {
                bar
                GlassStepProgress(labels: copy.frame.steps(for: state.tier), current: FirstRunFrameLayout.current(state))
                    .frame(maxWidth: .infinity)
                if let notice {
                    GlassNotice(tone: .outside) {
                        Text(notice)
                    }
                }
                content
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                footerRow
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(copy.frame.eyebrow(for: state.tier))
    }

    private var bar: some View {
        HStack(spacing: GlassTokens.Space.s6) {
            if let onBack {
                Button(copy.passkey.back, action: onBack)
                    .buttonStyle(GlassButtonStyle(.link))
            }
            Text(copy.frame.eyebrow(for: state.tier))
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textTertiary)
            Spacer(minLength: 0)
        }
    }

    private var footerRow: some View {
        HStack(spacing: GlassTokens.Space.s6) {
            if FirstRunFrameLayout.offersCustomSetupInstead(state) {
                Button(copy.frame.customSetupInstead) {
                    state = FirstRunNavigation.switchTier(state, to: .custom)
                }
                .buttonStyle(GlassButtonStyle(.link))
            } else if let note = footer.note {
                Text(note)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            Spacer(minLength: 0)
            Button(footer.title, action: footer.action)
                .buttonStyle(GlassButtonStyle(.primary))
                .disabled(!footer.isEnabled)
                .help(FirstRunFrameLayout.showsAnswerEveryTool(state, footer: footer) ? copy.frame.answerEveryTool : "")
        }
    }
}
