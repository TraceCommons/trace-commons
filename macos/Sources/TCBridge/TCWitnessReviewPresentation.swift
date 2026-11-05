import Foundation

/// The core owns which review failures have prose. The original daemon
/// state and retry timestamp remain facts, not translated display strings.
public enum TCWitnessReviewPresentation {
    public static func view(label: String, original: [String: Any]) -> [String: Any]? {
        guard let json = LocalizedPresentation.json("witness_review_failure_json", parameters: [
            "label": label, "view": original
        ]) else { return nil }
        return try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
    }
}
