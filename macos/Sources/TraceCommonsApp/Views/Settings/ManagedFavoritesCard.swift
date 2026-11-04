import SwiftUI
import TCDesign
import TCShellCore

/// "Favorite models": every model the person starred beside the Launch
/// sheet's picker, each with Remove. The rows are the stored list from
/// `get_settings`; the catalogue only orders them and marks one NEAR AI
/// stopped listing as no longer available. With no catalogue (offline, or
/// the read failed) none is marked, because unknown is not removed. Drawn
/// only while there is a favorite.
struct ManagedFavoritesCard: View {
    @EnvironmentObject private var model: AppModel
    let catalogue: ManagedModelCatalogue?

    /// The tool whose catalogue Settings reads. Its `unavailable_favorites`
    /// is computed against every NEAR AI chat model whichever tool asks,
    /// and a favorite it is not offered still has its row, so the choice
    /// only decides the order of the rows.
    static let catalogueTool = "claude"
    @State private var busy = false
    @State private var failed = false

    private var rows: [ManagedFavoriteRow] {
        ManagedSurface.favoriteRows(stored: model.daemonSettings?.managedFavoriteModels ?? [], catalogue: catalogue)
    }

    var body: some View {
        if !rows.isEmpty {
            GlassEyebrowCard(model.managedText("favorites_title")) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    ForEach(rows) { row($0) }
                    if failed { GlassAlert(model.managedText("action_failed")) }
                }
            }
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

    /// Unstarring reads nothing and is never refused for the bound or the
    /// id's shape. The model refreshes the settings this card reads.
    private func remove(_ id: String) {
        busy = true
        failed = false
        model.setFavoriteModel(id, favorite: false) { favorites, _ in
            busy = false
            failed = favorites == nil
        }
    }
}
