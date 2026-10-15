import SwiftUI
import AppKit
import TCDesign
import TCShellCore

/// Saved model accounts and managed sessions, in the glass design (#1146):
/// two eyebrow cards, accounts then sessions. Drawn in the Monitor's
/// Inference tab and on the legacy model-calls screen.
///
/// Every word comes from the shared table (`managed/copy.rs`) through
/// `AppModel.managedText`; no raw state, auth value or daemon code is drawn.
struct ManagedSessionsSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var adding = false
    @State private var launching = false
    @State private var renaming: ManagedAccount?
    @State private var removing: ManagedAccount?
    /// The account whose row menu is open.
    @State private var menuFor: String?

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            accountsCard
            if let snapshot = model.managedSnapshot {
                sessionsCard(snapshot)
            }
            if let key = model.managedErrorKey {
                GlassNotice(tone: .outside, title: model.managedText("action_failed")) {
                    if key != "action_failed" {
                        Text(model.managedText(key)).textSelection(.enabled)
                        if key == "unsupported_version" {
                            ForEach(model.managedSnapshot?.updateRecoveryTools(attemptedTool: model.managedErrorTool) ?? []) { availability in
                                ManagedInstallLink(tool: availability, copy: model.managedText, labelKey: "update", includeToolName: true)
                            }
                        }
                    }
                }
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

    // MARK: Accounts

    private var accountsCard: some View {
        // The header carries only the re-read link, as #1146's panels do;
        // the two actions sit on their own row under the description, so
        // no label is ever squeezed into breaking per syllable (V1).
        GlassEyebrowCard(model.managedText("accounts_title"), accessory: {
            if model.managedSnapshot != nil { refreshLink }
        }) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                Text(model.managedText("description"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let snapshot = model.managedSnapshot {
                    if snapshot.needsToolInstallation {
                        ManagedToolInstallation(tools: snapshot.tools ?? [], copy: model.managedText, retry: model.refreshManagedSessions)
                    }
                    accountActions(snapshot)
                    terminalLine(snapshot)
                    if !snapshot.accounts.isEmpty {
                        VStack(spacing: 0) {
                            ForEach(Array(snapshot.accounts.enumerated()), id: \.element.id) { index, account in
                                GlassTableRow(first: index == 0) { accountRow(account, snapshot: snapshot) }
                            }
                        }
                    }
                } else {
                    HStack(spacing: GlassTokens.Space.s4) {
                        Text(model.managedText("connecting"))
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                        Spacer(minLength: 0)
                        Button { model.refreshManagedSessions() } label: {
                            Label(model.managedText("retry"), systemImage: "arrow.clockwise").lineLimit(1)
                        }
                        .buttonStyle(GlassButtonStyle(.glass, small: true))
                        .fixedSize()
                    }
                }
            }
        }
    }

    private var refreshLink: some View {
        Button { model.refreshManagedSessions() } label: {
            Text(model.managedText("refresh")).lineLimit(1)
        }
        .buttonStyle(GlassButtonStyle(.link))
        .fixedSize()
        .disabled(model.managedBusy)
    }

    /// Add and Launch, each on one line at its own width: side by side when
    /// they fit, stacked when the pane is too narrow, never wrapped.
    private func accountActions(_ snapshot: ManagedSnapshot) -> some View {
        let add = Button { adding = true } label: {
            Text(model.managedText("add")).lineLimit(1)
        }
        .buttonStyle(GlassButtonStyle(.glass, small: true))
        .fixedSize()
        .disabled(snapshot.setupTools.isEmpty)
        let launch = Button { launching = true } label: {
            Text(model.managedText("launch")).lineLimit(1)
        }
        .buttonStyle(GlassButtonStyle(.primary, small: true))
        .fixedSize()
        .disabled(!snapshot.accounts.contains { snapshot.canLaunch(tool: $0.tool) } || !snapshot.capabilities.terminalLaunch)
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: GlassTokens.Space.s3) { add; launch }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) { add; launch }
        }
        .disabled(model.managedBusy)
    }

    @ViewBuilder
    private func terminalLine(_ snapshot: ManagedSnapshot) -> some View {
        if let destination = snapshot.capabilities.terminalDestination {
            Text(model.managedText("terminal_scope").replacingOccurrences(of: "{destination}", with: destination))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            GlassNotice(tone: .ask, title: model.managedText("terminal_unavailable")) { EmptyView() }
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
                    Text(model.managedText("tool_unavailable"))
                        .glassType(GlassTokens.TypeScale.caption)
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
                    if account.connection == "subscription" {
                        GlassMenuItem(model.managedText("reconnect")) {
                            menuFor = nil
                            model.managedAction("managed_account_reconnect", params: ["account_id": account.id], openTerminal: true)
                        }
                        .disabled(held || !snapshot.capabilities.terminalLaunch || !snapshot.canLaunch(tool: account.tool))
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

    private func sessionsCard(_ snapshot: ManagedSnapshot) -> some View {
        GlassEyebrowCard(model.managedText("title")) {
            if snapshot.sessions.isEmpty {
                Text(model.managedText("empty"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(snapshot.sessions.enumerated()), id: \.element.id) { index, session in
                        GlassTableRow(first: index == 0) { sessionRow(session) }
                    }
                }
            }
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
        return HStack(spacing: GlassTokens.Space.s4) {
            GlassToolTile(.tool(ManagedWords.glassTool(session.tool)))
            VStack(alignment: .leading, spacing: 1) {
                Text(session.purpose == "login" ? model.managedText("sign_in") : session.projectLabel)
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                Text(meta.joined(separator: " · "))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
            }
            Spacer(minLength: GlassTokens.Space.s4)
            GlassStatusLabel(model.managedText(ManagedSurface.stateKey(session.state)), status: ManagedWords.status(session.state))
            if !session.holdsAccount {
                Button(model.managedText("dismiss")) {
                    model.managedAction("managed_session_dismiss", params: ["session_id": session.id])
                }
                .buttonStyle(GlassButtonStyle(.link))
                .fixedSize()
                .disabled(model.managedBusy)
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// The "Change global settings" heading above the standard tool settings.
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
                            options: (tool == "opencode" ? ["api_key", "near_ai"] : ["subscription", "api_key", "near_ai"]).map { GlassPickerOption(model.managedText(tool == "opencode" && $0 == "api_key" ? "openai_api_key" : $0), value: $0) })
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
    private var selectedAccount: ManagedAccount? { model.managedSnapshot?.accounts.first { $0.id == accountID } }
    private var toolAvailable: Bool { selectedAccount.map { model.managedSnapshot?.canLaunch(tool: $0.tool) == true } ?? false }
    private var isOpenCode: Bool { model.managedSnapshot?.accounts.first(where: { $0.id == accountID })?.tool == "opencode" }
    @State private var project: URL?

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
                    ManagedNote(text: model.managedText("tool_unavailable"))
                    if let availability = model.managedSnapshot?.tools?.first(where: { $0.tool == account.tool }) {
                        ManagedInstallLink(tool: availability, copy: model.managedText)
                    }
                    Button(model.managedText("check_again")) { model.refreshManagedSessions() }
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

/// Installation recovery uses the core's official links and words.
struct ManagedInstallLink: View {
    let tool: ManagedToolAvailability
    let copy: (String) -> String
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

struct ManagedToolInstallation: View {
    let tools: [ManagedToolAvailability]
    let copy: (String) -> String
    let retry: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(copy("tools_title")).glassType(GlassTokens.TypeScale.bodyStrong)
            Text(copy("tools_empty")).glassType(GlassTokens.TypeScale.caption)
            ForEach(tools.filter { ManagedSurface.toolKey($0.tool) != nil }) { tool in
                HStack {
                    GlassToolTile(.tool(ManagedWords.glassTool(tool.tool)))
                    Text(copy(tool.tool))
                    Spacer()
                    ManagedInstallLink(tool: tool, copy: copy)
                }
            }
            Button(copy("check_again"), action: retry).buttonStyle(GlassButtonStyle(.glass, small: true))
        }
    }
}
