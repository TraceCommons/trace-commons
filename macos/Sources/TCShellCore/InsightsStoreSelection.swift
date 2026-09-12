import Foundation

public enum InsightsStoreSelection: Equatable, Sendable {
    case standard
    case custom(String)
    case refused(Refusal)

    public enum Refusal: String, Equatable, Sendable {
        case duplicateOption
        case missingPath
        case relativePath
        case pathMissing
        case notADirectory
    }

    public var storeDirectory: String? {
        if case .custom(let path) = self { return path }
        return nil
    }

    public var refusal: Refusal? {
        if case .refused(let reason) = self { return reason }
        return nil
    }

    public static func parse(
        arguments: [String],
        probe: @Sendable (String) -> StateDirectory.Probe.Verdict = { path in
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory) else {
                return .absent
            }
            return isDirectory.boolValue ? .directory : .file
        }
    ) -> Self {
        let indexes = arguments.indices.filter {
            arguments[$0] == "--insights-store" || arguments[$0].hasPrefix("--insights-store=")
        }
        guard !indexes.isEmpty else { return .standard }
        guard indexes.count == 1 else { return .refused(.duplicateOption) }
        let index = indexes[0]
        guard arguments[index] == "--insights-store", index + 1 < arguments.endIndex,
              !arguments[index + 1].hasPrefix("-") else { return .refused(.missingPath) }
        let supplied = arguments[index + 1]
        guard supplied.hasPrefix("/") else { return .refused(.relativePath) }
        let path = URL(fileURLWithPath: supplied).standardizedFileURL.path
        switch probe(path) {
        case .absent: return .refused(.pathMissing)
        case .file: return .refused(.notADirectory)
        case .directory: return .custom(path)
        }
    }
}
