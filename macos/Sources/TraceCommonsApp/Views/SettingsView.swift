import SwiftUI

/// What this machine is doing, and what permissions traces carry.
///
/// The consent list comes from `consent_options`, never hardcoded, and
/// nothing optional is pre-checked. `public_attribution` is visually
/// separated because it grants no data use at all.
struct SettingsView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                connection
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

    private var projects: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Projects").font(.headline)
            if model.projects.isEmpty {
                Text("No projects seen yet.").font(.callout).foregroundStyle(.secondary)
            } else {
                ForEach(model.projects) { project in
                    HStack {
                        Text(project.projectLabel)
                        Spacer()
                        Text(modeSentence(project.mode)).foregroundStyle(.secondary)
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
