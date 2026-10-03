import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Each answer writes straight through `set_settings`: there is no Save,
/// because each row is one declaration. What a row shows is the MODE the
/// daemon reports, which is all `get_settings` says. With no daemon answer
/// the section draws the core's unavailable sentence and no row, so no
/// control reads as working.
struct WatchedFoldersSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var candidates: [SourceCandidate] = []
    @State private var busy = false
    @State private var saveFailed = false

    var body: some View {
        // The container is always present, so `.onAppear` runs even when the
        // core's copy is missing and the card draws nothing.
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let copy = TCSourceChecks.settingsCopy() {
                GlassEyebrowCard(copy.heading) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        Text(copy.explanation)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                        if saveFailed {
                            GlassNotice(tone: .outside) { Text(copy.saveFailed) }
                        }
                        if model.daemonSettings != nil {
                            ForEach(SourceKind.allCases, id: \.self) { kind in
                                GlassSourceRow(
                                    kind: kind,
                                    candidate: candidates.first { $0.source == kind },
                                    choice: choice(for: kind),
                                    reportedMode: mode(for: kind),
                                    onWatchCandidate: { save(kind, .watch(path: $0.path)) },
                                    onChoose: { save(kind, .watch(path: $0)) },
                                    onDecline: { save(kind, .off) })
                            }
                        } else {
                            Text(copy.unavailable)
                                .glassType(GlassTokens.TypeScale.body)
                                .foregroundStyle(GlassColor.textSecondary)
                        }
                        if saveFailed || model.daemonSettings == nil {
                            Button(copy.retry) { model.refreshSettings() }
                                .buttonStyle(GlassButtonStyle(.glass))
                        }
                    }
                }
                .disabled(busy)
            }
        }
        .onAppear(perform: discover)
    }

    private func save(_ kind: SourceKind, _ choice: SourceChoice) {
        guard !busy else { return }
        busy = true
        saveFailed = false
        Task {
            saveFailed = !(await model.setSourceRoot(kind, choice))
            busy = false
        }
    }

    /// The daemon's answer for one source. The path is deliberately absent:
    /// the daemon only says that a folder is watched.
    private func choice(for kind: SourceKind) -> SourceChoice {
        switch mode(for: kind) {
        case "watch": return .watch(path: "")
        case "off": return .off
        default: return .undecided
        }
    }

    private func mode(for kind: SourceKind) -> String {
        guard let modes = model.daemonSettings?.routingSourceModes else { return "unset" }
        switch kind {
        case .claudeCode: return modes.claude
        case .codex: return modes.codex
        case .geminiCli: return modes.gemini
        case .cline: return modes.cline
        case .opencode: return model.daemonSettings?.opencodeSourceMode ?? ""
        }
    }

    /// Best-effort: a row can always be answered by hand.
    private func discover() {
        guard candidates.isEmpty, let json = TCDiscovery.sourcesJSON() else { return }
        candidates = (try? SourceCandidate.decodeList(from: json)) ?? []
    }
}
