import SwiftUI
import TCDesign
import TCShellCore

/// "Favorite models": every model the person starred beside the Launch
/// sheet's picker, each with Remove. One NEAR AI stopped listing reads as
/// no longer available; with no catalogue (offline, or the read failed)
/// none is marked, because unknown is not removed. Drawn only while there
/// is a favorite. The catalogue is read for Claude Code, which is offered
/// every NEAR AI chat model, so its two lists are the whole stored list.
/// The section reads it (`ManagedFavoritesCard.readCatalogue`), so the read
/// happens while the card is still hidden.
struct ManagedFavoritesCard: View {
    @EnvironmentObject private var model: AppModel
    @Binding var catalogue: ManagedModelCatalogue?
    /// The stored list from the last removal's answer, newer than either read.
    @State private var stored: [String]?

    /// The tool whose catalogue Settings reads: offered every chat model.
    static let catalogueTool = "claude"
    @State private var busy = false
    @State private var failed = false

    private var rows: [ManagedFavoriteRow] {
        let list = stored ?? ManagedSurface.storedFavorites(
            catalogue: catalogue, settings: model.daemonSettings?.managedFavoriteModels)
        return ManagedSurface.favoriteRows(stored: list, catalogue: catalogue)
    }

    var body: some View {
        if !rows.isEmpty {
            GlassEyebrowCard(model.managedText("favorites_title")) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    ForEach(rows) { row($0) }
                    if failed { GlassAlert(model.managedText("action_failed")) }
                }
            }
            // A fresh catalogue supersedes the last removal's answer.
            .onChange(of: catalogue) { _, _ in stored = nil }
        }
    }

    private func row(_ row: ManagedFavoriteRow) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(row.unavailable
                 ? model.managedText("model_favorite_unavailable").replacingOccurrences(of: "{model}", with: row.model)
                 : row.model)
                .foregroundStyle(row.unavailable ? GlassColor.textSecondary : GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer()
            Button(model.managedText("remove")) { remove(row.model) }
                .buttonStyle(GlassButtonStyle(.link))
                .disabled(busy)
                .accessibilityLabel(model.managedText("model_favorite_remove_named")
                    .replacingOccurrences(of: "{model}", with: row.model))
        }
    }

    /// Unstarring reads nothing and is never refused for the bound.
    private func remove(_ id: String) {
        busy = true
        failed = false
        model.setFavoriteModel(id, favorite: false) { favorites in
            busy = false
            guard let favorites else { failed = true; return }
            stored = favorites
            catalogue = catalogue?.withFavorites(favorites)
            model.refreshSettings()
        }
    }
}
