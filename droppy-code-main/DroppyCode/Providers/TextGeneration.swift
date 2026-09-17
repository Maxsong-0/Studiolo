import Foundation

/// One-shot generation of thread titles and commit messages through a provider CLI with no tools.
enum TextGeneration {
    enum Engine: Sendable {
        case claude(executable: URL, environment: [String: String])
        case codex(executable: URL, environment: [String: String], model: String)
    }

    static func threadTitle(for message: String, engine: Engine) async -> String? {
        let prompt = """
        Write a title for this coding task so the user can recognize it weeks later.
        Rules: 3 to 7 words, under 40 characters, sentence case, no quotes, no trailing punctuation.
        Name the subject and the outcome. Do not copy the request word for word.
        Reply with the title only.

        Request:
        \(message.prefix(4_000))
        """
        guard let text = await run(prompt, engine: engine, directory: FileManager.default.temporaryDirectory) else {
            return nil
        }
        let title = TextCleanup.singleLine(text, limit: 60)
            .trimmingCharacters(in: CharacterSet(charactersIn: "\"'`*#. "))
        return title.isEmpty ? nil : title
    }

    static func commitMessage(summary: String, patch: String, instructions: String, engine: Engine, directory: URL) async -> String? {
        let extra = instructions.isEmpty ? "" : "\nFollow these instructions too: \(instructions)\n"
        let prompt = """
        Write a git commit message for the changes below.
        Rules: an imperative subject line of at most 72 characters with no trailing period. \
        Optionally add a blank line and a few short bullet points. Reply with the message only, without code fences.
        \(extra)
        Changed files:
        \(summary.prefix(6_000))

        Patch:
        \(patch.prefix(40_000))
        """
        guard let text = await run(prompt, engine: engine, directory: directory) else { return nil }
        let message = text
            .replacingOccurrences(of: "```", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return message.isEmpty ? nil : message
    }

    private static func run(_ prompt: String, engine: Engine, directory: URL) async -> String? {
        switch engine {
        case .claude(let executable, let environment):
            let arguments = [
                "-p", "--output-format", "text", "--model", "haiku", "--tools", "",
                "--disable-slash-commands", "--strict-mcp-config", "--permission-mode", "dontAsk",
                "--no-session-persistence",
            ]
            guard let result = try? await Shell.run(executable, arguments, in: directory, environment: environment, input: Data(prompt.utf8), timeout: 120),
                  result.succeeded else { return nil }
            return result.trimmedOutput.nilIfEmpty
        case .codex(let executable, let environment, let model):
            let output = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-code-\(UUID().uuidString).txt")
            defer { try? FileManager.default.removeItem(at: output) }
            let arguments = [
                "exec", "--ephemeral", "--skip-git-repo-check", "-s", "read-only", "--model", model,
                "--config", "model_reasoning_effort=\"low\"", "--output-last-message", output.path, "-",
            ]
            guard let result = try? await Shell.run(executable, arguments, in: directory, environment: environment, input: Data(prompt.utf8), timeout: 180),
                  result.succeeded else { return nil }
            return (try? String(contentsOf: output, encoding: .utf8))?
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .nilIfEmpty
        }
    }
}
