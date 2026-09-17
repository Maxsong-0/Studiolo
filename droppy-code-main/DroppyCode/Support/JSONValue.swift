import Foundation

/// A Sendable JSON tree. Every provider protocol is decoded into this shape off the main actor.
enum JSONValue: Sendable, Hashable {
    case null
    case bool(Bool)
    case int(Int)
    case double(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])
}

extension JSONValue {
    static func parse(_ data: Data) -> JSONValue? {
        guard let object = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]) else {
            return nil
        }
        return JSONValue(foundation: object)
    }

    static func parse(_ string: String) -> JSONValue? {
        parse(Data(string.utf8))
    }

    init(foundation object: Any) {
        switch object {
        case let number as NSNumber:
            if CFGetTypeID(number) == CFBooleanGetTypeID() {
                self = .bool(number.boolValue)
            } else if CFNumberIsFloatType(number as CFNumber) {
                self = .double(number.doubleValue)
            } else {
                self = .int(number.intValue)
            }
        case let string as String:
            self = .string(string)
        case let array as [Any]:
            self = .array(array.map(JSONValue.init(foundation:)))
        case let dictionary as [String: Any]:
            self = .object(dictionary.mapValues(JSONValue.init(foundation:)))
        default:
            self = .null
        }
    }

    var foundationObject: Any {
        switch self {
        case .null: NSNull()
        case .bool(let value): value
        case .int(let value): value
        case .double(let value): value
        case .string(let value): value
        case .array(let values): values.map(\.foundationObject)
        case .object(let values): values.mapValues(\.foundationObject)
        }
    }

    func data(pretty: Bool = false) -> Data {
        var options: JSONSerialization.WritingOptions = [.fragmentsAllowed, .withoutEscapingSlashes]
        if pretty { options.formUnion([.prettyPrinted, .sortedKeys]) }
        return (try? JSONSerialization.data(withJSONObject: foundationObject, options: options)) ?? Data("null".utf8)
    }

    var compactString: String { String(decoding: data(), as: UTF8.self) }
    var prettyString: String { String(decoding: data(pretty: true), as: UTF8.self) }

    subscript(key: String) -> JSONValue? {
        guard case .object(let values) = self else { return nil }
        return values[key]
    }

    subscript(index: Int) -> JSONValue? {
        guard case .array(let values) = self, values.indices.contains(index) else { return nil }
        return values[index]
    }

    var string: String? {
        guard case .string(let value) = self else { return nil }
        return value
    }

    var int: Int? {
        switch self {
        case .int(let value): value
        case .double(let value): value.isFinite ? Int(value) : nil
        case .string(let value): Int(value)
        default: nil
        }
    }

    var double: Double? {
        switch self {
        case .int(let value): Double(value)
        case .double(let value): value
        default: nil
        }
    }

    var bool: Bool? {
        guard case .bool(let value) = self else { return nil }
        return value
    }

    var array: [JSONValue]? {
        guard case .array(let values) = self else { return nil }
        return values
    }

    var object: [String: JSONValue]? {
        guard case .object(let values) = self else { return nil }
        return values
    }

    var isNull: Bool {
        if case .null = self { return true }
        return false
    }

    /// Text rendering for display, unwrapping plain strings.
    var displayText: String {
        switch self {
        case .null: ""
        case .string(let value): value
        case .bool(let value): value ? "true" : "false"
        case .int(let value): String(value)
        case .double(let value): String(value)
        case .array, .object: prettyString
        }
    }

    static func optional(_ value: String?) -> JSONValue {
        value.map(JSONValue.string) ?? .null
    }
}

extension JSONValue: ExpressibleByNilLiteral {
    init(nilLiteral: ()) { self = .null }
}

extension JSONValue: ExpressibleByBooleanLiteral {
    init(booleanLiteral value: Bool) { self = .bool(value) }
}

extension JSONValue: ExpressibleByIntegerLiteral {
    init(integerLiteral value: Int) { self = .int(value) }
}

extension JSONValue: ExpressibleByFloatLiteral {
    init(floatLiteral value: Double) { self = .double(value) }
}

extension JSONValue: ExpressibleByStringLiteral {
    init(stringLiteral value: String) { self = .string(value) }
}

extension JSONValue: ExpressibleByArrayLiteral {
    init(arrayLiteral elements: JSONValue...) { self = .array(elements) }
}

extension JSONValue: ExpressibleByDictionaryLiteral {
    init(dictionaryLiteral elements: (String, JSONValue)...) {
        self = .object(Dictionary(elements, uniquingKeysWith: { _, last in last }))
    }
}
