import SwiftUI
import TCDesign
import TCShellCore

/// The command-line tool's row in Settings' system integrations card: the
/// link's state and the actions the core offers for it. Shows nothing where
/// there is no bundled CLI (a bare `swift run`) or no words.
struct CliToolRow: View {
    @ObservedObject private var controller = CliToolController.shared

    var body: some View {
        // A node that always exists, so the refresh runs even when the row
        // is empty.
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            if let words = MonitorWords.table?.cliTool, let status = controller.status,
                let line = words.statusLine(status)
            {
                GlassHairline()
                Text(words.title).glassType(GlassTokens.TypeScale.label)
                Text(line)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                if !status.actions.isEmpty {
                    HStack(spacing: GlassTokens.Space.s3) {
                        ForEach(status.actions) { action in
                            Button(words.button(action)) {
                                Task { await controller.perform(action) }
                            }
                            .buttonStyle(GlassButtonStyle(.glass))
                            .accessibilityLabel(words.label(action))
                            .disabled(controller.busy)
                        }
                    }
                }
                if let path = controller.failedPath {
                    GlassAlert(words.failedLine(path: path))
                }
            }
        }
        .onAppear { controller.refresh() }
    }
}

/// The app menu's item: the first action the core offers, which opens the
/// system's password prompt. Absent when nothing is offered.
struct CliToolMenuItem: View {
    @ObservedObject private var controller = CliToolController.shared

    var body: some View {
        if let words = MonitorWords.table?.cliTool, let action = controller.status?.primaryAction {
            Button(words.menuItem(action)) {
                Task { await controller.perform(action) }
            }
            .disabled(controller.busy)
        }
    }
}
