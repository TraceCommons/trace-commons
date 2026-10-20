import SwiftUI
import AppKit
import TCDesign
import TCShellCore

/// Managed sessions on the Private AI tab, in the glass design (#1146): the
/// NEAR AI card while signed in, then the sessions. Signed out, the NEAR AI
/// sign-in is a popover notice over the tab instead (owner, 2026-10-19,
/// `MonitorNotices`). Saved model accounts are not drawn here any more
/// (owner, 2026-10-19: they are not Private AI sessions).
///
/// Every word comes from the shared table (`managed/copy.rs`) through
/// `AppModel.managedText`; no raw state, auth value or daemon code is drawn.
struct ManagedSessionsSection: View {
    @EnvironmentObject private var model: AppModel
    /// The tool whose launch dialog is open: its folder, then Open.
    @State private var launching: ManagedFolderChoice?
    /// The session whose stop is being confirmed.
    @State private var stopping: ManagedSession?
    /// The session whose row menu is open.
    @State private var menuFor: String?

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let snapshot = model.managedSnapshot, snapshot.nearAiSignedIn {
                signInCard(snapshot)
            }
            if let snapshot = model.managedSnapshot {
                sessionsCard(snapshot)
            } else {
                connectingCard
            }
        }
        // Glass modals over the whole window, not stock sheets.
        .glassModal(item: $launching) { choice in
            ManagedFolderExplainerSheet(
                tool: choice.tool, onCancel: { launching = nil },
                onOpen: { path in
                    launching = nil
                    launch(choice.tool, in: path)
                }
            ).environmentObject(model)
        }
        .glassModal(item: $stopping) { session in
            ManagedStopSheet(session: session, onClose: { stopping = nil }).environmentObject(model)
        }
        // A tool chosen in the menu bar's popover opens its dialog here
        // (owner, 2026-10-19): the popover cannot hold one.
        .onChange(of: model.pendingManagedLaunch, initial: true) { _, tool in
            guard let tool else { return }
            model.pendingManagedLaunch = nil
            start(tool)
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

    /// Sessions on the NEAR AI sign-in, first (owner, 2026-10-12), drawn
    /// only while signed in. A button for each tool, its logo and name, in
    /// a row at the foot of the card (owner, 2026-10-19): Claude Code,
    /// Codex and, once OpenCode is installed, OpenCode. Each opens the
    /// launch dialog, where a folder is chosen and the tool opened in the
    /// terminal. Nothing here is saved: the entries come and go with the
    /// sign-in.
    @ViewBuilder
    private func signInCard(_ snapshot: ManagedSnapshot) -> some View {
        GlassIconCard(
            systemImage: "sparkles",
            title: model.managedText("near_ai"),
            subtitle: [ManagedSurface.signInScope(
                model.managedText("sign_in_scope"),
                destination: snapshot.capabilities.terminalDestination,
                fallback: model.managedText("terminal"))],
            accessory: { AnyView(EmptyView()) },
            content: { AnyView(toolButtons(snapshot)) })
    }

    /// What the NEAR AI card holds at its foot: the tool buttons; in their
    /// place, the no-terminal notice, or Install for each tool while none
    /// is installed. Only one, and only while it applies.
    enum ToolArea: Equatable {
        case buttons
        case noTerminal
        case install
    }

    static func toolArea(_ snapshot: ManagedSnapshot) -> ToolArea {
        if !snapshot.capabilities.terminalLaunch { return .noTerminal }
        if snapshot.needsToolInstallation { return .install }
        return .buttons
    }

    /// Whether a failure is said on the NEAR AI card, under the button that
    /// was pressed: a launch, while the card is drawn. Anything else (a
    /// stop, a dismiss, a start again) is said on the sessions card.
    static func failureOnSignInCard(method: String?, signInCardShown: Bool) -> Bool {
        signInCardShown && method == "managed_launch_prepare"
    }

    /// The card's foot (`toolArea`), then a launch that failed.
    @ViewBuilder
    private func toolButtons(_ snapshot: ManagedSnapshot) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            switch Self.toolArea(snapshot) {
            case .buttons:
                HistoryFilterFlow(spacing: GlassTokens.Space.s3) {
                    ForEach(snapshot.signInStartTools, id: \.self) { tool in
                        ManagedToolButton(tool: tool) { start(tool) }
                    }
                }
                .disabled(model.managedBusy)
            case .noTerminal:
                GlassNotice(tone: .ask, title: model.managedText("terminal_unavailable")) { EmptyView() }
            case .install:
                ManagedNote(text: model.managedText("tools_empty"))
                HistoryFilterFlow(spacing: GlassTokens.Space.s3) {
                    ForEach((snapshot.tools ?? []).filter { ManagedSurface.signInTools.contains($0.tool) }) { tool in
                        ManagedInstallLink(tool: tool, copy: model.managedText, includeToolName: true)
                    }
                }
            }
            if let key = model.managedErrorKey,
               Self.failureOnSignInCard(method: model.managedErrorMethod, signInCardShown: true) {
                actionFailure(key)
            }
        }
    }

    /// A tool's launch dialog: its folder, then Open.
    private func start(_ tool: String) {
        guard !model.managedBusy else { return }
        launching = ManagedFolderChoice(tool: tool)
    }

    /// The launch, in the folder chosen in the dialog.
    private func launch(_ tool: String, in path: String) {
        guard !model.managedBusy,
              let params = model.managedSnapshot?.signInLaunchParams(tool: tool, cwd: path, requestID: UUID().uuidString)
        else { return }
        model.managedAction("managed_launch_prepare", params: params, openTerminal: true)
    }

    // MARK: Failures and connecting

    /// A failed request: the failed request's red lines, unboxed (Ron,
    /// 2026-10-09), where it was asked (`failureOnSignInCard`). The
    /// headline, then the reason when it is a different word, then, for an
    /// unsupported version, each tool's update link.
    private func actionFailure(_ key: String) -> some View {
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

    /// The sessions card while the list has not been read: connecting,
    /// with Retry.
    private var connectingCard: some View {
        GlassIconCard(
            systemImage: "terminal", title: model.managedText("title"),
            subtitle: [model.managedText("connecting")],
            trailing: {
                Button { model.refreshManagedSessions() } label: {
                    Label(model.managedText("retry"), systemImage: "arrow.clockwise").lineLimit(1)
                }
                .buttonStyle(GlassButtonStyle(.glass, small: true))
                .fixedSize()
                .disabled(model.managedBusy)
            })
    }

    // MARK: Sessions

    /// The sessions card under its icon (owner, 2026-10-10): with none, the
    /// empty line is the head's subtitle and the card is one row; otherwise
    /// the sessions are listed under the head.
    ///
    /// A failure other than a launch on the NEAR AI card is said under the
    /// sessions, where Stop, Dismiss and Start again are.
    @ViewBuilder
    private func sessionsCard(_ snapshot: ManagedSnapshot) -> some View {
        let failure = model.managedErrorKey.flatMap { key in
            Self.failureOnSignInCard(method: model.managedErrorMethod, signInCardShown: snapshot.nearAiSignedIn) ? nil : key
        }
        if snapshot.sessions.isEmpty && failure == nil {
            GlassIconCard(
                systemImage: "terminal", title: model.managedText("title"),
                subtitle: [model.managedText("empty")], trailing: { EmptyView() })
        } else {
            GlassIconCard(
                systemImage: "terminal", title: model.managedText("title"),
                subtitle: snapshot.sessions.isEmpty ? [model.managedText("empty")] : [],
                content: {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        if !snapshot.sessions.isEmpty {
                            VStack(spacing: 0) {
                                ForEach(Array(snapshot.sessions.enumerated()), id: \.element.id) { index, session in
                                    GlassTableRow(first: index == 0) { sessionRow(session) }
                                }
                            }
                        }
                        if let failure { actionFailure(failure) }
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
    /// The model picked for Claude Code or Codex on a saved NEAR AI key;
    /// `ManagedSurface.defaultModel` lets the session take the tool's default.
    @State private var pickedModel = ManagedSurface.defaultModel
    /// NEAR AI's chat models for `catalogueTool`; nil until read, or when
    /// they could not be.
    @State private var catalogue: ManagedModelCatalogue?
    @State private var catalogueTool: String?
    @State private var collectTraces = false
    private var isOpenCode: Bool { model.managedSnapshot?.accounts.first(where: { $0.id == accountID })?.tool == "opencode" }
    @State private var project: URL?
    private var selectedAccount: ManagedAccount? { model.managedSnapshot?.accounts.first { $0.id == accountID } }
    private var toolAvailable: Bool { selectedAccount.map { model.managedSnapshot?.canLaunch(tool: $0.tool) == true } ?? false }
    private var offersModelChoice: Bool {
        selectedAccount.map { ManagedSurface.offersModelChoice(tool: $0.tool, connection: $0.connection) } ?? false
    }

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
                if offersModelChoice {
                    ManagedField(label: model.managedText("model_choice")) {
                        GlassPicker(
                            model.managedText("model_choice"),
                            selection: Binding(get: { pickedModel }, set: { pickedModel = $0 ?? ManagedSurface.defaultModel }),
                            options: ManagedSurface.modelOptions(catalogue, copy: model.managedText).map { GlassPickerOption($0.title, value: $0.value) },
                            placeholder: model.managedText("model_default_unknown"))
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
        .onAppear(perform: loadModels)
        .onChange(of: accountID) { _, _ in loadModels() }
    }

    private func launch() {
        guard toolAvailable, !model.managedBusy, let snapshot = model.managedSnapshot, let account = snapshot.accounts.first(where: { $0.id == accountID }), let project else { return }
        let params = ManagedSurface.launchParams(
            account: account, cwd: project.path, requestID: UUID().uuidString,
            generation: snapshot.generations[account.tool] ?? 0,
            model: account.tool == "opencode" ? selectedModel : pickedModel, collectTraces: collectTraces)
        model.managedAction("managed_launch_prepare", params: params, openTerminal: true)
        onClose()
    }

    /// Read the picker's models for the chosen account's tool, once per
    /// tool. A different account starts again from the default.
    private func loadModels() {
        guard let account = selectedAccount, offersModelChoice else { return }
        let tool = account.tool
        guard catalogueTool != tool else { return }
        catalogueTool = tool
        catalogue = nil
        pickedModel = ManagedSurface.defaultModel
        model.managedModelCatalogue(tool: tool) { result in
            if catalogueTool == tool { catalogue = result }
        }
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
