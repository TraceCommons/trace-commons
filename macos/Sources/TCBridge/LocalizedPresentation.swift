import CTraceCommons
import Foundation
import TCLocalization

/// The core selects explicit messages; only their prose leaves are localized.
/// Raw wire inputs and ordinary strings are never rewritten or inferred as copy.
internal enum LocalizedPresentation {
    static func json(_ operation: String, parameters: [String: Any] = [:]) -> String? {
        guard JSONSerialization.isValidJSONObject(parameters),
              let data = try? JSONSerialization.data(withJSONObject: parameters),
              let input = String(data: data, encoding: .utf8) else { return nil }
        return json(operation, inputJSON: input)
    }

    static func json(_ operation: String, inputJSON: String) -> String? {
        // Reject embedded NULs rather than silently truncating user content.
        guard !operation.utf8.contains(0), !inputJSON.utf8.contains(0) else { return nil }
        let raw = operation.withCString { operationPointer in
            inputJSON.withCString { tc_localized_presentation_json(operationPointer, $0) }
        }
        return renderOwned(raw)
    }

    static func line(_ operation: String, parameters: [String: Any] = [:]) -> String? {
        decodeLine(json(operation, parameters: parameters))
    }

    static func line(_ operation: String, inputJSON: String) -> String? {
        decodeLine(json(operation, inputJSON: inputJSON))
    }

    static func renderOwned(_ raw: UnsafeMutablePointer<CChar>?) -> String? {
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return Localization.current.presentationJSON(String(cString: raw))
    }

    private static func decodeLine(_ json: String?) -> String? {
        guard let json else { return nil }
        return try? JSONDecoder().decode(String.self, from: Data(json.utf8))
    }
}
