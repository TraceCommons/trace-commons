@testable import TCShellCore

/// The two channels as `tc_build_channel_json` reports them. Fixtures for
/// the injected value: TCShellCore never links the dylib, so the real call
/// is checked in TCBridgeTests (`BuildChannelBridgeTests`).
enum BuildChannelFixture {
    static let production = BuildChannel(
        channel: "production",
        stateDirName: "trace-commons",
        stateDirEnv: "TRACE_COMMONS_CONTRIBUTOR_DIR",
        urlScheme: "tracecommons",
        nearAiDashboardOrigin: "https://cloud.near.ai",
        nearAiSignInProviders: ["near", "github", "google"])

    static let staging = BuildChannel(
        channel: "staging",
        stateDirName: "trace-commons-staging",
        stateDirEnv: "TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR",
        urlScheme: "tracecommons-staging",
        nearAiDashboardOrigin: "https://cloud-stg.near.ai",
        nearAiSignInProviders: ["google"])
}
