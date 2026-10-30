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
                if !snapshot.nearAiSignedIn, !snapshot.missingInstallTools.isEmpty {
                    setupOnlyButtons(snapshot)
                }
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
                },
                onOpenWithSettings: { path, settings in
                    launching = nil
                    launch(choice.tool, in: path, settings: settings)
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
    /// only while signed in, under a title naming the tools that run
    /// Private AI (product owner, 2026-10-28). A button for each tool, its logo and name, in
    /// a row at the foot of the card (owner, 2026-10-19): Claude Code,
    /// Codex and, once OpenCode is installed, OpenCode. Each opens the
    /// launch dialog, where a folder is chosen and the tool opened in the
    /// terminal. Nothing here is saved: the entries come and go with the
    /// sign-in.
    @ViewBuilder
    private func signInCard(_ snapshot: ManagedSnapshot) -> some View {
        GlassIconCard(
            systemImage: "sparkles",
            title: model.managedText("sign_in_title"),
            subtitle: [ManagedSurface.signInScope(
                model.managedText("sign_in_scope"),
                destination: snapshot.capabilities.terminalDestination,
                fallback: model.managedText("terminal"))],
            accessory: { AnyView(EmptyView()) },
            content: { AnyView(toolButtons(snapshot)) })
    }

    /// What the NEAR AI card holds at its foot: the tool buttons; in their
    /// place, the no-terminal notice. Missing tools retain Install beside
    /// installed tools' launch buttons.
    enum ToolArea: Equatable {
        case buttons
        case noTerminal
        case install
    }

    nonisolated static func toolArea(_ snapshot: ManagedSnapshot) -> ToolArea {
        if !snapshot.capabilities.terminalLaunch { return .noTerminal }
        if snapshot.needsToolInstallation { return .install }
        return .buttons
    }

    /// Whether a failure is said on the NEAR AI card, under the button that
    /// was pressed: a launch or tool setup while the card is drawn. A stop,
    /// dismiss, or start again is said on the sessions card.
    static func failureOnSignInCard(method: String?, signInCardShown: Bool) -> Bool {
        signInCardShown && ["managed_launch_prepare", "managed_tool_setup"].contains(method ?? "")
    }

    /// The card's foot (`toolArea`), then a launch that failed.
    @ViewBuilder
    private func toolButtons(_ snapshot: ManagedSnapshot) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            switch Self.toolArea(snapshot) {
            case .noTerminal:
                GlassNotice(tone: .ask, title: model.managedText("terminal_unavailable")) { EmptyView() }
            case .install:
                ManagedNote(text: model.managedText("tools_empty"))
            case .buttons:
                EmptyView()
            }
            HistoryFilterFlow(spacing: GlassTokens.Space.s3) {
                if Self.toolArea(snapshot) == .buttons {
                    ForEach(snapshot.signInStartTools, id: \.self) { tool in
                        ManagedToolButton(tool: tool) { start(tool) }
                    }
                }
                ForEach(snapshot.missingInstallTools) { tool in
                    ManagedInstallLink(tool: tool, copy: model.managedText, includeToolName: true)
                }
            }
            .disabled(model.managedBusy)
            if let key = model.managedErrorKey,
               Self.failureOnSignInCard(method: model.managedErrorMethod, signInCardShown: true) {
                actionFailure(key)
            }
        }
    }

    /// Tool installation can be requested before a NEAR AI sign-in.
    private func setupOnlyButtons(_ snapshot: ManagedSnapshot) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HistoryFilterFlow(spacing: GlassTokens.Space.s3) {
                ForEach(snapshot.missingInstallTools) { tool in
                    ManagedInstallLink(tool: tool, copy: model.managedText, includeToolName: true)
                }
            }
            .disabled(model.managedBusy)
            if model.managedErrorMethod == "managed_tool_setup", let key = model.managedErrorKey {
                actionFailure(key)
            }
        }
    }

    /// A tool's launch dialog: an optional folder, then Open.
    private func start(_ tool: String) {
        guard !model.managedBusy else { return }
        launching = ManagedFolderChoice(tool: tool)
    }

    /// The launch, in the folder chosen in the dialog; with none, the daemon
    /// starts it in the person's home folder.
    private func launch(_ tool: String, in path: String?, settings: ManagedFolderDraft? = nil) {
        guard !model.managedBusy,
              var params = model.managedSnapshot?.signInLaunchParams(tool: tool, cwd: path, requestID: UUID().uuidString)
        else { return }
        if let settings { params = settings.applying(to: params) }
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
    /// with Retry as the card's own re-read icon, as the balance card above
    /// it draws its refresh (owner, 2026-10-22: it was a wide capsule round
    /// the glyph, then a larger round button than the balance card's).
    private var connectingCard: some View {
        GlassIconCard(
            systemImage: "terminal", title: model.managedText("title"),
            subtitle: [model.managedText("connecting")],
            refresh: .init(model.managedText("retry"), isDisabled: model.managedBusy) {
                model.refreshManagedSessions()
            },
            content: { EmptyView() })
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
            let setupRowShown = !snapshot.nearAiSignedIn && !snapshot.missingInstallTools.isEmpty
            let setupFailure = setupRowShown && model.managedErrorMethod == "managed_tool_setup"
            return Self.failureOnSignInCard(method: model.managedErrorMethod, signInCardShown: snapshot.nearAiSignedIn)
                || setupFailure ? nil : key
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
                            LazyVStack(spacing: 0) {
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
        #if DEBUG
        OtherCollectionRowBodies.record("managed", session.id)
        #endif
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
    @State private var folderDraft = ManagedFolderDraft()
    @State private var loadedAccountID: String?
    /// NEAR AI's chat models for `catalogueTool`; nil until read, or when
    /// they could not be.
    @State private var catalogue: ManagedModelCatalogue?
    @State private var catalogueTool: String?
    /// The last catalogue read failed (offline, timeout, NEAR AI down).
    @State private var catalogueFailed = false
    @State private var project: URL?
    private var selectedAccount: ManagedAccount? { model.managedSnapshot?.accounts.first { $0.id == accountID } }
    private var toolAvailable: Bool { selectedAccount.map { model.managedSnapshot?.canLaunch(tool: $0.tool) == true } ?? false }
    private var supportsFolderSettings: Bool { model.managedSnapshot?.capabilities.folderSettings == true }
    private var folderReady: Bool {
        guard let account = selectedAccount, loadedAccountID == accountID else { return false }
        return folderDraft.matches(tool: account.tool, cwd: project?.path)
            && ManagedFolderOptions.launchModelReady(folderDraft, tool: account.tool, connection: account.connection,
                                                     catalogue: catalogue, catalogueFailed: catalogueFailed)
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
                    isEnabled: toolAvailable && folderReady && !model.managedBusy, id: "launch", action: launch),
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
                if let account = selectedAccount, model.managedErrorMethod == "managed_tool_setup",
                   model.managedErrorTool == account.tool, let key = model.managedErrorKey {
                    GlassAlert(model.managedText(key))
                }
                if let account = selectedAccount {
                    ManagedFolderOptions(tool: account.tool, connection: account.connection, draft: $folderDraft,
                        catalogue: $catalogue, supportsFolderSettings: supportsFolderSettings,
                        retry: { loadFolderSettings() }, reset: { loadFolderSettings(reset: true) })
                }
                HStack(spacing: GlassTokens.Space.s4) {
                    Text(project?.path ?? model.managedText("home_folder"))
                        .glassType(project == nil ? GlassTokens.TypeScale.label.weight(.regular) : GlassTokens.TypeScale.mono)
                        .foregroundStyle(project == nil ? GlassColor.textTertiary : GlassColor.textPrimary)
                        .lineLimit(2)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                    Spacer(minLength: 0)
                    if project != nil {
                        Button(model.managedText("clear_folder")) { project = nil }
                            .buttonStyle(GlassButtonStyle(.glass, small: true))
                            .fixedSize()
                    }
                    GlassFolderButton(model.managedText("pick_folder")) {
                        if let path = FolderPanel.choose() { project = URL(fileURLWithPath: path, isDirectory: true) }
                    }
                }
                ManagedNote(text: model.managedText("launch_scope").replacingOccurrences(of: "{destination}", with: model.managedSnapshot?.capabilities.terminalDestination ?? model.managedText("terminal")))
            }
        }
        .onAppear { loadModels(); loadFolderSettings() }
        .onChange(of: accountID) { _, _ in loadModels(); loadFolderSettings() }
        .onChange(of: project) { _, _ in loadFolderSettings() }
    }

    private func launch() {
        guard toolAvailable, folderReady, !model.managedBusy, let snapshot = model.managedSnapshot, let account = snapshot.accounts.first(where: { $0.id == accountID }) else { return }
        var params = ManagedSurface.launchParams(
            account: account, cwd: project?.path, requestID: UUID().uuidString,
            generation: snapshot.generations[account.tool] ?? 0,
            model: folderDraft.model, collectTraces: folderDraft.collectTraces)
        if supportsFolderSettings { params = folderDraft.applying(to: params) }
        model.managedAction("managed_launch_prepare", params: params, openTerminal: true)
        onClose()
    }

    private func loadFolderSettings(reset: Bool = false) {
        guard let account = selectedAccount else { return }
        loadedAccountID = accountID
        let cwd = project?.path
        if !supportsFolderSettings {
            folderDraft.useDefaults(tool: account.tool, cwd: cwd,
                cliOptions: model.managedSnapshot?.cliOptions?[account.tool] ?? "")
            return
        }
        let token = folderDraft.begin(tool: account.tool, cwd: cwd)
        model.managedFolderSettings(tool: account.tool, cwd: cwd, reset: reset) { settings in
            if let settings { folderDraft.complete(settings, token: token) }
            else { folderDraft.fail(token: token) }
        }
    }

    /// Read the picker's models for the chosen account's tool, once per
    /// tool. A different account starts again from the default.
    private func loadModels() {
        guard let account = selectedAccount, ManagedSurface.folderModelMode(tool: account.tool, connection: account.connection) == .catalogue else { return }
        let tool = account.tool
        guard catalogueTool != tool else { return }
        catalogueTool = tool
        catalogue = nil
        catalogueFailed = false
        model.managedModelCatalogue(tool: tool) { result in
            guard catalogueTool == tool else { return }
            catalogue = result
            // A failed read is tried again on the next account change.
            if result == nil {
                catalogueFailed = true
                catalogueTool = nil
            }
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

/// Current daemons run the named setup action in Terminal. Older daemons
/// retain the official setup-guide link instead of guessing command support.
struct ManagedInstallLink: View {
    @EnvironmentObject private var model: AppModel
    let tool: ManagedToolAvailability
    let copy: (String) -> String
    var labelKey: String = "install"
    var includeToolName = true
    private var actionLabel: String { copy(labelKey).isEmpty ? copy("install") : copy(labelKey) }
    private var label: String { includeToolName ? actionLabel + " " + copy(tool.tool) : actionLabel }
    private var terminalHint: String {
        ManagedSurface.fillTerminal(
            copy(labelKey == "update" ? "update_in_terminal" : "install_in_terminal"),
            terminal: model.managedSnapshot?.capabilities.terminalDestination, fallback: copy("terminal"))
            .replacingOccurrences(of: "{tool}", with: copy(tool.tool))
    }
    var body: some View {
        if model.managedSnapshot?.capabilities.toolSetup == true {
            Button(label) { model.setupManagedTool(tool.tool, action: labelKey) }
                .accessibilityLabel(actionLabel + " " + copy(tool.tool))
                .help(terminalHint)
                .buttonStyle(GlassButtonStyle(.glass, small: true))
                .disabled(model.managedBusy)
        } else if let url = tool.url {
            Link(label, destination: url)
                .accessibilityLabel(actionLabel + " " + copy(tool.tool))
                .buttonStyle(GlassButtonStyle(.glass, small: true))
        }
    }
}
