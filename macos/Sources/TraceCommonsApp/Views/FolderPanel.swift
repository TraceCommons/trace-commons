import AppKit
import TCBridge
import TCShellCore

/// The one open panel every screen opens: the folder picker (Settings' tool
/// rows, the first run's Folders rows and its "add your tool" tile), one
/// directory, never a file, and no folder created from it.
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

/// Resolve only a newly chosen folder. Existing saved directory declarations
/// keep their export-only meaning until the contributor selects again.
enum SourceFolderSelection {
    static func path(for kind: SourceKind, chosenFolder: String) -> String {
        guard kind == .opencode,
              let json = TCDiscovery.describeFolderJSON(chosenFolder),
              let matches = try? FolderMatch.decodeList(from: json)
        else { return chosenFolder }
        return path(for: kind, chosenFolder: chosenFolder, matches: matches)
    }

    static func path(for kind: SourceKind, chosenFolder: String, matches: [FolderMatch]) -> String {
        guard kind == .opencode, matches.count == 1,
              let match = matches.first, match.kind == .source(.opencode),
              URL(fileURLWithPath: match.path).standardizedFileURL
                == URL(fileURLWithPath: chosenFolder).appendingPathComponent("opencode.db").standardizedFileURL
        else { return chosenFolder }
        return match.path
    }
}
