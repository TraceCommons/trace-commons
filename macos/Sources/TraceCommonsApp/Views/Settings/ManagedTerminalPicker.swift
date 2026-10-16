import SwiftUI
import TCDesign
import TCShellCore

/// "Open sessions in": the terminal app a managed session opens in, one of
/// the apps the daemon found installed. The label is the core's
/// (`terminal_choice`); the options are the apps' own names as the daemon
/// sends them. A choice that is no longer installed shows its name with no
/// option ticked. Moved only by a write the daemon accepted.
struct ManagedTerminalPicker: View {
    let snapshot: ManagedSnapshot
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let label = model.managedText("terminal_choice")
        HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            Text(label)
            Spacer(minLength: 0)
            GlassPicker(
                label,
                selection: Binding<String?>(
                    get: { snapshot.capabilities.terminalCurrent },
                    set: { wanted in
                        guard let wanted, wanted != snapshot.capabilities.terminalCurrent else { return }
                        model.setManagedTerminal(wanted)
                    }),
                options: snapshot.terminalChoices.map { GlassPickerOption($0.name, value: $0.id) },
                placeholder: snapshot.capabilities.terminalDestination ?? "")
            .disabled(model.managedBusy)
        }
    }
}
