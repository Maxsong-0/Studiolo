// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import Foundation

/// The interface's words. English, like the rest of Bottega; what LMAS and the agents write is shown as they wrote it.
enum Wording {
    static func time(_ ms: UInt64) -> String {
        let date = Date(timeIntervalSince1970: Double(ms) / 1000)
        if Calendar.current.isDateInToday(date) { return date.formatted(date: .omitted, time: .shortened) }
        let day = Date.FormatStyle.dateTime.month(.abbreviated).day().hour().minute()
        return date.formatted(Calendar.current.isDate(date, equalTo: .now, toGranularity: .year) ? day : day.year())
    }

    static func exactTime(_ ms: UInt64) -> String {
        Date(timeIntervalSince1970: Double(ms) / 1000).formatted(date: .abbreviated, time: .standard)
    }

    static func waited(since ms: UInt64, now: Date) -> String {
        let seconds = max(0, now.timeIntervalSince1970 - Double(ms) / 1000)
        switch seconds {
        case ..<60: return "just now"
        case ..<3600: return "\(Int(seconds / 60)) min"
        case ..<86400: return "\(Int(seconds / 3600)) h"
        default: return "\(Int(seconds / 86400)) d"
        }
    }

    static func count(_ number: Int, _ one: String, _ many: String) -> String { "\(number) \(number == 1 ? one : many)" }

    static func duration(_ ms: UInt64) -> String {
        ms < 60_000 ? String(format: "%.1f s", Double(ms) / 1000) : "\(ms / 60_000) min \(ms / 1000 % 60) s"
    }

    /// Words LMAS may add to are shown as they come when they are new.
    static func messageKind(_ kind: String) -> String {
        switch kind {
        case "task": "Task"
        case "question": "Question"
        case "result": "Result"
        case "notification": "Note"
        default: UntrustedText.line(kind, limit: 24)
        }
    }

    static func messageState(_ state: Message.State) -> (text: String, symbol: String) {
        switch state {
        case .queued: ("Queued", "clock")
        case .handed: ("Delivered", "checkmark")
        case .answered(let by): ("Answered by \(by)", "arrowshape.turn.up.left")
        case .failed(let why): ("Not delivered. \(UntrustedText.line(why.text))", "xmark")
        }
    }

    static func column(_ column: WorkshopState.Column) -> String {
        switch column {
        case .queued: "Queued"
        case .working: "Working"
        case .needsYou: "Needs you"
        case .done: "Done"
        case .failed: "Failed"
        }
    }

    static func columnSymbol(_ column: WorkshopState.Column) -> String {
        switch column {
        case .queued: "circle.dashed"
        case .working: "circle.lefthalf.filled"
        case .needsYou: "hand.raised.fill"
        case .done: "checkmark.circle"
        case .failed: "xmark.octagon"
        }
    }

    /// One line for something waiting on the user: the heading of its card, and its row in the sidebar.
    static func waiting(_ item: Waiting, in state: WorkshopState) -> String {
        switch item.what {
        case .ask(let id): "Approval for \(UntrustedText.line(state.asks[id]?.by ?? "LMAS", limit: 40))"
        case .question(let id): "\(UntrustedText.line(state.messages[id]?.from ?? "An agent", limit: 40)) asks you"
        case .trip(let id): "A letter was held back" + (state.trips[id].map { ": \(limit($0.limit))" } ?? "")
        }
    }

    static func lineKind(_ kind: WorkshopState.LineKind) -> String {
        switch kind {
        case .dispatcher: "With the dispatcher"
        case .direct: "With an agent"
        case .planner: "Planner"
        case .agents: "Between agents"
        }
    }

    static func answer(_ answer: String, via: Event.AskVia) -> String {
        let word = switch answer {
        case "approved": "Approved"
        case "declined": "Declined"
        case "withdrawn": "Withdrawn"
        default: UntrustedText.line(answer, limit: 24)
        }
        let place = switch via.at {
        case "session": "by \(via.name.map { UntrustedText.line($0, limit: 40) } ?? "a session")"
        case "dialog": "in the Mac dialog"
        case "coordinator": "by LMAS"
        case "device": "on \(via.device.map { UntrustedText.line($0, limit: 40) } ?? "a device")"
        default: UntrustedText.line(via.at, limit: 40)
        }
        return "\(word) \(place)"
    }

    static func limit(_ limit: String) -> String {
        switch limit {
        case "hop": "passed on too many times"
        case "budget": "too many letters on this line"
        case "pair": "too many letters between these two"
        default: UntrustedText.line(limit, limit: 40)
        }
    }

    static func mergeStage(_ stage: Merge.Stage) -> (text: String, symbol: String) {
        switch stage {
        case .requested: ("Requested", "circle.dashed")
        case .checked: ("Checked", "magnifyingglass")
        case .prepared: ("Merged on the side, testing", "hammer")
        case .restarted(let why, _): ("Starting over. \(UntrustedText.line(why.text))", "arrow.counterclockwise")
        case .landing: ("Landing", "arrow.down.to.line")
        case .finished: ("Merged", "checkmark.circle")
        case .failed(let why, _): ("Not merged. \(UntrustedText.line(why.text))", "xmark.octagon")
        }
    }

    static func testStage(_ stage: TestRun.Stage) -> (text: String, symbol: String) {
        switch stage {
        case .submitted: ("Submitted", "circle.dashed")
        case .handedOver: ("Handed to the test machine", "arrow.up.right")
        case .accepted: ("Running", "circle.lefthalf.filled")
        case .finished(let outcome, _, _):
            (UntrustedText.line(outcome.text), outcome.code == "passed" ? "checkmark.circle" : "xmark.octagon")
        }
    }

    static func review(_ review: Card.Review) -> (text: String, symbol: String) {
        switch review.outcome {
        case nil: ("Reviewing", "circle.lefthalf.filled")
        case .finished(let word, _, _): (verdict(word), word == "approve" ? "checkmark.circle" : "pencil.circle")
        case .notDone(let why, _): ("Not reviewed. \(UntrustedText.line(why.text))", "xmark.octagon")
        case .requeued(let action, let why, _): ("Starts again, \(requeue(action)). \(UntrustedText.line(why.text))", "arrow.counterclockwise")
        }
    }

    /// How a card or review that stopped halfway carries on.
    static func requeue(_ action: String) -> String {
        switch action {
        case "retry": "a fresh process tries once more"
        case "hold": "it waits for that vendor to be back, then a fresh process carries on (not counted as a try)"
        default: "it carries on (\(UntrustedText.line(action, limit: 30)))"
        }
    }

    static func verdict(_ verdict: String) -> String {
        switch verdict {
        case "approve": "Approved"
        case "changes": "Asked for changes"
        default: UntrustedText.line(verdict, limit: 40)
        }
    }

    // MARK: Lines and models

    static func arrangement(_ event: Event.LineArranged) -> String {
        var parts: [String] = []
        if let title = event.title {
            parts.append(title.isEmpty ? "title taken away" : "named “\(UntrustedText.line(title, limit: 80))”")
        }
        if let pinned = event.pinned { parts.append(pinned ? "pinned" : "unpinned") }
        if let archived = event.archived { parts.append(archived ? "archived" : "taken out of the archive") }
        return parts.isEmpty ? "arranged" : parts.joined(separator: ", ")
    }

    static func lineWorker(_ event: Event.LineWorker) -> (text: String, symbol: String) {
        switch event.state {
        // A later one starts over, with what was said on the line to read.
        case "up": (event.run == 1 ? "its agent is up" : "its agent is up again, for the \(ordinal(event.run)) time", "bolt.circle")
        case "down": ("its agent is down. \(event.why.map { UntrustedText.line($0.text, limit: 200) } ?? "")", "moon.circle")
        default: (UntrustedText.line(event.state, limit: 40), "questionmark.circle")
        }
    }

    private static func ordinal(_ number: Int) -> String {
        NumberFormatter.localizedString(from: number as NSNumber, number: .ordinal)
    }

    // MARK: Bots

    /// What an agent can be on a line, as `models_listed` says.
    static func role(_ role: String) -> String {
        switch role {
        case "coder": "Write code"
        case "reviewer": "Review"
        default: UntrustedText.line(role, limit: 24)
        }
    }

    static func roles(_ roles: [String]) -> String {
        switch (roles.contains("coder"), roles.contains("reviewer")) {
        case (true, true): "Writes code and reviews"
        case (true, false): "Writes code"
        case (false, true): "Reviews"
        case (false, false): ""
        }
    }

    /// `high` → High. What the vendor calls it, made to read as a name.
    static func effort(_ effort: String) -> String {
        let shown = UntrustedText.line(effort, limit: 24).replacingOccurrences(of: "_", with: " ")
        return shown.prefix(1).uppercased() + shown.dropFirst()
    }

    // MARK: Quota gates

    /// How far a quota gate is shut.
    static func gateLevel(_ level: String) -> String {
        switch level {
        case "open": "Open"
        case "started_only": "No new cards"
        case "closed": "Stopped"
        default: UntrustedText.line(level, limit: 30)
        }
    }

    static func gateSymbol(_ level: String) -> String {
        switch level {
        case "open": "gauge.with.dots.needle.33percent"
        case "started_only": "gauge.with.dots.needle.67percent"
        default: "gauge.with.dots.needle.100percent"
        }
    }

    /// Whose gate: the vendor's own, or one of its models'.
    static func gateOwner(_ gate: Gate) -> String {
        let vendor = Maker(vendor: gate.vendor)?.title ?? UntrustedText.line(gate.vendor, limit: 40)
        return gate.model.map { "\(vendor) · \(UntrustedText.line($0, limit: 40))" } ?? vendor
    }

    /// What the gate means for the work, why, and when LMAS looks again.
    static func gateDetail(_ gate: Gate) -> String {
        let meaning = switch gate.level {
        case "open": "Takes work again."
        case "started_only": "Gets no new cards; what started carries on."
        case "closed": "Gets no work and no letters once the turn that runs is over."
        default: ""
        }
        var parts = [meaning]
        if let why = gate.why {
            // LMAS's reason may end without a stop; the next sentence is Bottega's.
            let text = UntrustedText.line(why.text, limit: 200)
            parts.append(text.last.map { "。.!?！？".contains($0) } == true ? text : text + "。")
        }
        if gate.streak > 0 { parts.append("Ran into its quota \(count(Int(clamping: gate.streak), "time", "times")) in a row.") }
        if let until = gate.until { parts.append("LMAS looks again \(time(until)).") }
        return parts.filter { !$0.isEmpty }.joined(separator: " ")
    }

    static func gate(_ gate: Gate) -> String {
        "\(gateOwner(gate)): quota gate \(gateLevel(gate.level).lowercased()). \(gateDetail(gate))"
    }

    /// The cards one waits for before it first starts.
    static func after(_ cards: [String]) -> String {
        "Waits for " + cards.map { UntrustedText.line($0, limit: 24) }.joined(separator: ", ")
    }

    /// A question a card's AI asked that nobody has answered: whom LMAS reminded, and how long it has been.
    static func reminder(of card: String, _ reminder: Card.Reminder, to: String?) -> String {
        let since = switch reminder.level {
        case 1: "an hour"
        case 2: "12 hours"
        default: "a long time (reminder \(reminder.level))"
        }
        let whom = to.map { $0 == "user" ? "you" : UntrustedText.line($0, limit: 40) } ?? "the one it asked"
        return "LMAS reminded \(whom): \(UntrustedText.line(card, limit: 40))'s question \(UntrustedText.line(reminder.question, limit: 24)) "
            + "has had no answer for \(since)"
    }

    static func reassignment(of card: String, _ move: Card.Reassignment) -> String {
        let card = UntrustedText.line(card, limit: 60)
        let to = UntrustedText.line(move.to, limit: 40)
        let from = move.from.map { " instead of \(UntrustedText.line($0, limit: 40))" } ?? ""
        switch move.what {
        case .card:
            let reviewer = move.reviewer.map { ", reviewed by \(UntrustedText.line($0, limit: 40))" } ?? ""
            return "\(card) goes to \(to)\(from)\(reviewer): the quota gate it had is not open"
        case .review:
            return "The review of \(card) goes to \(to)\(from): the quota gate it had is shut"
        }
    }

    static func directLine(_ line: DirectLine) -> (text: String, symbol: String, isTrouble: Bool) {
        if let failure = line.failure { return ("Did not open. \(UntrustedText.line(failure.why.text, limit: 200))", "xmark.octagon", true) }
        if line.isBeingSetUp { return ("Being set up", "circle.dotted", false) }
        guard let worker = line.worker else { return ("Ready", "checkmark.circle", false) }
        return worker.isUp ? ("Its agent is up", "bolt.circle", false) : ("Its agent is down", "moon.circle", false)
    }

    static func model(_ model: String, _ effort: String?) -> String {
        UntrustedText.line(model, limit: 60) + (effort.map { " · \(UntrustedText.line($0, limit: 24))" } ?? "")
    }

    // MARK: Turns

    static func turnOutcome(_ outcome: Why) -> (text: String, symbol: String, isFailure: Bool) {
        switch outcome.code {
        case "completed": ("Finished", "checkmark.circle", false)
        case "stopped": ("Stopped by LMAS", "stop.circle", false)
        case "stopped_by_user": ("Stopped by you", "stop.circle", false)
        case "session_closed": ("The session was closed", "stop.circle", false)
        case "superseded": ("Another turn began", "arrow.uturn.right.circle", false)
        default: (UntrustedText.line(outcome.text, limit: 200), "xmark.octagon", true)
        }
    }

    static func took(_ turn: Turn) -> String? {
        guard let start = turn.startedAt, let end = turn.end?.at, end >= start else { return nil }
        return duration(end - start)
    }

    static func tools(_ tools: [String: Int]) -> String? {
        guard !tools.isEmpty else { return nil }
        return tools.sorted { ($1.value, $0.key) < ($0.value, $1.key) }
            .map { "\(UntrustedText.line($0.key, limit: 40)) ×\($0.value)" }.joined(separator: ", ")
    }

    static func bytes(_ count: UInt64) -> String { Int64(clamping: count).formatted(.byteCount(style: .file)) }

    static func turnProblem(_ why: Why) -> String {
        switch why.code {
        case "unknown_request": "This LMAS does not tell what goes on inside a turn."
        case "no_details": "LMAS did not keep what went on inside this turn."
        case "no_such_turn": "LMAS has no such turn."
        default: UntrustedText.line(why.text)
        }
    }

    static func turnItem(_ item: TurnItem, hasText: Bool = true) -> (text: String, symbol: String, isTrouble: Bool) {
        let tool = item.tool.map { UntrustedText.line($0, limit: 60) }
        return switch item.kind {
        case "text": ("Said", "text.bubble", false)
        // The dispatcher's every question to its model is one, thoughts or not.
        case "reasoning": (hasText || !item.isOver ? "Thought" : "Asked the model", "brain", false)
        case "command": ("Command", "terminal", false)
        case "edit": (item.files.isEmpty ? "Edit" : "Edited \(count(item.files.count, "file", "files"))", "pencil", false)
        case "read": ("Read", "doc.text", false)
        case "search": ("Searched", "magnifyingglass", false)
        case "coordinator": ("LMAS: \(tool ?? "tool")", "gearshape", false)
        case "tool": (tool ?? "Tool", "wrench.and.screwdriver", false)
        case "plan": ("Plan", "checklist", false)
        case "notice":
            switch item.code {
            case "denied": ("Denied", "nosign", true)
            case "too_much": ("Too much to keep", "ellipsis.circle", true)
            default: ("Error", "exclamationmark.triangle", true)
            }
        default: (item.kind.isEmpty ? "Unreadable" : UntrustedText.line(item.kind, limit: 40), "questionmark.square.dashed", false)
        }
    }

    /// nil for what went as it should.
    static func turnItemStatus(_ status: String) -> (text: String, symbol: String)? {
        switch status {
        case "done", "": nil
        case "failed": ("Failed", "xmark.octagon")
        case "denied": ("Denied", "nosign")
        case "interrupted": ("Not seen to end", "pause.circle")
        default: (UntrustedText.line(status, limit: 24), "questionmark.circle")
        }
    }

    static func step(_ status: String) -> (text: String, symbol: String) {
        switch status {
        case "done": ("Done", "checkmark.circle")
        case "running": ("Now", "circle.lefthalf.filled")
        case "pending": ("Not yet", "circle")
        default: (UntrustedText.line(status, limit: 24), "questionmark.circle")
        }
    }

    static func fileChange(_ change: String) -> (text: String, symbol: String) {
        switch change {
        case "add": ("Added", "plus.circle")
        case "modify": ("Changed", "pencil.circle")
        case "delete": ("Deleted", "minus.circle")
        case "move": ("Moved", "arrow.right.circle")
        default: (UntrustedText.line(change, limit: 24), "questionmark.circle")
        }
    }

    /// A running item in a few words.
    static func doing(_ item: TurnItem) -> String {
        func file(_ path: String) -> String { UntrustedText.line(path.split(separator: "/").last.map(String.init) ?? path, limit: 60) }
        return switch item.kind {
        case "text": "Writing"
        case "reasoning": "Thinking"
        case "command": "Running \(UntrustedText.line(item.commandInShort ?? "a command", limit: 80))"
        case "edit": "Editing \(item.files.first.map { file($0.path) } ?? "files")"
        case "read": "Reading \(item.paths.first.map(file) ?? "files")"
        case "search": "Searching"
        case "plan": "Planning"
        default: turnItem(item).text
        }
    }

    static func outcome(_ outcome: Outcome) -> (text: String, symbol: String) {
        switch outcome {
        case .done(_, let line?): ("Done, opened \(UntrustedText.line(line, limit: 40))", "checkmark.circle")
        case .done(let message?, nil): ("Done, became \(UntrustedText.line(message, limit: 40))", "checkmark.circle")
        case .done(nil, nil): ("Done", "checkmark.circle")
        case .already(let number): ("Already done by intent \(number)", "equal.circle")
        case .refused(let why): ("Refused. \(UntrustedText.line(why.text))", "xmark.octagon")
        case .other(let word): (UntrustedText.line(word, limit: 40), "questionmark.circle")
        }
    }

    static func status(_ model: LinkClient.Status?) -> (text: String, symbol: String) {
        switch model {
        case nil: ("Not connected", "bolt.horizontal.circle")
        case .connecting: ("Connecting…", "circle.dotted")
        case .needsRegistration: ("This Mac is not registered", "person.badge.key")
        case .registering: ("Registering…", "person.badge.key")
        case .catchingUp: ("Catching up…", "circle.dotted")
        case .live: ("Connected to LMAS", "bolt.horizontal.circle.fill")
        case .retrying(let why): ("Reconnecting. \(UntrustedText.line(why.text))", "bolt.horizontal.circle")
        case .stopped(let why): (UntrustedText.line(why.text), "exclamationmark.triangle")
        }
    }

    static func why(_ problem: IntentDraft.Problem) -> Why {
        let text = switch problem {
        case .empty: "There is nothing to send."
        case .textTooLong(let bytes): "Too long: \(bytes / 1024) KiB, at most 64."
        case .tooManyFiles(let count): "Too many files: \(count), at most 10."
        case .fileTooLong(let name): "\(UntrustedText.line(name, limit: 60)) is over 64 KiB."
        case .filesTooLong(let bytes): "The files come to \(bytes / 1024) KiB, at most 128."
        case .fileWithoutName: "A file has no name."
        case .newThreadMustWantReply: "A new line always wants a reply."
        case .nothingToArrange: "There is nothing to change."
        case .badTitle: "A title is one line of at most 200 bytes."
        case .badProjectName: "A project's name is one line of 1 to 64 bytes, without spaces around it."
        case .badProjectPath: "A project is a folder's whole path."
        case .filesWithoutText: "Files go with something said."
        case .tooLongOnTheWire(let bytes):
            "Too long to send: \(bytes / 1024) KiB once written out as one line, at most \(Wire.maxLineBytes / 1024). "
                + "Characters that have to be spelled out take up to seven times their size."
        }
        return Why(code: "draft", text: text)
    }

    /// Why a press did not count (`HumanInput`), and what to do instead where there is something.
    static func why(_ refusal: InputGate.Refusal) -> Why {
        let text = switch refusal {
        case .notInFront: "Bottega is not in front, so nothing was sent."
        case .noEvent:
            "That did not come straight from a key press or a click in the window, so nothing was sent. "
                + "A menu item picked with the mouse, or a button pressed through an accessibility tool, does not count: "
                + "use the keyboard shortcut, or click the button in the window."
        case .notAPress: "That was not one press of a key or of the mouse, so nothing was sent."
        case .synthetic: "That came from another program, not from the keyboard or the mouse, so nothing was sent."
        case .stale: "That press was too long ago, so nothing was sent. Press again."
        case .otherWindow: "That press went to another window, so nothing was sent."
        case .usedAlready: "One press sends one thing. Press again for the next."
        }
        return Why(code: "input", text: text)
    }

    /// Why what is decided about how things are now cannot be sent, in a few words: for a tooltip. Empty when it can.
    static func notLive(_ status: LinkClient.Status?) -> String {
        switch status {
        case .live: ""
        case .catchingUp: "Still catching up with LMAS"
        case .needsRegistration, .registering: "This Mac is not registered with LMAS"
        case .stopped: "The link to LMAS is stopped"
        case nil, .connecting, .retrying: "Not connected to LMAS"
        }
    }

    /// The same for something asked for all the same. Approving, declining, stopping, changing a model go by how
    /// things are now, so they are not kept for later: what to do before asking again.
    static func notNow(_ status: LinkClient.Status?) -> Why {
        let (what, then) = switch status {
        // The link said no while this still read live: the connection went a moment ago.
        case .live: ("The connection to LMAS was just lost", "Do it again once connected.")
        case .catchingUp: (notLive(status), "Do it again in a moment.")
        case .needsRegistration, .registering: (notLive(status), "Register it first.")
        case .stopped: (notLive(status), "Reconnect or reload the events first.")
        case nil, .connecting, .retrying: (notLive(status), "Do it again once connected.")
        }
        return Why(code: "offline", text: "\(what), so that was not sent. \(then)")
    }

    /// Why the link did not take something over (`LinkClient.submit`), with the link as `status` says.
    static func whyNotTaken(_ error: any Error, while status: LinkClient.Status?) -> Why {
        switch error {
        case is LinkClient.ConnectionLost: notNow(status)
        case let problem as IntentDraft.Problem: why(problem)
        default: Why(code: "send", text: error.localizedDescription)
        }
    }
}
