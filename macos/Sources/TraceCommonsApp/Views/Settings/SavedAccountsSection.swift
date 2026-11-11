import SwiftUI
import TCDesign
import TCShellCore

/// Saved model accounts: the Claude Code, Codex and OpenCode logins and
/// API keys the daemon keeps one isolated profile each for, with Add,
/// Launch, and each account's menu. They left the Private AI tab on
/// 2026-10-19 (#1490: they are not Private AI sessions) and had no other
/// home, so nothing could add, sign in to, rename or remove one; this
/// section is that home (owner, 2026-11-11).
///
/// Every word comes from the shared table (`managed/copy.rs`) through
/// `AppModel.managedText`; no raw state, auth value or daemon code is drawn.
struct SavedAccountsSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var adding = false
    @State private var launching = false
    @State private var renaming: ManagedAccount?
    @State private var removing: ManagedAccount?
    /// The account whose row menu is open.
    @State private var menuFor: String?

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if model.managedSnapshot == nil {
                connectingCard
            } else if let snapshot = model.managedSnapshot {
                accountsCard(snapshot)
            }
        }
        // Glass modals over the whole window, not stock sheets.
        .glassModal(isPresented: $adding) {
            ManagedAccountSheet(onClose: { adding = false }).environmentObject(model)
        }
        .glassModal(isPresented: $launching) {
            ManagedLaunchSheet(onClose: { launching = false }).environmentObject(model)
        }
        .glassModal(item: $renaming) { account in
            ManagedRenameSheet(account: account, onClose: { renaming = nil }).environmentObject(model)
        }
        .glassModal(item: $removing) { account in
            ManagedRemoveSheet(account: account, onClose: { removing = nil }).environmentObject(model)
        }
        .task { model.refreshManagedSessions() }
    }

    /// The list has not been read: connecting, with the card's re-read.
    private var connectingCard: some View {
        GlassIconCard(
            systemImage: Self.symbol, title: model.managedText("accounts_title"),
            subtitle: [model.managedText("connecting")],
            refresh: .init(model.managedText("refresh"), isDisabled: model.managedBusy) {
                model.refreshManagedSessions()
            },
            accessory: { EmptyView() }, content: { EmptyView() })
    }

    static let symbol = "person.crop.circle.badge.checkmark"

    /// The accounts card, laid out as it was on the Private AI tab before
    /// #1490: the re-read after the title, where a launch opens behind the
    /// info icon, Add and Launch on the right of the head, then a failure,
    /// a missing terminal and the accounts under it.
    private func accountsCard(_ snapshot: ManagedSnapshot) -> some View {
        let info = snapshot.capabilities.terminalDestination.map {
            model.managedText("terminal_scope").replacingOccurrences(of: "{destination}", with: $0)
        }
        return GlassIconCard(
            systemImage: Self.symbol, title: model.managedText("accounts_title"),
            subtitle: [model.managedText("description")], info: info,
            refresh: .init(model.managedText("accounts_refresh_accessibility"), isDisabled: model.managedBusy) {
                model.refreshManagedSessions()
            },
            accessory: { accountActions(snapshot) },
            content: { accountsBody(snapshot) })
    }

    /// Add, and Launch while there is a saved account whose tool is
    /// installed and a terminal to launch in: left out, not drawn
    /// disabled, when it could not be used (owner, 2026-10-10).
    private func accountActions(_ snapshot: ManagedSnapshot) -> some View {
        let add = Button { adding = true } label: {
            Text(model.managedText("add")).lineLimit(1)
        }
        .buttonStyle(GlassButtonStyle(.glass, small: true))
        .fixedSize()
        let launch = Button { launching = true } label: {
            Text(model.managedText("launch")).lineLimit(1)
        }
        .buttonStyle(GlassButtonStyle(.primary, small: true))
        .fixedSize()
        let addable = !snapshot.setupTools.isEmpty
        let launchable = Self.offersLaunch(snapshot)
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: GlassTokens.Space.s3) { if addable { add }; if launchable { launch } }
            VStack(alignment: .trailing, spacing: GlassTokens.Space.s3) { if addable { add }; if launchable { launch } }
        }
        .disabled(model.managedBusy)
    }

    /// Whether Launch is offered: a saved account whose tool is installed
    /// to launch with, and a terminal to launch in.
    static func offersLaunch(_ snapshot: ManagedSnapshot) -> Bool {
        snapshot.accounts.contains { snapshot.canLaunch(tool: $0.tool) } && snapshot.capabilities.terminalLaunch
    }

    private func accountsBody(_ snapshot: ManagedSnapshot) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            actionFailure
            if snapshot.capabilities.terminalDestination == nil {
                GlassNotice(tone: .ask, title: model.managedText("terminal_unavailable")) { EmptyView() }
            }
            if !snapshot.accounts.isEmpty {
                VStack(spacing: 0) {
                    ForEach(Array(snapshot.accounts.enumerated()), id: \.element.id) { index, account in
                        GlassTableRow(first: index == 0) { accountRow(account, snapshot: snapshot) }
                    }
                }
            }
        }
    }

    /// A failed request: its headline, then the reason when it is a
    /// different word.
    @ViewBuilder
    private var actionFailure: some View {
        if let key = model.managedErrorKey {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                GlassAlert(model.managedText("action_failed"))
                if key != "action_failed" {
                    // `{terminal}` is the chosen app the daemon names.
                    GlassAlert(ManagedSurface.fillTerminal(
                        model.managedText(key),
                        terminal: model.managedSnapshot?.capabilities.terminalDestination,
                        fallback: model.managedText("terminal")))
                        .textSelection(.enabled)
                }
            }
        }
    }

    private func accountRow(_ account: ManagedAccount, snapshot: ManagedSnapshot) -> some View {
        // A tool or sign-in value this shell does not know is left out,
        // never drawn raw.
        let meta = [
            ManagedWords.tool(account.tool, model: model),
            model.managedText(account.connection),
            ManagedSurface.authKey(account.authState).map(model.managedText),
        ].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
        return HStack(spacing: GlassTokens.Space.s4) {
            GlassToolTile(.tool(ManagedWords.glassTool(account.tool)))
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: GlassTokens.Space.s2) {
                    Text(account.label)
                        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                        .foregroundStyle(GlassColor.textPrimary)
                        .lineLimit(1)
                    if snapshot.isDefault(account) {
                        GlassTag(model.managedText("default"), tone: .accent)
                    }
                }
                if !snapshot.canLaunch(tool: account.tool) {
                    Text(ManagedSurface.toolUnavailable(account.tool, copy: model.managedText))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let availability = snapshot.tools?.first(where: { $0.tool == account.tool }) {
                        ManagedInstallLink(tool: availability, copy: model.managedText)
                    }
                }
                Text(meta)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
            }
            .accessibilityElement(children: .combine)
            Spacer(minLength: GlassTokens.Space.s4)
            accountMenu(account, snapshot: snapshot)
        }
    }

    private func accountMenu(_ account: ManagedAccount, snapshot: ManagedSnapshot) -> some View {
        let held = snapshot.isHeld(account)
        let open = Binding(get: { menuFor == account.id }, set: { if !$0 { menuFor = nil } })
        return GlassKebab(model.managedText("manage"), open: menuFor == account.id) { menuFor = account.id }
            .disabled(model.managedBusy)
            .popover(isPresented: open, arrowEdge: .bottom) {
                GlassMenu(onDismiss: { menuFor = nil }) {
                    GlassMenuItem(model.managedText("use_default")) {
                        menuFor = nil
                        let generation = snapshot.generations[account.tool] ?? 0
                        model.managedAction("managed_select", params: ["selection": ["tool": account.tool, "connection": account.connection, "account_id": account.id, "generation": generation], "expected_generation": generation])
                    }
                    GlassMenuItem(model.managedText("rename_short")) {
                        menuFor = nil
                        DispatchQueue.main.async { renaming = account }
                    }
                    // Hidden, not disabled, while the tool is missing (Ron,
                    // 2026-10-15): the row says so and offers Install.
                    if account.connection == "subscription" && snapshot.canLaunch(tool: account.tool) {
                        GlassMenuItem(model.managedText("reconnect")) {
                            menuFor = nil
                            model.managedAction("managed_account_reconnect", params: ["account_id": account.id], openTerminal: true)
                        }
                        .disabled(held || !snapshot.capabilities.terminalLaunch)
                    }
                    GlassMenuItem(model.managedText("verify")) {
                        menuFor = nil
                        model.managedAction("managed_account_verify", params: ["account_id": account.id])
                    }
                    GlassMenuSeparator()
                    GlassMenuItem(model.managedText("remove")) {
                        menuFor = nil
                        DispatchQueue.main.async { removing = account }
                    }
                    .disabled(held)
                }
                .disabled(model.managedBusy)
            }
    }
}
