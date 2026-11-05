import CTraceCommons

/// Credit figures across the C ABI, so this shell draws them by the core's
/// one rule and not a formatter of its own.
public enum TCCredit {
    /// A credit figure as every shell draws it (`tc_credit_figure`): tenths
    /// rounded half away from zero (4.25 is 4.3), whole is whole ("48"),
    /// one decimal otherwise, "<0.1" above zero and under 0.05, a comma
    /// between thousands ("1,234.5"). Nil only if the call failed.
    public static func figure(_ value: Double) -> String? {
        guard let raw = tc_credit_figure(value) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// A count beside a credit figure (`tc_count_figure`): a comma between
    /// thousands ("1,234"). Nil only if the call failed.
    public static func count(_ value: UInt64) -> String? {
        guard let raw = tc_count_figure(value) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
