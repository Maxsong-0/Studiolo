// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit

/// Which part of the log the Activity page shows. Every kind of event is sorted here on purpose: a new kind does not
/// compile until it is given a place.
enum ActivityFilter: CaseIterable, Identifiable {
    case everything, letters, cards, asks, devices, lines

    var id: Self { self }

    var title: String {
        switch self {
        case .everything: "Everything"
        case .letters: "Letters"
        case .cards: "Cards, tests and merges"
        case .asks: "Approvals"
        case .devices: "Devices"
        case .lines: "Lines and models"
        }
    }

    func shows(_ event: Event) -> Bool {
        self == .everything || Self.part(of: event) == self
    }

    static func part(of event: Event) -> ActivityFilter {
        switch event {
        case .messageQueued, .messageHanded, .messageFailed, .limitTripped: .letters
        case .cardCreated, .cardStarted, .cardDone, .cardFailed, .cardRequeued, .reviewRequeued, .gateChanged, .cardReassigned,
             .reviewReassigned,
             .testsSubmitted, .testsHandedOver, .testsAccepted, .testsFinished,
             .reviewStarted, .reviewFinished, .reviewNotDone,
             .mergeRequested, .mergeChecked, .mergePrepared, .mergeRestarted, .mergeLanding, .mergeFinished, .mergeFailed,
             .mergeCleaned:
            .cards
        case .askOpened, .askAnswered: .asks
        case .deviceRegistered, .intentEchoed: .devices
        case .sessionRegistered, .sessionClosed, .turnStarted, .turnFinished, .turnStopRequested, .turnNotStopped,
             .lineArranged, .lineRead, .projectAdded, .projectRemoved, .lineOpened, .lineReady, .lineFailed, .lineWorker,
             .modelsListed, .modelsUnavailable, .modelRequested, .modelChanged, .modelNotChanged:
            .lines
        case .unreadable: .everything
        }
    }
}
