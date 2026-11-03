import SwiftUI
import TCDesign
import TCShellCore

/// The launch dialog's window list (owner, 2026-11-02, #1852): "New
/// window", then each Terminal window holding a running session the daemon
/// opened and can still vouch for, named as those sessions' rows name them.
/// The newest window is the default (owner, 2026-11-03: work stays in one
/// Terminal window). The list comes from the daemon
/// (`managed_terminal_windows`), never from reading Terminal here. Drawn
/// only when there is a window to offer. Picking one asks macOS for the
/// permissions a tab needs, so its prompts come now and not during the
/// launch. The words are the core's, approved by the owner 2026-11-02.
struct ManagedLaunchWindowRow: View {
    /// The windows offered now, read by the dialog when it opens
    /// ([`load`]); none until the daemon answers.
    let options: [ManagedWindowOption]
    /// The picked window's session id, or nil for "New window".
    @Binding var beside: String?
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let label = model.managedText("open_in_window")
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

    /// Read the windows to offer, for a dialog's `onAppear`: the row draws
    /// nothing until there is one, so it cannot load them itself. A pick
    /// no longer offered goes to the newest window, as does no pick; with
    /// a window chosen, macOS's prompts for tabs are asked for now.
    @MainActor
    static func load(model: AppModel, beside: Binding<String?>, into options: Binding<[ManagedWindowOption]>) {
        model.managedTerminalWindows { windows in
            options.wrappedValue = ManagedSurface.windowOptions(
                windows, sessions: model.managedSnapshot?.sessions ?? [],
                tool: { ManagedWords.tool($0, model: model) },
                template: model.managedText("window_option"))
            let picked = ManagedSurface.launchBeside(beside.wrappedValue, options: options.wrappedValue)
                ?? ManagedSurface.launchDefault(options: options.wrappedValue)
            if picked != nil { model.managedTerminalTabAccess() }
            beside.wrappedValue = picked
        }
    }
}
