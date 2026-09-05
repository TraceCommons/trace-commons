import Foundation

/// Shared Rust wording; no shell-owned sentences or status interpretations.
public struct ComputeCopy: Decodable, Equatable, Sendable {
    public let destination: String
    public let introduction: String
    public let allowanceLabel: String
    public let allowanceDetail: String
    public let enable: String
    public let resume: String
    public let pause: String
    public let disable: String
    public let quitDetail: String?
    public let quitRefused: String?

    public static func decode(_ json: String) -> Self? {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try? decoder.decode(Self.self, from: Data(json.utf8))
    }
}

public struct ComputeSnapshot: Decodable, Equatable, Sendable {
    public let schema: String
    public let state: String
    public let reason: String
    public let title: String
    public let detail: String
    public let consentGranted: Bool
    public let ramAllowanceGib: UInt64?
    public let available: Bool
    public let canEnable: Bool
    public let canResume: Bool
    public let canPause: Bool
    public let commandPending: Bool?
    public let workerStopped: Bool?
    public let copy: ComputeCopy

    public static func decode(_ json: String) -> Self? {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        guard let value = try? decoder.decode(Self.self, from: Data(json.utf8)),
              value.schema == "trace_commons.compute_status.v1",
              ["disabled", "unavailable", "starting", "waiting", "training", "serving",
               "draining", "paused", "stale", "error"].contains(value.state) else { return nil }
        return value
    }
}
