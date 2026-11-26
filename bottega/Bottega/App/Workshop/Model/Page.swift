// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit

/// The five places along the top of the sidebar; each has its own list under the switcher.
enum SidebarSection: Int, CaseIterable, Identifiable {
    case rooms, bots, waiting, cards, activity

    var id: Self { self }

    var title: String {
        switch self {
        case .rooms: "Rooms"
        case .bots: "Bots"
        case .waiting: "Waiting for You"
        case .cards: "Cards"
        case .activity: "Activity"
        }
    }

    var symbol: String {
        switch self {
        case .rooms: "bubble.left.and.bubble.right"
        case .bots: "square.grid.2x2"
        case .waiting: "tray"
        case .cards: "rectangle.split.3x1"
        case .activity: "waveform.path.ecg"
        }
    }
}

/// What the middle of the window shows.
enum Page: Hashable {
    case rooms
    case dispatcher
    case waiting
    case board
    case activity
    case devices
    /// Repositories agents may work on, registered with Touch ID.
    case projects
    case line(String)
    /// A bot's home, by vendor: open a line straight to it, see the lines it has.
    case bot(String)

    /// The section the sidebar shows while this page is up; devices and projects belong to none and leave it as it was.
    var section: SidebarSection? {
        switch self {
        case .rooms: .rooms
        case .dispatcher, .line, .bot: .bots
        case .waiting: .waiting
        case .board: .cards
        case .activity: .activity
        case .devices, .projects: nil
        }
    }

    var fixedTitle: String {
        switch self {
        case .rooms: "Rooms"
        case .dispatcher: "Dispatcher"
        case .waiting: "Waiting for You"
        case .board: "Cards"
        case .activity: "Activity"
        case .devices: "Devices"
        case .projects: "Projects"
        case .line(let id): id
        case .bot(let vendor): Maker(vendor: vendor)?.title ?? vendor
        }
    }

    var symbol: String {
        switch self {
        case .rooms: SidebarSection.rooms.symbol
        case .dispatcher: "bubble.left.and.text.bubble.right"
        case .waiting: "hand.raised"
        case .board: "rectangle.split.3x1"
        case .activity: "list.bullet.rectangle"
        case .devices: "macbook.and.iphone"
        case .projects: "folder"
        case .line: "text.bubble"
        case .bot: "person.crop.circle"
        }
    }

    #if DEBUG
    init?(word: String) {
        switch word {
        case "rooms": self = .rooms
        case "dispatcher": self = .dispatcher
        case "waiting": self = .waiting
        case "board": self = .board
        case "activity": self = .activity
        case "devices": self = .devices
        case "projects": self = .projects
        default:
            if word.hasPrefix("bot:") { self = .bot(String(word.dropFirst(4))); return }
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
