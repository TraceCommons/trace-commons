import SwiftUI

/// What this machine is doing, and what permissions traces carry.
///
/// The consent list comes from `consent_options`, never hardcoded, and
/// nothing optional is pre-checked. `public_attribution` is visually
/// separated because it grants no data use at all.
struct SettingsView: View {
    @EnvironmentObject private var model: AppModel

    // Read fresh on appear rather than cached across the view's lifetime:
    // the user can flip this in System Settings -> General -> Login Items
    // while this window is open, and a value captured once at init would
    // then claim a state that is no longer true. See `LoginItemManager`.
    @State private var loginItemState: LoginItemManager.State = LoginItemManager.currentState
    @State private var loginItemActionError: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                connection
                Divider()
                loginItem
                Divider()
                consent
                Divider()
                watching
                Divider()
                projects
            }
            .padding(18)
            .frame(maxWidth: 620, alignment: .leading)
        }
        .onAppear { loginItemState = LoginItemManager.currentState }
    }

    private var connection: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Connection").font(.headline)
            if model.status.loggedIn {
                Text("Connected.").font(.callout)
            } else {
                Text("Not connected. Sessions are being queued, but nothing can be sent.")
                    .font(.callout)
            }
            if let settings = model.daemonSettings {
                // Booleans only. The contract reports the credential and both
                // session roots as configured-or-not, and this view has
                // nowhere to put a value even if it were sent one.
                checkRow("Claude Code sessions folder set", settings.claudeRootConfigured)
                checkRow("Codex sessions folder set", settings.codexRootConfigured)
                checkRow("Extra privacy scan configured", settings.nearAIConfigured)
            }
        }
    }

    /// Reflects the live `SMAppService.mainApp.status`, not a locally cached
    /// bool -- see `loginItemState`'s doc comment. `.requiresApproval` is
    /// rendered as guidance, not an error: it is the normal result of the
    /// user not yet approving the app in System Settings, or having denied
    /// it there, and retrying `register()` again would not change that.
    private var loginItem: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Startup").font(.headline)
            switch loginItemState {
            case .enabled:
                Toggle("Start Trace Commons when you log in", isOn: Binding(
                    get: { true },
                    set: { newValue in if !newValue { setLoginItem(enabled: false) } }
                ))
                .font(.callout)
            case .notRegistered, .notFound:
                Toggle("Start Trace Commons when you log in", isOn: Binding(
                    get: { false },
                    set: { newValue in if newValue { setLoginItem(enabled: true) } }
                ))
                .font(.callout)
            case .requiresApproval:
                Text("Waiting on approval in System Settings.")
                    .font(.callout)
                Text("""
                Turn it on in System Settings -> General -> Login Items to let \
                Trace Commons start automatically.
                """)
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            if let loginItemActionError {
                Text(loginItemActionError)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func setLoginItem(enabled: Bool) {
        loginItemActionError = nil
        if enabled {
            switch LoginItemManager.register() {
            case .enabled, .requiresApproval:
                break
            case .failed(let message):
                loginItemActionError = "Couldn't turn this on: \(message)"
            }
        } else {
            if case .failed(let message) = LoginItemManager.unregister() {
                loginItemActionError = "Couldn't turn this off: \(message)"
            }
        }
        loginItemState = LoginItemManager.currentState
    }

    private var consent: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("How may your traces be used?").font(.headline)
            Text("Applies to traces you send from now on.")
                .font(.callout)
                .foregroundStyle(.secondary)

            let granted = Set(model.status.consentScopes)
            let alwaysOn = model.consentScopes.filter(\.alwaysOn)
            let optional = model.consentScopes.filter { !$0.alwaysOn && $0.grantsDataUse }
            let nonDataUse = model.consentScopes.filter { !$0.alwaysOn && !$0.grantsDataUse }

            if !alwaysOn.isEmpty {
                Text("Always included").font(.subheadline.weight(.semibold))
                ForEach(alwaysOn) { scope in
                    scopeRow(scope, checked: true, alwaysOn: true)
                }
            }
            if !optional.isEmpty {
                Text("Optional — each one lets your traces do more")
                    .font(.subheadline.weight(.semibold))
                ForEach(optional) { scope in
                    scopeRow(scope, checked: granted.contains(scope.name), alwaysOn: false)
                }
            }
            if !nonDataUse.isEmpty {
                // Visually separated: it grants no data use at all, and
                // listing it beside four real scopes misleads both ways.
                Divider().frame(maxWidth: 320)
                Text("Credit").font(.subheadline.weight(.semibold))
                ForEach(nonDataUse) { scope in
                    scopeRow(scope, checked: granted.contains(scope.name), alwaysOn: false)
                }
            }
            Text("""
            Changing permissions needs an enrolled account, which this build does \
            not set up yet, so these show what is in force rather than offering to \
            change it. Nothing here is pre-selected on your behalf.
            """)
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    private func scopeRow(_ scope: ConsentScope, checked: Bool, alwaysOn: Bool) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: checked ? "checkmark.square" : "square")
                .foregroundStyle(checked ? .primary : .secondary)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(ScopeCopy.title(for: scope.name, options: model.consentScopes))
                        .font(.callout.weight(.semibold))
                    if alwaysOn {
                        Text("always on").font(.caption).foregroundStyle(.secondary)
                    }
                }
                Text(scope.description).font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    private var watching: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Watching").font(.headline)
            if let settings = model.daemonSettings {
                Text("A session counts as finished after \(settings.quiescenceSecs) seconds of quiet.")
                    .font(.callout)
                Text("At most one notification every \(settings.digestIntervalSecs / 3600) hours, and none when nothing is waiting.")
                    .font(.callout)
                Text("Undecided sessions are dropped after \(settings.queueTtlDays) days. Dropped means never sent.")
                    .font(.callout)
                checkRow("Notifications rendered by this app", !settings.localNotifications)
            }
            if model.status.paused {
                Text("Paused. Nothing is being queued or sent.").font(.callout)
            }
        }
    }

    // Only `ask` <-> `ignore` is offered here, same as onboarding screen 5:
    // arming `auto_upload` outside a deliberate confirmation flow is still
    // not built. Changing a mind about "ignore" -- set during onboarding or
    // never revisited since -- should not require a terminal, so that half
    // is wired to `AppModel.setProjectMode`, the same real `set_project_mode`
    // call the onboarding screen uses. See `DaemonClient.setProjectMode` for
    // why that call is expected to fail with `project-key-unrecognized` for
    // every real project today; this view surfaces that the same way
    // onboarding does rather than pretending the toggle always lands.
    private var projects: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Projects").font(.headline)
            if let error = model.lastActionError {
                Text(error).font(.callout).foregroundStyle(.secondary)
            }
            if model.projects.isEmpty {
                Text("No projects seen yet.").font(.callout).foregroundStyle(.secondary)
            } else {
                ForEach(model.projects) { project in
                    HStack {
                        Text(project.projectLabel)
                        Spacer()
                        Text(modeSentence(project.mode)).foregroundStyle(.secondary)
                        if project.mode == .ask || project.mode == .ignore {
                            Button(project.mode == .ignore ? "Ask again" : "Ignore") {
                                model.setProjectMode(
                                    project,
                                    mode: project.mode == .ignore ? .ask : .ignore
                                )
                            }
                            .buttonStyle(.bordered)
                        }
                    }
                    .font(.callout)
                }
                Text("""
                Arming a project so it contributes without asking is a deliberate \
                confirmation flow, and it is not built yet.
                """)
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
    }

    private func modeSentence(_ mode: ProjectMode) -> String {
        switch mode {
        case .ask: return "Asks you first"
        case .autoUpload: return "Contributed without asking"
        case .ignore: return "Never offered"
        }
    }

    private func checkRow(_ title: String, _ value: Bool) -> some View {
        HStack(spacing: 8) {
            Image(systemName: value ? "checkmark.circle" : "circle")
                .foregroundStyle(value ? .primary : .secondary)
            Text(title).font(.callout)
        }
    }
}
