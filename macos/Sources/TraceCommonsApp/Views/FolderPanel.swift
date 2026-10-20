import AppKit
import TCBridge
import TCShellCore
import UniformTypeIdentifiers

/// The one open panel every screen opens: the folder picker (Settings' tool
/// rows, the first run's Folders rows and its "add your tool" tile), one
/// directory, never a file, and no folder created from it; and Analyze's
/// trace-file picker, one file.
@MainActor
enum FolderPanel {
    /// The chosen folder's path, or nil when the panel is dismissed.
    static func choose() -> String? {
        let panel = openPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url.path
    }

    /// Analyze's trace file, or nil when the panel is dismissed. It opens
    /// where the harness keeps its traces, with hidden folders shown: both
    /// harnesses keep them under a hidden folder in the home folder.
    static func chooseTraceFile(source: String) -> URL? {
        let panel = openPanel()
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        panel.showsHiddenFiles = true
        panel.allowedContentTypes = [.data]
        panel.directoryURL = TraceFileStart.directory(source: source,
                                                      home: FileManager.default.homeDirectoryForCurrentUser) {
            var isDirectory: ObjCBool = false
            return FileManager.default.fileExists(atPath: $0.path, isDirectory: &isDirectory) && isDirectory.boolValue
        }
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url
    }

    /// The panel both pickers are made from, so the app has one.
    private static func openPanel() -> NSOpenPanel { NSOpenPanel() }
}

/// Where Analyze's file panel opens: the chosen harness's trace folder when
/// it exists, then Claude Code's, then Codex's, then the home folder.
enum TraceFileStart {
    static let claudeCode = ".claude/projects"
    static let codex = ".codex/sessions"

    static func directory(source: String, home: URL, exists: (URL) -> Bool) -> URL {
        let order = source == "codex" ? [codex, claudeCode] : [claudeCode, codex]
        return order
            .map { home.appendingPathComponent($0, isDirectory: true) }
            .first(where: exists) ?? home
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
