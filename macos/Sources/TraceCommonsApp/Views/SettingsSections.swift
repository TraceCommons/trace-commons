import TCBridge
import TCShellCore
import TCLocalization

/// The Settings window's sections, in the Settings IA review's grouped
/// order (owner, 2026-11-04, D1-D20): Account, then what is watched, then
/// what is shared, then Private AI, then the app's own preferences, then the
/// advanced sections. It began as #1146's twelve (spec, "Settings
/// navigation"; R11 of #1173). The body draws every case in this order; the
/// list names the listed ones, with a hairline between groups (D17). Each
/// names the section `GlassSettingsContent` draws for it.
enum SettingsSection: String, CaseIterable, Identifiable {
    // Account (D1, D2, D5).
    case account
    /// NEAR AI credit payouts: the payout link and, for a wallet-origin
    /// account, "Payouts need a NEAR AI login" (D2). Its row is listed only
    /// while something in it applies.
    case payouts
    /// Referral codes (referral spec section 6), beside Payouts, where the
    /// referral credit is paid (D5).
    case referrals
    // Capture (D12, D13).
    case watching
    /// Drawn under Watching's name (D12); a request for it still scrolls to
    /// its folders.
    case watchedFolders
    case tools
    /// The Insights switches, out of Tools (D13).
    case insights
    // Sharing (D15).
    case projects
    case consent
    case publicProfile
    case changes
    // Private AI.
    case privateAI
    // App (D18, D19).
    case general
    case startup
    case notifications
    case updates
    // Advanced (D14).
    case witness
    case compute

    var id: String { rawValue }

    /// Whether the Settings window draws this section while onboarding is
    /// required (R-43): only the sections that ask nothing first run asks.
    /// Consent, the public profile, the watched folders, the tools, the
    /// witness, the projects -- and the pause and change log beside them --
    /// wait for first run, so no write surface exists outside it.
    var availableBeforeOnboarding: Bool {
        switch self {
        case .general, .account, .startup, .notifications, .updates, .privateAI, .compute: true
        // Payouts and Referrals wait: before first run there is no account
        // to pay or to hold a code.
        case .payouts, .referrals, .watching, .watchedFolders, .tools, .insights, .projects, .consent,
             .publicProfile, .changes, .witness: false
        }
    }

    /// The group a section belongs to; the list draws a hairline where the
    /// group changes (D17: no group headings).
    enum Group: Equatable {
        case account, capture, sharing, privateAI, app, advanced
    }

    var group: Group {
        switch self {
        case .account, .payouts, .referrals: .account
        case .watching, .watchedFolders, .tools, .insights: .capture
        case .projects, .consent, .publicProfile, .changes: .sharing
        case .privateAI: .privateAI
        case .general, .startup, .notifications, .updates: .app
        case .witness, .compute: .advanced
        }
    }

    /// Whether the list names this section. Notifications is drawn in the
    /// body under Startup's name, as #1146 draws it under "Startup &
    /// notifications", and Watched folders under Watching's (D12); a request
    /// for either still scrolls to it.
    var isListed: Bool { self != .notifications && self != .watchedFolders }

    /// Whether the list draws a hairline above this row: the first row of
    /// every group but the first (D17).
    static func startsGroup(_ section: SettingsSection, in rows: [SettingsSection]) -> Bool {
        guard let index = rows.firstIndex(of: section), index > 0 else { return false }
        return rows[index - 1].group != section.group
    }

    /// The sections the list names, in its order.
    static let listed: [SettingsSection] = allCases.filter(\.isListed)

    /// The name #1146 lists this section by, from the core's table: the
    /// list's row and the rule that opens the section in the body. Nil for
    /// a section drawn under the one before it, and while the table has not
    /// loaded; the list then draws a disabled placeholder (`ListRow`)
    /// rather than inventing a name or hiding the section.
    func navName(_ nav: MonitorSettingsNavCopy?) -> String? {
        // General's name is this app's own word, and its section never
        // waits on the core, so its row does not either.
        if self == .general { return SettingsWords.general }
        guard let nav else { return nil }
        switch self {
        case .general: return SettingsWords.general
        case .account: return nav.account
        case .payouts: return nav.payouts
        // Not in the core's nav table: the referral words' own title, a
        // draft for owner review (`ReferralCopy.title`), so the list row and
        // the section heading are one string.
        case .referrals: return SettingsWords.referrals
        case .watching: return nav.watching
        case .watchedFolders, .notifications: return nil
        case .tools: return nav.tools
        // The core's Insights title: the Insights tab's name, and the
        // heading of the card this section draws.
        case .insights: return HomeTabView.insightsHeading
        case .projects: return nav.projects
        case .consent: return nav.uses
        case .publicProfile: return nav.profile
        case .changes: return nav.log
        case .privateAI: return nav.privateAI
        case .startup: return nav.startup
        // The Updates card's own heading (D19).
        case .updates: return SettingsWords.updates
        case .witness: return nav.witness
        case .compute: return nav.compute
        }
    }

    /// The row the list draws for this section.
    func listRow(_ nav: MonitorSettingsNavCopy?) -> ListRow {
        ListRow.row(title: navName(nav))
    }

    /// What the list draws for a section: its title, or, while the copy
    /// that names it has not loaded, a disabled placeholder. A row never
    /// vanishes: a section whose copy never loads must still be visible,
    /// and a restored selection must still point at a row.
    struct ListRow: Equatable {
        let text: String
        let enabled: Bool

        static func row(title: String?) -> ListRow {
            guard let title, !title.isEmpty else { return ListRow(text: "—", enabled: false) }
            return ListRow(text: title, enabled: true)
        }
    }

    /// An SF Symbol for the row.
    var symbol: String {
        switch self {
        case .account: "person.badge.key"
        case .payouts: "banknote"
        case .referrals: "person.2"
        case .watching: "eye"
        case .watchedFolders: "folder"
        case .tools: "wrench.and.screwdriver"
        case .insights: "chart.bar"
        case .projects: "square.stack"
        case .consent: "checkmark.shield"
        case .publicProfile: "person.crop.circle"
        case .changes: "clock.arrow.circlepath"
        case .privateAI: "key"
        case .general: "gearshape"
        case .startup: "power"
        case .notifications: "bell"
        case .updates: "arrow.down.circle"
        case .witness: "seal"
        case .compute: "cpu"
        }
    }
}

/// The single words the section list and the sections' cards show.
/// General's are this shell's own: General never waits on the core (owner
/// decision D20, 2026-11-04). The rest are read from the core's tables
/// (copy-guide rule 8; spelled here until 2026-11-04), empty while a table
/// has not loaded, never a Swift fallback.
enum SettingsWords {
    private static var nav: MonitorSettingsNavCopy? { MonitorWords.table?.settingsNav }
    private static var shell: MonitorShellCopy? { MonitorWords.table?.shell }
    static let general = Localization.current.text("app.settings.general")
    static let appearance = Localization.current.text("app.settings.appearance")
    static let light = Localization.current.text("app.settings.light")
    static let dark = Localization.current.text("app.settings.dark")
    static let system = Localization.current.text("app.settings.system")
    static var updates: String { shell?.updatesEyebrow ?? "" }
    static var projects: String { nav?.projects ?? "" }
    /// The core's referral title (DRAFT, NEEDS APPROVAL), read once; nil
    /// while the export has not arrived, which the list draws as its
    /// placeholder row.
    static let referrals: String? = TCConsentCopy.referralCopyJSON()
        .flatMap(ReferralCopy.decode(fromJSON:))?.title
}
