// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// A place words are typed.
public enum DraftKey: Hashable, Sendable {
    /// The dispatcher's page: starts a line.
    case dispatcher
    /// A line's page: answers the question waiting there, or interjects.
    case line(String)
    /// A question answered where it is listed as waiting.
    case question(String)
}

/// Words not sent yet, and who they are for.
public struct UnsentDraft: Equatable, Sendable {
    public var text = ""
    public var files: [TextFile] = []
    /// nil: whoever comes first.
    public var recipient: String?
    public var wantsReply = true
    /// With a question to the user open: interject instead of answering it.
    public var interjects = false
    /// Why the last try came back.
    public var problem: Why?

    public init() {}

    public var isBlank: Bool { text.allSatisfy(\.isWhitespace) }
}

/// What sending a draft would do.
public enum DraftTarget: Equatable, Sendable {
    case start(String)
    case interject(String)
    case answer(Message)
}

/// Everything typed and not sent, and where it goes when it is. Pure.
public struct Drafts: Equatable, Sendable {
    private var drafts: [DraftKey: UnsentDraft] = [:]

    public init() {}

    public subscript(key: DraftKey) -> UnsentDraft {
        get { drafts[key] ?? UnsentDraft() }
        set { drafts[key] = newValue }
    }

    /// nil when there is nobody to send it to.
    public func target(of key: DraftKey, in state: WorkshopState) -> DraftTarget? {
        let draft = self[key]
        if !draft.interjects, let question = state.questionHere(key) { return .answer(question) }
        // Someone picked who has left since is not quietly swapped for someone else.
        let names = state.recipients(for: key).map(\.name)
        guard let name = draft.recipient ?? names.first, names.contains(name) else { return nil }
        if case .line = key { return .interject(name) }
        return .start(name)
    }

    public func intent(for key: DraftKey, in state: WorkshopState) -> IntentDraft? {
        let draft = self[key]
        switch (target(of: key, in: state), key) {
        case (.answer(let question), _): return .answer(message: question.id, text: draft.text, files: draft.files)
        case (.interject(let to), .line(let line)):
            return .say(to: to, thread: line, text: draft.text, files: draft.files, wantsReply: draft.wantsReply)
        case (.start(let to), _): return .say(to: to, thread: nil, text: draft.text, files: draft.files, wantsReply: true)
        case (.interject, _), (nil, _): return nil
        }
    }

    /// It is on its way: the box is empty for what comes next, still set to the same person.
    public mutating func cleared(_ key: DraftKey) {
        var next = UnsentDraft()
        next.recipient = self[key].recipient
        next.wantsReply = self[key].wantsReply
        next.interjects = self[key].interjects
        drafts[key] = next
    }

    /// It was not taken: back in the box, in front of whatever has been typed since, with the reason.
    public mutating func putBack(_ text: String, _ files: [TextFile], _ why: Why, in key: DraftKey) {
        var draft = self[key]
        draft.text = draft.isBlank ? text : text + "\n\n" + draft.text
        draft.files = files + draft.files
        draft.problem = why
        drafts[key] = draft
    }
}

extension IntentDraft {
    /// The box a draft belongs in when the one it was typed in is not known: it was left over from an earlier launch.
    public var home: DraftKey {
        switch self {
        case .say(_, let thread?, _, _, _): .line(thread)
        case .answer(let message, _, _): .question(message)
        case .say, .approve, .decline: .dispatcher
        }
    }
}

extension WorkshopState {
    /// The question to the user that a box answers unless told otherwise: the one that has waited longest.
    public func questionHere(_ key: DraftKey) -> Message? {
        let candidates: [String] = switch key {
        case .dispatcher: []
        case .line(let id): threads[id]?.messages ?? []
        case .question(let id): [id]
        }
        return candidates.lazy.compactMap { self.messages[$0] }.first { $0.isToUser && $0.isOpenQuestion }
    }

    public func recipients(for key: DraftKey) -> [Session] {
        switch key {
        case .dispatcher: recipients
        case .line(let id): threads[id].map(recipients(on:)) ?? []
        case .question: []
        }
    }

    /// The box to put words back in, so that they can be seen: a question that is closed by now has none of its own,
    /// and a line LMAS has cleared has no page.
    public func reachable(_ key: DraftKey) -> DraftKey {
        switch key {
        case .dispatcher: key
        case .line(let id): threads[id] == nil ? .dispatcher : key
        case .question(let id): questionHere(key) != nil ? key : messages[id].map { .line($0.thread) } ?? .dispatcher
        }
    }

    /// Whether an intent on its way is shown above a box.
    public func belongs(_ draft: IntentDraft, above key: DraftKey) -> Bool {
        switch (key, draft) {
        case (.dispatcher, .say(_, nil, _, _, _)): true
        case (.line(let id), .say(_, let thread, _, _, _)): thread == id
        case (.line(let id), .answer(let message, _, _)): messages[message]?.thread == id
        case (.question(let id), .answer(let message, _, _)): message == id
        case (_, .say), (_, .answer), (_, .approve), (_, .decline): false
        }
    }

    /// Lines with something on them that waits for the user.
    public func linesWaiting(_ waiting: [Waiting]) -> Set<String> {
        Set(waiting.compactMap { item in
            switch item.what {
            case .ask(let id): asks[id]?.about.merge.flatMap { merges[$0]?.card }.flatMap { cards[$0]?.thread }
            case .question(let id): messages[id]?.thread
            case .trip(let id): trips[id]?.thread
            }
        })
    }
}
