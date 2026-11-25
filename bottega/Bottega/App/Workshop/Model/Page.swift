// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit

/// What the middle of the window shows.
enum Page: Hashable {
    case dispatcher
    case waiting
    case board
    case activity
    case devices
    case line(String)

    static let fixed: [Page] = [.dispatcher, .waiting, .board, .activity, .devices]

    var fixedTitle: String {
        switch self {
        case .dispatcher: "Dispatcher"
        case .waiting: "Waiting for You"
        case .board: "Cards"
        case .activity: "Activity"
        case .devices: "Devices"
        case .line(let id): id
        }
    }

    var symbol: String {
        switch self {
        case .dispatcher: "bubble.left.and.text.bubble.right"
        case .waiting: "hand.raised"
        case .board: "rectangle.split.3x1"
        case .activity: "list.bullet.rectangle"
        case .devices: "macbook.and.iphone"
        case .line: "text.bubble"
        }
    }

    #if DEBUG
    init?(word: String) {
        switch word {
        case "dispatcher": self = .dispatcher
        case "waiting": self = .waiting
        case "board": self = .board
        case "activity": self = .activity
        case "devices": self = .devices
        default:
            guard word.hasPrefix("line:") else { return nil }
            self = .line(String(word.dropFirst(5)))
        }
    }
    #endif
}

/// What the inspector shows.
enum Detail: Hashable {
    case message(String)
    case card(String)
    case merge(String)
    case test(String)
    case turn(String)

    #if DEBUG
    /// `card:card-1`, `message:msg-1`…
    init?(word: String) {
        let parts = word.split(separator: ":", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return nil }
        switch parts[0] {
        case "message": self = .message(parts[1])
        case "card": self = .card(parts[1])
        case "merge": self = .merge(parts[1])
        case "test": self = .test(parts[1])
        case "turn": self = .turn(parts[1])
        default: return nil
        }
    }
    #endif
}
