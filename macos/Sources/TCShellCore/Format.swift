import Foundation

/// Sizes and times as the shell prints them, beside the core's words.
///
/// Both formatters are built once. A view calls these while it draws, per
/// row, and building one costs far more than using one. Neither
/// `ByteCountFormatter` nor `RelativeDateTimeFormatter` is documented as
/// thread-safe, so a lock guards each (the `DaemonTimestamp` pattern).
public enum Format {
    public static func bytes(_ count: Int) -> String {
        shared.bytes(count)
    }

    public static func when(_ date: Date) -> String {
        shared.when(date, relativeTo: Date())
    }

    /// `when` against a given "now", so a test can freeze the reference.
    static func when(_ date: Date, relativeTo now: Date) -> String {
        shared.when(date, relativeTo: now)
    }

    /// How many formatters have been built, for a test that counts them.
    static var formattersBuilt: Int { shared.built }

    public static func tomorrowMorning() -> Date {
        let calendar = Calendar.current
        let tomorrow = calendar.date(byAdding: .day, value: 1, to: Date()) ?? Date()
        return calendar.date(bySettingHour: 9, minute: 0, second: 0, of: tomorrow) ?? tomorrow
    }

    private static let shared = Formatters()

    private final class Formatters: @unchecked Sendable {
        private let byteLock = NSLock()
        private let relativeLock = NSLock()
        private(set) var built = 0
        private let byteFormatter: ByteCountFormatter
        private let relativeFormatter: RelativeDateTimeFormatter

        init() {
            let bytes = ByteCountFormatter()
            bytes.countStyle = .file
            bytes.allowedUnits = [.useKB, .useMB, .useGB]
            byteFormatter = bytes
            let relative = RelativeDateTimeFormatter()
            relative.unitsStyle = .full
            relativeFormatter = relative
            built = 2
        }

        func bytes(_ count: Int) -> String {
            byteLock.lock()
            defer { byteLock.unlock() }
            return byteFormatter.string(fromByteCount: Int64(count))
        }

        func when(_ date: Date, relativeTo now: Date) -> String {
            relativeLock.lock()
            defer { relativeLock.unlock() }
            return relativeFormatter.localizedString(for: date, relativeTo: now)
        }
    }
}
