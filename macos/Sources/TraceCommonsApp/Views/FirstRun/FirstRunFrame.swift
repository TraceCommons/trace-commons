import SwiftUI
import TCDesign
import TCShellCore

/// The step's primary action and what sits beside it. Every string is the
/// core's; the screen picks which.
struct FirstRunFooter {
    let title: String
    let isEnabled: Bool
    /// A note about the action, centred above the action bar (Join's "no
    /// sharing" line).
    let note: String?
    /// The action is running: the button carries a spinner.
    let busy: Bool
    /// A small secondary button beside the spinning action that ends what it
    /// waits on, when the screen has one (the near.ai browser sign-in).
    let cancel: Cancel?
    /// A link left of the action that opens more of the step in place
    /// (Customize on Tools, Ron 2026-10-21).
    let link: Link?
    let action: () -> Void

    struct Cancel {
        let title: String
        let action: () -> Void
    }

    struct Link {
        let title: String
        let isEnabled: Bool
        let isOpen: Bool
        let action: () -> Void
    }

    init(
        title: String, isEnabled: Bool, note: String? = nil, busy: Bool = false, cancel: Cancel? = nil,
        link: Link? = nil, action: @escaping () -> Void
    ) {
        self.title = title
        self.isEnabled = isEnabled
        self.note = note
        self.busy = busy
        self.cancel = cancel
        self.link = link
        self.action = action
    }
}

extension FirstRunCopy {
    /// The step thread's name for a step. Welcome has none: it shows no
    /// thread.
    func threadLabel(for step: FirstRunStep) -> String? {
        switch step {
        case .welcome: return nil
        case .tools: return frame.stepTools
        case .uses: return frame.stepUses
        case .traces: return frame.stepTraces
        case .privateAI: return frame.stepPrivateAi
        case .join: return frame.stepJoin
        }
    }

    /// The thread's names, in `FirstRunNavigation`'s order: the order lives
    /// there, this only names each step.
    var threadLabels: [String] {
        FirstRunNavigation.threadSteps.compactMap(threadLabel(for:))
    }
}

/// A step's title under its graphic: the light half, then the bold half, in
/// one line, centred.
struct FirstRunTitle: View {
    let light: String
    let bold: String

    var body: some View {
        Text("\(Text(light).foregroundStyle(GlassColor.textSecondary))\(Text(bold).fontWeight(.bold))")
            .glassType(GlassTokens.TypeScale.display.weight(.regular))
            .foregroundStyle(GlassColor.textPrimary)
            .multilineTextAlignment(.center)
            .accessibilityAddTraits(.isHeader)
    }
}

/// The frame's decisions, apart from the view so they can be tested.
enum FirstRunFrameLayout {
    /// Every screen splits its free height equally three ways: above the
    /// graphic, between the graphic and the title, and below the card
    /// (Ron, 2026-10-21). None is ever smaller than this.
    static let minimumGap: CGFloat = 24
    /// Between a title and its card.
    static let titleGap: CGFloat = 20
    /// Where the free height starts: below the step thread, which the host
    /// draws over every step.
    static let threadBottom: CGFloat = 44
    /// Where an opened card's top rests (Customize), below the thread.
    static let openTop: CGFloat = 40
    /// The action bar's side and bottom margins.
    static let barInset: CGFloat = 40
    static let barBottom: CGFloat = 28

    /// Back, at the bar's left, on every step after Welcome. It goes to the
    /// previous step with every answer kept (`FirstRunNavigation.back`), and
    /// is withdrawn while a commit runs, so the runner never moves the step
    /// on from a step the person has already left.
    static func offersBack(_ state: FirstRunState, isCommitting: Bool = false) -> Bool {
        !isCommitting && FirstRunNavigation.steps.first != state.step
    }

    /// The notes above the action bar: the footer's, when it has one.
    static func actionNotes(footer: FirstRunFooter) -> [String] {
        [footer.note].compactMap { $0 }.filter { !$0.isEmpty }
    }
}

/// One step of the animated first run: its graphic, the title under it and
/// its card, spaced evenly in the free height, then the action bar -- Back
/// on its left after Welcome, the step's action on its right with any link
/// or Cancel beside it. The step thread and the scene are the host's.
///
/// `notice` is the sentence a screen maps the runner's failure to; the frame
/// shows it under the card and decides nothing about it. `open` (Customize)
/// fades the graphic and title and lets the card grow up from the bottom
/// edge it rests on.
struct FirstRunFrame<Graphic: View, Title: View, Content: View>: View {
    private let copy: FirstRunCopy
    @Binding private var state: FirstRunState
    private let isCommitting: Bool
    private let notice: String?
    private let footer: FirstRunFooter
    private let open: Bool
    private let graphic: Graphic
    private let title: Title
    private let content: Content
    @State private var restingGap: CGFloat = FirstRunFrameLayout.minimumGap
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(
        copy: FirstRunCopy,
        state: Binding<FirstRunState>,
        isCommitting: Bool = false,
        notice: String? = nil,
        footer: FirstRunFooter,
        open: Bool = false,
        @ViewBuilder graphic: () -> Graphic,
        @ViewBuilder title: () -> Title,
        @ViewBuilder content: () -> Content
    ) {
        self.copy = copy
        self._state = state
        self.isCommitting = isCommitting
        self.notice = notice
        self.footer = footer
        self.open = open
        self.graphic = graphic()
        self.title = title()
        self.content = content()
    }

    var body: some View {
        VStack(spacing: 0) {
            if open {
                Color.clear.frame(height: FirstRunFrameLayout.openTop)
            } else {
                Spacer(minLength: FirstRunFrameLayout.minimumGap)
                graphic
                    .transition(.opacity)
                Spacer(minLength: FirstRunFrameLayout.minimumGap)
                title
                    .padding(.horizontal, FirstRunFrameLayout.barInset)
                    .transition(.opacity)
                Color.clear.frame(height: FirstRunFrameLayout.titleGap)
            }
            content
                .frame(maxHeight: open ? .infinity : nil, alignment: .bottom)
            if let notice {
                // A failed step: the failed request's red line, unboxed.
                GlassAlert(notice)
                    .frame(maxWidth: 560)
                    .padding(.top, GlassTokens.Space.s6)
            }
            if open {
                Color.clear.frame(height: restingGap)
            } else {
                // Its height is the bottom gap an opened card keeps.
                Spacer(minLength: FirstRunFrameLayout.minimumGap)
                    .onGeometryChange(for: CGFloat.self, of: \.size.height) { restingGap = $0 }
            }
            ForEach(FirstRunFrameLayout.actionNotes(footer: footer), id: \.self) { note in
                Text(note)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .multilineTextAlignment(.center)
                    .padding(.bottom, GlassTokens.Space.s6)
            }
            actionBar
        }
        .padding(.top, FirstRunFrameLayout.threadBottom)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .animation(GlassMotion.spatial(reduceMotion, duration: 0.76), value: open)
        .accessibilityElement(children: .contain)
    }

    private var actionBar: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            if FirstRunFrameLayout.offersBack(state, isCommitting: isCommitting) {
                Button(copy.frame.back) {
                    state = FirstRunNavigation.back(state)
                }
                .buttonStyle(GlassButtonStyle(.glass, size: .stage))
            }
            Spacer(minLength: 0)
            if let link = footer.link {
                Button(action: link.action) {
                    HStack(spacing: GlassTokens.Space.s3) {
                        Text(link.title)
                        Image(systemName: "chevron.up")
                            .glassGlyph(9, weight: .semibold)
                            .rotationEffect(.degrees(link.isOpen ? 180 : 0))
                            .animation(GlassMotion.standard(reduceMotion), value: link.isOpen)
                            .accessibilityHidden(true)
                    }
                }
                .buttonStyle(GlassButtonStyle(.link, size: .stage))
                .disabled(!link.isEnabled)
                .accessibilityAddTraits(link.isOpen ? .isSelected : [])
            }
            if let cancel = footer.cancel {
                Button(cancel.title, action: cancel.action)
                    .buttonStyle(GlassButtonStyle(.secondary, size: .stage))
            }
            Button(action: footer.action) {
                HStack(spacing: GlassTokens.Space.s3) {
                    if footer.busy { GlassSpinner() }
                    Text(footer.title)
                }
            }
            .buttonStyle(GlassButtonStyle(.primary, size: .stage))
            .disabled(!footer.isEnabled)
        }
        .padding(.horizontal, FirstRunFrameLayout.barInset)
        .padding(.bottom, FirstRunFrameLayout.barBottom)
    }
}
