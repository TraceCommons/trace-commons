import SwiftUI
import TCDesign
import TCShellCore

/// Shared by the signed-in folder dialog and the saved-account launch dialog.
struct ManagedFolderOptions: View {
    let tool: String
    let connection: String
    @Binding var draft: ManagedFolderDraft
    /// Starring a model applies the daemon's answer to its favorites.
    @Binding var catalogue: ManagedModelCatalogue?
    /// Reads the catalogue again after a star or removal. The caller owns
    /// the read, so an answer for a tool it no longer shows is dropped.
    let refreshCatalogue: () -> Void
    let supportsFolderSettings: Bool
    let retry: () -> Void
    let reset: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var favoriteBusy = false
    /// The copy key of the last refused star or removal; cleared by the
    /// next pick, star or removal, and by a new account or folder.
    @State private var favoriteError: String?

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
                        set: { draft.choose($0.flatMap { $0.isEmpty ? nil : $0 }, in: catalogue); favoriteError = nil }),
                        options: modelOptions, placeholder: model.managedText("model_default_unknown"),
                        menuDots: Self.modelMenuDots)
                    if let chosen = draft.model, ManagedSurface.offersFavorite(chosen, in: catalogue) {
                        favoriteStar(chosen)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // The catalogue may arrive before or after the folder settings:
            // this runs when the fields appear and on every new catalogue.
            .onChange(of: catalogue, initial: true) { _, _ in reconcileModel() }
            // A new account or folder takes these fields away while its
            // settings load; a refused star's line was about the old one.
            .onDisappear { favoriteError = nil }
            if offersBackground {
                backgroundPicker
            }
            ForEach(ManagedSurface.unavailableFavorites(catalogue), id: \.self) { stale in
                unavailableFavoriteRow(stale)
            }
            if let favoriteError {
                Text(model.managedText(favoriteError))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            if draft.keepsSavedModel {
                Text(model.managedText("folder_model_unavailable"))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            if ManagedSurface.showsProjectWorkflows(draft, tool: tool, connection: connection) {
                let line = model.managedText("project_workflows")
                Text(line)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityLabel(line)
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

    /// A model NEAR AI no longer lists falls back to the default, visibly,
    /// for this launch only: the folder keeps its saved model
    /// (`keep_saved_model`). Without a catalogue nothing changes: unknown
    /// is not removed.
    private func reconcileModel() {
        guard ManagedSurface.reconciledModel(draft.model, in: catalogue).fellBack else { return }
        draft.fallBackToDefault(in: catalogue)
    }

    /// Whether Launch may go as far as the model is concerned. A pending
    /// catalogue read holds a saved model, as `modelIsAvailable` does; a
    /// failed one does not, since nothing on the sheet would say why
    /// (#1754 review). The daemon still refuses a model NEAR AI does not
    /// list, with `managed-model-unavailable`.
    static func launchModelReady(_ draft: ManagedFolderDraft, tool: String, connection: String,
                                 catalogue: ManagedModelCatalogue?, catalogueFailed: Bool) -> Bool {
        if catalogueFailed, catalogue == nil,
           ManagedSurface.folderModelMode(tool: tool, connection: connection) == .catalogue {
            return true
        }
        return modelIsAvailable(draft, tool: tool, connection: connection, catalogue: catalogue)
    }

    /// A star or removal shows the setter's answer, the stored list, at
    /// once, then the caller reads the catalogue again; a failed read keeps
    /// what is shown (#1754 review).
    private func favoritesChanged(_ favorites: [String]) {
        favoriteBusy = false
        catalogue = catalogue?.projecting(storedFavorites: favorites)
        refreshCatalogue()
    }

    /// A favorite NEAR AI no longer lists, and its removal.
    private func unavailableFavoriteRow(_ stale: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(model.managedText("model_favorite_unavailable").replacingOccurrences(of: "{model}", with: stale))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer()
            Button(model.managedText("remove")) {
                favoriteBusy = true
                favoriteError = nil
                model.setFavoriteModel(stale, favorite: false) { favorites, label in
                    guard let favorites else {
                        favoriteBusy = false
                        favoriteError = ManagedSurface.favoriteErrorKey(label)
                        return
                    }
                    favoritesChanged(favorites)
                }
            }
            .buttonStyle(GlassButtonStyle(.link))
            .disabled(favoriteBusy)
            .accessibilityLabel(model.managedText("model_favorite_remove_named").replacingOccurrences(of: "{model}", with: stale))
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
            favoriteError = nil
            model.setFavoriteModel(chosen, favorite: !starred) { favorites, label in
                guard let favorites else {
                    favoriteBusy = false
                    favoriteError = ManagedSurface.favoriteErrorKey(label)
                    return
                }
                favoritesChanged(favorites)
            }
        }
        .disabled(favoriteBusy)
        .accessibilityAddTraits(starred ? .isSelected : [])
    }

    private var modelOptions: [GlassPickerOption<String>] {
        Self.pickerOptions(catalogue, chosen: draft.model, copy: model.managedText)
    }

    /// The background picker shows only to a daemon that reads the choice,
    /// for Claude Code and OpenCode on NEAR AI's catalogue.
    private var offersBackground: Bool {
        ManagedSurface.offersBackgroundModel(tool: tool, connection: connection)
            && model.managedSnapshot?.capabilities.backgroundModel == true
    }

    /// The model for background work, under the model picker: its title,
    /// then a picker of the models no less private than the main model.
    private var backgroundPicker: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(model.managedText("background_model_title"))
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textTertiary)
            GlassPicker(model.managedText("background_model_choice"), selection: Binding(
                get: { draft.backgroundModel ?? ManagedSurface.defaultModel },
                set: { draft.chooseBackground($0.flatMap { $0.isEmpty ? nil : $0 }) }),
                options: Self.backgroundPickerOptions(catalogue, main: draft.backgroundMain(in: catalogue),
                                                      chosen: draft.backgroundModel, copy: model.managedText),
                placeholder: model.managedText("model_default_unknown"),
                menuDots: Self.modelMenuDots)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // A saved choice the folder's main model no longer allows goes back
        // to the default once the catalogue says so.
        .onChange(of: catalogue, initial: true) { _, _ in draft.reconcileBackground(in: catalogue) }
        // The model the CLI options name runs instead of the picked one, so
        // the core reads them and the choice is weighed against that model.
        .task(id: draft.cliOptions) {
            let options = draft.cliOptions
            model.cliModel(tool: tool, options: options) { named in
                draft.setCLIModel(named, for: options, in: catalogue)
            }
        }
    }

    /// The model and background pickers draw each option's tier dot in the
    /// open menu too; other pickers keep their plain menus (#1798 review).
    static let modelMenuDots = true

    /// The background picker's options, with tiers shown as the model
    /// picker shows them (`pickerOptions`). A chosen model with no
    /// catalogue to weigh it against is kept, with no tier.
    static func backgroundPickerOptions(_ catalogue: ManagedModelCatalogue?, main: String?, chosen: String?,
                                        copy: (String) -> String) -> [GlassPickerOption<String>] {
        // A chosen model is listed only when there is no catalogue to weigh
        // it against; with one, the core's list is the whole list.
        glassOptions(ManagedSurface.backgroundOptions(catalogue, main: main, copy: copy),
                     chosen: catalogue == nil ? chosen : nil)
    }

    /// The dot a model's privacy takes (owner, 2026-10-31): NEAR AI private
    /// is on, External is ask, Incognito is outside. The core reads the
    /// tier (`ManagedSurface.modelPrivacy`); this is only the palette.
    static func tierDot(_ privacy: ManagedModelPrivacy?) -> GlassStatus? {
        switch privacy {
        case .nearAIPrivate: .on
        case .externalPrivate: .ask
        case .incognito: .outside
        case nil: nil
        }
    }

    /// The picker's options with every tiered model's tier, and the default
    /// row's, shown by its dot and its tier's existing group heading. A
    /// chosen model the catalogue does not list is kept, with no tier.
    static func pickerOptions(_ catalogue: ManagedModelCatalogue?, chosen: String?, copy: (String) -> String) -> [GlassPickerOption<String>] {
        glassOptions(ManagedSurface.modelOptions(catalogue, copy: copy), chosen: chosen)
    }

    /// Core options as glass picker options, each tier shown by its dot
    /// and heading, plus `chosen` when the options lack it.
    private static func glassOptions(_ options: [ManagedModelOption], chosen: String?) -> [GlassPickerOption<String>] {
        var values = options
            .map { GlassPickerOption($0.title, value: $0.value, dot: tierDot($0.privacy), section: $0.section, detail: $0.tierHeading, help: $0.help) }
        if let chosen, !values.contains(where: { $0.value == chosen }) {
            values.append(GlassPickerOption(chosen, value: chosen))
        }
        return values
    }
}
