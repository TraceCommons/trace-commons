import Foundation

/// The build's channel identity, as the dylib reports it through
/// `tc_build_channel_json` (staging design, sections 4.1 and 4.3).
///
/// Swift holds no copy of any of these values. The channel module in the
/// contributor crate is their one spelling; the app reads them from the
/// dylib it links and passes this value to everything that needs one, so a
/// staging dylib names the staging directory, and a production one names
/// production's.
public struct BuildChannel: Decodable, Equatable, Sendable {
    /// `production` or `staging`.
    public let channel: String
    /// The state directory's name under Application Support.
    public let stateDirName: String
    /// The one environment variable that overrides the state directory.
    public let stateDirEnv: String
    /// The URL scheme the bundle registers.
    public let urlScheme: String
    /// The NEAR AI dashboard the funding row's credits link opens.
    public let nearAiDashboardOrigin: String
    /// The NEAR AI sign-in providers this build offers, in the order a
    /// picker lists them (the daemon's `provider` values). A staging build
    /// names `google` only: it signs in through NEAR AI's Google login.
    public let nearAiSignInProviders: [String]

    public init(
        channel: String, stateDirName: String, stateDirEnv: String,
        urlScheme: String, nearAiDashboardOrigin: String,
        nearAiSignInProviders: [String]
    ) {
        self.channel = channel
        self.stateDirName = stateDirName
        self.stateDirEnv = stateDirEnv
        self.urlScheme = urlScheme
        self.nearAiDashboardOrigin = nearAiDashboardOrigin
        self.nearAiSignInProviders = nearAiSignInProviders
    }

    enum CodingKeys: String, CodingKey {
        case channel
        case stateDirName = "state_dir_name"
        case stateDirEnv = "state_dir_env"
        case urlScheme = "url_scheme"
        case nearAiDashboardOrigin = "near_ai_dashboard_origin"
        case nearAiSignInProviders = "near_ai_sign_in_providers"
    }

    /// The dylib's answer, or nil when it is absent, unreadable, or has an
    /// empty field (an empty provider list included). Nil is a refusal to
    /// start, never a fallback.
    public static func decode(fromJSON json: String?) -> BuildChannel? {
        guard let json,
            let channel = try? JSONDecoder().decode(BuildChannel.self, from: Data(json.utf8))
        else { return nil }
        let fields = [
            channel.channel, channel.stateDirName, channel.stateDirEnv,
            channel.urlScheme, channel.nearAiDashboardOrigin,
        ]
        let providers = channel.nearAiSignInProviders
        if providers.isEmpty || providers.contains(where: \.isEmpty) { return nil }
        return fields.contains(where: \.isEmpty) ? nil : channel
    }
}

/// Whether the bundle and the dylib are the same channel (spec 4.3).
///
/// The app is a Swift bundle around a Rust dylib, built in two steps. A
/// staging bundle around a production dylib would show one channel while
/// talking to the other, so the app starts nothing unless its Info.plist
/// `TCChannel` is exactly the dylib's channel. A bundle with no `TCChannel`
/// and a dylib whose channel cannot be read both refuse.
public enum ChannelAgreement {
    public enum Verdict: Equatable, Sendable {
        case agreed(BuildChannel)
        case refused
    }

    public static func verdict(plistChannel: String?, dylib: BuildChannel?) -> Verdict {
        // `!plistChannel.isEmpty` is belt and braces: `decode` already
        // refuses an empty dylib channel, so an empty plist value could
        // never match. It stays so this check does not lean on that one.
        guard let dylib, let plistChannel, !plistChannel.isEmpty, plistChannel == dylib.channel
        else { return .refused }
        return .agreed(dylib)
    }
}
