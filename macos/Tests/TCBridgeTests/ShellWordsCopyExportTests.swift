import TCBridge
import TCShellCore
import XCTest

/// The words this shell used to write itself cross the ABI whole
/// (`tc_shell_words_copy_json`, #1146 parity 2026-10-07), and this shell
/// decodes exactly the fields the core exports: a word added in the core is
/// one a screen can read, and a field this shell expects is one the core
/// sends.
final class ShellWordsCopyExportTests: XCTestCase {
    private static func camel(_ snake: String) -> String {
        let parts = snake.split(separator: "_")
        return ([String(parts[0])] + parts.dropFirst().map { $0.prefix(1).uppercased() + $0.dropFirst() }).joined()
    }

    private static func labels(_ value: Any) -> [String] {
        Mirror(reflecting: value).children.compactMap(\.label).sorted()
    }

    func testTheExportCarriesExactlyTheFieldsThisShellDecodes() throws {
        let json = try XCTUnwrap(TCCoreCopy.shellWordsCopyJSON())
        let copy = try XCTUnwrap(ShellWordsCopy.decode(fromJSON: json))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.map(Self.camel).sorted(), Self.labels(copy))
        let tables: [(String, Any)] = [
            ("withdrawal", copy.withdrawal), ("public_profile", copy.publicProfile), ("queue", copy.queue),
            ("history", copy.history), ("scrubbing", copy.scrubbing), ("settings", copy.settings),
            ("inference", copy.inference),
        ]
        for (name, decoded) in tables {
            let nested = try XCTUnwrap(object[name] as? [String: Any], name)
            XCTAssertEqual(nested.keys.map(Self.camel).sorted(), Self.labels(decoded), name)
        }
    }

    /// The canonical withdrawal bodies arrive verbatim, and the label maps
    /// keep their hyphenated keys through the snake-case decoder.
    func testTheWithdrawalAndProfileTablesDecodeWhole() throws {
        let copy = try XCTUnwrap(ShellWordsCopy.decode(fromJSON: TCCoreCopy.shellWordsCopyJSON()))
        XCTAssertTrue(copy.withdrawal.commonsDistributed.contains("cannot be recalled"))
        XCTAssertEqual(copy.withdrawal.confirmTitle, "Confirm withdrawal")
        XCTAssertNotNil(copy.publicProfile.failureReasons["handle-too-short"])
        XCTAssertNotNil(copy.settings.auditActions["armed-auto-upload"])
    }

    /// A table with an empty string is refused whole.
    func testAnEmptyStringRefusesTheTable() throws {
        let json = try XCTUnwrap(TCCoreCopy.shellWordsCopyJSON())
        let blanked = json.replacingOccurrences(of: "\"Confirm withdrawal\"", with: "\"\"")
        XCTAssertNotEqual(blanked, json)
        XCTAssertNil(ShellWordsCopy.decode(fromJSON: blanked))
    }
}
