#if DEBUG
import Foundation

/// Counts row construction in native collection regression tests. No
/// observation: recording a row must never invalidate a SwiftUI body.
@MainActor
enum OtherCollectionRowBodies {
    private static var recording = false
    static var built: [String: Set<String>] = [:]

    static func record(_ collection: String, _ id: String) {
        guard recording else { return }
        built[collection, default: []].insert(id)
    }

    static func startRecording() {
        built = [:]
        recording = true
    }

    static func stopRecording() {
        recording = false
        built = [:]
    }
}
#endif
