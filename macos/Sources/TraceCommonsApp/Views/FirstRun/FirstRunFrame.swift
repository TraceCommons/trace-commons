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
    /// The action is running: the button carries a spinner (Ron's Start
    /// sharing).
    let busy: Bool
    /// A small secondary button beside the spinning action that ends what it
    /// waits on, when the screen has one (the near.ai browser sign-in).
    let cancel: Cancel?
    let action: () -> Void

    struct Cancel {
        let title: String
        let action: () -> Void
    }

    init(
        title: String, isEnabled: Bool, note: String? = nil, busy: Bool = false, cancel: Cancel? = nil,
        action: @escaping () -> Void
    ) {
        self.title = title
        self.isEnabled = isEnabled
        self.note = note
        self.busy = busy
        self.cancel = cancel
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

/// Ron's `ScreenTitle` (`ftux-frame.tsx`): the light half, then the bold
/// half, in one style and one weight on every screen.
struct FirstRunTitle: View {
    let light: String
    let bold: String

    var body: some View {
        Text("\(Text(light))\(Text(bold).fontWeight(.bold))")
            .glassType(GlassTokens.TypeScale.display.weight(.regular))
            .foregroundStyle(GlassColor.textPrimary)
            .accessibilityAddTraits(.isHeader)
    }
}

/// The frame's decisions, apart from the view so they can be tested.
enum FirstRunFrameLayout {
    /// The current step's index within its tier's steps.
    static func current(_ state: FirstRunState) -> Int {
        FirstRunNavigation.steps(for: state.tier).firstIndex(of: state.step) ?? 0
    }

    /// "Customize" (switch to Custom setup) sits on Quick's Folders only
    /// (#1030 `tool-screens.tsx`): it is the one place Quick asks something
    /// Custom asks differently. It is a secondary button in the footer, just
    /// left of Continue (owner, 2026-10-08). It is withdrawn while a commit
    /// runs: the runner moves the step on from wherever the state is when
    /// its calls finish, so a switch mid-commit would skip Custom's Tools.
    static func offersCustomSetupInstead(_ state: FirstRunState, isCommitting: Bool = false) -> Bool {
        !isCommitting && state.tier == .quick && state.step == .folders
    }

    /// Back, at the footer's left, on every step after Join in either tier
    /// (owner, 2026-10-08, reversing Ron's review of #1235, item 9): the
    /// frame draws it, so every step that is not the first has it, and Join
    /// never does. It goes to the previous step with every answer kept
    /// (`FirstRunNavigation.back`). Like the tier switch it is withdrawn
    /// while a commit runs, so the runner never moves the step on from a
    /// step the person has already left.
    static func offersBack(_ state: FirstRunState, isCommitting: Bool = false) -> Bool {
        !isCommitting && current(state) > 0
    }

    /// The tier tag in the pane's bar is Custom setup's only: Quick setup
    /// shows none (owner, 2026-10-08), so the bar is left out there. The
    /// pane's accessible name still names the tier on both.
    static func showsTierTag(_ state: FirstRunState) -> Bool {
        state.tier == .custom
    }

    /// The disabled Continue explains itself only on the tool screens, where
    /// the reason is an unanswered tool; elsewhere the reason differs.
    static func showsAnswerEveryTool(_ state: FirstRunState, footer: FirstRunFooter) -> Bool {
        guard !footer.isEnabled else { return false }
        return state.step == .folders || state.step == .tools
    }
}

/// Ron's first-run frame (#1030 `ftux-frame.tsx`) in glass: one pane with
/// the tier on the right of its bar, the step progress, the screen's fixed
/// header (its title), the cards scrolling beneath it with the runner's
/// failure after them as a notice, and the footer: Back on its left after
/// Join, the step's action on its right (owner, 2026-10-08).
///
/// `notice` is the sentence a screen maps the runner's failure to; the
/// frame shows it and decides nothing about it. `isCommitting` withdraws
/// Back and the tier switch while the runner's calls are in flight.
struct FirstRunFrame<Header: View, Content: View>: View {
    private let copy: FirstRunCopy
    @Binding private var state: FirstRunState
    private let isCommitting: Bool
    private let notice: String?
    private let footer: FirstRunFooter
    private let header: Header
    private let content: Content

    init(
        copy: FirstRunCopy,
        state: Binding<FirstRunState>,
        isCommitting: Bool = false,
        notice: String? = nil,
        footer: FirstRunFooter,
        @ViewBuilder header: () -> Header,
        @ViewBuilder content: () -> Content
    ) {
        self.copy = copy
        self._state = state
        self.isCommitting = isCommitting
        self.notice = notice
        self.footer = footer
        self.header = header()
        self.content = content()
    }

    var body: some View {
        GlassPane {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s8) {
                if FirstRunFrameLayout.showsTierTag(state) {
                    bar
                }
                GlassStepProgress(labels: copy.frame.steps(for: state.tier), current: FirstRunFrameLayout.current(state))
                    .frame(maxWidth: .infinity)
                header
                ScrollView {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                        content
                        if let notice {
                            GlassNotice(tone: .outside) {
                                Text(notice)
                                    .fixedSize(horizontal: false, vertical: true)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                footerRow
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(copy.frame.eyebrow(for: state.tier))
    }

    /// Ron's `ftux-pane__bar`: the tier's name on the right, on Custom
    /// setup only (`FirstRunFrameLayout.showsTierTag`).
    private var bar: some View {
        HStack(spacing: GlassTokens.Space.s6) {
            Spacer(minLength: 0)
            Text(copy.frame.eyebrow(for: state.tier))
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textTertiary)
        }
    }

    private var footerRow: some View {
        HStack(spacing: GlassTokens.Space.s6) {
            // The secondary buttons are TCDesign's neutral glass pill, the
            // folder picker's style.
            if FirstRunFrameLayout.offersBack(state, isCommitting: isCommitting) {
                Button(copy.frame.back) {
                    state = FirstRunNavigation.back(state)
                }
                .buttonStyle(GlassButtonStyle(.glass))
            }
            if let note = footer.note {
                Text(note)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            Spacer(minLength: 0)
            if FirstRunFrameLayout.offersCustomSetupInstead(state, isCommitting: isCommitting) {
                Button(copy.frame.customSetupInstead) {
                    state = FirstRunNavigation.switchTier(state, to: .custom)
                }
                .buttonStyle(GlassButtonStyle(.glass))
            }
            if let cancel = footer.cancel {
                Button(cancel.title, action: cancel.action)
                    .buttonStyle(GlassButtonStyle(.secondary))
            }
            Button(action: footer.action) {
                HStack(spacing: GlassTokens.Space.s3) {
                    if footer.busy { GlassSpinner() }
                    Text(footer.title)
                }
            }
                .buttonStyle(GlassButtonStyle(.primary))
                .disabled(!footer.isEnabled)
                .help(FirstRunFrameLayout.showsAnswerEveryTool(state, footer: footer) ? copy.frame.answerEveryTool : "")
        }
    }
}
