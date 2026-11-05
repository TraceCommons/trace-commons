import CTraceCommons
import Foundation
import TCLocalization

public struct TCOnboardingCopy: Decodable, Sendable {
    public let welcomeBody: String
    public let doneBody: String
    public let watcherStartFailed: String
    public let notificationPurpose: String
    public let notificationHeading: String
    public let notificationOffer: String
    public let notificationAllowed: String
    public let notificationDenied: String
    public let notificationUnknown: String
    public let notificationNotAsked: String
    public let notificationAllow: String
    public let notNow: String
    public let systemSettings: String

    public static func load() -> Self? {
        guard let ptr = tc_onboarding_copy() else { return nil }
        defer { tc_string_free(ptr) }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        guard let json = Localization.current.copyJSON(String(cString: ptr), namespace: "onboarding_copy") else { return nil }
        return try? decoder.decode(Self.self, from: Data(json.utf8))
    }
}
