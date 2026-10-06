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
                    }
                }
            }
        }
        .sheet(isPresented: $adding) { ManagedAccountSheet().environmentObject(model) }
        .sheet(isPresented: $launching) { ManagedLaunchSheet().environmentObject(model) }
        .sheet(item: $renaming) { account in ManagedRenameSheet(account: account).environmentObject(model) }
        .sheet(item: $removing) { account in ManagedRemoveSheet(account: account).environmentObject(model) }
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
        GlassEyebrowCard(model.managedText("accounts_title"), accessory: {
            if let snapshot = model.managedSnapshot { headerActions(snapshot) }
        }) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                Text(model.managedText("description"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let snapshot = model.managedSnapshot {
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
                            Label(model.managedText("retry"), systemImage: "arrow.clockwise")
                        }
                        .buttonStyle(GlassButtonStyle(.glass, small: true))
                    }
                }
            }
        }
    }

    private func headerActions(_ snapshot: ManagedSnapshot) -> some View {
        HStack(spacing: GlassTokens.Space.s3) {
            Button(model.managedText("refresh")) { model.refreshManagedSessions() }
                .buttonStyle(GlassButtonStyle(.glass, small: true))
            Button(model.managedText("add")) { adding = true }
                .buttonStyle(GlassButtonStyle(.glass, small: true))
            Button(model.managedText("launch")) { launching = true }
                .buttonStyle(GlassButtonStyle(.primary, small: true))
                .disabled(snapshot.accounts.isEmpty || !snapshot.capabilities.terminalLaunch)
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

/// The one frame every managed dialog uses: a glass sheet with its buttons
/// on the right. When `GlassModal` lands (#1241) this is the only view to
/// change.
struct ManagedDialog<Content: View, Footer: View>: View {
    private let title: String
    private let content: Content
    private let footer: Footer

    init(_ title: String, @ViewBuilder content: () -> Content, @ViewBuilder footer: () -> Footer) {
        self.title = title
        self.content = content()
        self.footer = footer()
    }

    var body: some View {
        GlassSheet(title: title) {
            content
            HStack(spacing: GlassTokens.Space.s3) {
                Spacer(minLength: 0)
                footer
            }
        }
        .frame(width: 460)
    }
}

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

/// The key field: `GlassTextField`'s inset well, secure.
private struct ManagedSecureField: View {
    let label: String
    @Binding var text: String

    var body: some View {
        ManagedField(label: label) {
            SecureField(label, text: $text)
                .textFieldStyle(.plain)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textPrimary)
                .padding(.horizontal, 10)
                .frame(minHeight: GlassTokens.Size.controlLarge)
                .background(
                    RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                        .fill(GlassTokens.Color.fieldFill.color)
                )
                .labelsHidden()
        }
    }
}

struct ManagedAccountSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var tool = "claude"
    @State private var connection = "subscription"
    @State private var label = ""
    @State private var key = ""

    private var subscription: Bool { connection == "subscription" }

    var body: some View {
        ManagedDialog(model.managedText("save_title")) {
            HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
                ManagedField(label: model.managedText("tool")) {
                    GlassPicker(
                        model.managedText("tool"),
                        selection: Binding(get: { tool }, set: { if let value = $0 { tool = value } }),
                        options: ["claude", "codex"].map { GlassPickerOption(model.managedText($0), value: $0) },
                        placeholder: model.managedText("tool"))
                }
                ManagedField(label: model.managedText("connection")) {
                    GlassPicker(
                        model.managedText("connection"),
                        selection: Binding(get: { connection }, set: { if let value = $0 { connection = value } }),
                        options: ["subscription", "api_key", "near_ai"].map { GlassPickerOption(model.managedText($0), value: $0) },
                        placeholder: model.managedText("connection"))
                }
            }
            GlassTextField(model.managedText("label"), text: $label, prompt: model.managedText("label_placeholder"))
            if subscription {
                note(model.managedText("login_description"))
            } else {
                ManagedSecureField(label: model.managedText("api_key"), text: $key)
                note(model.managedText("key_storage"))
            }
        } footer: {
            Button(model.managedText("cancel"), role: .cancel) { dismiss() }
                .buttonStyle(GlassButtonStyle(.glass))
                .keyboardShortcut(.cancelAction)
            Button(subscription ? model.managedText("save_login") : model.managedText("save_account")) {
                model.managedAction("managed_account_add", params: ["tool": tool, "connection": connection, "label": label], openTerminal: subscription, newAccountKey: subscription ? nil : key)
                key = ""
                dismiss()
            }
            .buttonStyle(GlassButtonStyle(.primary))
            .keyboardShortcut(.defaultAction)
            .disabled(label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (!subscription && key.isEmpty) || (subscription && model.managedSnapshot?.capabilities.terminalLaunch != true))
        }
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

struct ManagedLaunchSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var accountID = ""
    @State private var project: URL?

    var body: some View {
        ManagedDialog(model.managedText("launch")) {
            ManagedField(label: model.managedText("saved_account")) {
                GlassPicker(
                    model.managedText("saved_account"),
                    selection: Binding(get: { accountID.isEmpty ? nil : accountID }, set: { accountID = $0 ?? "" }),
                    options: (model.managedSnapshot?.accounts ?? []).map { account in
                        GlassPickerOption(
                            [ManagedWords.tool(account.tool, model: model), account.label, model.managedText(account.connection)]
                                .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "),
                            value: account.id)
                    },
                    placeholder: model.managedText("choose_account"))
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
                    let panel = NSOpenPanel()
                    panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.allowsMultipleSelection = false
                    if panel.runModal() == .OK { project = panel.url }
                }
            }
            Text(model.managedText("launch_scope").replacingOccurrences(of: "{destination}", with: model.managedSnapshot?.capabilities.terminalDestination ?? model.managedText("terminal")))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        } footer: {
            Button(model.managedText("cancel"), role: .cancel) { dismiss() }
                .buttonStyle(GlassButtonStyle(.glass))
                .keyboardShortcut(.cancelAction)
            Button(model.managedText("launch_short")) { launch() }
                .buttonStyle(GlassButtonStyle(.primary))
                .keyboardShortcut(.defaultAction)
                .disabled(accountID.isEmpty || project == nil || model.managedBusy)
        }
    }

    private func launch() {
        guard let snapshot = model.managedSnapshot, let account = snapshot.accounts.first(where: { $0.id == accountID }), let project else { return }
        model.managedAction("managed_launch_prepare", params: ["request_id": UUID().uuidString, "purpose": "coding", "tool": account.tool, "connection": account.connection, "account_id": account.id, "cwd": project.path, "expected_generation": snapshot.generations[account.tool] ?? 0, "save_default": false], openTerminal: true)
        dismiss()
    }
}

struct ManagedRenameSheet: View {
    let account: ManagedAccount
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var label: String

    init(account: ManagedAccount) {
        self.account = account
        _label = State(initialValue: account.label)
    }

    var body: some View {
        ManagedDialog(model.managedText("rename")) {
            GlassTextField(model.managedText("label"), text: $label, prompt: model.managedText("label_placeholder"))
        } footer: {
            Button(model.managedText("cancel"), role: .cancel) { dismiss() }
                .buttonStyle(GlassButtonStyle(.glass))
                .keyboardShortcut(.cancelAction)
            Button(model.managedText("save")) {
                model.managedAction("managed_account_rename", params: ["account_id": account.id, "label": label])
                dismiss()
            }
            .buttonStyle(GlassButtonStyle(.primary))
            .keyboardShortcut(.defaultAction)
        }
    }
}

/// The removal confirmation: cancel first, the destructive action on the right.
struct ManagedRemoveSheet: View {
    let account: ManagedAccount
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        ManagedDialog(model.managedText("remove_question")) {
            Text(model.managedText("remove_description"))
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        } footer: {
            Button(model.managedText("cancel"), role: .cancel) { dismiss() }
                .buttonStyle(GlassButtonStyle(.glass))
                .keyboardShortcut(.cancelAction)
            Button(model.managedText("remove"), role: .destructive) {
                model.managedAction("managed_account_remove", params: ["account_id": account.id])
                dismiss()
            }
            .buttonStyle(GlassButtonStyle(.primary))
        }
    }
}
