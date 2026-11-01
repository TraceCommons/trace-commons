import Foundation
import TCBridge
import TCShellCore

/// The channel of the dylib the suite links, read through the same ABI the
/// app uses (`tc_build_channel_json`). Tests that pin a host, a link or an
/// origin derive it from here, so the production suite asserts production's
/// values exactly and the staging release workflow, which links the staging
/// dylib, asserts staging's. Nothing here is a default: no answer fails the
/// test.
enum LinkedChannel {
    static let channel: BuildChannel = {
        guard let channel = BuildChannel.decode(fromJSON: TCBuildChannel.json()) else {
            fatalError("the linked dylib reported no readable build channel")
        }
        return channel
    }()

    /// The NEAR AI dashboard origin, such as `https://host`.
    static var dashboardOrigin: String { channel.nearAiDashboardOrigin }

    /// Its host.
    static var dashboardHost: String {
        guard let host = URL(string: dashboardOrigin)?.host else {
            fatalError("the dashboard origin has no host")
        }
        return host
    }

    /// The public site host (`channel::PUBLIC_SITE_HOST`). The dylib does
    /// not report it, so it is named here per channel; an unknown channel
    /// fails the test.
    static var siteHost: String {
        switch channel.channel {
        case "production": return "tracecommons.ai"
        case "staging": return "staging.tracecommons.ai"
        default: fatalError("unknown linked channel \(channel.channel)")
        }
    }

    /// The host of an invite issued for this channel (`issuer.` + the site).
    static var inviteHost: String { "issuer.\(siteHost)" }

    /// A well-formed invite for this channel.
    static func invite(port: String = "") -> String {
        "https://\(inviteHost)\(port)/onboard#VQWWPGYSG8Y4LTP6"
    }

    /// A public run link for this channel.
    static func runLink(_ slug: String) -> String { "https://\(siteHost)/runs/\(slug)" }
}
