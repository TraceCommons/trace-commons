import Foundation
import XCTest
@testable import TCLocalization

final class LocalizationTests: XCTestCase {
    func testUnsupportedLanguageFallsBackToEnglishAndKeepsRegion() {
        let words = Localization(preferredLanguages: ["zz-ZZ"], formattingLocale: Locale(identifier: "de_DE"))
        XCTAssertEqual(words.language, "en")
        XCTAssertEqual(words.text("app.language.title"), "Language")
        XCTAssertEqual(words.number(12345), "12.345")
        XCTAssertEqual(words.locale.region?.identifier, "DE")
    }

    func testParametersAreSinglePassAndMayChangeOrder() {
        XCTAssertEqual(Localization.fill("{folder}: {count}", ["folder": "my {count} %@ folder", "count": "23"]),
                       "my {count} %@ folder: 23")
        XCTAssertEqual(Localization.fill("{count} — {folder}", ["folder": "日本語 العربية", "count": "23"]),
                       "23 — 日本語 العربية")
        XCTAssertEqual(Localization.fill("{future} /usr/bin/near-ai", [:]), "{future} /usr/bin/near-ai")
    }

    func testFixedCopyPreservesUnknownFieldsAndChangedSource() throws {
        let words = Localization(preferredLanguages: ["en"])
        let json = #"{"frame":{"back":"Back","user_content":"Back"},"id":"auto_upload","path":"/tmp/Back","count":17,"enabled":true}"#
        let input = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? NSDictionary
        let output = try XCTUnwrap(words.copyJSON(json, namespace: "first_run_copy"))
        XCTAssertEqual(try JSONSerialization.jsonObject(with: Data(output.utf8)) as? NSDictionary, input)
        XCTAssertNil(words.copyJSON(nil, namespace: "first_run_copy"))
        XCTAssertEqual(words.copyJSON("broken JSON", namespace: "first_run_copy"), "broken JSON")
    }

    func testLanguagePreferenceDoesNotAdvertiseUnreviewedLocales() throws {
        let suite = "TCLocalizationTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.setVolatileDomain(["AppleLanguages": ["en"]], forName: UserDefaults.argumentDomain)
        XCTAssertEqual(Localization.languagePreference(defaults: defaults, applicationID: suite), "")
        Localization.setLanguagePreference("en", defaults: defaults)
        XCTAssertEqual(Localization.languagePreference(defaults: defaults, applicationID: suite), "en")
        Localization.setLanguagePreference("zz-ZZ", defaults: defaults)
        XCTAssertEqual(Localization.languagePreference(defaults: defaults, applicationID: suite), "en")
        Localization.setLanguagePreference("", defaults: defaults)
        XCTAssertEqual(Localization.languagePreference(defaults: defaults, applicationID: suite), "")
    }

    func testNativePluralMessagesAndUnknownMessageRefusal() {
        let words = Localization(preferredLanguages: ["en"], formattingLocale: Locale(identifier: "en_US"))
        XCTAssertEqual(words.messageJSON(#"{"key":"privacy.residual.unchecked"}"#),
                       "This trace could not be checked for leftover secrets, so treat it as holding one.")
        XCTAssertNil(words.messageJSON(#"{"key":"future.message"}"#))
        XCTAssertNil(words.messageJSON(#"{"key":"privacy.residual.secrets"}"#))
        let huge = words.presentationJSON(#"{"key":"privacy.residual.secrets","english":{"one":"{count} secret is still in this trace","other":"{count} secrets are still in this trace"},"parameters":{"count":18446744073709551615}}"#)
        XCTAssertEqual(huge.flatMap { try? JSONDecoder().decode(String.self, from: Data($0.utf8)) },
                       "18446744073709551615 secrets are still in this trace")
        #if os(macOS)
        // Apple's Foundation resolves .stringsdict; swift-corelibs does not.
        for (count, expected) in [(0, "0 secrets are still in this trace"), (1, "1 secret is still in this trace"),
                                  (2, "2 secrets are still in this trace"), (21, "21 secrets are still in this trace")] {
            XCTAssertEqual(words.messageJSON("{\"key\":\"privacy.residual.secrets\",\"parameters\":{\"count\":\(count)}}"), expected)
        }
        #endif
    }

    func testKeyedTranslationDoesNotTranslateUserContentOrObsoleteClaims() throws {
        // A small reviewed-language fixture exercises actual Bundle lookup,
        // rather than asserting that English round-trips through English.
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".bundle")
        defer { try? FileManager.default.removeItem(at: root) }
        for language in ["en", "fr", "zh-Hans", "zh-Hant", "ar"] {
            try FileManager.default.createDirectory(at: root.appendingPathComponent(language + ".lproj"), withIntermediateDirectories: true)
        }
        let catalog = #"{"locales":{"en":{"name":"English","enabled":true},"fr":{"name":"Français","enabled":true},"zh-Hans":{"name":"Simplified","enabled":true},"zh-Hant":{"name":"Traditional","enabled":true},"ar":{"name":"Arabic","enabled":true}},"messages":{"test.title":{"english":"Remove {folder}?"},"test.fallback":{"english":"Fallback"}}}"#
        try Data(catalog.utf8).write(to: root.appendingPathComponent("Catalog.json"))
        try Data(#""test.title" = "Supprimer {folder} ?";"#.utf8)
            .write(to: root.appendingPathComponent("fr.lproj/Localizable.strings"))
        try Data(#""test.fallback" = "English fallback";"#.utf8)
            .write(to: root.appendingPathComponent("en.lproj/Localizable.strings"))
        let bundle = try XCTUnwrap(Bundle(url: root))
        let words = Localization(preferredLanguages: ["fr-CA"], formattingLocale: Locale(identifier: "en_CA"), resources: bundle)
        XCTAssertEqual(words.language, "fr")
        XCTAssertEqual(words.text("test.fallback"), "English fallback")
        for (requested, expected) in [("zh-TW", "zh-Hant"), ("zh-CN", "zh-Hans"), ("fr_CA", "fr"), ("zz", "en")] {
            XCTAssertEqual(Localization(preferredLanguages: [requested], formattingLocale: .current, resources: bundle).language, expected)
        }
        XCTAssertTrue(Localization(preferredLanguages: ["ar"], formattingLocale: .current, resources: bundle).isRightToLeft)
        XCTAssertEqual(words.text("test.title", parameters: ["folder": "my {count} folder"]), "Supprimer my {count} folder ?")
        let output = try XCTUnwrap(words.copyJSON(#"{"title":"Remove {folder}?","user":"Remove {folder}?"}"#, namespace: "test"))
        let fields = try JSONDecoder().decode([String: String].self, from: Data(output.utf8))
        XCTAssertEqual(fields["title"], "Supprimer {folder} ?")
        XCTAssertEqual(fields["user"], "Remove {folder}?")
        let presentation = #"{"title":{"key":"test.title","english":"Remove {folder}?","parameters":{"folder":"my {count} %@ folder"}},"user":"Remove {folder}?","id":"auto_upload","count":17,"enabled":true}"#
        let rendered = try XCTUnwrap(words.presentationJSON(presentation))
        let tree = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(rendered.utf8)) as? [String: Any])
        XCTAssertEqual(tree["title"] as? String, "Supprimer my {count} %@ folder ?")
        XCTAssertEqual(tree["user"] as? String, "Remove {folder}?")
        XCTAssertEqual(tree["id"] as? String, "auto_upload")
        XCTAssertEqual(tree["count"] as? Int, 17)
        XCTAssertEqual(tree["enabled"] as? Bool, true)
        let stale = try XCTUnwrap(words.presentationJSON(#"{"key":"test.title","english":"New claim for {folder}","parameters":{"folder":"raw {name}"}}"#))
        XCTAssertEqual(try JSONDecoder().decode(String.self, from: Data(stale.utf8)), "New claim for raw {name}")
        let amount = try XCTUnwrap(words.presentationJSON(#"{"key":"future.amount","english":"Paid {amount}","parameters":{"amount":{"currency":"12.34","code":"USD","english":"$12.34"}}}"#))
        XCTAssertEqual(try JSONDecoder().decode(String.self, from: Data(amount.utf8)), "Paid $12.34")
        func translatedParameter(_ parameter: [String: Any]) throws -> String {
            let tree: [String: Any] = ["key": "test.title", "english": "Remove {folder}?", "parameters": ["folder": parameter]]
            let json = String(decoding: try JSONSerialization.data(withJSONObject: tree), as: UTF8.self)
            let rendered = try XCTUnwrap(words.presentationJSON(json))
            return try JSONDecoder().decode(String.self, from: Data(rendered.utf8))
        }
        XCTAssertEqual(try translatedParameter(["decimal": "6.0", "english": "6.0", "minimum_fraction_digits": 1, "maximum_fraction_digits": 1]),
                       "Supprimer 6,0 ?")
        XCTAssertEqual(try translatedParameter(["date": "2026-10-08", "english": "8 October"]), "Supprimer 8 octobre ?")
        XCTAssertTrue(try translatedParameter(["currency": "12.3", "code": "USD", "english": "$12.30", "minimum_fraction_digits": 2, "maximum_fraction_digits": 2]).contains("12,30"))
        let nested = try XCTUnwrap(words.presentationJSON(#"{"key":"future.join","english":"Wrap {first}","parameters":{"first":{"key":"test.title","english":"Remove {folder}?","parameters":{"folder":"raw {count}"}}}}"#))
        XCTAssertEqual(try JSONDecoder().decode(String.self, from: Data(nested.utf8)), "Wrap Supprimer raw {count} ?")
        let changed = try XCTUnwrap(words.copyJSON(#"{"title":"New privacy claim"}"#, namespace: "test"))
        XCTAssertEqual(try JSONDecoder().decode([String: String].self, from: Data(changed.utf8))["title"], "New privacy claim")
    }

    #if os(macOS)
    func testNativePluralCategoriesUseSelectedLanguageAndEnglishFallbackRules() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".bundle")
        defer { try? FileManager.default.removeItem(at: root) }
        for language in ["en", "ar", "ru"] {
            try FileManager.default.createDirectory(at: root.appendingPathComponent(language + ".lproj"), withIntermediateDirectories: true)
        }
        let catalog = #"{"locales":{"en":{"name":"English","enabled":true},"ar":{"name":"Arabic fixture","enabled":true},"ru":{"name":"Russian fixture","enabled":true}},"messages":{"test.category":{"english":{"one":"one","other":"other"}}}}"#
        try Data(catalog.utf8).write(to: root.appendingPathComponent("Catalog.json"))
        for language in ["en", "ar"] {
            var selector: [String: Any] = ["NSStringFormatSpecTypeKey": "NSStringPluralRuleType", "NSStringFormatValueTypeKey": "lld"]
            for category in language == "en" ? ["one", "other"] : ["zero", "one", "two", "few", "many", "other"] {
                selector[category] = category
            }
            let native: [String: Any] = ["test.category": ["NSStringLocalizedFormatKey": "%#@count@", "count": selector]]
            let data = try PropertyListSerialization.data(fromPropertyList: native, format: .xml, options: 0)
            try data.write(to: root.appendingPathComponent(language + ".lproj/Localizable.stringsdict"))
        }
        let bundle = try XCTUnwrap(Bundle(url: root))
        let arabic = Localization(preferredLanguages: ["ar"], formattingLocale: Locale(identifier: "en_US"), resources: bundle)
        for (count, category) in [(0, "zero"), (1, "one"), (2, "two"), (3, "few"), (11, "many"), (102, "other")] {
            XCTAssertEqual(arabic.text("test.category", count: Int64(count)), category)
        }
        let missingRussian = Localization(preferredLanguages: ["ru"], formattingLocale: Locale(identifier: "en_US"), resources: bundle)
        XCTAssertEqual(missingRussian.text("test.category", count: 21), "other", "English fallback must not use Russian's singular category for 21")
    }
    #endif
}
