import CTraceCommons
import Foundation

/// The command-line tool's state, from the core (`tc_cli_tool_json`).
/// Decoded by `TCShellCore.CliToolStatus`; nothing here is a path or a word.
public enum TCCliTool {
    /// `tc_cli_tool_json` for the app at `bundlePath`. Reads only. Nil only
    /// on a caught panic.
    public static func json(bundlePath: String) -> String? {
        guard let raw = bundlePath.withCString({ tc_cli_tool_json($0) }) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
