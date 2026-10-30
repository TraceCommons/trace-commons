import SwiftUI
import TCDesign
import TCShellCore

/// Shared by the signed-in folder dialog and the saved-account launch dialog.
struct ManagedFolderOptions: View {
    let tool: String
    let connection: String
    @Binding var draft: ManagedFolderDraft
    /// Starring a model replaces its `favorites` with the daemon's answer.
    @Binding var catalogue: ManagedModelCatalogue?
    let supportsFolderSettings: Bool
    let retry: () -> Void
    let reset: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var favoriteBusy = false
    @State private var favoriteRefused = false

    static func modelIsAvailable(_ draft: ManagedFolderDraft, tool: String, connection: String,
                                 catalogue: ManagedModelCatalogue?) -> Bool {
        switch ManagedSurface.folderModelMode(tool: tool, connection: connection) {
        case .none: return true
        case .text: return draft.model?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
        case .catalogue:
            guard let chosen = draft.model else { return true }
            return catalogue?.models.contains { $0.id == chosen } == true
        }
    }

    var body: some View {
        switch draft.phase {
        case .idle: EmptyView()
        case .loading:
            Text(model.managedText("folder_settings_loading"))
                .glassType(GlassTokens.TypeScale.caption)
        case .failed:
            Text(model.managedText("folder_settings_failed"))
                .glassType(GlassTokens.TypeScale.caption)
            Button(model.managedText("retry"), action: retry).buttonStyle(GlassButtonStyle(.link))
        case .ready:
            fields
        }
    }

    @ViewBuilder
    private var fields: some View {
        let mode = ManagedSurface.folderModelMode(tool: tool, connection: connection)
        if mode == .catalogue {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(model.managedText("model_choice"))
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                HStack(spacing: GlassTokens.Space.s2) {
                    GlassPicker(model.managedText("model_choice"), selection: Binding(
                        get: { draft.model ?? ManagedSurface.defaultModel },
                        set: { draft.model = $0.flatMap { $0.isEmpty ? nil : $0 }; favoriteRefused = false }),
                        options: modelOptions, placeholder: model.managedText("model_default_unknown"))
                    if let chosen = draft.model, ManagedSurface.offersFavorite(chosen, in: catalogue) {
                        favoriteStar(chosen)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if favoriteRefused {
                Text(model.managedText("model_unavailable"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            if !Self.modelIsAvailable(draft, tool: tool, connection: connection, catalogue: catalogue) {
                Text(model.managedText("folder_model_unavailable"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        } else if mode == .text {
            GlassTextField(model.managedText("model"), text: Binding(
                get: { draft.model ?? "" }, set: { draft.model = $0.isEmpty ? nil : $0 }))
        }
        if supportsFolderSettings {
            GlassTextField(model.managedText("cli_options_label"), text: $draft.cliOptions)
            Toggle(model.managedText("collect_managed_trace"), isOn: $draft.collectTraces)
                .toggleStyle(GlassToggleStyle(.settings))
            Text(model.managedText("managed_collection_scope"))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(alignment: .firstTextBaseline) {
                Text(model.managedText("folder_settings_scope"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer()
                if draft.remembered {
                    Button(model.managedText("folder_settings_reset"), action: reset)
                        .buttonStyle(GlassButtonStyle(.link))
                        .accessibilityLabel(model.managedText("folder_settings_reset_accessibility")
                            .replacingOccurrences(of: "{tool}", with: model.managedText(tool)))
                }
            }
        } else if tool == "opencode" {
            Toggle(model.managedText("collect_traces"), isOn: $draft.collectTraces)
                .toggleStyle(GlassToggleStyle(.settings))
            Text(model.managedText("collection_scope")).glassType(GlassTokens.TypeScale.caption)
        }
    }

    /// The star for the chosen model: filled while it is a favorite.
    private func favoriteStar(_ chosen: String) -> some View {
        let starred = ManagedSurface.isFavoriteModel(chosen, in: catalogue)
        return GlassRoundButton(
            model.managedText(starred ? "model_favorite_remove" : "model_favorite_add"),
            systemImage: starred ? "star.fill" : "star", small: true
        ) {
            favoriteBusy = true
            favoriteRefused = false
            model.setFavoriteModel(chosen, favorite: !starred) { favorites in
                favoriteBusy = false
                if let favorites {
                    catalogue = catalogue?.withFavorites(favorites)
                } else {
                    favoriteRefused = true
                }
            }
        }
        .disabled(favoriteBusy)
        .accessibilityAddTraits(starred ? .isSelected : [])
    }

    private var modelOptions: [GlassPickerOption<String>] {
        var values = ManagedSurface.modelOptions(catalogue, copy: model.managedText)
            .map { GlassPickerOption($0.title, value: $0.value) }
        if let chosen = draft.model, !values.contains(where: { $0.value == chosen }) {
            values.append(GlassPickerOption(chosen, value: chosen))
        }
        return values
    }
}
