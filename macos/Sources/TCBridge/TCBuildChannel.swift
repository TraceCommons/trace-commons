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
}
