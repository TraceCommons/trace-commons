import Foundation

/// A `Decoder` over a `JSONSerialization` value that records every key a
/// `Decodable` type asks for, so a test can prove that a model DECLARES
/// every key a real reply carries.
///
/// Asked for, not found: a synthesized `decodeIfPresent` calls `contains`,
/// so an optional field records its key whether the reply's value is a
/// number, `null`, or missing. That is what makes the check run in the
/// right direction -- a key the daemon sends and the model never asks for
/// is exactly a field Swift would drop.
///
/// Paths are dotted keys with `[]` for every array element
/// (`pending[].attested_inference.state`). A type that reads a whole object
/// or array through a single-value container (`GrantVoidWire` keeps its
/// element verbatim) marks that path `opaque`: everything under it is kept,
/// so nothing under it is dropped.
final class KeyRecorder {
    private(set) var declared: Set<String> = []
    private(set) var opaque: Set<String> = []

    func declare(_ path: String) { declared.insert(path) }
    func markOpaque(_ path: String) { opaque.insert(path) }

    /// Decodes `T` from `json` and records what it asked for.
    static func decode<T: Decodable>(_ type: T.Type, from json: Any, recorder: KeyRecorder) throws -> T {
        try T(from: RecordingDecoder(value: json, path: "", codingPath: [], recorder: recorder))
    }

    /// Every key path in `json`, in the recorder's notation.
    static func keyPaths(in json: Any, prefix: String = "") -> Set<String> {
        var out: Set<String> = []
        if let object = json as? [String: Any] {
            for (key, value) in object {
                let path = prefix.isEmpty ? key : "\(prefix).\(key)"
                out.insert(path)
                out.formUnion(keyPaths(in: value, prefix: path))
            }
        } else if let array = json as? [Any] {
            for element in array { out.formUnion(keyPaths(in: element, prefix: "\(prefix)[]")) }
        }
        return out
    }

    /// The paths in `json` this recorder's model neither declared nor kept
    /// whole.
    func undeclared(in json: Any) -> [String] {
        Self.keyPaths(in: json).filter { path in
            !declared.contains(path) && !opaque.contains { path.hasPrefix($0 + ".") || path.hasPrefix($0 + "[]") }
        }.sorted()
    }
}

private struct AnyKey: CodingKey {
    var stringValue: String
    var intValue: Int?
    init(stringValue: String) { self.stringValue = stringValue }
    init(intValue: Int) {
        self.stringValue = String(intValue)
        self.intValue = intValue
    }
}

private func isBool(_ value: Any) -> Bool {
    guard let number = value as? NSNumber else { return false }
    return CFGetTypeID(number) == CFBooleanGetTypeID()
}

private func mismatch<T>(_ type: T.Type, _ codingPath: [CodingKey]) -> DecodingError {
    DecodingError.typeMismatch(type, .init(codingPath: codingPath, debugDescription: "type mismatch"))
}

/// One primitive from a JSON value, or a type mismatch.
private func primitive<T>(_ type: T.Type, _ value: Any, _ codingPath: [CodingKey]) throws -> T {
    if value is NSNull { throw DecodingError.valueNotFound(type, .init(codingPath: codingPath, debugDescription: "null")) }
    if T.self == Bool.self {
        guard isBool(value), let b = value as? Bool else { throw mismatch(type, codingPath) }
        return b as! T
    }
    if T.self == String.self {
        guard let s = value as? String else { throw mismatch(type, codingPath) }
        return s as! T
    }
    guard let number = value as? NSNumber, !isBool(value) else { throw mismatch(type, codingPath) }
    switch T.self {
    case is Double.Type: return number.doubleValue as! T
    case is Float.Type: return number.floatValue as! T
    default: break
    }
    // Integers: refuse a fraction, as JSONDecoder does.
    guard number.doubleValue.rounded() == number.doubleValue else { throw mismatch(type, codingPath) }
    switch T.self {
    case is Int.Type: return number.intValue as! T
    case is Int8.Type: return number.int8Value as! T
    case is Int16.Type: return number.int16Value as! T
    case is Int32.Type: return number.int32Value as! T
    case is Int64.Type: return number.int64Value as! T
    case is UInt.Type:
        guard number.int64Value >= 0 else { throw mismatch(type, codingPath) }
        return number.uintValue as! T
    case is UInt8.Type: return number.uint8Value as! T
    case is UInt16.Type: return number.uint16Value as! T
    case is UInt32.Type: return number.uint32Value as! T
    case is UInt64.Type:
        guard number.int64Value >= 0 else { throw mismatch(type, codingPath) }
        return number.uint64Value as! T
    default: throw mismatch(type, codingPath)
    }
}

private func parseDate(_ text: String) -> Date? {
    let withFraction = ISO8601DateFormatter()
    withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    if let date = withFraction.date(from: text) { return date }
    return ISO8601DateFormatter().date(from: text)
}

/// A nested value: dates the way `DaemonDataDecoding` reads them (RFC 3339
/// strings), everything else through its own `init(from:)`.
private func nested<T: Decodable>(
    _ type: T.Type, _ value: Any, path: String, codingPath: [CodingKey], recorder: KeyRecorder
) throws -> T {
    if T.self == Date.self {
        guard let text = value as? String, let date = parseDate(text) else { throw mismatch(type, codingPath) }
        return date as! T
    }
    if value is NSNull, !(T.self is ExpressibleByNilLiteral.Type) {
        throw DecodingError.valueNotFound(type, .init(codingPath: codingPath, debugDescription: "null"))
    }
    return try T(from: RecordingDecoder(value: value, path: path, codingPath: codingPath, recorder: recorder))
}

private struct RecordingDecoder: Decoder {
    let value: Any
    let path: String
    let codingPath: [CodingKey]
    let recorder: KeyRecorder
    var userInfo: [CodingUserInfoKey: Any] { [:] }

    func container<Key: CodingKey>(keyedBy type: Key.Type) throws -> KeyedDecodingContainer<Key> {
        guard let object = value as? [String: Any] else { throw mismatch([String: Any].self, codingPath) }
        return KeyedDecodingContainer(Keyed<Key>(object: object, path: path, codingPath: codingPath, recorder: recorder))
    }

    func unkeyedContainer() throws -> UnkeyedDecodingContainer {
        guard let array = value as? [Any] else { throw mismatch([Any].self, codingPath) }
        return Unkeyed(array: array, path: path + "[]", codingPath: codingPath, recorder: recorder)
    }

    func singleValueContainer() throws -> SingleValueDecodingContainer {
        if value is [String: Any] || value is [Any] { recorder.markOpaque(path) }
        return Single(value: value, path: path, codingPath: codingPath, recorder: recorder)
    }
}

private struct Keyed<Key: CodingKey>: KeyedDecodingContainerProtocol {
    let object: [String: Any]
    let path: String
    let codingPath: [CodingKey]
    let recorder: KeyRecorder

    var allKeys: [Key] { object.keys.compactMap(Key.init(stringValue:)) }

    private func childPath(_ key: Key) -> String {
        path.isEmpty ? key.stringValue : "\(path).\(key.stringValue)"
    }

    private func value(_ key: Key) throws -> Any {
        recorder.declare(childPath(key))
        guard let value = object[key.stringValue] else {
            throw DecodingError.keyNotFound(key, .init(codingPath: codingPath, debugDescription: "missing"))
        }
        return value
    }

    func contains(_ key: Key) -> Bool {
        recorder.declare(childPath(key))
        return object[key.stringValue] != nil
    }

    func decodeNil(forKey key: Key) throws -> Bool { try value(key) is NSNull }
    func decode(_ type: Bool.Type, forKey key: Key) throws -> Bool { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: String.Type, forKey key: Key) throws -> String { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: Double.Type, forKey key: Key) throws -> Double { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: Float.Type, forKey key: Key) throws -> Float { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: Int.Type, forKey key: Key) throws -> Int { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: Int8.Type, forKey key: Key) throws -> Int8 { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: Int16.Type, forKey key: Key) throws -> Int16 { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: Int32.Type, forKey key: Key) throws -> Int32 { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: Int64.Type, forKey key: Key) throws -> Int64 { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: UInt.Type, forKey key: Key) throws -> UInt { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: UInt8.Type, forKey key: Key) throws -> UInt8 { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: UInt16.Type, forKey key: Key) throws -> UInt16 { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: UInt32.Type, forKey key: Key) throws -> UInt32 { try primitive(type, value(key), codingPath + [key]) }
    func decode(_ type: UInt64.Type, forKey key: Key) throws -> UInt64 { try primitive(type, value(key), codingPath + [key]) }

    func decode<T: Decodable>(_ type: T.Type, forKey key: Key) throws -> T {
        try nested(type, value(key), path: childPath(key), codingPath: codingPath + [key], recorder: recorder)
    }

    func nestedContainer<NestedKey: CodingKey>(keyedBy type: NestedKey.Type, forKey key: Key) throws
        -> KeyedDecodingContainer<NestedKey>
    {
        try RecordingDecoder(value: value(key), path: childPath(key), codingPath: codingPath + [key], recorder: recorder)
            .container(keyedBy: type)
    }

    func nestedUnkeyedContainer(forKey key: Key) throws -> UnkeyedDecodingContainer {
        try RecordingDecoder(value: value(key), path: childPath(key), codingPath: codingPath + [key], recorder: recorder)
            .unkeyedContainer()
    }

    func superDecoder() throws -> Decoder {
        RecordingDecoder(value: object, path: path, codingPath: codingPath, recorder: recorder)
    }

    func superDecoder(forKey key: Key) throws -> Decoder {
        RecordingDecoder(value: try value(key), path: childPath(key), codingPath: codingPath + [key], recorder: recorder)
    }
}

private struct Unkeyed: UnkeyedDecodingContainer {
    let array: [Any]
    let path: String
    let codingPath: [CodingKey]
    let recorder: KeyRecorder
    var currentIndex = 0

    init(array: [Any], path: String, codingPath: [CodingKey], recorder: KeyRecorder) {
        self.array = array
        self.path = path
        self.codingPath = codingPath
        self.recorder = recorder
    }

    var count: Int? { array.count }
    var isAtEnd: Bool { currentIndex >= array.count }

    private mutating func next() throws -> (Any, [CodingKey]) {
        guard !isAtEnd else {
            throw DecodingError.valueNotFound(Any.self, .init(codingPath: codingPath, debugDescription: "at end"))
        }
        defer { currentIndex += 1 }
        return (array[currentIndex], codingPath + [AnyKey(intValue: currentIndex)])
    }

    mutating func decodeNil() throws -> Bool {
        guard !isAtEnd else { return false }
        if array[currentIndex] is NSNull {
            currentIndex += 1
            return true
        }
        return false
    }

    mutating func decode(_ type: Bool.Type) throws -> Bool {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: String.Type) throws -> String {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: Double.Type) throws -> Double {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: Float.Type) throws -> Float {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: Int.Type) throws -> Int {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: Int8.Type) throws -> Int8 {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: Int16.Type) throws -> Int16 {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: Int32.Type) throws -> Int32 {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: Int64.Type) throws -> Int64 {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: UInt.Type) throws -> UInt {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: UInt8.Type) throws -> UInt8 {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: UInt16.Type) throws -> UInt16 {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: UInt32.Type) throws -> UInt32 {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }
    mutating func decode(_ type: UInt64.Type) throws -> UInt64 {
        let (value, at) = try next()
        return try primitive(type, value, at)
    }

    mutating func decode<T: Decodable>(_ type: T.Type) throws -> T {
        let (value, at) = try next()
        return try nested(type, value, path: path, codingPath: at, recorder: recorder)
    }

    mutating func nestedContainer<NestedKey: CodingKey>(keyedBy type: NestedKey.Type) throws
        -> KeyedDecodingContainer<NestedKey>
    {
        let (value, at) = try next()
        return try RecordingDecoder(value: value, path: path, codingPath: at, recorder: recorder).container(keyedBy: type)
    }

    mutating func nestedUnkeyedContainer() throws -> UnkeyedDecodingContainer {
        let (value, at) = try next()
        return try RecordingDecoder(value: value, path: path, codingPath: at, recorder: recorder).unkeyedContainer()
    }

    mutating func superDecoder() throws -> Decoder {
        let (value, at) = try next()
        return RecordingDecoder(value: value, path: path, codingPath: at, recorder: recorder)
    }
}

private struct Single: SingleValueDecodingContainer {
    let value: Any
    let path: String
    let codingPath: [CodingKey]
    let recorder: KeyRecorder

    func decodeNil() -> Bool { value is NSNull }
    func decode(_ type: Bool.Type) throws -> Bool { try primitive(type, value, codingPath) }
    func decode(_ type: String.Type) throws -> String { try primitive(type, value, codingPath) }
    func decode(_ type: Double.Type) throws -> Double { try primitive(type, value, codingPath) }
    func decode(_ type: Float.Type) throws -> Float { try primitive(type, value, codingPath) }
    func decode(_ type: Int.Type) throws -> Int { try primitive(type, value, codingPath) }
    func decode(_ type: Int8.Type) throws -> Int8 { try primitive(type, value, codingPath) }
    func decode(_ type: Int16.Type) throws -> Int16 { try primitive(type, value, codingPath) }
    func decode(_ type: Int32.Type) throws -> Int32 { try primitive(type, value, codingPath) }
    func decode(_ type: Int64.Type) throws -> Int64 { try primitive(type, value, codingPath) }
    func decode(_ type: UInt.Type) throws -> UInt { try primitive(type, value, codingPath) }
    func decode(_ type: UInt8.Type) throws -> UInt8 { try primitive(type, value, codingPath) }
    func decode(_ type: UInt16.Type) throws -> UInt16 { try primitive(type, value, codingPath) }
    func decode(_ type: UInt32.Type) throws -> UInt32 { try primitive(type, value, codingPath) }
    func decode(_ type: UInt64.Type) throws -> UInt64 { try primitive(type, value, codingPath) }

    func decode<T: Decodable>(_ type: T.Type) throws -> T {
        try nested(type, value, path: path, codingPath: codingPath, recorder: recorder)
    }
}
