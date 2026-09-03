import CTraceCommons
import Foundation

/// The settings screen's session-source rows, read from the Rust rather than
/// written here.
///
/// Handle-free like `TCRoutingCopy`: this is wording about a declaration, not
/// a call into a running daemon.
///
/// Nothing in this file is a word. The sentence crosses already assembled, so
/// there is no template for this shell to fill in and therefore no fourth
/// place the wording can drift. The GTK and Windows shells render the same
/// row from the same Rust.
public enum TCSourceChecks {
    /// The wire key for Claude Code's session source.
    public static let claude = "claude"

    /// The wire key for Codex's session source.
    public static let codex = "codex"

    /// One tool's row, from `get_settings`'s `*_source_mode` -- `watch`,
    /// `off` or `unset`.
    ///
    /// PASS THE MODE, NOT THE BOOLEAN. `claudeRootConfigured` is
    /// `mode == "watch"`, so it is false for a source the contributor
    /// declared OFF as well as for one nobody was asked about. The GTK and
    /// Windows shells branched on it and told a contributor who does not use
    /// Claude Code that its sessions were being read from the usual place.
    /// Nothing is read from an off source; an unset one is scanned where it
    /// usually lives. Three facts, three sentences, and the Rust picks which.
    ///
    /// Nil when the ABI refused -- an unknown tool key, or a caught panic.
    /// The caller shows nothing rather than a sentence written in Swift.
    public static func checkLine(tool: String, sourceMode: String) -> String? {
        guard
            let raw = tool.withCString({ toolPtr in
                sourceMode.withCString { modePtr in
                    tc_source_check_line(toolPtr, modePtr)
                }
            })
        else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
