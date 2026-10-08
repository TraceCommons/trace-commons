import AppKit

/// The one folder picker every screen opens (Settings' tool rows, the first
/// run's Folders rows and its "add your tool" tile): one directory, never a
/// file, and no folder created from it.
@MainActor
enum FolderPanel {
    /// The chosen folder's path, or nil when the panel is dismissed.
    static func choose() -> String? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url.path
    }
}
