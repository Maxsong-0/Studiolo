// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit

/// One event in one line of words, as it was when it happened.
struct Summary {
    let symbol: String
    let text: String
    var isFailure = false
    var detail: Detail?

    init(_ event: Event, in state: WorkshopState) {
        func word(_ text: String, _ limit: Int = 60) -> String { UntrustedText.line(text, limit: limit) }

        switch event {
        case .sessionRegistered(let event):
            symbol = "person.badge.plus"
            text = "\(word(event.name)) joined as \(word(event.role))"
        case .sessionClosed(let event):
            symbol = "person.badge.minus"
            text = "\(word(event.name)) left"

        case .messageQueued(let event):
            let message = event.message
            symbol = "envelope"
            text = "\(word(message.from)) → \(word(message.to))  \(Wording.messageKind(message.kind)) \(word(message.id)): \(word(message.body, 160))"
            detail = .message(message.id)
        case .messageHanded(let event):
            symbol = "envelope.open"
            text = "\(word(event.id)) was delivered"
            detail = .message(event.id)
        case .messageFailed(let event):
            symbol = "xmark"
            text = "\(word(event.id)) was not delivered. \(word(event.why.text, 200))"
            isFailure = true
            detail = .message(event.id)

        case .cardCreated(let event):
            symbol = "rectangle.badge.plus"
            text = "\(word(event.author)) made \(word(event.card)): \(word(event.title, 120))"
            detail = .card(event.card)
        case .cardStarted(let event):
            symbol = "play"
            text = "\(word(event.worker)) started \(word(event.card)) on \(word(event.branch, 80))"
            detail = .card(event.card)
        case .cardDone(let event):
            symbol = "checkmark.circle"
            text = "\(word(event.card)) is done, see \(word(event.result))"
            detail = .card(event.card)
        case .cardFailed(let event):
            symbol = "xmark.octagon"
            text = "\(word(event.card)) failed. \(word(event.why.text, 200))"
            isFailure = true
            detail = .card(event.card)
        case .cardRequeued(let event):
            symbol = "arrow.counterclockwise"
            text = "\(word(event.card)) went back to wait: \(Wording.requeue(event.action)). \(word(event.why.text, 200))"
            detail = .card(event.card)

        case .limitTripped(let event):
            symbol = "gauge.with.dots.needle.100percent"
            text = "Held back, \(Wording.limit(event.limit)): \(word(event.from)) → \(word(event.to))  \(Wording.messageKind(event.kind))"
            isFailure = true

        case .testsSubmitted(let event):
            symbol = "testtube.2"
            text = "\(word(event.by)) submitted \(word(event.test)) of \(word(event.card)): \(word(event.request.command, 120))"
            detail = .test(event.test)
        case .testsHandedOver(let event):
            symbol = "arrow.up.right"
            text = "\(word(event.test)) was handed to the test machine"
            detail = .test(event.test)
        case .testsAccepted(let event):
            symbol = "circle.lefthalf.filled"
            text = "\(word(event.test)) is running as \(word(event.job))"
            detail = .test(event.test)
        case .testsFinished(let event):
            isFailure = event.outcome.code != "passed"
            symbol = isFailure ? "xmark.octagon" : "checkmark.circle"
            text = "\(word(event.test)) finished: \(word(event.outcome.text, 120))"
            detail = .test(event.test)

        case .reviewStarted(let event):
            symbol = "eye"
            text = "\(word(event.reviewer)) started reviewing \(word(event.card))"
            detail = .card(event.card)
        case .reviewFinished(let event):
            symbol = event.verdict == "approve" ? "checkmark.circle" : "pencil.circle"
            text = "Review of \(word(event.card)): \(Wording.verdict(event.verdict)), see \(word(event.result))"
            detail = .card(event.card)
        case .reviewNotDone(let event):
            symbol = "xmark.octagon"
            text = "\(word(event.card)) was not reviewed. \(word(event.why.text, 200))"
            isFailure = true
            detail = .card(event.card)
        case .reviewRequeued(let event):
            symbol = "arrow.counterclockwise"
            text = "The review of \(word(event.card)) starts again: \(Wording.requeue(event.action)). \(word(event.why.text, 200))"
            detail = .card(event.card)

        case .mergeRequested(let event):
            symbol = "arrow.triangle.merge"
            text = "\(word(event.by)) asked to merge \(word(event.card)) into \(word(event.target, 80)), \(word(event.merge))"
            detail = .merge(event.merge)
        case .mergeChecked(let event):
            symbol = "magnifyingglass"
            text = "\(word(event.merge)) was checked: \(Wording.count(event.files, "file", "files")), +\(event.insertions) −\(event.deletions)"
            detail = .merge(event.merge)
        case .mergePrepared(let event):
            symbol = "hammer"
            text = "\(word(event.merge)) was put together on \(word(event.branch, 80))"
            detail = .merge(event.merge)
        case .mergeRestarted(let event):
            symbol = "arrow.counterclockwise"
            text = "\(word(event.merge)) starts over. \(word(event.why.text, 200))"
            detail = .merge(event.merge)
        case .mergeLanding(let event):
            symbol = "arrow.down.to.line"
            text = "\(word(event.merge)) is landing"
            detail = .merge(event.merge)
        case .mergeFinished(let event):
            symbol = "checkmark.circle"
            text = "\(word(event.merge)) is merged"
            detail = .merge(event.merge)
        case .mergeFailed(let event):
            symbol = "xmark.octagon"
            text = "\(word(event.merge)) was not merged. \(word(event.why.text, 200))"
            isFailure = true
            detail = .merge(event.merge)
        case .mergeCleaned(let event):
            symbol = "sparkles"
            text = "\(word(event.merge)) was cleaned up"
            detail = .merge(event.merge)

        case .askOpened(let event):
            symbol = "hand.raised"
            text = "\(word(event.ask)), for \(word(event.by)): \(word(event.text, 160))"
            detail = event.about.merge.map(Detail.merge)
        case .askAnswered(let event):
            symbol = event.answer == "approved" ? "checkmark.circle" : "xmark.circle"
            text = "\(word(event.ask)): \(Wording.answer(event.answer, via: event.via))"
            detail = state.asks[event.ask]?.about.merge.map(Detail.merge)

        case .deviceRegistered(let event):
            symbol = "macbook.and.iphone"
            text = "\(word(event.device)) was registered: \(word(event.name, 64))"
        case .intentEchoed(let event):
            let outcome = Wording.outcome(event.outcome)
            symbol = outcome.symbol
            text = "Intent \(event.number) from \(word(event.device)): \(outcome.text)"
            if case .refused = event.outcome { isFailure = true }
        case .turnStarted(let event):
            symbol = "play.circle"
            text = "\(word(event.session)) began \(word(event.turn))"
            detail = .turn(event.turn)
        case .turnFinished(let event):
            let outcome = Wording.turnOutcome(event.outcome)
            symbol = outcome.symbol
            text = "\(word(event.session)) ended \(word(event.turn)): \(outcome.text)"
            isFailure = outcome.isFailure
            detail = .turn(event.turn)
        case .turnStopRequested(let event):
            symbol = "stop.circle"
            text = "You asked \(word(event.session)) to stop \(word(event.turn))"
            detail = .turn(event.turn)
        case .turnNotStopped(let event):
            symbol = "xmark.octagon"
            text = "\(word(event.session)) did not stop \(word(event.turn)) and goes on"
            isFailure = true
            detail = .turn(event.turn)

        case .lineArranged(let event):
            symbol = "slider.horizontal.3"
            text = "\(word(event.line)): \(Wording.arrangement(event))"
        case .lineRead(let event):
            symbol = "eye"
            text = "You have read \(word(event.line)) up to event \(event.through)"
        case .projectAdded(let event):
            symbol = "folder.badge.plus"
            text = "Project \(word(event.project, 80)) was added: \(word(event.path, 160))"
        case .projectRemoved(let event):
            symbol = "folder.badge.minus"
            text = "Project \(word(event.project, 80)) was removed"
        case .lineOpened(let event):
            symbol = "plus.bubble"
            text = "\(word(event.line)) was opened in \(word(event.project, 80)) with \(word(event.vendor)) as \(word(event.role))"
                + (event.model.map { ", \(Wording.model($0, event.effort))" } ?? "")
        case .lineReady(let event):
            symbol = "checkmark.circle"
            text = "\(word(event.line)) is ready on \(word(event.branch, 80))"
        case .lineFailed(let event):
            symbol = "xmark.octagon"
            text = "\(word(event.line)) could not be opened. \(word(event.why.text, 200))"
            isFailure = true
        case .lineWorker(let event):
            let state = Wording.lineWorker(event)
            symbol = state.symbol
            text = "\(word(event.line)): \(state.text)"

        case .modelsListed(let event):
            symbol = "list.bullet"
            text = "\(word(event.vendor)) offers \(Wording.count(event.models.count, "model", "models")): "
                + word(event.models.map(\.name).joined(separator: ", "), 160)
        case .modelsUnavailable(let event):
            symbol = "list.bullet"
            text = "\(word(event.vendor)) cannot be asked for its models. \(word(event.why.text, 200))"
        case .modelRequested(let event):
            symbol = "arrow.left.arrow.right"
            text = "\(word(event.session)) is to use \(Wording.model(event.model, event.effort)) from its next turn on"
        case .modelChanged(let event):
            symbol = "checkmark.circle"
            text = "\(word(event.session)) uses \(Wording.model(event.model, event.effort))"
        case .modelNotChanged(let event):
            symbol = "xmark.octagon"
            text = "\(word(event.session)) was not changed to \(Wording.model(event.model, event.effort)). \(word(event.why.text, 200))"
            isFailure = true

        case .unreadable(let type, let body):
            symbol = "questionmark.square.dashed"
            text = Self.unreadable(type, body)
        }
    }

    /// A timeline's plain rows.
    init(_ row: Row, in state: WorkshopState) {
        func word(_ text: String, _ limit: Int = 60) -> String { UntrustedText.line(text, limit: limit) }

        switch row.kind {
        case .cardStarted(let id):
            let work = state.cards[id]?.work
            symbol = "play"
            text = work.map { "\(word($0.worker)) started \(word(id)) on \(word($0.branch, 80))" } ?? "\(word(id)) started"
            detail = .card(id)
        case .cardRequeued(let id):
            let last = state.cards[id]?.requeues.last
            symbol = "arrow.counterclockwise"
            text = "\(word(id)) went back to wait" + (last.map { ": \(Wording.requeue($0.action)). \(word($0.why.text, 200))" } ?? "")
            detail = .card(id)
        case .cardEnded(let id):
            switch state.cards[id]?.end {
            case .done:
                symbol = "checkmark.circle"
                text = "\(word(id)) is done"
            case .failed(let why, _):
                symbol = "xmark.octagon"
                text = "\(word(id)) failed. \(word(why.text, 200))"
                isFailure = true
            case nil:
                symbol = "circle"
                text = "\(word(id)) ended"
            }
            detail = .card(id)
        case .unreadable(let type, let body):
            symbol = "questionmark.square.dashed"
            text = Self.unreadable(type, body)
        case .message(let id), .card(let id), .test(let id), .review(let id, _), .merge(let id), .ask(let id):
            symbol = "circle"
            text = word(id)
        case .trip:
            symbol = "gauge.with.dots.needle.100percent"
            text = "A limit was reached"
        }
    }

    private static func unreadable(_ type: String, _ text: String?) -> String {
        "\(UntrustedText.line(type, limit: 80)) (this version of Bottega does not know it)"
            + (text.map { ": \(UntrustedText.line($0, limit: 200))" } ?? "")
    }
}
