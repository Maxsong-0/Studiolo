// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import Foundation

/// A line not opened yet: what the box on a bot's home holds.
struct LineDraft: Equatable {
    var text = ""
    /// nil until one is picked; the first registered project is offered.
    var project: String?
    var role: String?
    /// nil for the vendor's default.
    var choice: ModelChoice?
    /// Why LMAS did not open the last one.
    var problem: Why?
}

/// What LMAS section 11 lets the user do besides talking (片 5): change a model, stop a turn, open a line straight to
/// an agent, register a repository. Each goes through the same gate as sending: from the keyboard or the mouse only.
extension WorkshopModel {
    /// The one a box talks to: whom it answers, interjects to, or starts a line with.
    func session(of key: DraftKey) -> String? {
        switch target(of: key) {
        case .start(let name), .interject(let name): name
        case .answer(let question): question.from
        case nil: nil
        }
    }

    // MARK: Models

    /// Whom a box on this page talks to, when that is an agent LMAS runs and its vendor has said what it offers.
    func modelTarget(of key: DraftKey) -> (line: String, session: String)? {
        guard case .line(let line) = key, let session = session(of: key) else { return nil }
        return state.canChangeModel(of: session) ? (line, session) : nil
    }

    func setModel(_ choice: ModelChoice, for session: String, on line: String) {
        guard let client, canDecideNow(), isFromTheUser() else { return }
        submit(.setModel(line: line, session: session, model: choice.model, effort: choice.effort), to: client)
    }

    // MARK: Turns

    /// The turn running for whom a box talks to.
    func runningTurn(for key: DraftKey) -> Turn? {
        session(of: key).flatMap { state.runningTurn(of: $0) }
    }

    func canStop(_ turn: Turn) -> Bool { !state.isCardAgent(turn.session) }

    /// ⌘. on a page whose box talks to an agent in the middle of a turn.
    var turnToStop: Turn? {
        let key: DraftKey? = switch page {
        case .dispatcher: .dispatcher
        case .line(let id): .line(id)
        default: nil
        }
        guard let key, let turn = runningTurn(for: key), canStop(turn) else { return nil }
        // Asked once already: LMAS says within 15 seconds whether it stopped.
        if case .requested = turn.stop { return nil }
        return turn
    }

    func stop(_ turn: Turn) {
        guard let client, canStop(turn), canDecideNow(), isFromTheUser() else { return }
        submit(.stopTurn(turn.id), to: client)
    }

    // MARK: Lines straight to one agent

    subscript(lineDraft vendor: String) -> LineDraft {
        get { lineDrafts[vendor] ?? LineDraft() }
        set { lineDrafts[vendor] = newValue }
    }

    /// What `open_line` would be sent as from a bot's home, or nil while something it needs is missing.
    func lineToOpen(for vendor: String) -> LineToOpen? {
        let draft = self[lineDraft: vendor]
        guard let project = draft.project ?? state.projectList.first?.name,
              let role = draft.role ?? state.catalogs[vendor]?.listed?.roles.first
        else { return nil }
        let text = draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : draft.text
        return LineToOpen(
            project: project, vendor: vendor, role: role, model: draft.choice?.model, effort: draft.choice?.effort,
            text: text)
    }

    func openLine(for vendor: String) {
        guard let client, let line = lineToOpen(for: vendor), isFromTheUser() else { return }
        do throws(IntentDraft.Problem) {
            try IntentDraft.openLine(line).check()
        } catch {
            self[lineDraft: vendor].problem = Wording.why(error)
            return
        }
        self[lineDraft: vendor].text = ""
        self[lineDraft: vendor].problem = nil
        Task {
            do {
                openings[try await client.submit(.openLine(line))] = vendor
            } catch {
                putBack(line.text, Wording.whyNotTaken(error, while: status), for: vendor)
            }
        }
    }

    /// What came of an `open_line`: refused, its words go back into the box they came from.
    /// Opened with something said: go to that line, unless the user has moved on from this bot's home.
    func opened(_ settled: Outbox.Settled, from vendor: String) {
        switch settled.result {
        case .done(let message?, _?): if page == .bot(vendor) { page = .line(message) }
        case .done, .already: break
        case .refused(let why), .notSent(let why): putBack(settled.item.draft.typed?.text, why, for: vendor)
        }
    }

    private func putBack(_ text: String?, _ why: Why, for vendor: String) {
        var draft = self[lineDraft: vendor]
        if let text, draft.text.isEmpty { draft.text = text }
        draft.problem = why
        self[lineDraft: vendor] = draft
    }

    // MARK: Projects

    /// Both ask for Touch ID: a registered repository is one agents may write in.
    func addProject(name: String, path: String) {
        guard let client, canDecideNow() else { return }
        let intent = IntentDraft.addProject(name: name, path: path)
        do throws(IntentDraft.Problem) {
            try intent.check()
        } catch {
            notice = Wording.why(error)
            return
        }
        guard isFromTheUser() else { return }
        let reason = "register “\(UntrustedText.line(name, limit: 64))” at \(UntrustedText.line(path, limit: 120))"
        submit(intent, to: client, touchIDReason: reason)
    }

    func removeProject(_ name: String) {
        guard let client, canDecideNow(), isFromTheUser() else { return }
        submit(.removeProject(name: name), to: client, touchIDReason: "remove the project “\(UntrustedText.line(name, limit: 64))”")
    }

    /// Asked for and not settled yet, so its button is not offered again.
    func isSending(_ kind: String, about name: String) -> Bool {
        outbox.contains { item in
            switch item.draft {
            case .addProject(let project, _), .removeProject(let project): item.draft.kind == kind && project == name
            default: false
            }
        }
    }
}
