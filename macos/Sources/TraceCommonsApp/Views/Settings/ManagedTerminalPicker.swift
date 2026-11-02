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
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
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
            if let tabs = Self.tabsSwitch(snapshot) {
                tabsToggle(tabs)
            }
        }
    }

    /// The switch's value when it is drawn: only for Terminal, the one
    /// app the daemon opens tabs in, and only from a daemon that sends it.
    static func tabsSwitch(_ snapshot: ManagedSnapshot) -> Bool? {
        snapshot.capabilities.terminalCurrent == "terminal" ? snapshot.capabilities.terminalTabs : nil
    }

    /// "Open new sessions as tabs" (owner, 2026-11-02): moved only by a
    /// write the daemon accepted.
    @ViewBuilder
    private func tabsToggle(_ on: Bool) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            Toggle(model.managedText("terminal_tabs"), isOn: Binding(
                get: { on },
                set: { model.setManagedTerminalTabs($0) }
            ))
            .toggleStyle(GlassToggleStyle(.settings))
            .disabled(model.managedBusy)
            Text(model.managedText("terminal_tabs_detail"))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// "Open sessions in", asked in a launch dialog until the person has chosen
/// a terminal (`ManagedSnapshot.asksForTerminal`): the same label and the
/// same installed apps as Settings, starting on the current one. Nothing is
/// written here; the dialog's Open saves the choice before the launch, so
/// it is remembered and the row is not drawn again. Settings still changes
/// it later.
struct ManagedLaunchTerminalRow: View {
    let snapshot: ManagedSnapshot
    @Binding var selection: String?
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let label = model.managedText("terminal_choice")
        HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            Text(label)
            Spacer(minLength: 0)
            GlassPicker(
                label,
                selection: $selection,
                options: snapshot.terminalChoices.map { GlassPickerOption($0.name, value: $0.id) },
                placeholder: "")
            .disabled(model.managedBusy)
        }
        .onAppear { if selection == nil { selection = snapshot.proposedTerminal } }
    }
}
