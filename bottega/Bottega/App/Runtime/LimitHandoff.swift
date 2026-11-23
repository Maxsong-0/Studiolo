// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// The message a chat that stopped on its provider's usage limit hands to the new chat
/// taking the work over: the request it was answering and what it had already done in
/// that turn, so another model can finish it.
enum LimitHandoff {
    /// The hand-off text for the chat's last turn.
    @MainActor
    static func text(for runtime: ThreadRuntime, title: String) -> String {
        let turnID = runtime.turns.last?.id
        let turn = turnID.map { id in runtime.entries.filter { $0.turnID == id } } ?? runtime.entries

        var request: String?
        for entry in runtime.entries.reversed() {
            if case .user(let message) = entry.item.content {
                request = flatten(message.text)
                break
            }
        }
        let requestText = request ?? "(the request was not recorded)"

        var lines: [String] = []
        for entry in turn {
            guard lines.count < 40 else { break }
            switch entry.item.content {
            case .assistant(let message):
                let flat = flatten(message.text)
                guard !flat.isEmpty else { continue }
                if flat.count > 400 {
                    lines.append("- It said: " + String(flat.prefix(400)) + "…")
                } else {
                    lines.append("- It said: " + flat)
                }
            case .tool(let call):
                var line = "- It ran: " + call.title
                if !call.landedEdits.isEmpty {
                    var seen: [String] = []
                    for edit in call.landedEdits where !seen.contains(edit.path) {
                        seen.append(edit.path)
                    }
                    var suffix = seen.prefix(5).joined(separator: ", ")
                    if seen.count > 5 { suffix += " and \(seen.count - 5) more" }
                    line += " (changed \(suffix))"
                }
                lines.append(line)
            default:
                break
            }
        }

        var paths: [String] = []
        for entry in turn {
            if case .tool(let call) = entry.item.content {
                for edit in call.landedEdits where !paths.contains(edit.path) {
                    paths.append(edit.path)
                    if paths.count == 20 { break }
                }
            }
            if paths.count == 20 { break }
        }

        var blocks: [String] = []
        blocks.append("Continue the work from the chat \"\(title)\". It stopped when its provider's usage limit was reached, and it will not continue there.")
        blocks.append("The request it was answering:\n\n\(requestText)")
        if !lines.isEmpty {
            blocks.append("What it had already done in that turn:\n\n" + lines.joined(separator: "\n"))
        }
        if !paths.isEmpty {
            blocks.append("Files it had changed:\n\n" + paths.map { "- " + $0 }.joined(separator: "\n"))
        }
        blocks.append("Take it over from here and finish it, asking me about anything unclear.")
        return blocks.joined(separator: "\n\n")
    }

    private static func flatten(_ text: String) -> String {
        text.components(separatedBy: .whitespacesAndNewlines).filter { !$0.isEmpty }.joined(separator: " ")
    }
}
