import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// A yes/no Settings fact. The state is a glyph for sighted readers and the
/// words "title: yes" / "title: no" for VoiceOver, as the legacy check row
/// did; colour alone never carries it.
struct SettingsStateRow: View {
    let title: String
    let isOn: Bool

    var body: some View {
        HStack(spacing: GlassTokens.Space.s3) {
            Image(systemName: isOn ? "checkmark.circle.fill" : "circle")
                .accessibilityHidden(true)
            Text(title)
        }
        .glassType(GlassTokens.TypeScale.label)
        .foregroundStyle(GlassColor.textSecondary)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(SettingsLegacyWords.stateLabel(title, isOn))
    }
}

/// What a section draws where the daemon has not answered yet: the glass
/// spinner (R-20), which VoiceOver reads as a native progress indicator.
/// Never an empty card, never a value read from a default.
struct SettingsAwaiting: View {
    var body: some View {
        GlassSpinner(standalone: true)
    }
}

/// Where one daemon read stands, for the section that draws from it.
enum SettingsRead: Equatable {
    /// Asked, and not answered yet.
    case awaiting
    /// Answered at least once; what it said is real, even if stale.
    case answered
    /// The last call failed and nothing has answered before it.
    case failed
    /// The core never started, so no read can run.
    case coreDown

    /// An answer wins: stale data is still data the daemon gave. Otherwise
    /// a core that refused to start, or a failed call, is never loading.
    static func resolve(answered: Bool, failed: Bool, startup: AppModel.Startup) -> SettingsRead {
        if answered { return .answered }
        switch startup {
        case .refused, .needsRoots: return .coreDown
        case .starting, .running: return failed ? .failed : .awaiting
        }
    }
}

/// What a section draws where its read has not answered: the glass spinner
/// while it is in flight, and the core's failure line
/// once it has failed or the core is down. Nothing once it has answered,
/// where the section draws the answer.
struct SettingsReadNotice: View {
    let read: SettingsRead
    let retry: () -> Void

    init(_ read: SettingsRead, retry: @escaping () -> Void) {
        self.read = read
        self.retry = retry
    }

    var body: some View {
        switch read {
        case .awaiting:
            SettingsAwaiting()
        case .failed, .coreDown:
            SettingsUnavailable(read: read, retry: retry)
        case .answered:
            EmptyView()
        }
    }
}

/// A read that will not answer by waiting, in the core's words. A failed
/// call can be asked again, with the core's retry word; a core that never
/// started cannot, so it offers nothing to press.
struct SettingsUnavailable: View {
    let read: SettingsRead
    let retry: () -> Void

    static func line(_ read: SettingsRead, screens: MonitorScreensCopy?) -> String {
        guard let screens else { return "\u{2014}" }
        return read == .coreDown ? screens.coreUnreachable : screens.requestFailed
    }

    static func offersRetry(_ read: SettingsRead) -> Bool { read == .failed }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
            Text(Self.line(read, screens: ConsentScopeRows.screens))
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if Self.offersRetry(read), let word = TCSourceChecks.settingsCopy()?.retry {
                Button(word, action: retry)
                    .buttonStyle(GlassButtonStyle(.glass))
            }
        }
    }
}

extension DaemonStatus {
    /// False until the daemon has answered `status` at all. `.unknown` is
    /// the placeholder held before the first answer, and a real status
    /// always carries a schema version, so the two cannot be confused.
    var answered: Bool { self != .unknown }
}
