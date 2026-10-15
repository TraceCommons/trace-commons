import TCDesign
import TCShellCore

// Not `#if DEBUG`: first run's tool rows (`ToolAnswerRow`), which a release
// build compiles, draw a tool's tile too, as the Monitor's tree does.
extension SourceKind {
    /// The tool's glass tile.
    var glassTool: GlassTool {
        switch self {
        case .claudeCode: .claudeCode
        case .codex: .codex
        case .geminiCli: .geminiCLI
        case .cline: .cline
        case .opencode: .openCode
        }
    }
}
