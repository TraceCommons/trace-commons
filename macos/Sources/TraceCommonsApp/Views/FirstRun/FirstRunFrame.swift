import SwiftUI
import TCDesign
import TCShellCore

/// The step's primary action and the caption beside it (Ron's
/// `ScreenFooter`). Every string is the core's; the screen picks which.
struct FirstRunFooter {
    let title: String
    let isEnabled: Bool
    /// A note about the action in its current state (Join's "what Skip
    /// means"), when the screen has one. It sits above the action bar, after
    /// the frame's `actionNote` (owner ruling, 2026-10-08).
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
    /// The page content's extra side margin, inside the pane: the header,
    /// the cards, the pinned row and the action notes, but not the step
    /// progress above or the button row below (owner ruling, 2026-10-08).
    static let contentInset: CGFloat = GlassTokens.Space.s9

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

    /// The disabled Continue explains itself only on the tool screens, where
    /// the reason is an unanswered tool; elsewhere the reason differs.
    /// The notes above the action bar, top to bottom: the screen's standing
    /// note about its action, then the footer's note for the action's
    /// current state (owner ruling, 2026-10-08: text about taking the
    /// primary action sits directly above the action bar, not in the
    /// scrolling content).
    static func actionNotes(actionNote: String?, footer: FirstRunFooter) -> [String] {
        [actionNote, footer.note].compactMap { $0 }.filter { !$0.isEmpty }
    }

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
struct FirstRunFrame<Header: View, Content: View, Pinned: View>: View {
    private let copy: FirstRunCopy
    @Binding private var state: FirstRunState
    private let isCommitting: Bool
    private let notice: String?
    private let actionNote: String?
    private let footer: FirstRunFooter
    private let header: Header
    private let content: Content
    private let pinned: Pinned

    /// `pinned` sits under the scrolling cards and above the footer, always
    /// in view (the tool screens' add tile, owner 2026-10-08). `actionNote`
    /// is the screen's text about taking its primary action, drawn directly
    /// above the action bar (owner ruling, 2026-10-08).
    init(
        copy: FirstRunCopy,
        state: Binding<FirstRunState>,
        isCommitting: Bool = false,
        notice: String? = nil,
        actionNote: String? = nil,
        footer: FirstRunFooter,
        @ViewBuilder header: () -> Header,
        @ViewBuilder content: () -> Content,
        @ViewBuilder pinned: () -> Pinned
    ) {
        self.copy = copy
        self._state = state
        self.isCommitting = isCommitting
        self.notice = notice
        self.actionNote = actionNote
        self.footer = footer
        self.header = header()
        self.content = content()
        self.pinned = pinned()
    }

    var body: some View {
        GlassPane {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s8) {
                GlassStepProgress(labels: copy.frame.steps(for: state.tier), current: FirstRunFrameLayout.current(state))
                    .frame(maxWidth: .infinity)
                header
                    .padding(.horizontal, FirstRunFrameLayout.contentInset)
                ScrollView {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                        content
                        // A failed step: the failed request's red line,
                        // unboxed (Ron, 2026-10-09).
                        if let notice {
                            GlassAlert(notice)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                    .padding(.horizontal, FirstRunFrameLayout.contentInset)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                pinned
                    .padding(.horizontal, FirstRunFrameLayout.contentInset)
                GlassActionBar(
                    notes: FirstRunFrameLayout.actionNotes(actionNote: actionNote, footer: footer),
                    noteInset: FirstRunFrameLayout.contentInset
                ) {
                    footerRow
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(copy.frame.eyebrow(for: state.tier))
    }

    // No tier tag on either setup (owner, 2026-10-08): the pane's
    // accessible name still names the tier.

    private var footerRow: some View {
        HStack(spacing: GlassTokens.Space.s6) {
            // The secondary buttons are TCDesign's neutral glass pill at the
            // action bar's size: every button in the bar is Continue's size
            // (owner ruling, 2026-10-08).
            if FirstRunFrameLayout.offersBack(state, isCommitting: isCommitting) {
                Button(copy.frame.back) {
                    state = FirstRunNavigation.back(state)
                }
                .buttonStyle(GlassButtonStyle(.glass, size: .bar))
            }
            Spacer(minLength: 0)
            if FirstRunFrameLayout.offersCustomSetupInstead(state, isCommitting: isCommitting) {
                Button(copy.frame.customSetupInstead) {
                    state = FirstRunNavigation.switchTier(state, to: .custom)
                }
                .buttonStyle(GlassButtonStyle(.glass, size: .bar))
            }
            if let cancel = footer.cancel {
                Button(cancel.title, action: cancel.action)
                    .buttonStyle(GlassButtonStyle(.secondary, size: .bar))
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

extension FirstRunFrame where Pinned == EmptyView {
    /// A step with nothing pinned under its cards.
    init(
        copy: FirstRunCopy,
        state: Binding<FirstRunState>,
        isCommitting: Bool = false,
        notice: String? = nil,
        actionNote: String? = nil,
        footer: FirstRunFooter,
        @ViewBuilder header: () -> Header,
        @ViewBuilder content: () -> Content
    ) {
        self.init(
            copy: copy, state: state, isCommitting: isCommitting, notice: notice, actionNote: actionNote,
            footer: footer, header: header, content: content, pinned: { EmptyView() })
    }
}
