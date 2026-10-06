import XCTest

/// A release build compiles without the `#if DEBUG` code, and `swift test`
/// only ever builds debug, so nothing in the suite saw a release-visible
/// line reach for a debug-only type: #1241 failed `swift build -c release`
/// twice that way (`TracesStore` read `MonitorWords` in a default argument,
/// `ToolAnswerRow` called `TracesTreeView.glassTool`). Only the tag-push
/// release workflow builds release, so this reads the source instead: every
/// type declared only under `#if DEBUG` is named nowhere a release build
/// compiles.
final class ReleaseBuildGateTests: XCTestCase {
    static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp")

    /// Each line of `source` but the directives, its 1-based line number in
    /// the file, and whether a release build compiles it. A `DEBUG` branch
    /// is hidden; a branch after `#if !DEBUG` (or `#elseif !DEBUG`) is
    /// hidden, since that one is always taken in release; any other
    /// condition, and an `#else` after only those, is taken as compiled.
    static func releaseLines(_ source: String) -> [(line: String, number: Int, release: Bool)] {
        // Per open `#if`: whether the current branch is hidden in release,
        // and whether an earlier branch is always taken in release.
        var frames: [(hidden: Bool, settled: Bool)] = []
        func branch(_ condition: String, settled: Bool) -> (hidden: Bool, settled: Bool) {
            if settled { return (true, true) }
            if condition == "DEBUG" { return (true, false) }
            if condition == "!DEBUG" { return (false, true) }
            return (false, false)
        }
        var out: [(String, Int, Bool)] = []
        for (index, raw) in source.components(separatedBy: "\n").enumerated() {
            let trimmed = raw.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("#if ") {
                frames.append(branch(trimmed.dropFirst(4).trimmingCharacters(in: .whitespaces), settled: false))
                continue
            }
            if trimmed.hasPrefix("#elseif "), let last = frames.indices.last {
                let condition = trimmed.dropFirst(8).trimmingCharacters(in: .whitespaces)
                frames[last] = branch(condition, settled: frames[last].settled)
                continue
            }
            if trimmed == "#else" || trimmed.hasPrefix("#else "), let last = frames.indices.last {
                frames[last] = (frames[last].settled, frames[last].settled)
                continue
            }
            if trimmed.hasPrefix("#endif") {
                _ = frames.popLast()
                continue
            }
            out.append((raw, index + 1, !frames.contains { $0.hidden }))
        }
        return out
    }

    /// A line without its string literals and its `//` comment.
    static func code(_ line: String) -> String {
        let bare = line.replacingOccurrences(of: #""[^"]*""#, with: "\"\"", options: .regularExpression)
        guard let comment = bare.range(of: "//") else { return bare }
        return String(bare[..<comment.lowerBound])
    }

    /// A top-level type: unindented, so a nested `Kind` or `Page` inside a
    /// debug-only view is not mistaken for a release type's of the same name.
    static let declaration = try! NSRegularExpression(
        pattern: #"^(?:@\w+\s+)*(?:(?:public|private|fileprivate|internal|final|nonisolated)\s+)*(?:struct|enum|class|actor|protocol|typealias)\s+(\w+)"#)

    static func declared(in lines: [String]) -> Set<String> {
        var names: Set<String> = []
        for line in lines {
            let range = NSRange(line.startIndex..., in: line)
            if let match = declaration.firstMatch(in: line, range: range),
               let name = Range(match.range(at: 1), in: line) {
                names.insert(String(line[name]))
            }
        }
        return names
    }

    /// A member of a top-level type or extension: one level in, so a local inside
    /// one of its functions is not taken for a member.
    static let member = try! NSRegularExpression(
        pattern: #"^    (?:@\w+(?:\([^)]*\))?\s+)*(?:(?:public|private|fileprivate|internal|static|class|final|nonisolated|override|mutating)\s+)*(?:var|let|func|struct|enum|class|actor|typealias)\s+(\w+)"#)

    static func names(_ pattern: NSRegularExpression, in lines: [String]) -> Set<String> {
        var names: Set<String> = []
        for line in lines {
            let code = code(line)
            for match in pattern.matches(in: code, range: NSRange(code.startIndex..., in: code)) {
                if let name = Range(match.range(at: 1), in: code) { names.insert(String(code[name])) }
            }
        }
        return names
    }

    static func sources() throws -> [(path: String, lines: [(line: String, number: Int, release: Bool)])] {
        let walker = try XCTUnwrap(FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil))
        var out: [(String, [(line: String, number: Int, release: Bool)])] = []
        for case let url as URL in walker where url.pathExtension == "swift" {
            let text = try String(contentsOf: url, encoding: .utf8)
            out.append((url.path.replacingOccurrences(of: root.path + "/", with: ""), releaseLines(text)))
        }
        return out
    }

    func test_theScannerHidesDebugBodiesAndShowsTheirElse() {
        let lines = Self.releaseLines("a\n#if DEBUG\nb\n#else\nc\n#endif\n#if !DEBUG\nd\n#endif\ne")
        XCTAssertEqual(lines.filter(\.release).map(\.line), ["a", "c", "d", "e"])
        // A finding names the file's own line, directives counted.
        XCTAssertEqual(lines.filter(\.release).map(\.number), [1, 5, 8, 10])
    }

    /// The two that broke #1241's release build are found by this scan.
    func test_theScannerSeesADebugTypeInReleaseCode() {
        let lines = Self.releaseLines("#if DEBUG\nenum Words {}\n#endif\nlet x = Words.self")
        let debugOnly = Self.declared(in: lines.filter { !$0.release }.map(\.line))
        XCTAssertEqual(debugOnly, ["Words"])
        XCTAssertTrue(lines.filter(\.release).contains { $0.line.contains("Words.") })
    }

    /// `#elseif` is its own branch: after `#if !DEBUG` it is never
    /// compiled in release, `#elseif DEBUG` is hidden, any other condition
    /// is taken as compiled. Read as `#else`, `#if X ... #elseif DEBUG`
    /// would show its debug body to a release build.
    func test_theScannerReadsElseifAsItsOwnBranch() {
        let source = [
            "#if DEBUG", "b", "#elseif X", "c", "#else", "d", "#endif",
            "#if !DEBUG", "e", "#elseif X", "f", "#else", "g", "#endif",
            "#if X", "h", "#elseif DEBUG", "i", "#endif",
        ].joined(separator: "\n")
        XCTAssertEqual(Self.releaseLines(source).filter(\.release).map(\.line), ["c", "d", "e", "h"])
    }

    /// A member declared in a `#if DEBUG` extension of a type that release
    /// builds also have (`MonitorWords`' words for a debug-only screen stay
    /// beside that screen) is as absent from release as a debug-only type,
    /// when release code reaches it through the type's name. A same-named
    /// local, or a member the release type declares itself, is not one.
    func test_theScannerSeesADebugExtensionMemberInReleaseCode() {
        let source = [
            "struct W {", "    static var shared: Int { 0 }", "}",
            "#if DEBUG", "extension W {", "    static var onlyInDebug: Int { 1 }",
            "    @MainActor static func debugOnlyCall() {}", "    static var local: Int { 2 }",
            "    static var final: Int { 3 }", "}", "#endif",
            "let a = W.onlyInDebug", "let b = W.debugOnlyCall()", "let c = W.shared",
            "let local = 0", "final class Z {}",
        ].joined(separator: "\n")
        let found = Self.findings([("W.swift", Self.releaseLines(source))])
        XCTAssertEqual(found, ["W.swift:12: W.onlyInDebug", "W.swift:13: W.debugOnlyCall"])
    }

    /// Every release-compiled line that names something only a debug build
    /// declares, as `path:line: name`: a debug-only type wherever it is
    /// named, and a member of a debug-only extension where it is reached
    /// through its type (`MonitorWords.word`). A bare or implicit use
    /// (`.word`) is not found: only `swift build -c release` sees everything.
    static func findings(_ sources: [(path: String, lines: [(line: String, number: Int, release: Bool)])]) -> [String] {
        let debugOnlyTypes = debugOnlyTypes(sources)
        let debugOnlyMembers = debugOnlyMembers(sources)
        // Each line's identifiers once, matched against the set: a regex per
        // name per line took most of a minute.
        var found: [String] = []
        for (path, lines) in sources {
            for entry in lines where entry.release {
                let code = code(entry.line)
                let named = identifiers(code).intersection(debugOnlyTypes)
                    .union(qualified(code).intersection(debugOnlyMembers))
                for name in named.sorted() {
                    found.append("\(path):\(entry.number): \(name)")
                }
            }
        }
        return found
    }

    static func debugOnlyTypes(_ sources: [(path: String, lines: [(line: String, number: Int, release: Bool)])]) -> Set<String> {
        let all = sources.flatMap(\.lines)
        let releaseDeclared = declared(in: all.filter(\.release).map(\.line))
        return declared(in: all.filter { !$0.release }.map(\.line)).subtracting(releaseDeclared)
    }

    /// `Type.member` for each member a debug-only top-level block (an
    /// extension, in practice) declares on a type release builds also
    /// have, less the members release code declares on that type.
    static func debugOnlyMembers(_ sources: [(path: String, lines: [(line: String, number: Int, release: Bool)])]) -> Set<String> {
        let release = members(sources.flatMap { $0.lines.filter(\.release) })
        let debug = members(sources.flatMap { $0.lines.filter { !$0.release } })
        var out: Set<String> = []
        for (type, names) in debug where release[type] != nil {
            for name in names.subtracting(release[type] ?? []) { out.insert("\(type).\(name)") }
        }
        return out
    }

    /// A top-level type or extension that opens a block on its line.
    static let block = try! NSRegularExpression(
        pattern: #"^(?:@\w+\s+)*(?:(?:public|private|fileprivate|internal|final|nonisolated)\s+)*(?:extension|struct|enum|class|actor)\s+(\w+)[^{}]*\{\s*$"#)

    /// The members each top-level type or extension block in `lines`
    /// declares one level in, by the block's type name.
    static func members(_ lines: [(line: String, number: Int, release: Bool)]) -> [String: Set<String>] {
        var out: [String: Set<String>] = [:]
        var current: String?
        for entry in lines {
            let line = entry.line
            if current == nil {
                let range = NSRange(line.startIndex..., in: line)
                if let match = block.firstMatch(in: line, range: range), let name = Range(match.range(at: 1), in: line) {
                    current = String(line[name])
                    out[current!, default: []] = out[current!, default: []]
                }
            } else if line.hasPrefix("}") {
                current = nil
            } else if let type = current {
                out[type, default: []].formUnion(names(member, in: [line]))
            }
        }
        return out
    }

    static let identifier = try! NSRegularExpression(pattern: #"[A-Za-z_][A-Za-z0-9_]*"#)

    static func identifiers(_ code: String) -> Set<String> {
        Set(identifier.matches(in: code, range: NSRange(code.startIndex..., in: code))
            .compactMap { Range($0.range, in: code).map { String(code[$0]) } })
    }

    static let qualifiedIdentifier = try! NSRegularExpression(pattern: #"\b([A-Z][A-Za-z0-9_]*)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)"#)

    /// Each `Type.member` a line names.
    static func qualified(_ code: String) -> Set<String> {
        Set(qualifiedIdentifier.matches(in: code, range: NSRange(code.startIndex..., in: code)).compactMap { match in
            guard let type = Range(match.range(at: 1), in: code), let name = Range(match.range(at: 2), in: code) else { return nil }
            return "\(code[type]).\(code[name])"
        })
    }

    func test_noReleaseCodeNamesADebugOnlyType() throws {
        let sources = try Self.sources()
        XCTAssertFalse(Self.debugOnlyTypes(sources).isEmpty,
                       "no debug-only types found: the scanner is not reading the sources")
        XCTAssertTrue(Self.debugOnlyMembers(sources).contains("MonitorWords.creditNotCurrency"),
                      "the debug-only MonitorWords extension is not read: the scanner is not reading extensions")
        XCTAssertEqual(Self.findings(sources), [],
                       "release code names a type or member that exists only in debug builds")
    }
}
