import Foundation

/// Home's Credit tile reads a day-over-day change in total credit (owner,
/// 2026-10-12). The core keeps no balance history, so this Mac keeps its
/// own: the total last seen each local day, for `keptDays` days. A change
/// is drawn only against an earlier day this Mac saw; with none it is
/// unknown, never zero. A daemon method for the balance's history is to
/// replace this, so every shell and device reads the same answer.
struct CreditDailyTotals: Equatable {
    /// Total credit by local day (`yyyy-MM-dd`).
    private(set) var totals: [String: Double]

    static let defaultsKey = "TCCreditDailyTotals"
    static let keptDays = 30

    init(totals: [String: Double] = [:]) {
        self.totals = totals
    }

    /// `day`'s key, in the person's own calendar.
    static func key(_ day: Date, calendar: Calendar = .current) -> String {
        let parts = calendar.dateComponents([.year, .month, .day], from: day)
        return String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
    }

    /// Records `total` as `day`'s, keeping the newest `keptDays` days.
    mutating func record(_ total: Double, on day: Date, calendar: Calendar = .current) {
        totals[Self.key(day, calendar: calendar)] = total
        let kept = totals.keys.sorted().suffix(Self.keptDays)
        totals = totals.filter { kept.contains($0.key) }
    }

    /// The change from the newest earlier day this Mac saw to `day`; nil
    /// without both. Keys sort as dates, so the newest earlier is the
    /// greatest key below `day`'s.
    func delta(on day: Date, calendar: Calendar = .current) -> Double? {
        let today = Self.key(day, calendar: calendar)
        guard let now = totals[today],
              let before = totals.keys.filter({ $0 < today }).max(),
              let then = totals[before]
        else { return nil }
        return now - then
    }

    static func load(_ defaults: UserDefaults = .standard) -> CreditDailyTotals {
        CreditDailyTotals(totals: (defaults.dictionary(forKey: defaultsKey) as? [String: Double]) ?? [:])
    }

    func save(_ defaults: UserDefaults = .standard) {
        defaults.set(totals, forKey: Self.defaultsKey)
    }
}
