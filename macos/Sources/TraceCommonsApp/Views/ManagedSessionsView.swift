import SwiftUI
import AppKit
import TCDesign
import TCShellCore

/// Saved model accounts and managed sessions, in the glass design (#1146):
/// the NEAR AI sign-in card, then accounts, then sessions. Drawn in the
/// Monitor's Inference tab and on the legacy model-calls screen.
///
/// Every word comes from the shared table (`managed/copy.rs`) through
/// `AppModel.managedText`; no raw state, auth value or daemon code is drawn.
struct ManagedSessionsSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var adding = false
    @State private var launching = false
    @State private var renaming: ManagedAccount?
    @State private var removing: ManagedAccount?
    /// The tool whose Start is explaining the folder choice (#1408).
    @State private var explaining: ManagedFolderChoice?
    /// The session whose stop is being confirmed.
    @State private var stopping: ManagedSession?
    /// The account whose row menu is open.
    @State private var menuFor: String?
    @Environment(\.openURL) private var openURL

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let snapshot = model.managedSnapshot {
                signInCard(snapshot)
            }
            accountsCard
            if let snapshot = model.managedSnapshot {
                sessionsCard(snapshot)
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
        .glassModal(item: $explaining) { choice in
            ManagedFolderExplainerSheet(
                tool: choice.tool, onCancel: { explaining = nil },
                onChoose: {
                    explaining = nil
                    // The picker opens once the sheet is gone.
                    DispatchQueue.main.async { chooseAndStart(choice.tool) }
                }
            ).environmentObject(model)
        }
        .glassModal(item: $stopping) { session in
            ManagedStopSheet(session: session, onClose: { stopping = nil }).environmentObject(model)
        }
        // Kept: `managed_changed` is published only when a request changes
        // the session revision, so an expiry or a reconciled exit is seen
        // on the next read.
        .task {
            while !Task.isCancelled {
                model.refreshManagedSessions()
                try? await Task.sleep(for: .seconds(10))
            }
        }
    }

    // MARK: NEAR AI sign-in

    /// Sessions on the NEAR AI sign-in, first (owner, 2026-10-12): with a
    /// sign-in, Start Claude Code and Start Codex, each a folder choice and
    /// a launch, with no saved account; without one, the line that asks for
    /// it and the button that begins it. Nothing here is saved: the entries
    /// come and go with the sign-in.
    @ViewBuilder
    private func signInCard(_ snapshot: ManagedSnapshot) -> some View {
        let signedIn = snapshot.nearAiSignedIn
        GlassIconCard(
            systemImage: "sparkles",
            title: model.managedText("near_ai"),
            subtitle: [signedIn
                ? ManagedSurface.signInScope(
                    model.managedText("sign_in_scope"),
                    destination: snapshot.capabilities.terminalDestination,
                    fallback: model.managedText("terminal"))
                : model.managedText("sign_in_needed")],
            trailing: { AnyView(signInActions(snapshot)) })
    }

    /// The Start buttons, side by side when they fit and stacked when not;
    /// left out while there is no terminal to start in, which the accounts
    /// card says. Signed out, the sign-in button.
    @ViewBuilder
    private func signInActions(_ snapshot: ManagedSnapshot) -> some View {
        if snapshot.nearAiSignedIn {
            if snapshot.capabilities.terminalLaunch {
                let buttons = ForEach(ManagedSurface.signInTools.filter { snapshot.signInEntry($0) != nil && snapshot.canLaunch(tool: $0) }, id: \.self) { tool in
                    Button { start(tool) } label: {
                        Text(ManagedSurface.startKey(tool).map(model.managedText) ?? "").lineLimit(1)
                    }
                    .buttonStyle(GlassButtonStyle(.primary, small: true))
                    .fixedSize()
                }
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: GlassTokens.Space.s3) { buttons }
                    VStack(alignment: .trailing, spacing: GlassTokens.Space.s3) { buttons }
                }
                .disabled(model.managedBusy)
            }
        } else {
            Button { signIn() } label: {
                Text(model.managedText("sign_in_near_ai")).lineLimit(1)
            }
            .buttonStyle(GlassButtonStyle(.primary, small: true))
            .fixedSize()
            .accessibilityLabel(model.managedText("sign_in_near_ai_accessibility"))
            .disabled(model.credentialBusy)
        }
    }

    /// Until a session has started on this computer, a sheet that says why
    /// a folder is asked for comes first (#1408); then the picker.
    private func start(_ tool: String) {
        guard !model.managedBusy else { return }
        if model.managedSnapshot?.explainsFolderChoice == true {
            explaining = ManagedFolderChoice(tool: tool)
        } else {
            chooseAndStart(tool)
        }
    }

    /// The folder chooser the launch sheet uses, then the launch.
    private func chooseAndStart(_ tool: String) {
        guard !model.managedBusy, let path = FolderPanel.choose(),
              let params = model.managedSnapshot?.signInLaunchParams(tool: tool, cwd: path, requestID: UUID().uuidString)
        else { return }
        model.managedAction("managed_launch_prepare", params: params, openTerminal: true)
    }

    /// The NEAR AI sign-in, begun as Settings > Private AI begins it.
    private func signIn() {
        Task {
            guard let url = await model.startNearAiCredential(provider: "near") else { return }
            let accepted = await withCheckedContinuation { continuation in
                openURL(url) { continuation.resume(returning: $0) }
            }
            // A browser that never opened leaves a ceremony nobody can
            // finish, so it is stopped rather than left to time out.
            if !accepted { model.cancelNearAiCredential() }
        }
    }

    // MARK: Accounts

    /// The accounts card under its icon (owner, 2026-10-10), laid out as
    /// every icon card: the re-read icon after the title, where a launch
    /// opens behind the info icon after the description, and Add and
    /// Launch on the right of the head (Retry there while the list has not
    /// been read). A failure, a missing terminal and the accounts sit under
    /// the head.
    @ViewBuilder
    private var accountsCard: some View {
        let snapshot = model.managedSnapshot
        let info = snapshot?.capabilities.terminalDestination.map {
            model.managedText("terminal_scope").replacingOccurrences(of: "{destination}", with: $0)
        }
        let refresh: GlassCardRefresh? = snapshot == nil ? nil : .init(
            model.managedText("accounts_refresh_accessibility"),
            isDisabled: model.managedBusy, action: { model.refreshManagedSessions() })
        // Nothing under the head: a head-only card, so its actions centre
        // on the whole card rather than on a head above an empty body.
        if Self.accountsBodyEmpty(snapshot, failing: model.managedErrorKey != nil) {
            GlassIconCard(
                systemImage: "person.crop.circle.badge.checkmark",
                title: model.managedText("accounts_title"),
                subtitle: [model.managedText("description")], info: info, refresh: refresh,
                trailing: { AnyView(accountsAccessory(snapshot)) })
        } else {
            GlassIconCard(
                systemImage: "person.crop.circle.badge.checkmark",
                title: model.managedText("accounts_title"),
                subtitle: [model.managedText("description")], info: info, refresh: refresh,
                accessory: { AnyView(accountsAccessory(snapshot)) },
                content: { AnyView(accountsBody(snapshot)) })
        }
    }

    /// Whether the accounts card has nothing to draw under its head: the
    /// list was read, nothing failed, there is a terminal, and no account.
    static func accountsBodyEmpty(_ snapshot: ManagedSnapshot?, failing: Bool) -> Bool {
        guard let snapshot else { return false }
        return !(snapshot.tools != nil && snapshot.setupTools.isEmpty) && !failing && snapshot.capabilities.terminalDestination != nil && snapshot.accounts.isEmpty
    }

    /// Add and Launch once the list is read; Retry until then.
    @ViewBuilder
    private func accountsAccessory(_ snapshot: ManagedSnapshot?) -> some View {
        if let snapshot {
            accountActions(snapshot)
        } else {
            Button { model.refreshManagedSessions() } label: {
                Label(model.managedText("retry"), systemImage: "arrow.clockwise").lineLimit(1)
            }
            .buttonStyle(GlassButtonStyle(.glass, small: true))
            .fixedSize()
        }
    }

    /// Under the head: connecting, a failure, a missing terminal, and the
    /// accounts.
    private func accountsBody(_ snapshot: ManagedSnapshot?) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            if snapshot == nil {
                Text(model.managedText("connecting"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            actionFailure
            if let snapshot {
                if snapshot.needsToolInstallation {
                    ManagedToolInstallation(tools: snapshot.tools ?? [], copy: model.managedText, retry: model.refreshManagedSessions)
                }
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
    }

    /// A failed request: the failed request's red lines, unboxed, under
    /// the account actions (Ron, 2026-10-09). The headline, then the
    /// reason when it is a different word.
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
                if key == "unsupported_version" {
                    ForEach(model.managedSnapshot?.updateRecoveryTools(attemptedTool: model.managedErrorTool) ?? []) { availability in
                        ManagedInstallLink(tool: availability, copy: model.managedText, labelKey: "update", includeToolName: true)
                    }
                }
            }
        }
    }

    /// Add and Launch, each on one line at its own width: side by side when
    /// they fit, stacked when the pane is too narrow, never wrapped. Launch
    /// is left out while it could not be used -- no saved account, or no
    /// terminal to launch in (owner, 2026-10-10) -- rather than drawn
    /// disabled.
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
        let launchable = Self.offersLaunch(snapshot)
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: GlassTokens.Space.s3) { if !snapshot.setupTools.isEmpty { add }; if launchable { launch } }
            VStack(alignment: .trailing, spacing: GlassTokens.Space.s3) { if !snapshot.setupTools.isEmpty { add }; if launchable { launch } }
        }
        .disabled(model.managedBusy)
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

    // MARK: Sessions

    /// The sessions card under its icon (owner, 2026-10-10): with none, the
    /// empty line is the head's subtitle and the card is one row; otherwise
    /// the sessions are listed under the head.
    @ViewBuilder
    private func sessionsCard(_ snapshot: ManagedSnapshot) -> some View {
        if snapshot.sessions.isEmpty {
            GlassIconCard(
                systemImage: "terminal", title: model.managedText("title"),
                subtitle: [model.managedText("empty")], trailing: { EmptyView() })
        } else {
            GlassIconCard(systemImage: "terminal", title: model.managedText("title"), content: {
                VStack(spacing: 0) {
                    ForEach(Array(snapshot.sessions.enumerated()), id: \.element.id) { index, session in
                        GlassTableRow(first: index == 0) { sessionRow(session) }
                    }
                }
            })
        }
    }

    private func sessionRow(_ session: ManagedSession) -> some View {
        var meta = [
            ManagedWords.tool(session.tool, model: model),
            session.accountLabel,
            model.managedText(session.connection),
        ].compactMap { $0 }.filter { !$0.isEmpty }
        if let selectedModel = session.model { meta.append(selectedModel) }
        if let collection = session.collection, ["off", "pending", "collected", "failed"].contains(collection) {
            meta.append(model.managedText("collection_" + collection))
        }
        if let code = session.exitCode {
            meta.append(model.managedText("exit_code").replacingOccurrences(of: "{code}", with: String(code)))
        }
        let row = HStack(spacing: GlassTokens.Space.s4) {
            GlassToolTile(.tool(ManagedWords.glassTool(session.tool)))
            VStack(alignment: .leading, spacing: 1) {
                Text(session.purpose == "login" ? model.managedText("sign_in") : session.projectLabel)
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                Text(ManagedSurface.caption(meta, startedAt: session.startedAt))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
            }
            Spacer(minLength: GlassTokens.Space.s4)
            GlassStatusLabel(model.managedText(ManagedSurface.stateKey(session.state)), status: ManagedWords.status(session.state))
            ManagedSessionActions(session: session, menuFor: $menuFor, onStop: { stopping = session })
        }
        return ManagedSessionRowActions(session: session, onStop: { stopping = session }) { row }
    }
}

/// The "Change global settings" heading above the standard tool settings.
extension ManagedSessionsSection {
    /// Whether Launch is offered: a saved account whose tool is installed
    /// to launch with, and a terminal to launch in.
    static func offersLaunch(_ snapshot: ManagedSnapshot) -> Bool {
        snapshot.accounts.contains { snapshot.canLaunch(tool: $0.tool) } && snapshot.capabilities.terminalLaunch
    }
}

struct ManagedGlobalSettingsHeader: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if !model.managedText("global_title").isEmpty {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                GlassSectionRule(model.managedText("global_title"))
                Text(model.managedText("global_scope"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// Lookups the managed views share. No words of their own.
enum ManagedWords {
    @MainActor
    static func tool(_ tool: String, model: AppModel) -> String? {
        ManagedSurface.toolKey(tool).map(model.managedText)
    }

    static func glassTool(_ tool: String) -> GlassTool {
        switch tool {
        case "claude": .claudeCode
        case "codex": .codex
        case "opencode": .openCode
        default: .other(initials: String(tool.prefix(2)).capitalized)
        }
    }

    /// Running is on, starting is waiting, failed is outside; exited and
    /// anything not confirmed are off.
    static func status(_ state: String) -> GlassStatus {
        switch state {
        case "running": .on
        case "starting": .ask
        case "failed": .outside
        default: .off
        }
    }
}

// MARK: - Dialogs

/// An eyebrow label over a control, as `GlassTextField` lays out its own.
private struct ManagedField<Content: View>: View {
    let label: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(label).glassType(GlassTokens.TypeScale.eyebrow).foregroundStyle(GlassColor.textTertiary)
            content
        }
    }
}

/// A caption under a dialog's fields.
private struct ManagedNote: View {
    let text: String

    var body: some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

struct ManagedAccountSheet: View {
    let onClose: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var tool = "claude"
    @State private var connection = "subscription"
    @State private var label = ""
    @State private var key = ""

    private var availableTools: [String] { model.managedSnapshot?.setupTools ?? ManagedSurface.supportedTools }
    private var subscription: Bool { connection == "subscription" }

    private var canSave: Bool {
        availableTools.contains(tool) && !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (subscription || !key.isEmpty)
            && (!subscription || model.managedSnapshot?.capabilities.terminalLaunch == true)
    }

    var body: some View {
        GlassModal(
            title: model.managedText("save_title"), width: .narrow,
            actions: [
                .cancel(model.managedText("cancel"), action: onClose),
                GlassModalAction(
                    subscription ? model.managedText("save_login") : model.managedText("save_account"),
                    isDefault: true, isEnabled: canSave, id: "save", action: save),
            ],
            onCancel: onClose
        ) {
            GlassModalBody {
                HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
                    ManagedField(label: model.managedText("tool")) {
                        GlassSelect(
                            model.managedText("tool"), selection: Binding(get: { tool }, set: { tool = $0; if tool == "opencode" && connection == "subscription" { connection = "api_key" } }),
                            options: availableTools.map { GlassPickerOption(model.managedText($0), value: $0) })
                    }
                    ManagedField(label: model.managedText("connection")) {
                        GlassSelect(
                            model.managedText("connection"), selection: $connection,
                            options: ManagedSurface.connectionOptions(signedIn: model.managedSnapshot?.nearAiSignedIn == true, tool: tool)
                                .map { GlassPickerOption(model.managedText(tool == "opencode" && $0 == "api_key" ? "openai_api_key" : $0), value: $0) })
                    }
                }
                GlassTextField(model.managedText("label"), text: $label, prompt: model.managedText("label_placeholder"))
                if subscription {
                    ManagedNote(text: model.managedText("login_description"))
                } else {
                    GlassTextField(model.managedText("api_key"), text: $key, secure: true)
                    ManagedNote(text: model.managedText("key_storage"))
                }
            }
        }
        .onAppear { reconcileTool() }
        .onChange(of: availableTools) { reconcileTool() }
    }

    private func reconcileTool() {
        if !availableTools.contains(tool), let first = availableTools.first { tool = first }
        if tool == "opencode" && connection == "subscription" { connection = "api_key" }
    }

    private func save() {
        guard canSave else { return }
        model.managedAction("managed_account_add", params: ["tool": tool, "connection": connection, "label": label], openTerminal: subscription, newAccountKey: subscription ? nil : key)
        key = ""
        onClose()
    }
}

struct ManagedLaunchSheet: View {
    let onClose: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var accountID = ""
    @State private var selectedModel = ""
    @State private var collectTraces = false
    private var isOpenCode: Bool { model.managedSnapshot?.accounts.first(where: { $0.id == accountID })?.tool == "opencode" }
    @State private var project: URL?
    private var selectedAccount: ManagedAccount? { model.managedSnapshot?.accounts.first { $0.id == accountID } }
    private var toolAvailable: Bool { selectedAccount.map { model.managedSnapshot?.canLaunch(tool: $0.tool) == true } ?? false }

    init(onClose: @escaping () -> Void, accountID: String = "") {
        self.onClose = onClose
        _accountID = State(initialValue: accountID)
    }

    var body: some View {
        GlassModal(
            title: model.managedText("launch_title"), width: .narrow,
            actions: [
                .cancel(model.managedText("cancel"), action: onClose),
                GlassModalAction(
                    model.managedText("launch_short"), isDefault: true,
                    isEnabled: toolAvailable && !accountID.isEmpty && project != nil && (!isOpenCode || !selectedModel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) && !model.managedBusy, id: "launch", action: launch),
            ],
            busy: model.managedBusy,
            onCancel: onClose
        ) {
            GlassModalBody {
                ManagedField(label: model.managedText("saved_account")) {
                    GlassPicker(
                        model.managedText("saved_account"),
                        selection: Binding(get: { accountID.isEmpty ? nil : accountID }, set: { accountID = $0 ?? "" }),
                        options: (model.managedSnapshot?.accounts ?? []).filter { model.managedSnapshot?.canLaunch(tool: $0.tool) == true || $0.id == accountID }.map { account in
                            GlassPickerOption(
                                [ManagedWords.tool(account.tool, model: model), account.label, model.managedText(account.connection)]
                                    .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "),
                                value: account.id)
                        },
                        placeholder: model.managedText("choose_account"))
                }
                if let account = selectedAccount, !toolAvailable {
                    ManagedNote(text: ManagedSurface.toolUnavailable(account.tool, copy: model.managedText))
                    HStack(spacing: GlassTokens.Space.s3) {
                        if let availability = model.managedSnapshot?.tools?.first(where: { $0.tool == account.tool }) {
                            ManagedInstallLink(tool: availability, copy: model.managedText)
                        }
                        GlassRefreshButton(model.managedText("refresh")) { model.refreshManagedSessions() }
                    }
                }
                if let account = selectedAccount, model.managedErrorKey == "unsupported_version",
                   model.managedErrorTool == nil || model.managedErrorTool == account.tool {
                    ManagedNote(text: model.managedText("unsupported_version"))
                    if let availability = model.managedSnapshot?.tools?.first(where: { $0.tool == account.tool }) {
                        ManagedInstallLink(tool: availability, copy: model.managedText, labelKey: "update", includeToolName: true)
                    }
                }
                if isOpenCode {
                    GlassTextField(model.managedText("model"), text: $selectedModel)
                    ManagedNote(text: model.managedText("opencode_scope"))
                    Toggle(model.managedText("collect_traces"), isOn: $collectTraces)
                        .toggleStyle(GlassToggleStyle(.settings))
                    ManagedNote(text: model.managedText("collection_scope"))
                }
                HStack(spacing: GlassTokens.Space.s4) {
                    Text(project?.path ?? model.managedText("project_placeholder"))
                        .glassType(project == nil ? GlassTokens.TypeScale.label.weight(.regular) : GlassTokens.TypeScale.mono)
                        .foregroundStyle(project == nil ? GlassColor.textTertiary : GlassColor.textPrimary)
                        .lineLimit(2)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                    Spacer(minLength: 0)
                    GlassFolderButton(model.managedText("choose_folder")) {
                        if let path = FolderPanel.choose() { project = URL(fileURLWithPath: path, isDirectory: true) }
                    }
                }
                ManagedNote(text: model.managedText("launch_scope").replacingOccurrences(of: "{destination}", with: model.managedSnapshot?.capabilities.terminalDestination ?? model.managedText("terminal")))
            }
        }
    }

    private func launch() {
        guard toolAvailable, !model.managedBusy, let snapshot = model.managedSnapshot, let account = snapshot.accounts.first(where: { $0.id == accountID }), let project else { return }
        var params: [String: Any] = ["request_id": UUID().uuidString, "purpose": "coding", "tool": account.tool, "connection": account.connection, "account_id": account.id, "cwd": project.path, "expected_generation": snapshot.generations[account.tool] ?? 0, "save_default": false]
        if account.tool == "opencode" {
            params["model"] = selectedModel.trimmingCharacters(in: .whitespacesAndNewlines)
            params["collect_traces"] = collectTraces
        }
        model.managedAction("managed_launch_prepare", params: params, openTerminal: true)
        onClose()
    }
}

struct ManagedRenameSheet: View {
    let account: ManagedAccount
    let onClose: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var label: String

    init(account: ManagedAccount, onClose: @escaping () -> Void) {
        self.account = account
        self.onClose = onClose
        _label = State(initialValue: account.label)
    }

    var body: some View {
        GlassModal(
            title: model.managedText("rename"), width: .narrow,
            actions: [
                .cancel(model.managedText("cancel"), action: onClose),
                GlassModalAction(model.managedText("save"), isDefault: true, id: "save") {
                    model.managedAction("managed_account_rename", params: ["account_id": account.id, "label": label])
                    onClose()
                },
            ],
            onCancel: onClose
        ) {
            GlassModalBody {
                GlassTextField(model.managedText("label"), text: $label, prompt: model.managedText("label_placeholder"))
            }
        }
    }
}

/// The removal confirmation: cancel first, the destructive action on the
/// right, never one Return away.
struct ManagedRemoveSheet: View {
    let account: ManagedAccount
    let onClose: () -> Void
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassConfirmation(
            title: model.managedText("remove_question"),
            message: model.managedText("remove_description"),
            actions: [
                .cancel(model.managedText("cancel"), action: onClose),
                .destructive(model.managedText("remove")) {
                    model.managedAction("managed_account_remove", params: ["account_id": account.id])
                    onClose()
                },
            ],
            onCancel: onClose)
    }
}

// MARK: - Session row actions (#1407)

/// A session row's trailing actions (Ron, #1407): a running row's menu with
/// Show and Stop; an ended row's Start again, then Dismiss as a link. What
/// the daemon cannot do is left out, never drawn disabled.
struct ManagedSessionActions: View {
    let session: ManagedSession
    /// The row whose menu is open, shared with the account rows.
    @Binding var menuFor: String?
    let onStop: () -> Void
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let actions = ManagedSurface.sessionActions(session, terminalLaunch: model.managedSnapshot?.capabilities.terminalLaunch == true)
        if actions.contains(.show) || actions.contains(.stop) {
            menu(actions)
        }
        if actions.contains(.startAgain) {
            Button { startAgain() } label: {
                Text(model.managedText("start_again")).lineLimit(1)
            }
            .buttonStyle(GlassButtonStyle(.glass, small: true))
            .fixedSize()
            .disabled(model.managedBusy)
        }
        if actions.contains(.dismiss) {
            Button(model.managedText("dismiss")) {
                model.managedAction("managed_session_dismiss", params: ["session_id": session.id])
            }
            .buttonStyle(GlassButtonStyle(.link))
            .fixedSize()
            .disabled(model.managedBusy)
        }
    }

    private func menu(_ actions: [ManagedSessionAction]) -> some View {
        let open = Binding(get: { menuFor == session.id }, set: { if !$0 { menuFor = nil } })
        return GlassKebab(model.managedText("manage"), open: menuFor == session.id) { menuFor = session.id }
            .disabled(model.managedBusy)
            .popover(isPresented: open, arrowEdge: .bottom) {
                GlassMenu(onDismiss: { menuFor = nil }) {
                    if actions.contains(.show) {
                        GlassMenuItem(model.managedText("show")) {
                            menuFor = nil
                            model.showManagedSession(session.id)
                        }
                    }
                    if actions.contains(.stop) {
                        GlassMenuItem(model.managedText("stop")) {
                            menuFor = nil
                            DispatchQueue.main.async { onStop() }
                        }
                    }
                }
                .disabled(model.managedBusy)
            }
    }

    /// Same tool, account and folder, with no folder chooser; the terminal
    /// opens as for any launch.
    private func startAgain() {
        model.managedAction(
            "managed_session_start_again",
            params: ManagedSurface.startAgainParams(sessionID: session.id, requestID: UUID().uuidString),
            openTerminal: true)
    }
}

/// The whole row's click and its VoiceOver actions: a click on a running
/// row the daemon can show brings its window to the front, and Show and
/// Stop are the row's named accessibility actions. A row with neither
/// reads as before.
struct ManagedSessionRowActions<Content: View>: View {
    let session: ManagedSession
    let onStop: () -> Void
    @ViewBuilder let content: Content
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let menu = ManagedSurface.menuActions(session)
        if menu.isEmpty {
            content.accessibilityElement(children: .contain)
        } else {
            content
                .contentShape(Rectangle())
                .onTapGesture { if ManagedSurface.clickShows(session) { model.showManagedSession(session.id) } }
                .accessibilityElement(children: .contain)
                .accessibilityActions {
                    if menu.contains(.show) {
                        Button(model.managedText("show")) { model.showManagedSession(session.id) }
                    }
                    if menu.contains(.stop) {
                        Button(model.managedText("stop")) { onStop() }
                    }
                }
        }
    }

}

extension AppModel {
    /// Bring a session's window to the front.
    func showManagedSession(_ id: String) {
        managedAction("managed_session_focus", params: ["session_id": id])
    }
}

/// The stop confirmation (Ron, #1407): Cancel first, Stop on the right,
/// never one Return away.
struct ManagedStopSheet: View {
    let session: ManagedSession
    let onClose: () -> Void
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassConfirmation(
            title: model.managedText("stop_question"),
            message: ManagedSurface.stopDescription(
                model.managedText("stop_description"),
                tool: ManagedWords.tool(session.tool, model: model) ?? "",
                folder: session.projectLabel),
            actions: [
                .cancel(model.managedText("cancel"), action: onClose),
                .destructive(model.managedText("stop")) {
                    model.managedAction("managed_session_stop", params: ["session_id": session.id])
                    onClose()
                },
            ],
            onCancel: onClose)
    }
}

/// Installation recovery uses the core's official links and words.
struct ManagedInstallLink: View {
    let tool: ManagedToolAvailability
    let copy: (String) -> String
    /// The link's copy key: `install`, or `update` after an unsupported-version
    /// refusal, with the tool's name appended when `includeToolName` is set.
    var labelKey: String = "install"
    var includeToolName = false
    private var actionLabel: String { copy(labelKey).isEmpty ? copy("install") : copy(labelKey) }
    private var label: String { includeToolName ? actionLabel + " " + copy(tool.tool) : actionLabel }
    var body: some View {
        if let url = tool.url {
            Link(label, destination: url)
                .accessibilityLabel(actionLabel + " " + copy(tool.tool))
                .buttonStyle(GlassButtonStyle(.glass, small: true))
        }
    }
}

/// Shown only while no coding tool is installed: each managed tool with
/// Install, and one bare refresh glyph beside the heading (Ron, 2026-10-15).
struct ManagedToolInstallation: View {
    let tools: [ManagedToolAvailability]
    let copy: (String) -> String
    let retry: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(spacing: GlassTokens.Space.s2) {
                Text(copy("tools_title"))
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                GlassRefreshButton(copy("refresh"), action: retry)
            }
            Text(copy("tools_empty"))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(tools.filter { ManagedSurface.supportedTools.contains($0.tool) }) { tool in
                HStack {
                    GlassToolTile(.tool(ManagedWords.glassTool(tool.tool)))
                    Text(copy(tool.tool))
                        .glassType(GlassTokens.TypeScale.label)
                        .foregroundStyle(GlassColor.textPrimary)
                    Spacer()
                    ManagedInstallLink(tool: tool, copy: copy)
                }
            }
        }
    }
}
