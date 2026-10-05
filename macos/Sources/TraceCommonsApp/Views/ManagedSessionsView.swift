import SwiftUI
import AppKit
import TCShellCore

struct ManagedSessionsSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var adding = false
    @State private var launching = false
    @State private var renaming: ManagedAccount?
    @State private var renameLabel = ""
    @State private var removing: ManagedAccount?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(model.managedText("title")).font(.title2)
            Text(model.managedText("description"))
                .foregroundStyle(.secondary)
            if let snapshot = model.managedSnapshot {
                HStack {
                    Button(model.managedText("launch")) { launching = true }
                        .disabled(snapshot.accounts.isEmpty || !snapshot.capabilities.terminalLaunch)
                    Button(model.managedText("add")) { adding = true }
                    Button(model.managedText("refresh")) { model.refreshManagedSessions() }
                }.disabled(model.managedBusy)
                if let destination = snapshot.capabilities.terminalDestination {
                    Text(model.managedText("terminal_scope").replacingOccurrences(of: "{destination}", with: destination))
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    Text(model.managedText("terminal_unavailable"))
                        .font(.caption).foregroundStyle(.secondary)
                }
                ForEach(snapshot.accounts) { account in
                    accountRow(account, snapshot: snapshot)
                }
                if snapshot.sessions.isEmpty {
                    Text(model.managedText("empty")).foregroundStyle(.secondary)
                }
                ForEach(snapshot.sessions) { session in
                    HStack(alignment: .top) {
                        VStack(alignment: .leading) {
                            Text("\(session.purpose == "login" ? model.managedText("sign_in") : session.projectLabel) · \(session.tool.capitalized)")
                            Text("\(session.accountLabel) · \(model.managedText(session.connection)) · \(session.state.capitalized)")
                                .foregroundStyle(.secondary)
                            if let code = session.exitCode { Text(model.managedText("exit_code").replacingOccurrences(of: "{code}", with: String(code))).font(.caption) }
                        }
                        Spacer()
                        if !session.holdsAccount {
                            Button(model.managedText("dismiss")) { model.managedAction("managed_session_dismiss", params: ["session_id": session.id]) }
                        }
                    }.padding(.vertical, 4)
                }
            } else {
                Text(model.managedText("connecting")).foregroundStyle(.secondary)
                Button(model.managedText("retry")) { model.refreshManagedSessions() }
            }
            if let error = model.managedError { Text(error).foregroundStyle(.red).textSelection(.enabled) }
        }
        .sheet(isPresented: $adding) { ManagedAccountSheet().environmentObject(model) }
        .sheet(isPresented: $launching) { ManagedLaunchSheet().environmentObject(model) }
        .alert(model.managedText("rename"), isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField(model.managedText("label"), text: $renameLabel)
            Button(model.managedText("save")) {
                if let account = renaming { model.managedAction("managed_account_rename", params: ["account_id": account.id, "label": renameLabel]) }
                renaming = nil
            }
            Button(model.managedText("cancel"), role: .cancel) { renaming = nil }
        }
        .confirmationDialog(model.managedText("remove_question"), isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            Button(model.managedText("remove"), role: .destructive) {
                if let account = removing { model.managedAction("managed_account_remove", params: ["account_id": account.id]) }
                removing = nil
            }
        } message: { Text(model.managedText("remove_description")) }
        .task {
            while !Task.isCancelled {
                model.refreshManagedSessions()
                try? await Task.sleep(for: .seconds(10))
            }
        }
    }

    private func accountRow(_ account: ManagedAccount, snapshot: ManagedSnapshot) -> some View {
        let held = snapshot.sessions.contains { $0.accountID == account.id && $0.holdsAccount }
        let selected = snapshot.defaults.contains { $0.accountID == account.id }
        return HStack {
            VStack(alignment: .leading) {
                Text("\(account.label) · \(account.tool.capitalized)")
                Text("\(model.managedText(account.connection)) · \(account.authState.replacingOccurrences(of: "_", with: " "))\(selected ? " · \(model.managedText("default"))" : "")")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Menu(model.managedText("manage")) {
                Button(model.managedText("use_default")) {
                    let generation = snapshot.generations[account.tool] ?? 0
                    model.managedAction("managed_select", params: ["selection": ["tool": account.tool, "connection": account.connection, "account_id": account.id, "generation": generation], "expected_generation": generation])
                }
                Button(model.managedText("rename_short")) { renameLabel = account.label; renaming = account }
                if account.connection == "subscription" {
                    Button(model.managedText("reconnect")) {
                        model.managedAction("managed_account_reconnect", params: ["account_id": account.id], openTerminal: true)
                    }.disabled(held || !snapshot.capabilities.terminalLaunch)
                }
                Button(model.managedText("verify")) { model.managedAction("managed_account_verify", params: ["account_id": account.id]) }
                Button(model.managedText("remove"), role: .destructive) { removing = account }.disabled(held)
            }.disabled(model.managedBusy)
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
    var body: some View {
        Form {
            Text(model.managedText("save_title")).font(.title2)
            Picker(model.managedText("tool"), selection: $tool) { Text(model.managedText("claude")).tag("claude"); Text(model.managedText("codex")).tag("codex") }
            Picker(model.managedText("connection"), selection: $connection) {
                Text(model.managedText("subscription")).tag("subscription"); Text(model.managedText("api_key")).tag("api_key"); Text(model.managedText("near_ai")).tag("near_ai")
            }
            TextField(model.managedText("label"), text: $label, prompt: Text(model.managedText("label_placeholder")))
            if connection == "subscription" {
                Text(model.managedText("login_description"))
            } else {
                SecureField("API key", text: $key)
                Text(model.managedText("key_storage"))
            }
            HStack {
                Button(model.managedText("cancel"), role: .cancel) { dismiss() }
                Button(connection == "subscription" ? model.managedText("save_login") : model.managedText("save_account")) {
                    model.managedAction("managed_account_add", params: ["tool": tool, "connection": connection, "label": label], openTerminal: connection == "subscription", newAccountKey: connection == "subscription" ? nil : key)
                    key = ""
                    dismiss()
                }.disabled(label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (connection != "subscription" && key.isEmpty) || (connection == "subscription" && model.managedSnapshot?.capabilities.terminalLaunch != true))
            }
        }.padding(24).frame(minWidth: 440)
    }
}

struct ManagedLaunchSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var accountID = ""
    @State private var project: URL?
    var body: some View {
        Form {
            Text(model.managedText("launch")).font(.title2)
            Picker(model.managedText("saved_account"), selection: $accountID) {
                Text(model.managedText("choose_account")).tag("")
                ForEach(model.managedSnapshot?.accounts ?? []) { account in
                    Text("\(account.tool.capitalized) · \(account.label) · \(model.managedText(account.connection))").tag(account.id)
                }
            }
            HStack {
                Text(project?.path ?? model.managedText("project_placeholder")).lineLimit(2).textSelection(.enabled)
                Button(model.managedText("choose_folder")) {
                    let panel = NSOpenPanel()
                    panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.allowsMultipleSelection = false
                    if panel.runModal() == .OK { project = panel.url }
                }
            }
            Text(model.managedText("launch_scope").replacingOccurrences(of: "{destination}", with: model.managedSnapshot?.capabilities.terminalDestination ?? model.managedText("terminal")))
                .foregroundStyle(.secondary)
            HStack {
                Button(model.managedText("cancel"), role: .cancel) { dismiss() }
                Button(model.managedText("launch_short")) { launch() }.disabled(accountID.isEmpty || project == nil || model.managedBusy)
            }
        }.padding(24).frame(minWidth: 480)
    }
    private func launch() {
        guard let snapshot = model.managedSnapshot, let account = snapshot.accounts.first(where: { $0.id == accountID }), let project else { return }
        model.managedAction("managed_launch_prepare", params: ["request_id": UUID().uuidString, "purpose": "coding", "tool": account.tool, "connection": account.connection, "account_id": account.id, "cwd": project.path, "expected_generation": snapshot.generations[account.tool] ?? 0, "save_default": false], openTerminal: true)
        dismiss()
    }
}
