import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

// The re-engagement nudges on the glass system: the Traces and History
// cards, the Traces order control and idle filter, and a session row's
// tags. Every sentence is the daemon's (`status.nudge.text`) or the core's
// fixed nudge words (`tc_nudge_copy_json`, `tc_nudge_entry_tags_json`); this
// file lays them out and composes none. Built from existing glass
// components for the designer's review.

/// A nudge card: the daemon's title, its body when it has one, and its
/// buttons in the daemon's order. "Not now" is the lesser action, drawn as
/// a link after the action, as every card's decline is. A refused request
/// is said under the buttons, in the core's line for it.
struct NudgeGlassCard: View {
    let card: NudgeSurface.Card
    let busy: Bool
    let refusal: String?
    let act: (NudgeSurface.Intent) -> Void

    /// The button style for one action.
    static func style(_ intent: NudgeSurface.Intent) -> GlassButtonKind {
        if case .notNow = intent { return .link }
        return .glass
    }

    var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                Text(card.title)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                if let body = card.body {
                    Text(body)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if !card.actions.isEmpty {
                    HStack(spacing: GlassTokens.Space.s4) {
                        ForEach(Array(card.actions.enumerated()), id: \.offset) { _, action in
                            Button(action.label) { act(action.intent) }
                                .buttonStyle(GlassButtonStyle(Self.style(action.intent)))
                                .disabled(busy)
                        }
                    }
                }
                if let refusal { GlassAlert(refusal) }
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// Above the Traces tree: the idle filter while it is on, with its way out,
/// and the order control. Each is drawn only with the core's words for it.
/// Suggested first is selected until another order is chosen.
struct TracesListControls: View {
    let store: TracesStore

    var body: some View {
        let copy = store.nudgeCopy
        HStack(spacing: GlassTokens.Space.s4) {
            if store.idleOnly, let label = copy?[.listFilterIdle], let clear = copy?[.listFilterClear] {
                GlassTag(label, tone: .accent)
                Button(clear) { Task { await store.showIdleOnly(false) } }
                    .buttonStyle(GlassButtonStyle(.link))
                    .fixedSize()
            }
            Spacer(minLength: 0)
            if let suggested = copy?[.listOrderSuggested], let queue = copy?[.listOrderQueue],
               let label = Self.orderLabel(copy)
            {
                GlassSegmentedTabs(
                    label,
                    selection: Binding(
                        get: { store.order },
                        set: { chosen in
                            guard let chosen, chosen != store.order else { return }
                            Task { await store.setOrder(chosen) }
                        }),
                    segments: [
                        GlassSegment(suggested, value: Optional(DaemonData.PendingOrder.suggested)),
                        GlassSegment(queue, value: Optional(DaemonData.PendingOrder.queue)),
                    ])
                    .fixedSize()
            }
        }
    }

    /// The order control's accessible name, the core's; nil without it,
    /// and the control is then not drawn.
    static func orderLabel(_ copy: NudgeCopy?) -> String? {
        copy?[.listOrderLabel]
    }
}

/// A session row's tags, under the row: "Fits a mission" and the estimate
/// tier as tags, then the estimate band with its explainer behind an info
/// button. Drawn only from the core's answer for the row; nothing when it
/// has nothing to say.
struct NudgeRowTags: View {
    let tags: NudgeEntryTags
    @State private var explaining = false

    var body: some View {
        if !tags.isEmpty {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                if let fit = tags.missionFit { GlassTag(fit, tone: .accent) }
                if let tier = tags.estimateTier { GlassTag(tier) }
                if let band = tags.estimateBand {
                    Text(band)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let explainer = tags.estimateExplainer {
                        Button { explaining.toggle() } label: {
                            Image(systemName: "info.circle").glassGlyph(11)
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(GlassColor.textTertiary)
                        .accessibilityLabel(band)
                        .accessibilityHint(explainer)
                        .help(explainer)
                        .popover(isPresented: $explaining, arrowEdge: .bottom) {
                            Text(explainer)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textSecondary)
                                .fixedSize(horizontal: false, vertical: true)
                                .frame(width: 260, alignment: .leading)
                                .padding(GlassTokens.Space.s5)
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, TracesTreeView.noteInset)
            .padding(.bottom, GlassTokens.Space.s2)
        }
    }
}
