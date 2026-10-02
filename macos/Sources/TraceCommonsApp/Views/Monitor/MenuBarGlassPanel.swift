#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// The menu-bar panel in the glass system (R13 of #1173; spec, "Menu-bar
/// popover"): a `MenuBarExtra(.window)` panel drawn as a `GlassPopover`,
/// with a status row and the decisions-owed badge above the menu's own
/// content.
///
/// The content is `MenuBarContent`, unchanged: the same rows, the same
/// actions, the same words. This view adds only the frame, the status row
/// and the badge, and dresses the menu's buttons as menu rows. Debug-only,
/// in place of the shipping menu, until R15 (`TRACE_COMMONS_GLASS_MENU=1`).
struct MenuBarGlassPanel: View {
    @EnvironmentObject private var model: AppModel
    let navigation: MainWindowNavigation

    static let width: CGFloat = 320

    var body: some View {
        GlassPopover {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                statusRow
                Rectangle()
                    .fill(GlassTokens.Color.menuSeparator.color)
                    .frame(height: 1)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    MenuBarContent(navigation: navigation)
                }
                .buttonStyle(GlassMenuRowStyle())
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
            }
        }
        .frame(width: Self.width)
    }

    /// The status row: a dot and the core's own status line for the menu
    /// bar, with the badge counting decisions owed (never queue depth).
    private var statusRow: some View {
        let state = MenuBarStatus.state(
            decisionsOwed: model.decisionsOwed, unhealthy: model.health != nil,
            paused: model.status.paused, available: model.startup == .running)
        return HStack(spacing: GlassTokens.Space.s4) {
            GlassStatusDot(MenuBarPanelStatus.dot(state), ring: true)
            Text(MenuBarStatus.accessibilityLabel(
                decisionsOwed: model.decisionsOwed, unhealthy: model.health != nil,
                paused: model.status.paused, available: model.startup == .running))
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            if let badge = MenuBarPanelStatus.badge(model.decisionsOwed) {
                GlassBadge(count: badge)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// The status row's dot and badge for a menu-bar state. Pure, so tested.
enum MenuBarPanelStatus {
    /// Attention outranks everything, then decisions owed, then paused;
    /// idle and watching is on. The dot is always paired with the label.
    static func dot(_ state: MenuBarState) -> GlassStatus {
        switch state {
        case .attention: .outside
        case .count: .ask
        case .paused: .off
        case .idle: .on
        }
    }

    /// The badge: decisions owed when there are any. Zero and unknown draw
    /// none; the status line says which.
    static func badge(_ decisionsOwed: Int?) -> Int? {
        guard let decisionsOwed, decisionsOwed > 0 else { return nil }
        return decisionsOwed
    }
}
#endif
