import Foundation

/// Presentation only. Protocol identifiers, paths and daemon/user data never
/// pass through a value-based translation lookup. The catalog is generated
/// from the shared copy contracts; only explicitly keyed fields are eligible.
public struct Localization: Sendable {
    public static let current: Localization = {
        #if DEBUG
        if let language = ProcessInfo.processInfo.environment["TC_LOCALIZATION_LANGUAGE"] {
            return Localization(preferredLanguages: [language])
        }
        #endif
        return Localization(preferredLanguages: Locale.preferredLanguages)
    }()

    public let language: String
    public let locale: Locale
    public var isRightToLeft: Bool {
        locale.language.characterDirection == .rightToLeft
            || UserDefaults.standard.bool(forKey: "NSForceRightToLeftWritingDirection")
    }

    private let catalog: Catalog
    private let languageBundle: Bundle
    private let englishBundle: Bundle

    public struct Language: Identifiable, Sendable {
        public let id: String
        public let name: String
    }

    public static var availableLanguages: [Language] {
        catalogResource.locales.filter { $0.value.enabled }
            .map { Language(id: $0.key, name: $0.value.name) }
            .sorted { $0.id < $1.id }
    }

    /// macOS uses AppleLanguages for per-app language selection. Tables are
    /// immutable for a process lifetime; changing this requires a relaunch.
    public static func languagePreference(
        defaults: UserDefaults = .standard, applicationID: String? = Bundle.main.bundleIdentifier
    ) -> String {
        // Inherited global AppleLanguages means “System default”, not an
        // explicit app preference. Read only the app's persistent domain.
        guard let applicationID,
              let languages = defaults.persistentDomain(forName: applicationID)?["AppleLanguages"] as? [String],
              let language = languages.first,
              availableLanguages.contains(where: { $0.id == language }) else { return "" }
        return language
    }

    public static func setLanguagePreference(_ language: String, defaults: UserDefaults = .standard) {
        guard !language.isEmpty else {
            defaults.removeObject(forKey: "AppleLanguages")
            return
        }
        guard availableLanguages.contains(where: { $0.id == language }) else { return }
        defaults.set([language], forKey: "AppleLanguages")
    }

    public init(preferredLanguages: [String], formattingLocale: Locale = .current) {
        self.init(preferredLanguages: preferredLanguages, formattingLocale: formattingLocale, resources: Self.resources)
    }

    init(preferredLanguages: [String], formattingLocale: Locale, resources: Bundle) {
        catalog = Self.loadCatalog(resources)
        let supported = ["en"] + catalog.locales.filter { $0.value.enabled && $0.key != "en" }.map(\.key).sorted()
        // Match exact regions first, then language AND inferred script. Never
        // confuse zh-TW (Traditional) with zh-Hans (Simplified), or let the
        // dictionary's iteration order pick an unsupported-language fallback.
        language = preferredLanguages.lazy.compactMap { preference -> String? in
            let normalized = preference.replacingOccurrences(of: "_", with: "-")
            if let exact = supported.first(where: { $0.caseInsensitiveCompare(normalized) == .orderedSame }) { return exact }
            let wanted = Locale.Language(identifier: preference)
            return supported.first {
                let candidate = Locale.Language(identifier: $0)
                return candidate.languageCode == wanted.languageCode && candidate.script == wanted.script
            }
        }.first ?? "en"
        // Keep the person's region for numbers, currency and dates even when
        // they choose a different UI language.
        let region = formattingLocale.region?.identifier
        locale = Locale(identifier: language + (region.map { "_\($0)" } ?? ""))
        guard let path = resources.path(forResource: language, ofType: "lproj"),
              let bundle = Bundle(path: path) else {
            preconditionFailure("Generated localization resources are missing")
        }
        languageBundle = bundle
        guard let path = resources.path(forResource: "en", ofType: "lproj"),
              let bundle = Bundle(path: path) else {
            preconditionFailure("Generated English fallback resources are missing")
        }
        englishBundle = bundle
    }

    /// Render an entire message. Substitution is single-pass: a user value
    /// containing `{count}`, `%@` or another parameter is never interpreted.
    public func text(_ key: String, parameters: [String: String] = [:], count: Int64? = nil) -> String {
        guard let message = catalog.messages[key] else { return key }
        let localized = languageBundle.localizedString(forKey: key, value: nil, table: "Localizable")
        let template = localized == key
            ? englishBundle.localizedString(forKey: key, value: message.english.other, table: "Localizable")
            : localized
        var values = parameters
        let rendered: String
        if message.english.isPlural, let count {
            let pluralLocale = localized == key ? Locale(identifier: "en") : locale
            rendered = String(format: template, locale: pluralLocale, arguments: [count])
            values["count"] = number(count)
        } else {
            rendered = template
        }
        return Self.fill(rendered, values)
    }

    public func number(_ value: Int64) -> String { value.formatted(.number.locale(locale)) }
    public func decimal(_ value: Decimal) -> String { value.formatted(.number.locale(locale)) }
    public func currency(_ value: Decimal, code: String) -> String {
        value.formatted(.currency(code: code).locale(locale))
    }
    public func date(_ value: Date) -> String {
        value.formatted(Date.FormatStyle(date: .abbreviated, time: .shortened).locale(locale))
    }

    /// Render ONLY an explicit core presentation tree, never an arbitrary
    /// daemon result. Incoming English is checked before using translations.
    public func presentationJSON(_ json: String?) -> String? {
        guard let data = json?.data(using: .utf8),
              let tree = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]) else { return nil }
        func parameter(_ value: Any) -> String {
            if let fields = value as? [String: Any] {
                if let amount = fields["currency"] as? String,
                   let code = fields["code"] as? String,
                   let decimal = Decimal(string: amount, locale: Locale(identifier: "en_US_POSIX")) {
                    if let minimum = fields["minimum_fraction_digits"] as? Int,
                       let maximum = fields["maximum_fraction_digits"] as? Int {
                        return decimal.formatted(.currency(code: code).precision(.fractionLength(minimum...maximum)).locale(locale))
                    }
                    return currency(decimal, code: code)
                }
                if let amount = fields["decimal"] as? String,
                   let value = Decimal(string: amount, locale: Locale(identifier: "en_US_POSIX")) {
                    if let minimum = fields["minimum_fraction_digits"] as? Int,
                       let maximum = fields["maximum_fraction_digits"] as? Int {
                        return value.formatted(.number.precision(.fractionLength(minimum...maximum)).locale(locale))
                    }
                    return decimal(value)
                }
                if let day = fields["date"] as? String {
                    let parser = DateFormatter()
                    parser.locale = Locale(identifier: "en_US_POSIX")
                    parser.timeZone = TimeZone(secondsFromGMT: 0)
                    parser.dateFormat = "yyyy-MM-dd"
                    if let date = parser.date(from: day) {
                        return date.formatted(Date.FormatStyle(locale: locale, timeZone: TimeZone(secondsFromGMT: 0)!)
                            .day().month(.wide))
                    }
                }
                if let text = render(fields) as? String { return text }
            }
            if let value = value as? String { return value }
            if let value = value as? NSNumber { return decimal(value.decimalValue) }
            return ""
        }
        func render(_ node: Any) -> Any {
            if let fields = node as? [String: Any] {
                if let key = fields["key"] as? String,
                   let source = fields["english"],
                   let data = try? JSONSerialization.data(withJSONObject: source, options: [.fragmentsAllowed]),
                   let english = try? JSONDecoder().decode(English.self, from: data) {
                    let raw = fields["parameters"] as? [String: Any] ?? [:]
                    let values = raw.mapValues(parameter)
                    let count = (raw["count"] as? NSNumber).flatMap { value -> Int64? in
                        guard value.decimalValue >= Decimal(Int64.min),
                              value.decimalValue <= Decimal(Int64.max) else { return nil }
                        return value.int64Value
                    }
                    if let message = catalog.messages[key], message.english == english,
                       !message.english.isPlural || count != nil {
                        return text(key, parameters: values, count: count)
                    }
                    // A newer core's changed claim stays canonical English.
                    // This branch applies English rules, never a guessed
                    // translation or a plural rule for the selected locale.
                    let template: String
                    switch english {
                    case .text(let value): template = value
                    case .plural(let forms): template = forms[count == 1 ? "one" : "other"] ?? english.other
                    }
                    return Self.fill(template, raw.mapValues { value in
                        if let fields = value as? [String: Any], fields["key"] is String { return parameter(value) }
                        if let fields = value as? [String: Any], let original = fields["english"] as? String { return original }
                        if let value = value as? NSNumber { return value.stringValue }
                        return parameter(value)
                    })
                }
                return fields.mapValues(render)
            }
            if let values = node as? [Any] { return values.map(render) }
            return node
        }
        guard let result = try? JSONSerialization.data(withJSONObject: render(tree), options: [.fragmentsAllowed, .sortedKeys]) else { return nil }
        return String(data: result, encoding: .utf8)
    }

    /// Locale-neutral core messages currently carry integer parameters. Do
    /// not render unknown keys or an incomplete plural as a safety claim.
    public func messageJSON(_ json: String?) -> String? {
        struct Message: Decodable { let key: String; let parameters: [String: Int64]? }
        guard let data = json?.data(using: .utf8),
              let message = try? JSONDecoder().decode(Message.self, from: data),
              let entry = catalog.messages[message.key],
              !entry.english.isPlural || message.parameters?["count"] != nil else { return nil }
        return text(message.key, parameters: message.parameters?.mapValues(number) ?? [:], count: message.parameters?["count"])
    }

    /// Only call on a fixed copy-table export, never a daemon response. A
    /// newer/older core's changed English remains visible in English rather
    /// than inheriting a translation of an obsolete privacy claim.
    public func copyJSON(_ json: String?, namespace: String) -> String? {
        guard let json, let data = json.data(using: .utf8),
              let tree = try? JSONSerialization.jsonObject(with: data) else { return json }
        func translate(_ node: Any, key: String) -> Any {
            if let fields = node as? [String: Any] {
                return Dictionary(uniqueKeysWithValues: fields.map {
                    ($0.key, translate($0.value, key: key + "." + $0.key))
                })
            }
            if let list = node as? [Any] {
                return list.enumerated().map { translate($0.element, key: key + "." + String($0.offset)) }
            }
            guard let original = node as? String, let message = catalog.messages[key],
                  !message.english.isPlural, message.english.other == original else { return node }
            return text(key)
        }
        guard let result = try? JSONSerialization.data(withJSONObject: translate(tree, key: namespace), options: [.sortedKeys]) else {
            return json
        }
        return String(data: result, encoding: .utf8)
    }

    public static func fill(_ template: String, _ values: [String: String]) -> String {
        let pattern = /\{([A-Za-z_][A-Za-z0-9_]*)\}/
        var result = ""
        var cursor = template.startIndex
        for match in template.matches(of: pattern) {
            result += template[cursor..<match.range.lowerBound]
            result += values[String(match.1)] ?? String(match.0)
            cursor = match.range.upperBound
        }
        result += template[cursor...]
        return result
    }

    private static let resources: Bundle = {
        #if DEBUG
        // Development pseudolanguages must never enter a signed release's
        // resources or language picker. Preview them from a separate bundle.
        if let path = ProcessInfo.processInfo.environment["TC_LOCALIZATION_RESOURCES"],
           let bundle = Bundle(path: path) { return bundle }
        #endif
        return .module
    }()

    private static let catalogResource = loadCatalog(resources)

    private static func loadCatalog(_ bundle: Bundle) -> Catalog {
        guard let url = bundle.url(forResource: "Catalog", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let catalog = try? JSONDecoder().decode(Catalog.self, from: data) else {
            preconditionFailure("Generated translation catalog is missing or invalid")
        }
        return catalog
    }

    private struct Catalog: Decodable, Sendable {
        struct LanguageInfo: Decodable, Sendable { let name: String; let enabled: Bool }
        struct Message: Decodable, Sendable { let english: English }
        let locales: [String: LanguageInfo]
        let messages: [String: Message]
    }

    private enum English: Decodable, Sendable, Equatable {
        case text(String)
        case plural([String: String])
        init(from decoder: any Decoder) throws {
            let container = try decoder.singleValueContainer()
            if let text = try? container.decode(String.self) { self = .text(text) }
            else { self = .plural(try container.decode([String: String].self)) }
        }
        var other: String {
            switch self { case .text(let text): text; case .plural(let forms): forms["other"] ?? "" }
        }
        var isPlural: Bool { if case .plural = self { true } else { false } }
    }
}
