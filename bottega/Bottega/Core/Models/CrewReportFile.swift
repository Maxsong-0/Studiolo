// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A hand's report as a Markdown file of its own: what it was asked, what it said back,
/// where its work went and what the work took. The lead hears the report in its chat; the
/// file is for everyone else, the user or a session steering this one or a later run
/// picking up an interrupted job, who should not have to dig it out of a thread's history.
/// Built from plain values; the app decides where the file goes (`Storage.reportsDirectory`).
enum CrewReportFile {
    /// Everything the file says, gathered by the app when the hand finishes.
    struct Contents {
        /// The hand's record as it stands once finished: its status, times and counts.
        var info: CrewHandInfo
        var handID: UUID
        var leadID: UUID
        var leadTitle: String
        var project: String?
        /// What the hand ran on, in words: "Claude · claude-opus-5 · max".
        var model: String?
        /// The first message the hand was given, its brief.
        var brief: String?
        /// The last message the hand was given, when it was steered on past its brief.
        var latestInstruction: String?
        /// What the hand said back.
        var report: String
        /// Where this report's work went; nil for a hand that recorded no landing.
        var landing: CrewLanding?
        /// The hand's own copy of the checkout, for work that did not land.
        var copyPath: String?
    }

    /// The folder a lead's reports go in: the first eight characters of its id, so one
    /// lead's reports sit together.
    static func folderName(forLead leadID: UUID) -> String {
        String(leadID.uuidString.lowercased().prefix(8))
    }

    /// The report's file name: when it was written, whose it is and which hand, as in
    /// "2026-09-22-153012-alfa-1a2b3c4d.md". A hand steered on reports again, and each of
    /// its reports keeps a file of its own.
    static func fileName(persona: CrewPersona, handID: UUID, at date: Date) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .current
        formatter.dateFormat = "yyyy-MM-dd-HHmmss"
        let slug = persona.name.replacingOccurrences(of: " ", with: "-").lowercased()
        return "\(formatter.string(from: date))-\(slug)-\(handID.uuidString.lowercased().prefix(8)).md"
    }

    static func markdown(_ contents: Contents) -> String {
        let info = contents.info
        let task = TextCleanup.singleLine(info.task, limit: 200)
        var lines = ["# \(info.persona.name)\(task.isEmpty ? "" : ": " + task)", ""]

        var facts = ["- Status: \(status(info.status))"]
        facts.append("- Lead: \(TextCleanup.singleLine(contents.leadTitle, limit: 120)) (`\(contents.leadID.uuidString.lowercased())`)")
        facts.append("- Hand: `\(contents.handID.uuidString.lowercased())`, \(info.kind == .native ? "native" : "hosted"), \(origin(info.origin))")
        if let project = contents.project { facts.append("- Project: \(project)") }
        if let model = contents.model { facts.append("- Ran on: \(model)") }
        if let profile = info.profile { facts.append("- Profile: \(profile)") }
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        formatter.timeZone = .current
        facts.append("- Started: \(formatter.string(from: info.startedAt))")
        if let finished = info.finishedAt {
            facts.append("- Finished: \(formatter.string(from: finished)) (\(duration(finished.timeIntervalSince(info.startedAt))))")
        }
        if info.toolCalls > 0 { facts.append("- Tools: \(info.toolCalls)") }
        if info.tokens > 0 { facts.append("- Tokens: \(info.tokens)") }
        lines += facts

        lines += ["", "## Where the work went", "", landing(contents)]

        let report = contents.report.trimmingCharacters(in: .whitespacesAndNewlines)
        lines += ["", "## Report", "", report.isEmpty ? "No report." : report]

        if let brief = contents.brief?.trimmingCharacters(in: .whitespacesAndNewlines), !brief.isEmpty {
            lines += ["", "## Brief", "", fenced(brief)]
            if let latest = contents.latestInstruction?.trimmingCharacters(in: .whitespacesAndNewlines),
               !latest.isEmpty, latest != brief {
                lines += ["", "## Latest instruction", "", fenced(latest)]
            }
        }
        return lines.joined(separator: "\n") + "\n"
    }

    /// Where the work went, in the words the report message uses for the lead.
    private static func landing(_ contents: Contents) -> String {
        guard let landing = contents.landing else {
            if let copy = contents.copyPath {
                return contents.info.status == .completed
                    ? "Changed nothing."
                    : "Nothing landed; the unfinished work is in the hand's copy at `\(copy)`."
            }
            return "Worked in the lead's checkout itself; whatever it changed is there already."
        }
        if landing.isEmpty { return "Changed nothing." }
        var lines: [String] = []
        if let patch = landing.patchPath {
            lines.append("Did not land\(landing.error.map { " (\($0))" } ?? ""); kept as a patch at `\(patch)`. Apply it with `git apply --3way \(patch)`.")
        } else if let error = landing.error {
            lines.append("Did not land (\(error)).")
        } else {
            lines.append("Landed in the checkout:")
        }
        lines.append("")
        for file in landing.files {
            let conflicted = landing.conflicts.contains(file.path) ? " with conflict markers" : ""
            lines.append("- `\(file.path)` (+\(file.additions) −\(file.deletions))\(conflicted)")
        }
        if landing.droppedBuildOutputFiles > 0 {
            lines.append("")
            lines.append("\(landing.droppedBuildOutputFiles) build-output file\(landing.droppedBuildOutputFiles == 1 ? " was" : "s were") left out.")
        }
        return lines.joined(separator: "\n")
    }

    private static func status(_ status: CrewHandInfo.Status) -> String {
        switch status {
        case .running: "Running"
        case .completed: "Completed"
        case .failed: "Failed"
        case .stopped: "Stopped before finishing"
        }
    }

    private static func origin(_ origin: CrewHandInfo.Origin) -> String {
        switch origin {
        case .delegated: "delegated by the lead"
        case .queued: "queued by the user"
        }
    }

    private static func duration(_ seconds: TimeInterval) -> String {
        let total = max(0, Int(seconds))
        if total < 60 { return "\(total)s" }
        if total < 3600 { return "\(total / 60)m \(total % 60)s" }
        return "\(total / 3600)h \((total % 3600) / 60)m"
    }

    /// The text in a fence longer than any run of backticks inside it, so a brief that
    /// quotes code of its own stays in one block.
    private static func fenced(_ text: String) -> String {
        var longest = 0
        var run = 0
        for character in text {
            if character == "`" {
                run += 1
                longest = max(longest, run)
            } else {
                run = 0
            }
        }
        let fence = String(repeating: "`", count: max(3, longest + 1))
        return "\(fence)text\n\(text)\n\(fence)"
    }
}
