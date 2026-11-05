import SwiftUI
import TCDesign
import TCShellCore

/// An ordinary tool row inside Settings' existing Private AI tools panel.
/// Older/unavailable daemons provide no copy and therefore draw no row.
struct ClaudeDesktopSettingsRow: View {
    let closeLabel: String
    @EnvironmentObject private var model: AppModel
    @State private var setup: ClaudeDesktopSetupModel?

    var body: some View {
        Group {
            if let setup, let status = setup.status {
                ClaudeDesktopToolRow(setup: setup, status: status)
            }
        }
        .task {
            var namedModel: String?
            while !Task.isCancelled {
                if setup == nil { setup = model.makeClaudeDesktopSetup() }
                if let setup {
                    await setup.refresh()
                    if let id = setup.status?.model, id != namedModel {
                        namedModel = id
                        model.managedModelCatalogue(tool: "claude", cacheOnly: true) { setup.readCachedCatalogue($0) }
                    }
                }
                do { try await Task.sleep(for: .seconds(5)) }
                catch { break }
            }
        }
        .onDisappear { setup?.close(); setup = nil }
        .glassModal(isPresented: Binding(
            get: { setup?.isPresented == true },
            set: { if !$0 { setup?.close() } })) {
                if let setup, setup.status != nil {
                    ClaudeDesktopSetupSheet(setup: setup, closeLabel: closeLabel)
                }
            }
    }

}

/// The same flat row, separated from its polling lifecycle for render QA.
struct ClaudeDesktopToolRow: View {
    let setup: ClaudeDesktopSetupModel
    let status: ClaudeDesktopStatus

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .top, spacing: 18) {
                details(setup, status: status)
                Spacer(minLength: 0)
                actions(setup, status: status)
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                details(setup, status: status)
                actions(setup, status: status)
            }
        }
        .padding(.vertical, 15)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .bottom) { GlassHairline(GlassColor.hairline) }
    }

    private func details(_ setup: ClaudeDesktopSetupModel, status: ClaudeDesktopStatus) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(status.text("title"))
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
            Text(status.statusLine)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
            if let id = status.model {
                Text(ManagedSurface.rowModel(id, names: setup.modelNames))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .help(id)
            }
            if setup.failed && !setup.isPresented {
                Text(status.text("error"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func actions(_ setup: ClaudeDesktopSetupModel, status: ClaudeDesktopStatus) -> some View {
        HStack(spacing: GlassTokens.Space.s3) {
            Button(status.text("setup")) { setup.open() }
                .buttonStyle(GlassButtonStyle(.glass))
                .disabled(setup.busy)
            if status.canDisconnect {
                Button(status.text("disconnect")) { Task { await setup.disconnect() } }
                    .buttonStyle(GlassButtonStyle(.destructive))
                    .disabled(setup.busy)
            }
        }
        .fixedSize()
    }
}

/// Model choice and the explicit environment export share one form.
/// All prose is the daemon's copy; picker words reuse managed model copy.
struct ClaudeDesktopSetupSheet: View {
    let setup: ClaudeDesktopSetupModel
    let closeLabel: String
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if let status = setup.status {
            GlassModal(title: status.text("title"), width: .regular, actions: [
                .cancel(closeLabel) { setup.close() },
                GlassModalAction(status.text("copy"), isEnabled: setup.canCopy) {
                    Task { await setup.copyEnvironment() }
                },
                GlassModalAction(status.text("save"), isDefault: true, isEnabled: setup.canSave) {
                    Task { await setup.save() }
                },
            ], busy: setup.busy, onCancel: { setup.close() }) {
                GlassModalBody {
                    form(status)
                }
            }
            .onAppear { refreshCatalogue() }
        }
    }

    private func form(_ status: ClaudeDesktopStatus) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            Text(status.statusLine)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
            modelPicker(status)
            backgroundPicker(status)
            Text(status.text("instructions"))
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if let environment = setup.environment {
                Text(verbatim: environment)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if setup.failed {
                Text(status.text("error"))
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func modelPicker(_ status: ClaudeDesktopStatus) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            HStack {
                Text(status.text("main_model"))
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                Spacer(minLength: 0)
                GlassRefreshButton(model.managedText("refresh"), action: refreshCatalogue)
                    .disabled(setup.busy || setup.catalogueBusy)
            }
            GlassPicker(status.text("main_model"), selection: Binding(
                get: { Optional(setup.selectedModel) },
                set: { setup.chooseModel($0 ?? "") }),
                options: ManagedFolderOptions.pickerOptions(setup.catalogue, chosen: setup.selectedModel,
                                                           copy: model.managedText),
                placeholder: model.managedText("model_default_unknown"),
                menuDots: ManagedFolderOptions.modelMenuDots)
                .disabled(setup.busy || !status.canConfigure)
        }
    }

    private func backgroundPicker(_ status: ClaudeDesktopStatus) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(status.text("background_model"))
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textTertiary)
            GlassPicker(status.text("background_model"), selection: Binding(
                get: { Optional(setup.selectedBackgroundModel) },
                set: { setup.chooseBackground($0 ?? "") }),
                options: ManagedFolderOptions.backgroundPickerOptions(
                    setup.catalogue, main: setup.resolvedModel, chosen: setup.selectedBackgroundModel,
                    copy: model.managedText),
                placeholder: model.managedText("model_default_unknown"),
                menuDots: ManagedFolderOptions.modelMenuDots)
                .disabled(setup.busy || !status.canConfigure)
        }
    }

    private func refreshCatalogue() {
        setup.loadCatalogue { done in model.managedModelCatalogue(tool: "claude", done: done) }
    }
}
