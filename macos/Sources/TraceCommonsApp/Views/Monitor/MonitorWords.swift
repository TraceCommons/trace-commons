import TCBridge
import TCShellCore

// Not `#if DEBUG`: `TracesStore`, which a release build compiles, reads this
// table (its safeguards' capacity-unreadable line), so the table must be
// there in a release build too. The words for debug-only screens stay in
// their `extension MonitorWords` beside those screens.

/// The monitor screens' words, read from the core's table
/// (`MonitorScreensCopy`, `tc_monitor_screens_copy_json`). This shell holds
/// none of its own: with no table a word is empty, never a Swift fallback.
enum MonitorWords {
    /// The core's table, decoded once.
    static let table: MonitorScreensCopy? = MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())

    static var computer: String { table?.computer ?? "" }
    static var commons: String { table?.commons ?? "" }
    static var waiting: String { table?.waiting ?? "" }
    static var folders: String { table?.folders ?? "" }
    static var watched: String { table?.watched ?? "" }
    static var off: String { table?.off ?? "" }
    static var connected: String { table?.connected ?? "" }
    static var reduce: String { table?.reduce ?? "" }
    static var enlarge: String { table?.enlarge ?? "" }
    static var calls: String { table?.calls ?? "" }
    static var models: String { table?.models ?? "" }
    static var priced: String { table?.priced ?? "" }
    static var unknown: String { table?.unknown ?? "" }
}
