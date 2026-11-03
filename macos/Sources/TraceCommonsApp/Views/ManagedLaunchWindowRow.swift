import SwiftUI
import TCDesign
import TCShellCore

/// The launch dialog's window list (owner, 2026-11-02, #1852): "New
/// window", the default, then each Terminal window holding a running
/// session the daemon opened and can still vouch for, named as those
/// sessions' rows name them. The list comes from the daemon
/// (`managed_terminal_windows`), never from reading Terminal here. Drawn
/// only when there is a window to offer. Picking one asks macOS for the
/// permissions a tab needs, so its prompts come now and not during the
/// launch. The words are the core's, DRAFT until approved.
struct ManagedLaunchWindowRow: View {
    /// The picked window's session id, or nil for "New window".
    @Binding var beside: String?
    /// The windows offered now; none until the daemon answers.
    @State private var options: [ManagedWindowOption] = []
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let label = model.managedText("open_in_window")
        Group {
            if !options.isEmpty {
                HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                    Text(label)
                    Spacer(minLength: 0)
                    GlassPicker(
                        label,
                        selection: Binding(
                            get: { beside ?? "" },
                            set: { picked in
                                let picked = (picked ?? "").isEmpty ? nil : picked
                                if picked != nil, picked != beside { model.managedTerminalTabAccess() }
                                beside = picked
                            }),
                        options: [GlassPickerOption(model.managedText("new_window"), value: "")]
                            + options.map { GlassPickerOption($0.title, value: $0.beside) },
                        placeholder: model.managedText("new_window"))
                    .disabled(model.managedBusy)
                }
            }
        }
        .onAppear(perform: load)
    }

    private func load() {
        model.managedTerminalWindows { windows in
            options = ManagedSurface.windowOptions(
                windows, sessions: model.managedSnapshot?.sessions ?? [],
                tool: { ManagedWords.tool($0, model: model) },
                template: model.managedText("window_option"))
            beside = ManagedSurface.launchBeside(beside, options: options)
        }
    }
}
