// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

// Every hand's report is also kept as a Markdown file under the app's state directory,
// one folder per lead (see `CrewReportFile`): the lead hears the report in its chat, and
// the file is there for whoever comes after, the chat deleted or the run cut short.

extension AppModel {
    /// Writes a finishing hand's report to a file of its own and hands back where it went;
    /// nil when it could not be written, which never holds up the report itself. `summary`
    /// and `landing` are this report's, as the lead receives them, not whatever an earlier
    /// report of the same hand left on its record.
    func writeCrewReport(of hand: ChatThread, info: CrewHandInfo, status: CrewHandInfo.Status, summary: String?, landing: CrewLanding?, live: ThreadRuntime?) -> String? {
        guard let leadID = hand.parentThreadID else { return nil }
        let finishedAt = Date.now
        var record = info
        record.status = status
        record.finishedAt = finishedAt
        if let live {
            record.toolCalls = max(record.toolCalls, live.crewToolCalls)
            record.tokens = max(record.tokens, live.crewSpentTokens)
        }
        // The hand's own messages: the first is its brief, the last what it was steered on
        // with. Crew's notes in its timeline are neither.
        var brief: String?
        var latest: String?
        for entry in live?.entries ?? [] where entry.kind == .user {
            guard case .user(let message) = entry.item.content, !message.isFromCrew else { continue }
            if brief == nil { brief = message.text }
            latest = message.text
        }
        let projectLine = self.project(hand.projectID).map { "\($0.name) (`\($0.path)`)" }
        let ranOn: [String?] = [hand.provider.displayName, hand.model, hand.effort.map { "effort \($0)" }]
        let contents = CrewReportFile.Contents(
            info: record,
            handID: hand.id,
            leadID: leadID,
            leadTitle: thread(leadID)?.title ?? ChatThread.untitled,
            project: projectLine,
            model: ranOn.compactMap { $0 }.joined(separator: " · "),
            brief: brief,
            latestInstruction: latest,
            report: summary ?? "",
            landing: landing,
            copyPath: info.hasOwnCopy ? hand.worktreePath : nil
        )
        let folder = Storage.reportsDirectory.appendingPathComponent(CrewReportFile.folderName(forLead: leadID), isDirectory: true)
        let url = folder.appendingPathComponent(CrewReportFile.fileName(persona: info.persona, handID: hand.id, at: finishedAt))
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try Data(CrewReportFile.markdown(contents).utf8).write(to: url, options: .atomic)
        } catch {
            return nil
        }
        return url.path
    }
}
