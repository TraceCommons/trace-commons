import Observation

/// Window selection is independent of watcher startup and trace onboarding.
/// Owning it above the window also preserves selection when the window closes.
@Observable @MainActor
final class MainWindowNavigation {
    var section: MainWindowView.Section = .queue
    var displaysCompute: Bool { section == .compute }
}
