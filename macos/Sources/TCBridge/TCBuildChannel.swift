import CTraceCommons
import Foundation

/// The build channel the linked dylib was compiled for, and the sentence the
/// app shows when the bundle disagrees with it (staging design, section 4.3).
/// Decoded by `TCShellCore.BuildChannel`; nothing here is a value or a word.
public enum TCBuildChannel {
    private static func take(_ raw: UnsafeMutablePointer<CChar>?) -> String? {
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// `tc_build_channel_json`. Nil only on a caught panic.
    public static func json() -> String? { take(tc_build_channel_json()) }

    /// `tc_channel_mismatch_line`. Nil only on a caught panic.
    public static func mismatchLine() -> String? { take(tc_channel_mismatch_line()) }

    /// `tc_channel_window_label`: the core's label for the main window on a
    /// staging build (spec S7, 4.8). Nil on production, whose answer is
    /// empty, and on a caught panic.
    public static func windowLabel() -> String? {
        guard let label = take(tc_channel_window_label()), !label.isEmpty else { return nil }
        return label
    }

    /// Whether `dir`, taken from this build's override variable, may be the
    /// state directory: nil to allow it, or the label `tc_resolve_state_dir`
    /// refused it with (`state-dir-other-channel` when it names the other
    /// channel's directory). A NULL answer with no label is reported as
    /// `null-response`, never as allowed.
    public static func overrideRefusal(_ dir: String) -> String? {
        var err: UnsafeMutablePointer<CChar>?
        if take(tc_resolve_state_dir(nil, dir, &err)) != nil { return nil }
        return take(err) ?? "null-response"
    }

    /// `tc_state_dir_override_refused_line`. Nil only on a caught panic.
    public static func overrideRefusedLine() -> String? {
        take(tc_state_dir_override_refused_line())
    }
}
