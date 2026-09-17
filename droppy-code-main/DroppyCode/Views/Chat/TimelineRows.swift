import AppKit
import SwiftUI

/// The timeline's shared geometry. One vertical rhythm governs every pair of rows —
/// reply to reply, reply to work line, work line to row — so the distance between
/// text and the steps never varies. A message's hover controls sit inside that gap
/// rather than widening it.
enum TimelineMetrics {
    /// The gap between any two consecutive rows in the timeline.
    static let rowSpacing: CGFloat = 20
    /// The room a message keeps for its hover line (copy, revert). Reserved inside
    /// the row so showing the controls never moves text, then subtracted from the
    /// row's own height so they fill the gap below instead of adding to it.
    static let hoverLineHeight: CGFloat = 22
    /// The icon column every transcript row starts with. Rows carry no leading inset,
    /// so the column sits on the reply text's own x and all labels line up after it.
    static let iconWidth: CGFloat = 16
    static let iconSpacing: CGFloat = 8
}

/// The pasteboard half of `CopyButton` (see MarkdownView.swift), for the message menus.
private func copyMessageText(_ text: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(text, forType: .string)
}

/// The last whole fenced code block's code, if the text holds one: a simple scan for
/// lines starting with three backticks.
private func lastCodeBlock(in text: String) -> String? {
    var last: String?
    var open: [String]?
    for line in text.components(separatedBy: "\n") {
        if line.hasPrefix("```") {
            if let done = open {
                last = done.joined(separator: "\n")
                open = nil
            } else {
                open = []
            }
        } else {
            open?.append(line)
        }
    }
    return last
}

/// Carries a message row's context-menu intents outside its @State, the way the sidebar's
/// menuRequests does: the AppKit menu outlives the right-click, so its callbacks must not
/// retain the row. The row takes the request up below.
@Observable
private final class MessageMenuRequests {
    var confirmRevert = false
}

struct UserMessageRow: View {
    let entry: TimelineEntry
    let runtime: ThreadRuntime
    /// Whether the message can be edited right now, which rewinds its turn. Decided by the
    /// timeline, which reads the provider and the turn list once for every row.
    let canRevert: Bool

    @State private var isHovering = false
    @State private var isConfirmingRevert = false
    /// Carries the edit-from-here intent out of the context menu without retaining the row.
    @State private var menuRequests = MessageMenuRequests()
    /// The editor popover, anchored to the pencil.
    @State private var editor = PromptEditorPopover<MessageEditor>()

    var body: some View {
        if case .user(let message) = entry.item.content, message.isFromHydra {
            HydraReportRow(message: message)
        } else if case .user(let message) = entry.item.content, message.isHydraBrief {
            HydraBriefRow(message: message, entry: entry, runtime: runtime)
        } else if case .user(let message) = entry.item.content {
            // Which message is open for editing is the thread's, so the editor survives
            // the row: leaving the thread closes the popover only, and the row coming
            // back reopens it on the edit as it was left.
            let isEditing = entry.turnID != nil && runtime.messageEdit?.id == entry.turnID
            VStack(alignment: .trailing, spacing: 6) {
                if !message.attachments.isEmpty {
                    AttachmentStrip(attachments: message.attachments)
                }
                let parts = ReplyQuote.peel(message.text)
                if !parts.quotes.isEmpty {
                    // The quotes the message led with, as the chips the chat box showed them in,
                    // so a reply reads as a reply and the bubble keeps only the typed words.
                    VStack(alignment: .trailing, spacing: 4) {
                        ForEach(parts.quotes) { quote in
                            HStack(spacing: 5) {
                                Image(systemName: "text.quote")
                                    .font(.system(size: 10, weight: .semibold))
                                    .foregroundStyle(Chrome.secondaryText)
                                Text(verbatim: quote.excerpt)
                                    .font(.system(size: 11))
                                    .foregroundStyle(Chrome.primaryText.opacity(0.9))
                                    .lineLimit(1)
                                    .truncationMode(.tail)
                                    .frame(maxWidth: 260, alignment: .leading)
                            }
                            .fixedSize()
                            .modifier(ChipChrome())
                            .help(quote.text)
                            .accessibilityElement(children: .combine)
                            .accessibilityLabel(Text(verbatim: "Quote: \(quote.excerpt)"))
                        }
                    }
                    .padding(.trailing, UserBubble.tail)
                }
                if !parts.body.isEmpty {
                    Text(parts.body)
                        .textSelection(.enabled)
                        .padding(.leading, 14)
                        // The tail hangs past the body; the text keeps its inset from the body.
                        .padding(.trailing, 14 + UserBubble.tail)
                        .padding(.vertical, 9)
                        .background(.tint.opacity(0.14), in: UserBubble())
                }
                HStack(spacing: 2) {
                    if canRevert, let turnID = entry.turnID {
                        Button {
                            // The pencil toggles: a second tap while open closes the editor.
                            runtime.messageEdit = isEditing ? nil : PromptEdit(id: turnID, text: message.text, attachments: message.attachments)
                        } label: {
                            Image(systemName: "pencil")
                                .frame(width: 22, height: 22)
                                .contentShape(.rect)
                        }
                        .buttonStyle(.borderless)
                        .foregroundStyle(isEditing ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                        .help("Edit message")
                        // The pointer settling on the pencil is the editor about to open: its
                        // file preview starts now, so the click finds it done or nearly so.
                        .onHover { hovering in
                            if hovering { _ = runtime.revertPreview(for: turnID) }
                        }
                        .background {
                            AttachmentAnchorCapture { anchor in
                                editor.setAnchor(anchor)
                                // The anchor lands in its window after the row appears: the
                                // moment a reopened thread can show the editor again.
                                syncEditor(isEditing)
                            }
                        }
                    }
                }
                // Pinned to the hover line's room, so the row keeps it (and the gap
                // math below holds) with no copy button left to size it.
                .frame(height: TimelineMetrics.hoverLineHeight)
                .opacity(isHovering || isEditing ? 1 : 0)
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
            .padding(.leading, 96)
            // The binding alone, so the hover responder never keeps this row (and its
            // message) alive after it scrolls away.
            .onHover { [hovering = $isHovering] in hovering.wrappedValue = $0 }
            // Snapshots only: the popover opens at the pointer and its builder must not
            // capture the row, so the popover cannot pin the row's state storage
            // after dismiss. The revert intent goes through the binding alone, the
            // way the hover responder does below.
            .rightClickPopover { [message, canRevert, confirming = $menuRequests.confirmRevert] in
                Self.messageActions(
                    text: message.text,
                    canRevert: canRevert,
                    confirmRevert: confirming
                )
            }
            // The room for the hover line lives inside the row but is taken back out
            // of its height, so the controls show up in the gap to the next row and
            // that gap stays the same whether or not they are showing. (6 = the
            // VStack's spacing above them.)
            .padding(.bottom, -(TimelineMetrics.hoverLineHeight + 6))
            .onChange(of: menuRequests.confirmRevert) { _, asked in
                guard asked else { return }
                // The menu posts the intent, the row takes it up, like the sidebar's
                // menuRequests: the confirmation dialog then opens on the row itself.
                menuRequests.confirmRevert = false
                isConfirmingRevert = true
            }
            .confirmationDialog("Edit from this message?", isPresented: $isConfirmingRevert) {
                Button("Revert and keep file changes") { revert(restoreFiles: false) }
                Button("Revert files too", role: .destructive) { revert(restoreFiles: true) }
            } message: {
                Text("This message and everything after it leave the thread, and your prompt returns to the composer.")
            }
            .onChange(of: isEditing) { _, editing in syncEditor(editing) }
            .onDisappear { editor.close() }
        }
    }

    private func revert(restoreFiles: Bool) {
        guard let turnID = entry.turnID else { return }
        // A revert that cannot start (a turn still running) says so in the thread itself.
        Task { _ = try? await runtime.revert(to: turnID, restoreFiles: restoreFiles) }
    }

    private func syncEditor(_ isEditing: Bool) {
        editor.sync(isEditing ? runtime.messageEdit : nil, dismiss: { runtime.messageEdit = nil }) { edit in
            MessageEditor(edit: edit, runtime: runtime)
        }
    }

    /// The context menu's items, from value snapshots with weak captures: the AppKit menu
    /// outlives the right-click, so its callbacks must not retain the row.
    private static func messageActions(text: String, canRevert: Bool, confirmRevert: Binding<Bool>) -> [RowAction] {
        let textSnapshot = text
        var items = [
            RowAction(title: "Copy message", symbol: "doc.on.doc", confirms: "Copied") { copyMessageText(MessageText.plain(textSnapshot)) },
            RowAction(title: "Copy as markdown", symbol: "number", confirms: "Copied") { copyMessageText(textSnapshot) },
        ]
        if canRevert {
            items.append(RowAction(title: "Edit from here", symbol: "arrow.uturn.backward", startsGroup: true) { [confirm = confirmRevert] in
                confirm.wrappedValue = true
            })
        }
        return items
    }
}

/// Heads reporting back to their lead: their glyphs and names on a line, and their reports
/// in a popover off it. It sits on the user's side, since that is where the lead reads it from,
/// but reads as the team's, not the user's. A note from Hydra itself (heads held back, the
/// team's work merged) takes the same row with the Hydra mark in the glyphs' place; a note
/// that leads with the merge request's address links to it from the pill.
struct HydraReportRow: View {
    @Environment(\.chatZoom) private var zoom
    let message: UserMessage

    @State private var isShowingReport = false

    var body: some View {
        let personas = (message.hydraHeads ?? []).map(HydraRoster.persona(at:))
        let names = personas.map(\.name)
        // A note from Hydra itself says what it is on its first line; the rest is the message.
        // That line ends like a sentence, and the pill reads it as a label.
        // A heads' report keeps its full text for the popover, but the pill reads a
        // short human summary with the heads first, never the file-heavy detail.
        let parts = message.text.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true)
        let title = personas.isEmpty ? Self.withoutTrailingStop(String(parts.first ?? "Hydra")) : Self.reportSummary(names: names, text: message.text)
        let body = personas.isEmpty ? String(parts.count > 1 ? parts[1] : "").trimmingCharacters(in: .whitespacesAndNewlines) : message.text
        // A merge note puts the merge request's address on the body's first line: that becomes
        // a link on the pill, and the chevron stays only for what the note says after it.
        let link = personas.isEmpty ? Self.leadingURL(in: body) : nil
        let details = link == nil ? body : Self.withoutFirstLine(body)
        // The link is a control of its own beside the button, never inside its label: a
        // link nested in a button takes the button's clicks on macOS, and the text
        // stopped opening the popover.
        HStack(spacing: 8) {
            Button {
                guard !details.isEmpty else { return }
                isShowingReport.toggle()
            } label: {
                HStack(spacing: 8) {
                    // The mark takes a glyph's slot, so both rows sit the same in the pill.
                    // (Only one or the other: an empty stack would still keep its spacing.)
                    if personas.isEmpty {
                        HydraMarkImage()
                            .foregroundStyle(Chrome.secondaryText)
                            .frame(width: 18, height: 18)
                    } else {
                        HStack(spacing: -4) {
                            ForEach(Array(personas.enumerated()), id: \.offset) { _, persona in
                                HydraGlyph(persona: persona, size: 18)
                            }
                        }
                    }
                    Text(verbatim: title)
                        .font(.chat(.callout, weight: .medium, zoom: zoom))
                        .foregroundStyle(Chrome.primaryText.opacity(0.9))
                        .lineLimit(1)
                        .truncationMode(.tail)
                    if !details.isEmpty {
                        Image(systemName: "chevron.right")
                            .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                            .foregroundStyle(.tertiary)
                    }
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(details.isEmpty)
            .help(details.isEmpty ? "" : "Show the message")
            .popover(isPresented: $isShowingReport, arrowEdge: .bottom) {
                // Reports from heads draw the digest; a merge note draws its own card;
                // every other note keeps the markdown view.
                if let link, let request = MergeRequestLink(url: link), let note = HydraMergeNote.parse(details, link: request) {
                    HydraMergePopover(note: note)
                        .presentedChrome()
                } else if !personas.isEmpty {
                    HydraReportsPopover(title: title, text: body, personas: personas)
                        .presentedChrome()
                } else {
                    HydraReportPopover(text: body)
                        .presentedChrome()
                }
            }
            if let link {
                Link(destination: link) {
                    Text("Open")
                        .font(.chat(.caption, weight: .semibold, zoom: zoom))
                        .foregroundStyle(Chrome.secondaryText)
                }
                .help("Open in the browser")
            }
        }
        .padding(.leading, 12)
        .padding(.trailing, 14)
        .padding(.vertical, 8)
        .background(.tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        // The report parses off the main thread as the pill appears, so the tap that
        // opens it finds the blocks ready rather than parsing the whole batch first.
        .task(id: body) {
            guard !details.isEmpty else { return }
            try? await MarkdownView.warm([body])
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.trailing, 96)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(title))
    }

    /// The title without the one full stop a note's first line ends on.
    static func withoutTrailingStop(_ title: String) -> String {
        title.hasSuffix(".") ? String(title.dropLast()) : title
    }

    /// The pill for heads reporting back: just who is done, since the report itself is
    /// one tap away in the popover. "Hank is done", "Hank and Walter are done"; a head
    /// whose `## Name: task (failed)` section says otherwise is named with its outcome
    /// instead, so a failure never hides behind "done".
    private static func reportSummary(names: [String], text: String) -> String {
        let outcomes = reportOutcomes(in: text)
        let done = names.filter { outcomes[$0] == nil }
        let others = names.filter { outcomes[$0] != nil }
        var parts: [String] = []
        if !done.isEmpty {
            parts.append("\(list(done)) \(done.count == 1 ? "is" : "are") done")
        }
        for name in others {
            parts.append("\(name) \(outcomes[name] ?? "failed")")
        }
        return TextCleanup.singleLine(parts.joined(separator: ", "), limit: 140)
    }

    /// "Hank", "Hank and Walter", "Hank, Walter and Ada".
    private static func list(_ names: [String]) -> String {
        if names.count <= 1 { return names.first ?? "" }
        return names.dropLast().joined(separator: ", ") + " and " + (names.last ?? "")
    }

    /// Each head's outcome off its `## Name: task (failed)` / `(stopped)` section, by
    /// name; a head that finished has no entry.
    private static func reportOutcomes(in text: String) -> [String: String] {
        var outcomes: [String: String] = [:]
        for rawLine in text.split(whereSeparator: \.isNewline) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            guard line.hasPrefix("## "), let colon = line.firstIndex(of: ":") else { continue }
            let name = String(line[line.index(line.startIndex, offsetBy: 3)..<colon]).trimmingCharacters(in: .whitespaces)
            let lower = line[colon...].lowercased()
            if lower.contains("(failed") { outcomes[name] = "failed" }
            else if lower.contains("(stopped") { outcomes[name] = "stopped" }
        }
        return outcomes
    }

    /// The address on the body's first line, when that line is an address and nothing else.
    static func leadingURL(in body: String) -> URL? {
        guard let line = body.split(whereSeparator: \.isNewline).first.map(String.init)?.trimmingCharacters(in: .whitespaces),
              line.lowercased().hasPrefix("http://") || line.lowercased().hasPrefix("https://"),
              !line.contains(where: \.isWhitespace) else { return nil }
        return URL(string: line)
    }

    /// The body past its first line: what a note says after the address it leads with.
    static func withoutFirstLine(_ body: String) -> String {
        guard let newline = body.firstIndex(where: \.isNewline) else { return "" }
        return String(body[newline...]).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// The heads still out on the lead's behalf: who they are, working, until they report back.
/// The report pill's twin while the work is still on.
struct HydraHeadsWorkingRow: View {
    @Environment(\.chatZoom) private var zoom
    @Environment(AppModel.self) private var model
    let heads: [Int]
    let runtime: ThreadRuntime

    var body: some View {
        let personas = heads.map(HydraRoster.persona(at:))
        let names = personas.map(\.name)
        let who: String = {
            if names.isEmpty { return "" }
            if names.count == 1 { return names[0] }
            return names.dropLast().joined(separator: ", ") + " and " + (names.last ?? "")
        }()
        // Every head out has gone quiet for three minutes (no tool, no word): the pill
        // says so. The watchdog labels a head's row at the same moment, and that label is
        // what redraws this one (see `AppModel.startHydraWatchdog`).
        // The whole team, not only the panel: a finished head leaves the panel on its own
        // (`hydraAutoClearFinished`), and its pill below has to stay until the report lands.
        // Read through this thread's own cells (`helpers(of:)` holds every unarchived head,
        // in the panel or not), the same list as `hydraTeam(of:)` without its scan of
        // `model.threads`: a write to some other chat never re-renders this pill.
        let team = model.helpers(of: runtime.threadID)
            .filter(\.isHydraHead)
            .sorted { ($0.hydra?.index ?? 0) < ($1.hydra?.index ?? 0) }
        let running = team.filter { $0.hydra?.status == .running && heads.contains($0.hydra?.index ?? -1) }
        let thinking = !running.isEmpty && running.allSatisfy { head in
            guard let live = model.existingRuntime(for: head.id) else { return false }
            return live.hydraActivity?.hasPrefix("Thinking for") == true || live.hydraIdleSeconds >= 180
        }
        let title = (names.isEmpty ? "Heads are working" : "\(who) \(names.count == 1 ? "is" : "are") working") + (thinking ? " · thinking" : "")
        // Heads already finished while the rest of their batch still works: one small
        // pill each below, with the report one tap away.
        let finished = Self.finishedHeads(in: team)
        // Every pill starts at the same left edge, glyphs in one column, whatever its
        // width: trailing alignment hung each narrower pill off the widest one's right
        // edge, and a head coming or going moved them all.
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                HStack(spacing: -4) {
                    ForEach(Array(personas.enumerated()), id: \.offset) { _, persona in
                        HydraGlyph(persona: persona, size: 18, isRunning: true)
                    }
                }
                Text(verbatim: title)
                    .font(.chat(.callout, weight: .medium, zoom: zoom))
                    .foregroundStyle(Chrome.primaryText.opacity(0.9))
                    .modifier(HydraShimmer())
                    .contentTransition(.opacity)
            }
            .padding(.leading, 12)
            .padding(.trailing, 14)
            .padding(.vertical, 8)
            .background(.tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .animation(.smooth(duration: 0.3), value: heads)
            .accessibilityElement(children: .combine)
            .accessibilityLabel(Text(title))
            ForEach(finished, id: \.index) { head in
                FinishedHeadPill(head: head)
            }
        }
        .animation(.smooth(duration: 0.3), value: finished.map(\.index))
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.trailing, 96)
    }

    /// The finished heads whose report has not reached the lead yet: a finished head whose
    /// batch still has a head at work, or — with no batch — a head that finished since the
    /// earliest running head set out, so heads done in an earlier turn stay out. In roster order.
    private static func finishedHeads(in threads: [ChatThread]) -> [HydraHeadInfo] {
        let infos = threads.compactMap(\.hydra)
        let running = infos.filter { $0.status == .running }
        guard !running.isEmpty else { return [] }
        let since = running.map(\.startedAt).min() ?? .distantPast
        return infos.filter { info in
            guard info.isFinished else { return false }
            guard let batch = info.batchID else { return (info.finishedAt ?? .distantPast) >= since }
            return running.contains { $0.batchID == batch }
        }
    }
}

/// A head finished while its batch still works: its outcome now, its report one tap away.
/// The working pill's small twin, without the shimmer.
private struct FinishedHeadPill: View {
    @Environment(\.chatZoom) private var zoom
    let head: HydraHeadInfo

    @State private var isShowingReport = false

    var body: some View {
        let title: String = switch head.status {
        case .failed: "\(head.persona.name) failed"
        case .stopped: "\(head.persona.name) was stopped"
        default: "\(head.persona.name) finished working"
        }
        let summary = head.summary?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let text = "## \(head.persona.name): \(head.task)\n" + (summary.isEmpty ? "No report yet." : summary)
        Button {
            isShowingReport.toggle()
        } label: {
            HStack(spacing: 8) {
                HydraGlyph(persona: head.persona, size: 18, status: head.status)
                Text(verbatim: title)
                    .font(.chat(.callout, weight: .medium, zoom: zoom))
                    .foregroundStyle(Chrome.primaryText.opacity(0.75))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Image(systemName: "chevron.right")
                    .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                    .foregroundStyle(.tertiary)
            }
            .padding(.leading, 12)
            .padding(.trailing, 14)
            .padding(.vertical, 8)
            .background(.tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .help("Show the report")
        .popover(isPresented: $isShowingReport, arrowEdge: .bottom) {
            HydraReportPopover(text: text)
                .presentedChrome()
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(title))
    }
}

/// A shimmer sweeping across the content on a loop: a narrow bright band sliding
/// left to right, masked to the content itself. Still when reduced motion is on.
private struct HydraShimmer: ViewModifier {
    @State private var width: CGFloat = 0

    /// One pass of the band, from off the left edge to off the right.
    private static let sweep: TimeInterval = 1.8

    func body(content: Content) -> some View {
        if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            content
        } else {
            // The band lives in a layer the size of the text and is masked to the text
            // there: masking the band alone would cut the text to the band's own width.
            let band = max(24, width / 3)
            content
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
                .overlay {
                    // Thirty frames a second is enough for a band this soft, a quarter of the
                    // display's rate; and it holds still while the reader scrolls anywhere, so
                    // the mask pass below never lands on a scrolled frame.
                    TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: ScrollActivity.shared.isScrolling)) { context in
                        let phase = CGFloat(context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: Self.sweep) / Self.sweep)
                        Color.clear
                            .overlay(alignment: .leading) {
                                LinearGradient(
                                    stops: [
                                        .init(color: Chrome.primaryText.opacity(0), location: 0),
                                        .init(color: Chrome.primaryText.opacity(0.9), location: 0.5),
                                        .init(color: Chrome.primaryText.opacity(0), location: 1),
                                    ],
                                    startPoint: .leading,
                                    endPoint: .trailing
                                )
                                .frame(width: band)
                                .offset(x: -band + phase * (width + band))
                            }
                            .mask { content }
                            .allowsHitTesting(false)
                    }
                }
        }
    }
}

/// The lead's brief to a head, as the report pill's twin: a small pill where the user's
/// message would sit, with the full brief one tap away in a popover. While the brief's
/// turn has no assistant output yet the head is still cooking, so the pill reads so with
/// the thinking shimmer; once the head has answered it reads as the brief.
struct HydraBriefRow: View {
    @Environment(\.chatZoom) private var zoom
    let message: UserMessage
    let entry: TimelineEntry
    let runtime: ThreadRuntime

    @State private var isShowingBrief = false
    /// Whether the brief's turn has output, as of an entry count: the row re-evaluates far
    /// more often than the timeline grows, so the scan runs once per count and is replayed.
    @State private var scanned: (count: Int, value: Bool)?

    var body: some View {
        let persona = runtime.thread?.hydra?.persona ?? HydraRoster.persona(at: 0)
        // The brief's turn has produced nothing of its own yet: no entry past the brief
        // itself on the same turn means the head is still cooking.
        let count = runtime.entries.count
        let hasOutput: Bool = if let scanned, scanned.count == count {
            scanned.value
        } else {
            entry.turnID != nil && runtime.entries.contains { $0.turnID == entry.turnID && $0.id != entry.id }
        }
        let cooking = !hasOutput && runtime.isRunning
        let title = cooking ? "\(persona.name) is starting to cook…" : "\(persona.name)'s brief"
        VStack(alignment: .trailing, spacing: 6) {
            if !message.attachments.isEmpty {
                AttachmentStrip(attachments: message.attachments)
            }
            Button {
                isShowingBrief.toggle()
            } label: {
                HStack(spacing: 8) {
                    HydraGlyph(persona: persona, size: 18, isRunning: cooking)
                    Group {
                        if cooking {
                            Text(verbatim: title)
                                .modifier(HydraShimmer())
                        } else {
                            Text(verbatim: title)
                        }
                    }
                    .font(.chat(.callout, weight: .medium, zoom: zoom))
                    .foregroundStyle(Chrome.primaryText.opacity(0.9))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    Image(systemName: "chevron.right")
                        .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                        .foregroundStyle(.tertiary)
                }
                .padding(.leading, 12)
                .padding(.trailing, 14)
                .padding(.vertical, 8)
                .background(.tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .focusable(false)
            .help("Show the brief")
            .popover(isPresented: $isShowingBrief, arrowEdge: .bottom) {
                HydraReportPopover(text: message.text, width: 520)
                    .presentedChrome()
            }
            .accessibilityLabel(Text(title))
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .padding(.leading, 96)
        .accessibilityElement(children: .contain)
        // The scan the body just made, kept for the evaluations until the count moves.
        .onChange(of: count, initial: true) { _, _ in
            if scanned?.count != count { scanned = (count, hasOutput) }
        }
    }
}

/// A head's report in the popover its pill opens: the markdown at reading width, scrolling
/// past the panel's height rather than pushing the timeline apart.
///
/// A batch of reports runs to tens of thousands of characters, and a popover cannot open
/// before its content has laid out: the blocks are parsed off the main thread (the pill
/// warms them as it appears, so a tap usually finds them cached) and built lazily, so
/// opening costs the blocks on screen rather than the whole report.
private struct HydraReportPopover: View {
    let text: String
    var width: CGFloat = 460

    @State private var blocks: [MarkdownBlock]?

    var body: some View {
        ScrollView {
            if let blocks {
                LazyVStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                        MarkdownBlockView(block: block)
                            .equatable()
                    }
                }
                // The note reads at the popover's own compact size, not the chat's.
                .font(.system(size: 12))
                .environment(\.markdownPointSize, 12)
                .environment(\.markdownDimmed, false)
                .environment(\.chatZoom, 1)
                .textSelection(.enabled)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                ProgressView()
                    .controlSize(.small)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 40)
            }
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(width: width)
        .frame(idealHeight: 320, maxHeight: 460)
        .task(id: text) {
            try? await MarkdownView.warm([text])
            guard !Task.isCancelled else { return }
            blocks = MarkdownView.blocks(for: text)
        }
    }
}

/// The lead's delegation block, the fenced `hydra` JSON at the end of its reply, as the
/// card it stands for: the mark, how many heads are going out and the task each one gets.
/// Never the JSON itself. While the block is still streaming, or when it cannot be read,
/// the card says the briefs are being written and nothing more.
struct HydraDelegationBlock: View {
    @Environment(\.chatZoom) private var zoom
    let json: String

    var body: some View {
        let tasks = Self.tasks(in: json)
        // The mark and the title share a line; the tasks run under both, flush with the
        // mark, so the list reads from the card's own left edge.
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .center, spacing: TimelineMetrics.iconSpacing) {
                HydraMarkImage()
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(width: 16, height: 16)
                    .accessibilityHidden(true)
                if tasks.isEmpty {
                    Text("Writing the heads' briefs…")
                        .font(.chat(.callout, zoom: zoom))
                        .foregroundStyle(Chrome.secondaryText)
                } else {
                    Text(tasks.count == 1 ? "Sending out a head" : "Sending out \(tasks.count) heads")
                        .font(.chat(.callout, weight: .medium, zoom: zoom))
                }
            }
            if !tasks.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(Array(tasks.enumerated()), id: \.offset) { _, task in
                        Text(verbatim: "· " + task)
                            .font(.chat(.caption, zoom: zoom))
                            .foregroundStyle(Chrome.secondaryText)
                    }
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(.tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .combine)
    }

    /// The tasks read out of each block's JSON, by source: a finished block is the same
    /// card on every evaluation, so its JSON parses once. Only whole JSON gets in; a block
    /// still streaming fails to parse and is read again on the next flush.
    @MainActor private static var taskCache = RecentCache<String, [String]>(limit: 200)

    /// What each head is given, in the block's order: its task, or the start of its prompt
    /// when it has none (as `HydraPrompts.delegations(in:)` titles it). Nothing until the
    /// JSON is whole and holds at least one head.
    @MainActor
    private static func tasks(in json: String) -> [String] {
        if let cached = taskCache.value(for: json) { return cached }
        guard let parsed = JSONValue.parse(json) else { return [] }
        let entries: [JSONValue] = parsed.array ?? (parsed.object == nil ? [] : [parsed])
        let tasks = entries.compactMap { entry -> String? in
            let task = entry["task"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if !task.isEmpty { return task }
            let prompt = entry["prompt"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return prompt.isEmpty ? nil : TextCleanup.singleLine(prompt, limit: 60)
        }
        taskCache.insert(tasks, for: json)
        return tasks
    }
}

/// What the merge pill shows: the merge under way, or the note about how it went.
enum HydraMergePhase: Equatable {
    case merging
    case outcome(UserMessage)
}

/// A merge outcome note read for the pill: the title it leads with, the merge request's
/// address when the body leads with one, and what the note says past that.
private struct HydraMergeOutcome {
    let title: String
    /// The note past its title, for the popover.
    let body: String
    /// The merge request's address, when the body leads with one: a link beside the pill.
    let link: URL?
    /// What the note says after the address, if any: with none, the pill has no popover.
    let details: String

    init(_ message: UserMessage) {
        let parts = message.text.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true)
        title = HydraReportRow.withoutTrailingStop(String(parts.first ?? "Hydra"))
        body = String(parts.count > 1 ? parts[1] : "").trimmingCharacters(in: .whitespacesAndNewlines)
        link = HydraReportRow.leadingURL(in: body)
        details = link == nil ? body : HydraReportRow.withoutFirstLine(body)
    }
}

/// The team's work on its way to the remote once the lead has finished, and then the word
/// on how it went, in one pill in the report pill's frame. While the merge runs: the mark,
/// the spinner, the stage the merge is at right now (the same words the sidebar shows), and
/// how long it has been at it. Once the outcome note lands: the note's title with its report
/// a tap away, and the merge request's link beside it. The timeline draws the pill under the
/// outcome note's own id from the first stage on (see `ThreadRuntime.hydraMergeNoteID`), so
/// this one row keeps its identity across the change and morphs: the contents crossfade
/// while the capsule glides to the new width. It holds no condition of its own.
struct HydraMergeRow: View {
    @Environment(\.chatZoom) private var zoom
    let runtime: ThreadRuntime
    let phase: HydraMergePhase

    @State private var isShowingReport = false

    /// The one motion for anything in the pill changing: the stage's words, the time's
    /// digits, and the merging state giving way to the outcome.
    private static let change = Animation.smooth(duration: 0.3)

    var body: some View {
        let outcome: HydraMergeOutcome? = if case .outcome(let message) = phase { HydraMergeOutcome(message) } else { nil }
        let isMerging = outcome == nil
        // A merging pill for a merge that is over (the flag is down and no outcome note
        // has taken the pill's place) has nothing to say: it goes, rather than reading
        // "Merging…" for good. Observed here, so the flag going down takes it away.
        if isMerging, !runtime.isHydraMerging {
            EmptyView()
        } else {
            pill(outcome: outcome, isMerging: isMerging)
        }
    }

    private func pill(outcome: HydraMergeOutcome?, isMerging: Bool) -> some View {
        // The same stage words the sidebar shows beside its own spinner.
        let stage = (runtime.hydraMergeStage ?? "Merging") + "…"
        // The stage alone, no clock: the merge's stages say what it is doing, and a
        // count of seconds beside them read as a stall.
        return HStack(spacing: 8) {
            // The mark stays put through the change; it opens the report like the title does.
            HydraMarkImage()
                .foregroundStyle(Chrome.secondaryText)
                .frame(width: 18, height: 18)
                .contentShape(.rect)
                .onTapGesture {
                    guard let outcome, !outcome.details.isEmpty else { return }
                    isShowingReport.toggle()
                }
                .accessibilityHidden(true)
            // Only the contents of the current state take part in layout: a still copy of
            // them sizes the slot, and it changes over at once (an identity transition), so
            // the pill glides straight from the one width to the other. The contents on
            // show are drawn over that slot at their own size and clipped to it, the old
            // ones fading out as the new ones fade in; laid out side by side instead, the
            // two would hold the pill wide for the length of the fade.
            ZStack(alignment: .leading) {
                if let outcome {
                    outcomeLabel(outcome)
                        .transition(.identity)
                } else {
                    mergingLabel(stage: stage)
                        .transition(.identity)
                }
            }
            .hidden()
            .overlay(alignment: .leading) {
                ZStack(alignment: .leading) {
                    if let outcome {
                        outcomeControls(outcome)
                            .fixedSize()
                            .transition(.opacity)
                    } else {
                        mergingContent(stage: stage)
                            .fixedSize()
                            .transition(.opacity)
                    }
                }
            }
            // Room above and below for the stage words' lift, and a hair either side for
            // the controls' own edges, given back after the clip.
            .padding(.vertical, 6)
            .padding(.horizontal, 2)
            .clipped()
            .padding(.horizontal, -2)
            .padding(.vertical, -6)
            // Scoped to the slot, so the shimmer's own frames beside it are never caught in
            // the transaction; the pill around it follows the slot's animated width.
            .animation(Self.change, value: isMerging)
        }
        .padding(.leading, 12)
        .padding(.trailing, 14)
        .padding(.vertical, 8)
        .background(.tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.trailing, 96)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(outcome?.title ?? "Merging the team's work: \(stage)"))
        // The report parses off the main thread as the pill appears, so the tap that
        // opens it finds the blocks ready rather than parsing the whole batch first.
        .task(id: outcome?.body) {
            guard let outcome, !outcome.details.isEmpty else { return }
            try? await MarkdownView.warm([outcome.body])
        }
    }

    // MARK: Merging

    /// The merging state as shown: the spinner, the stage with its shimmer, the time.
    private func mergingContent(stage: String) -> some View {
        HStack(spacing: 8) {
            WorkingSpinner(cellSize: 3)
                .frame(width: 14)
            // A new stage is a new text: the old one fades up and out as the new one fades in
            // from below, each with its own shimmer at its own width. A content transition
            // over the shimmer crossfaded the band's mask between the two lengths, so the
            // words tore mid-fade.
            //
            // Only the new words take part in layout: a hidden copy of them sizes the slot,
            // so the pill glides straight from the old width to the new one. The words on
            // show are drawn over that slot at their own size and clipped to it, so the
            // outgoing text neither holds the pill wide until its fade ends (it snapped
            // narrower afterwards) nor runs past the slot over the time beside it.
            Text(verbatim: stage)
                .font(.chat(.callout, weight: .medium, zoom: zoom))
                .hidden()
                .overlay(alignment: .leading) {
                    ZStack(alignment: .leading) {
                        Text(verbatim: stage)
                            .font(.chat(.callout, weight: .medium, zoom: zoom))
                            .foregroundStyle(Chrome.primaryText.opacity(0.9))
                            .modifier(HydraShimmer())
                            .fixedSize()
                            .id(stage)
                            .transition(.asymmetric(
                                insertion: .opacity.combined(with: .offset(y: 5)),
                                removal: .opacity.combined(with: .offset(y: -5))
                            ))
                    }
                }
                // Room above and below for the words' lift, given back after the clip.
                .padding(.vertical, 6)
                .clipped()
                .padding(.vertical, -6)
                // On the slot alone: the whole row's transaction would catch the shimmer's
                // next frame too and ease it, so the band hitched at every new stage.
                .animation(Self.change, value: stage)
        }
    }

    /// The merging state's still copy, for its size only: the spinner's slot and the
    /// words, in the font they show in.
    private func mergingLabel(stage: String) -> some View {
        HStack(spacing: 8) {
            Color.clear
                .frame(width: 14, height: 1)
            Text(verbatim: stage)
                .font(.chat(.callout, weight: .medium, zoom: zoom))
                .animation(Self.change, value: stage)
        }
    }

    // MARK: Outcome

    /// The outcome as shown: the title opens the report, the link opens the merge request.
    /// The link is a control of its own beside the button, never inside its label: a link
    /// nested in a button takes the button's clicks on macOS, and the text stopped opening
    /// the popover.
    private func outcomeControls(_ outcome: HydraMergeOutcome) -> some View {
        HStack(spacing: 8) {
            Button {
                guard !outcome.details.isEmpty else { return }
                isShowingReport.toggle()
            } label: {
                outcomeTitle(outcome)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(outcome.details.isEmpty)
            .help(outcome.details.isEmpty ? "" : "Show the message")
            .popover(isPresented: $isShowingReport, arrowEdge: .bottom) {
                // A merge note draws its own card; every other outcome keeps the markdown view.
                if let link = outcome.link, let request = MergeRequestLink(url: link), let note = HydraMergeNote.parse(outcome.details, link: request) {
                    HydraMergePopover(note: note)
                        .presentedChrome()
                } else {
                    HydraReportPopover(text: outcome.body)
                        .presentedChrome()
                }
            }
            if let link = outcome.link {
                Link(destination: link) {
                    openLabel
                }
                .help("Open in the browser")
            }
        }
    }

    /// The outcome's still copy, for its size only: the same title and link text without
    /// the controls around them, which add nothing to their size.
    private func outcomeLabel(_ outcome: HydraMergeOutcome) -> some View {
        HStack(spacing: 8) {
            outcomeTitle(outcome)
            if outcome.link != nil {
                openLabel
            }
        }
    }

    /// The title, and the chevron when there is a report behind it.
    private func outcomeTitle(_ outcome: HydraMergeOutcome) -> some View {
        HStack(spacing: 8) {
            Text(verbatim: outcome.title)
                .font(.chat(.callout, weight: .medium, zoom: zoom))
                .foregroundStyle(Chrome.primaryText.opacity(0.9))
                .lineLimit(1)
                .truncationMode(.tail)
            if !outcome.details.isEmpty {
                Image(systemName: "chevron.right")
                    .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                    .foregroundStyle(.tertiary)
            }
        }
    }

    private var openLabel: some View {
        Text("Open")
            .font(.chat(.caption, weight: .semibold, zoom: zoom))
            .foregroundStyle(Chrome.secondaryText)
    }
}

/// iMessage's outgoing bubble: rounded on three corners, the bottom-right one drawn out into
/// the little tail that curls back under the bubble. The shape only; the fill is the row's.
/// The tail's tip sits `tail` points past the body's trailing edge, at the very bottom.
struct UserBubble: Shape {
    static let cornerRadius: CGFloat = 18
    static let tail: CGFloat = 4

    func path(in rect: CGRect) -> Path {
        let r = Self.cornerRadius
        let left = rect.minX
        let top = rect.minY
        let bottom = rect.maxY
        // The body's trailing edge; the tail reaches past it.
        let edge = rect.maxX - Self.tail
        var path = Path()
        path.move(to: CGPoint(x: left + r, y: top))
        path.addLine(to: CGPoint(x: edge - r, y: top))
        path.addArc(center: CGPoint(x: edge - r, y: top + r), radius: r, startAngle: .degrees(-90), endAngle: .degrees(0), clockwise: false)
        path.addLine(to: CGPoint(x: edge, y: bottom - 11))
        // Out to the tip at the bottom corner, then curling back in under the body and
        // easing into the bottom edge.
        path.addCurve(
            to: CGPoint(x: edge + Self.tail, y: bottom),
            control1: CGPoint(x: edge, y: bottom - 1),
            control2: CGPoint(x: edge + Self.tail, y: bottom)
        )
        path.addCurve(
            to: CGPoint(x: edge - 7, y: bottom - 4),
            control1: CGPoint(x: edge, y: bottom + 0.5),
            control2: CGPoint(x: edge - 4, y: bottom - 1)
        )
        path.addCurve(
            to: CGPoint(x: edge - 21, y: bottom),
            control1: CGPoint(x: edge - 12, y: bottom),
            control2: CGPoint(x: edge - 16, y: bottom)
        )
        path.addLine(to: CGPoint(x: left + r, y: bottom))
        path.addArc(center: CGPoint(x: left + r, y: bottom - r), radius: r, startAngle: .degrees(90), endAngle: .degrees(180), clockwise: false)
        path.addLine(to: CGPoint(x: left, y: top + r))
        path.addArc(center: CGPoint(x: left + r, y: top + r), radius: r, startAngle: .degrees(180), endAngle: .degrees(270), clockwise: false)
        path.closeSubpath()
        return path
    }
}

struct AttachmentStrip: View {
    let attachments: [Attachment]

    /// One preview panel for the strip, so every photo opens in a single tap no matter
    /// how many are attached, and made on the first tap rather than with the row.
    @State private var preview = AttachmentPreviewSlot()

    var body: some View {
        // Weak/box captures only: hover responders and anchor views must not keep this
        // strip (and its panel) alive after it scrolls away.
        let slot = preview
        // A plain stack hugs the thumbnails, so a trailing-aligned row keeps the
        // photos over the bubble — and the anchor view fills just the photos.
        HStack(spacing: 6) {
            ForEach(attachments) { attachment in
                AttachmentThumbnail(attachment: attachment, preview: preview)
                    .help(attachment.name)
            }
        }
        .background {
            AttachmentAnchorCapture { [weak slot] in slot?.setAnchor($0) }
        }
        .onDisappear { [weak slot] in slot?.close() }
    }
}

struct AttachmentThumbnail: View {
    @Environment(\.chatZoom) private var zoom
    let attachment: Attachment
    var size: CGFloat = 56
    /// The panel the photo opens in: either a slot, which makes its panel on the first
    /// tap, or one the caller already holds. A strip that scrolls with the conversation
    /// wants the slot; the composer's own strips, which are made once, take either.
    private let slot: AttachmentPreviewSlot?
    private let panel: AttachmentPreviewCoordinator?

    @State private var image: CGImage?
    /// The thumbnail's own NSView, handed to the panel on tap so the arrow
    /// lands on the tapped photo rather than the strip's middle. Mounted once the
    /// pointer has been on the photo: a tap always follows a hover, and a strip
    /// scrolling past should not be building views for a tap that never comes.
    @State private var ownAnchor = WeakView()
    @State private var needsAnchor = false
    /// Never mounts an anchor view of its own: for a strip that keeps every AppKit view
    /// out of SwiftUI's key-view-loop walk (see `WindowRectAnchor`). The panel then aims
    /// at the tap inside the strip's own anchor, which is where the pointer is anyway.
    private let anchorless: Bool

    init(attachment: Attachment, size: CGFloat = 56, preview: AttachmentPreviewSlot, anchorless: Bool = false) {
        self.init(attachment: attachment, size: size, slot: preview, panel: nil, anchorless: anchorless)
    }

    init(attachment: Attachment, size: CGFloat = 56, preview: AttachmentPreviewCoordinator) {
        self.init(attachment: attachment, size: size, slot: nil, panel: preview, anchorless: false)
    }

    private init(attachment: Attachment, size: CGFloat, slot: AttachmentPreviewSlot?, panel: AttachmentPreviewCoordinator?, anchorless: Bool) {
        self.attachment = attachment
        self.size = size
        self.slot = slot
        self.panel = panel
        self.anchorless = anchorless
        // A photo shown before starts out drawn, so scrolling back to it never fades it in again.
        _image = State(initialValue: attachment.isImage ? ThumbnailMemory.image(for: attachment.path, pointSize: size) : nil)
    }

    /// Opens (or closes) the panel for this photo, making one only now if the caller
    /// handed over a slot.
    private func togglePanel() {
        if let slot {
            slot.panel.toggle(attachment, over: ownAnchor.value)
        } else {
            panel?.toggle(attachment, over: ownAnchor.value)
        }
    }

    var body: some View {
        // The anchor box alone: the hover responder and the anchor view must not keep
        // this row (and its attachment) alive after it scrolls away.
        let anchorBox = ownAnchor
        Button {
            togglePanel()
        } label: {
            if attachment.isImage {
                ZStack {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(.quaternary.opacity(0.6))
                    if let image {
                        Image(decorative: image, scale: 2)
                            .resizable()
                            .interpolation(.high)
                            .aspectRatio(contentMode: .fill)
                            .transition(.opacity)
                    }
                }
                .frame(width: size, height: size)
                .clipShape(.rect(cornerRadius: 12, style: .continuous))
            } else if attachment.isVideo {
                AttachmentVideoThumbnail(attachment: attachment, size: size)
            } else {
                VStack(spacing: 3) {
                    Image(nsImage: FileIcons.icon(for: attachment.path))
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(width: 16, height: 16)
                    Text(attachment.name)
                        .multilineTextAlignment(.center)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
                .font(.chat(.caption, zoom: zoom))
                .padding(.horizontal, 6)
                .frame(width: size, height: size)
                .background(.quaternary.opacity(0.6), in: .rect(cornerRadius: 10, style: .continuous))
                .clipShape(.rect(cornerRadius: 10, style: .continuous))
            }
        }
        .buttonStyle(.plain)
        .focusable(false)
        // The .fill image is drawn wider than the cell before it is clipped;
        // pinning the hit shape to the cell keeps the neighbour's remove badge
        // reachable no matter how hit-testing treats the overflow.
        .contentShape(.rect(cornerRadius: 12, style: .continuous))
        .onHover { [needed = $needsAnchor, anchorless] hovering in
            if hovering, !anchorless, !needed.wrappedValue { needed.wrappedValue = true }
        }
        .background {
            if needsAnchor, !anchorless {
                AttachmentAnchorCapture { [weak anchorBox] in anchorBox?.value = $0 }
            }
        }
        .task(id: attachment.path) {
            guard attachment.isImage else { return }
            if let known = ThumbnailMemory.image(for: attachment.path, pointSize: size) {
                if image !== known { image = known }
                return
            }
            guard let thumbnail = await ThumbnailCache.shared.thumbnail(for: attachment.path, pointSize: size, fillingSquare: true) else { return }
            ThumbnailMemory.store(thumbnail.image, for: attachment.path, pointSize: size)
            withAnimation(.easeOut(duration: 0.15)) { image = thumbnail.image }
        }
    }
}

struct AssistantMessageRow: View {
    @Environment(\.chatZoom) private var zoom
    let entry: TimelineEntry
    /// The turn's summary, set only on the turn's last reply. Precomputed by the
    /// timeline, so rows never scan the thread.
    let summary: TurnSummary?
    let runtime: ThreadRuntime
    @State private var isHovering = false

    var body: some View {
        if case .assistant(let message) = entry.item.content {
            VStack(alignment: .leading, spacing: 2) {
                MarkdownView(text: message.text, isStreaming: message.isStreaming).equatable()
                // One side for both kinds of message: the summary sits on the trailing
                // edge, where the copy control used to be.
                HStack(spacing: 8) {
                    if let summary {
                        Text(TurnEndRow.label(for: summary))
                            .font(.chat(.caption, zoom: zoom))
                            .foregroundStyle(.tertiary)
                    }
                    Spacer(minLength: 8)
                }
                // Pinned to the hover line's room, so the row keeps it (and the gap math
                // below holds) with no copy button left to size it.
                .frame(height: TimelineMetrics.hoverLineHeight)
                .opacity(isHovering && !message.isStreaming ? 1 : 0)
            }
            .onHover { [hovering = $isHovering] isOver in
                withAnimation(.easeOut(duration: 0.12)) { hovering.wrappedValue = isOver }
            }
            .messageMenu(text: message.text, runtime: runtime)
            // Same trade as the user row: the hover line's room stays inside the row
            // (so hovering it works and text never moves), but not in its height, so
            // the controls fill the gap below instead of adding to it. (2 = the
            // VStack's spacing above them.)
            .padding(.bottom, -(TimelineMetrics.hoverLineHeight + 2))
        }
    }

    /// The context menu's items, from value snapshots with weak captures: the AppKit menu
    /// outlives the right-click, so its callbacks must not retain the row.
    static func messageActions(text: String, code: String?, runtime: ThreadRuntime) -> [RowAction] {
        let textSnapshot = text
        let codeSnapshot = code
        var items = [
            RowAction(title: "Copy message", symbol: "doc.on.doc", confirms: "Copied") { copyMessageText(MessageText.plain(textSnapshot)) },
            RowAction(title: "Copy as markdown", symbol: "number", confirms: "Copied") { copyMessageText(textSnapshot) },
        ]
        if let codeSnapshot {
            items.append(RowAction(title: "Copy last code block", symbol: "chevron.left.forwardslash.chevron.right", confirms: "Copied") { [code = codeSnapshot] in
                copyMessageText(code)
            })
        }
        items.append(RowAction(title: "Add to reply", symbol: "arrow.turn.down.left", startsGroup: true, confirms: "Added") { [weak runtime] in
            guard let runtime else { return }
            guard !runtime.draft.quotes.contains(where: { $0.text == textSnapshot }) else { return }
            runtime.draft.quotes.append(ReplyQuote(text: textSnapshot))
        })
        return items
    }
}

extension View {
    /// The right-click popover of a reply, opened at the pointer, wherever its text
    /// is shown: copy, the last code block, quoting it into the chat box.
    /// Snapshots only: the popover builder must not capture the
    /// row, so the popover cannot pin the row's state storage after dismiss.
    func messageMenu(text: String, runtime: ThreadRuntime) -> some View {
        rightClickPopover { [text, runtime] in
            AssistantMessageRow.messageActions(
                text: text,
                code: lastCodeBlock(in: text),
                runtime: runtime
            )
        }
    }
}

private struct FoldedAnswerView: View {
    let text: String
    let runtime: ThreadRuntime

    var body: some View {
        MarkdownView(text: text).equatable()
            .messageMenu(text: text, runtime: runtime)
    }
}

/// Consecutive tool calls, rendered inline with no card: a summary header
/// ("Edited files, ran commands") whose chevron collapses the rows. A group the
/// agent has moved on from (reply text follows it) starts collapsed, so past work
/// reads as one tappable line above the answer; the live group stays expanded.
/// A lone tool renders as its row alone.
struct WorkGroup: View {
    @Environment(\.chatZoom) private var zoom
    let entries: [TimelineEntry]
    let runtime: ThreadRuntime
    var workingDirectory: String?
    var startsCollapsed = false
    @State private var isCollapsed: Bool
    @Environment(\.revealTimelineEnd) private var revealBox

    init(entries: [TimelineEntry], runtime: ThreadRuntime, workingDirectory: String? = nil, startsCollapsed: Bool = false) {
        self.entries = entries
        self.runtime = runtime
        self.workingDirectory = workingDirectory
        self.startsCollapsed = startsCollapsed
        _isCollapsed = State(initialValue: startsCollapsed)
    }

    var body: some View {
        if entries.count == 1, let only = entries.first {
            ToolRow(entry: only, runtime: runtime, workingDirectory: workingDirectory)
        } else {
            VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
                Button {
                    withAnimation(.snappy(duration: 0.24)) {
                        isCollapsed.toggle()
                        if !isCollapsed { revealBox?.action() }
                    }
                } label: {
                    HStack(spacing: TimelineMetrics.iconSpacing) {
                        Image(systemName: WorkGroupSummary.symbol(for: entries))
                            .font(.chat(.caption, zoom: zoom))
                            .foregroundStyle(.secondary)
                            .frame(width: TimelineMetrics.iconWidth)
                        Text(WorkGroupSummary.text(for: entries))
                            .foregroundStyle(.secondary)
                        Image(systemName: "chevron.right")
                            .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                            .foregroundStyle(.tertiary)
                            .rotationEffect(.degrees(isCollapsed ? 0 : 90))
                    }
                    .font(.chat(.callout, zoom: zoom))
                    .padding(.trailing, 12)
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help(isCollapsed ? "Show these steps" : "Hide these steps")
                .accessibilityLabel(Text(isCollapsed ? "Show these steps" : "Hide these steps"))
                if !isCollapsed {
                    WorkSteps(entries: entries, runtime: runtime, workingDirectory: workingDirectory)
                }
            }
            .onChange(of: startsCollapsed) { _, collapsed in
                // One-way: arriving reply text collapses the group, but a group
                // the reader opened never snaps shut on its own.
                if collapsed { isCollapsed = true }
            }
        }
    }
}

/// A tool group's rows: only the two latest steps show, the rest fold under
/// one "N earlier steps" line until tapped. Shared by the expanded work group
/// and the running turn's working line.
struct WorkSteps: View {
    @Environment(\.chatZoom) private var zoom
    let entries: [TimelineEntry]
    let runtime: ThreadRuntime
    var workingDirectory: String?
    @State private var showsAll = false
    @Environment(\.revealTimelineEnd) private var revealBox

    var body: some View {
        let hidden = showsAll ? 0 : max(0, entries.count - 2)
        if hidden > 0 {
            Button {
                withAnimation(.snappy(duration: 0.24)) {
                    showsAll = true
                    revealBox?.action()
                }
            } label: {
                HStack(spacing: TimelineMetrics.iconSpacing) {
                    Image(systemName: "ellipsis")
                        .font(.chat(.caption, zoom: zoom))
                        .foregroundStyle(.secondary)
                        .frame(width: TimelineMetrics.iconWidth)
                    Text(hidden == 1 ? "1 earlier step" : "\(hidden) earlier steps")
                        .foregroundStyle(.secondary)
                }
                .font(.chat(.callout, zoom: zoom))
                .padding(.trailing, 12)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
        }
        ForEach(entries.suffix(entries.count - hidden)) { entry in
            ToolRow(entry: entry, runtime: runtime, workingDirectory: workingDirectory)
        }
    }
}

/// The heads a turn sent out, one tool row each, as a plain list: no header, no
/// chevron, never folded. Sits below the turn's steps and replies while the turn runs
/// and under the folded turn once it is over, so a head never disappears with the steps.
struct HydraHeadsList: View {
    let entries: [TimelineEntry]
    let runtime: ThreadRuntime
    var workingDirectory: String?

    var body: some View {
        // Badges, not lines of text: they stack closer than rows of text do.
        VStack(alignment: .leading, spacing: 8) {
            ForEach(entries) { entry in
                ToolRow(entry: entry, runtime: runtime, workingDirectory: workingDirectory)
            }
        }
    }
}

/// The "Edited files, ran commands" line above a tool group, derived from the
/// calls' kinds rather than their localized titles. First part capitalized, the
/// rest lowered, in a fixed kind order. A kind with a call still running reads
/// in the present tense ("Editing files, ran commands").
///
/// Main-actor isolated like the entries it reads: `TimelineEntry.item` lives on
/// the main actor, so this can only run where view bodies run.
@MainActor
enum WorkGroupSummary {
    private static let order: [ToolCall.Kind] = [.edit, .read, .command, .search, .web, .mcp, .agent, .other]

    static func text(for entries: [TimelineEntry]) -> String {
        // Kind → whether a call of it is still running, in one pass; this runs once a
        // second under the working indicator.
        var running: [ToolCall.Kind: Bool] = [:]
        for entry in entries {
            guard case .tool(let call) = entry.item.content else { continue }
            running[call.kind, default: false] = running[call.kind, default: false] || call.status == .running
        }
        var parts: [String] = []
        for kind in order {
            guard let isRunning = running[kind] else { continue }
            parts.append(label(for: kind, running: isRunning))
        }
        if parts.isEmpty { return "Worked" }
        guard parts.count > 1 else { return parts[0] }
        let rest = parts.dropFirst().map { part -> String in
            guard let head = part.first else { return part }
            return String(head).lowercased() + String(part.dropFirst())
        }
        return ([parts[0]] + rest).joined(separator: ", ")
    }

    static func symbol(for entries: [TimelineEntry]) -> String {
        let kinds = Set(entries.compactMap { entry -> ToolCall.Kind? in
            guard case .tool(let call) = entry.item.content else { return nil }
            return call.kind
        })
        let order: [ToolCall.Kind] = [.edit, .read, .command, .search, .web, .mcp, .agent, .other]
        guard let first = order.first(where: { kinds.contains($0) }) else { return "wrench.and.screwdriver" }
        return ToolPresentation.symbol(for: first)
    }

    private static func label(for kind: ToolCall.Kind, running: Bool) -> String {
        switch kind {
        case .edit: running ? "Editing files" : "Edited files"
        case .read: running ? "Reading files" : "Read files"
        case .command: running ? "Running commands" : "Ran commands"
        case .search: running ? "Searching" : "Searched"
        case .web: running ? "Browsing" : "Browsed"
        case .mcp: running ? "Calling tools" : "Called tools"
        case .agent: running ? "Delegating tasks" : "Delegated tasks"
        case .other: running ? "Using tools" : "Used tools"
        }
    }
}

struct ToolRow: View {
    @Environment(\.chatZoom) private var zoom
    @Environment(AppModel.self) private var model
    let entry: TimelineEntry
    let runtime: ThreadRuntime
    var workingDirectory: String?
    @State private var isExpanded = false
    /// The row's label (icon through chevron), so the changes popover hangs from the
    /// text that was tapped, and the panel that shows an image the agent looked at.
    /// Both are made on first use: a thread holds hundreds of tool rows, and building a
    /// popover and an anchor view for each of them is work the scroll pays for a panel
    /// almost none of them ever opens.
    @State private var preview = AttachmentPreviewSlot()
    /// Whether the pointer has been on the row's label, which is when its anchor view is
    /// mounted. A click always follows a hover, and hovering is off while the timeline
    /// scrolls, so a row scrolling past never mounts one. Once mounted it stays: a popover
    /// is positioned against this view, and taking it away under an open one closes it.
    @State private var needsAnchor = false
    /// Whether the steps popover of the head this row sent out is open.
    @State private var isShowingHead = false
    @Environment(\.revealTimelineEnd) private var revealBox

    var body: some View {
        if case .tool(let call) = entry.item.content {
            // A row that carries a diff never unfolds: the diff opens in the changes
            // popover, below the row's label. A read of an image opens the image itself
            // the same way, whatever text came with it, and so does any other row with an
            // image and nothing else to show. Only a row whose content is its own output
            // (a command's text, a tool's detail) still expands in place.
            let edits = call.edits.filter { !$0.path.isEmpty }
            let opensDiff = !edits.isEmpty
            let imagePath = opensDiff ? nil : PreviewImages.resolveToolImagePath(for: call, workingDirectory: workingDirectory)
            let showsOutput = !opensDiff && (!call.output.isEmpty || !(call.detail ?? "").isEmpty)
            let opensImage = imagePath != nil && (call.kind == .read || !showsOutput)
            let opensPopover = opensDiff || opensImage
            // A row that sent out a head opens the head's steps in a popover of its own,
            // never its raw tool output.
            let headID = call.kind == .agent ? runtime.hydraHead(forTool: entry.id) : nil
            let opensHead = headID != nil
            // A call with no change to show, no image and no output of its own has
            // nothing to open. Rendered as a line of text rather than a control, so the
            // pointer and VoiceOver both treat it as what it is.
            let isTappable = opensPopover || opensHead || showsOutput
            // Values and weak boxes only below: the button action must not keep this
            // row's entry (and its text) alive through menus and hovers.
            let turn = entry.turnID
            let diffEdits = edits
            let slot = preview
            let rt = runtime
            VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
                if isTappable {
                    Button {
                        [weak slot, weak rt, revealBox, presented = $isShowingHead, expanded = $isExpanded] in
                        if opensHead {
                            presented.wrappedValue.toggle()
                        } else if opensDiff {
                            guard let view = slot?.anchor.value else { return }
                            rt?.showDiff(on: view, edge: .minY, turn: turn, focusEdits: diffEdits)
                        } else if opensImage, let imagePath {
                            slot?.panel.toggle(PreviewImages.attachment(for: imagePath), over: slot?.anchor.value, edge: .minY)
                        } else {
                            withAnimation(.snappy(duration: 0.24)) {
                                expanded.wrappedValue.toggle()
                                if expanded.wrappedValue { revealBox?.action() }
                            }
                        }
                    } label: {
                        line(call: call, imagePath: imagePath, opensPopover: opensPopover, opensHead: opensHead, showsOutput: showsOutput)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .help(helpText(opensDiff: opensDiff, opensImage: opensImage, opensHead: opensHead, showsOutput: showsOutput))
                } else {
                    line(call: call, imagePath: imagePath, opensPopover: opensPopover, opensHead: opensHead, showsOutput: showsOutput)
                }
                if isExpanded, showsOutput, !opensImage {
                    ToolDetailView(call: call, workingDirectory: workingDirectory)
                        .padding(.trailing, 12)
                }
            }
            .onDisappear { [weak slot] in slot?.close() }
        }
    }

    /// The head a delegation call is sending out, read from the call itself before its
    /// thread is linked: a roster name leading the title ("Pip: the merge popover") names
    /// the face; otherwise the team's first face stands in and the row speaks of "a head".
    /// The task is the title past the name.
    private static func departingHead(for call: ToolCall) -> (persona: HydraPersona, name: String?, task: String) {
        let title = call.title.trimmingCharacters(in: .whitespacesAndNewlines)
        // "Subagent" and the like are the tool's own word for itself, not a task.
        let generic: Set<String> = ["subagent", "agent", "task", ""]
        if let colon = title.firstIndex(of: ":") {
            let name = String(title[..<colon]).trimmingCharacters(in: .whitespacesAndNewlines)
            if let index = HydraRoster.index(named: name) {
                let task = String(title[title.index(after: colon)...]).trimmingCharacters(in: .whitespacesAndNewlines)
                return (HydraRoster.persona(at: index), HydraRoster.persona(at: index).name, task)
            }
        }
        return (HydraRoster.persona(at: 0), nil, generic.contains(title.lowercased()) ? "" : title)
    }

    /// What the departing row says: on its way, out, or never sent.
    private static func departureLabel(for call: ToolCall, name: String?, task: String) -> String {
        let who = name ?? "a head"
        let rest = task.isEmpty ? "" : ": \(task)"
        switch call.status {
        case .running: return "Sending out \(who)\(rest)"
        case .failed: return "Could not send out \(who)\(rest)"
        default: return "Sent out \(who)\(rest)"
        }
    }

    /// The glyph's badge for a departing row: nothing on its way, the outcome once known.
    private static func departureStatus(for call: ToolCall) -> HydraHeadInfo.Status? {
        switch call.status {
        case .running: return nil
        case .failed: return .failed
        default: return .completed
        }
    }

    /// The row itself: icon, text, stats and chevron, laid out the same whether or not
    /// the line can be tapped.
    @ViewBuilder
    private func line(call: ToolCall, imagePath: String?, opensPopover: Bool, opensHead: Bool, showsOutput: Bool) -> some View {
        // The slot alone: the hover responder and the anchor view must not keep this
        // row's entry (and its text) alive after it scrolls away.
        let previewSlot = preview
        // A row that sent out a head wears the head's glyph and name, in the badge the
        // finished turn's card and the working line wear, rather than as a line of text.
        let head = call.kind == .agent ? runtime.hydraHead(forTool: entry.id).flatMap { model.thread($0)?.hydra } : nil
        // Before the head's thread is linked to the call (the moment it goes out, or a
        // call that failed to send one), the row still wears a head's glyph and speaks of
        // sending it out: the name from the call's own title when it leads with one from
        // the roster, else the team's first face. Never a spinner and "Delegating Subagent".
        let departing: (persona: HydraPersona, name: String?, task: String)? = call.kind == .agent && head == nil ? Self.departingHead(for: call) : nil
        let isBadge = head != nil || departing != nil
        HStack(spacing: TimelineMetrics.iconSpacing) {
            HStack(spacing: isBadge ? 8 : TimelineMetrics.iconSpacing) {
                if let head {
                    HydraGlyph(persona: head.persona, size: 18, isRunning: call.status == .running && head.status == .running, status: head.status)
                } else if let departing {
                    HydraGlyph(persona: departing.persona, size: 18, isRunning: call.status == .running, status: Self.departureStatus(for: call))
                        .transition(.scale(scale: 0.6).combined(with: .opacity))
                } else {
                    ToolStatusIcon(call: call, symbol: imagePath != nil && call.kind == .read ? "photo" : nil)
                }
                // One text run after the icon, so the row reads as
                // icon + space + text instead of three spaced items.
                Text(head.map { "\(call.status == .running ? "Sending out" : "Sent out") \($0.persona.name): \(call.title)" }
                    ?? departing.map { Self.departureLabel(for: call, name: $0.name, task: $0.task) }
                    ?? ToolPresentation.label(for: call))
                    .font(isBadge ? .chat(.callout, weight: .medium, zoom: zoom) : .chat(.callout, zoom: zoom))
                    .foregroundStyle(isBadge ? AnyShapeStyle(Chrome.primaryText.opacity(0.9)) : AnyShapeStyle(.secondary))
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let stats = ToolPresentation.stats(for: call) {
                    DiffStatLabel(additions: stats.additions, deletions: stats.deletions)
                }
                // Beside the subject, not out at the trailing edge: the chevron
                // belongs to the row's own text.
                if opensPopover || opensHead {
                    Image(systemName: "chevron.down")
                        .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                        .foregroundStyle(.tertiary)
                } else if showsOutput {
                    Image(systemName: "chevron.right")
                        .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                }
            }
            .padding(.leading, isBadge ? 12 : 0)
            .padding(.trailing, isBadge ? 10 : 0)
            .padding(.vertical, isBadge ? 6 : 0)
            .background {
                if isBadge {
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(.quaternary.opacity(0.32))
                }
            }
            .onHover { [needed = $needsAnchor] hovering in
                if hovering, !needed.wrappedValue { needed.wrappedValue = true }
            }
            .background {
                if opensPopover, needsAnchor {
                    AttachmentAnchorCapture { [weak previewSlot] in previewSlot?.setAnchor($0) }
                }
            }
            // On the text run (the badge), not the full-width row: hung from the row, the
            // popover's arrow pointed at the row's middle, well to the right of the words.
            .modifier(HeadStepsPopoverModifier(headID: opensHead ? runtime.hydraHead(forTool: entry.id) : nil, isPresented: $isShowingHead))
            Spacer(minLength: 8)
            if call.status == .failed, let exitCode = call.exitCode {
                Text("exit \(exitCode)")
                    .font(.chat(.caption, zoom: zoom).monospacedDigit())
                    .foregroundStyle(Chrome.danger)
            } else if call.status == .declined {
                Text("Declined")
                    .font(.chat(.caption, zoom: zoom))
                    .foregroundStyle(Chrome.warning)
            }
        }
        .font(.chat(.callout, zoom: zoom))
        .padding(.trailing, 12)
    }

    private func helpText(opensDiff: Bool, opensImage: Bool, opensHead: Bool, showsOutput: Bool) -> String {
        if opensHead { return "Show the head's steps" }
        if opensDiff { return "Show this change" }
        if opensImage { return "Show the image" }
        guard showsOutput else { return "" }
        return isExpanded ? "Hide the output" : "Show the output"
    }
}

/// Hangs the head's steps popover from a row that sent out a head. A row that did not
/// mounts nothing: the popover, its state and its anchor exist only where they can open.
private struct HeadStepsPopoverModifier: ViewModifier {
    let headID: UUID?
    @Binding var isPresented: Bool

    func body(content: Content) -> some View {
        if let headID {
            content.popover(isPresented: $isPresented, arrowEdge: .bottom) {
                HydraHeadStepsPopover(headID: headID)
                    .presentedChrome()
            }
        } else {
            content
        }
    }
}

/// What a head did, in plain text: its brief, then every step it took, then its report.
/// Reads the head's own runtime straight from the model, so the list grows while the
/// head still works.
struct HydraHeadStepsPopover: View {
    @Environment(\.chatZoom) private var zoom
    let headID: UUID
    @Environment(AppModel.self) private var model

    private static let width: CGFloat = 440
    private static let maxHeight: CGFloat = 520

    var body: some View {
        let hydra = model.thread(headID)?.hydra
        let steps = Self.steps(in: model.runtime(for: headID).entries)
        let report = hydra.flatMap { head -> String? in
            guard head.isFinished else { return nil }
            let text = (head.summary ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return text.isEmpty ? nil : text
        }
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let hydra {
                    header(hydra)
                    if !hydra.task.isEmpty {
                        Text(verbatim: hydra.task)
                            .font(.chat(.callout, zoom: zoom))
                            .foregroundStyle(Chrome.primaryText)
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
                    sectionTitle("Steps")
                    if steps.isEmpty {
                        Text("No steps yet.")
                            .font(.chat(.callout, zoom: zoom))
                            .foregroundStyle(Chrome.secondaryText)
                    } else {
                        ForEach(steps) { entry in
                            step(entry)
                        }
                    }
                }
                if let report {
                    VStack(alignment: .leading, spacing: 8) {
                        sectionTitle("Report")
                        Text(verbatim: report)
                            .font(.chat(.callout, zoom: zoom))
                            .foregroundStyle(Chrome.primaryText)
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .padding(16)
            .frame(width: Self.width, alignment: .leading)
        }
        .frame(width: Self.width)
        .frame(maxHeight: Self.maxHeight)
    }

    /// The head's glyph and name, with where it stands on the trailing side.
    private func header(_ hydra: HydraHeadInfo) -> some View {
        HStack(alignment: .center, spacing: TimelineMetrics.iconSpacing) {
            HydraGlyph(persona: hydra.persona, size: 22, isRunning: hydra.status == .running, status: hydra.status)
            Text(verbatim: hydra.persona.name)
                .font(.chat(.headline, zoom: zoom))
                .foregroundStyle(Chrome.primaryText)
            Spacer(minLength: 12)
            Text(verbatim: Self.statusLine(for: hydra))
                .font(.chat(.caption, zoom: zoom))
                .foregroundStyle(Chrome.secondaryText)
                .lineLimit(1)
        }
    }

    private func sectionTitle(_ title: String) -> some View {
        Text(verbatim: title)
            .font(.chat(.caption, weight: .semibold, zoom: zoom))
            .foregroundStyle(Chrome.secondaryText)
    }

    /// One step, laid out like the timeline's own rows: the icon column, then the text.
    @ViewBuilder
    private func step(_ entry: TimelineEntry) -> some View {
        switch entry.item.content {
        case .tool(let call):
            HStack(alignment: .firstTextBaseline, spacing: TimelineMetrics.iconSpacing) {
                ToolStatusIcon(call: call)
                    .frame(width: TimelineMetrics.iconWidth)
                Text(verbatim: ToolPresentation.label(for: call))
                    .font(.chat(.callout, zoom: zoom))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .truncationMode(.middle)
                if let stats = ToolPresentation.stats(for: call) {
                    DiffStatLabel(additions: stats.additions, deletions: stats.deletions)
                }
            }
        case .assistant(let message):
            Text(verbatim: message.text)
                .font(.chat(.callout, zoom: zoom))
                .foregroundStyle(Chrome.primaryText)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        case .user(let message):
            // A later brief: the lead steering the head on.
            Text(verbatim: message.text)
                .font(.chat(.callout, zoom: zoom))
                .foregroundStyle(Chrome.secondaryText)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        case .notice(let notice):
            HStack(alignment: .firstTextBaseline, spacing: TimelineMetrics.iconSpacing) {
                Image(systemName: Self.noticeSymbol(for: notice.level))
                    .font(.chat(.caption, zoom: zoom))
                    .foregroundStyle(Self.noticeColor(for: notice.level))
                    .frame(width: TimelineMetrics.iconWidth)
                Text(verbatim: notice.message)
                    .font(.chat(.callout, zoom: zoom))
                    .foregroundStyle(Self.noticeColor(for: notice.level))
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
        default:
            EmptyView()
        }
    }

    /// "Working…" or the outcome, then the tool count and, once the head is done, how
    /// long it took.
    private static func statusLine(for hydra: HydraHeadInfo) -> String {
        let outcome: String = switch hydra.status {
        case .running: "Working…"
        case .completed: "Done"
        case .failed: "Failed"
        case .stopped: "Stopped"
        }
        var parts = [outcome]
        if hydra.toolCalls > 0 {
            parts.append(hydra.toolCalls == 1 ? "1 tool" : "\(hydra.toolCalls) tools")
        }
        if let finishedAt = hydra.finishedAt {
            parts.append(RelativeTime.duration(finishedAt.timeIntervalSince(hydra.startedAt)))
        }
        return parts.joined(separator: " · ")
    }

    /// The entries worth a line: every tool call, reply and notice, and any brief after
    /// the first. The first user entry is the task itself, shown above; thinking, plans,
    /// todo lists and turn ends are the head's own bookkeeping.
    private static func steps(in entries: [TimelineEntry]) -> [TimelineEntry] {
        var sawBrief = false
        return entries.filter { entry in
            switch entry.item.content {
            case .user(let message):
                if !sawBrief {
                    sawBrief = true
                    return false
                }
                return !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            case .assistant(let message):
                return !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            case .tool, .notice:
                return true
            case .reasoning, .turnEnd, .plan, .todos:
                return false
            }
        }
    }

    private static func noticeSymbol(for level: Notice.Level) -> String {
        switch level {
        case .info: "info.circle"
        case .warning: "exclamationmark.triangle"
        case .error: "exclamationmark.octagon"
        }
    }

    private static func noticeColor(for level: Notice.Level) -> Color {
        switch level {
        case .info: .secondary
        case .warning: Chrome.warning
        case .error: Chrome.danger
        }
    }
}

private struct ToolStatusIcon: View {
    @Environment(\.chatZoom) private var zoom
    let call: ToolCall
    /// Stands in for the kind's symbol: a photo for a read that looked at one.
    var symbol: String?

    var body: some View {
        Group {
            switch call.status {
            case .running:
                ProgressView()
                    .controlSize(.mini)
            case .failed:
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(Chrome.danger)
            case .declined:
                Image(systemName: "hand.raised.slash")
                    .foregroundStyle(Chrome.warning)
            case .completed:
                Image(systemName: symbol ?? ToolPresentation.symbol(for: call.kind))
                    .foregroundStyle(.secondary)
            }
        }
        .font(.chat(.caption, zoom: zoom))
        .frame(width: TimelineMetrics.iconWidth)
    }
}

private struct ToolDetailView: View {
    @Environment(\.chatZoom) private var zoom
    let call: ToolCall
    var workingDirectory: String?

    @State private var showsFullOutput = false

    /// Tool output renders inline and collapsed: a nested scroll view inside the
    /// timeline's scroll view fights gestures and hitches, and materializing tens of
    /// thousands of characters at once is what made expanding feel laggy.
    private static let outputLineLimit = 30

    /// The first `lines` lines of `text`, without splitting the rest.
    private static func head(of text: String, lines: Int) -> String {
        var remaining = lines
        for index in text.utf8.indices where text.utf8[index] == 0x0A {
            remaining -= 1
            if remaining == 0 { return String(text[..<index]) }
        }
        return text
    }

    var body: some View {
        VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
            if let imagePath = PreviewImages.resolveToolImagePath(for: call, workingDirectory: workingDirectory) {
                ToolImagePreview(path: imagePath)
            }
            if let detail = call.detail, !detail.isEmpty {
                Text(detail)
                    .font(.chat(.caption, design: .monospaced, zoom: zoom))
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .lineLimit(12)
            }
            ForEach(Array(call.edits.enumerated()), id: \.offset) { _, edit in
                if let diff = edit.diff, !diff.isEmpty {
                    VStack(alignment: .leading, spacing: 0) {
                        Text(edit.path)
                            .font(.chat(.caption, zoom: zoom))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                        DiffLinesView(file: DiffDetailCache.file(diff: diff, path: edit.path), showsLineNumbers: false)
                    }
                    .background(.quaternary.opacity(0.45), in: .rect(cornerRadius: 10, style: .continuous))
                    .clipShape(.rect(cornerRadius: 10, style: .continuous))
                }
            }
            if !call.output.isEmpty {
                // A running command's row re-renders on every flush of its output: the
                // preview scans for its thirty-first newline rather than splitting the
                // whole output into lines, as `CodeBlock` does.
                let output = call.output.trimmingCharacters(in: .newlines)
                let lineCount = 1 + output.utf8.count { $0 == 0x0A }
                let collapsed = !showsFullOutput && lineCount > Self.outputLineLimit
                let visible = collapsed ? Self.head(of: output, lines: Self.outputLineLimit) : output
                VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
                    Text(visible)
                        .font(.chat(.caption, design: .monospaced, zoom: zoom))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                        .background(.quaternary.opacity(0.45), in: .rect(cornerRadius: 10, style: .continuous))
                    if lineCount > Self.outputLineLimit {
                        Button(showsFullOutput ? "Show less" : "Show full output (\(lineCount) lines)") {
                            showsFullOutput.toggle()
                        }
                        .buttonStyle(.link)
                        .font(.chat(.caption, zoom: zoom))
                    }
                }
            }
        }
    }
}

/// Parsed per-file diffs for expanded tool rows. Parsing runs once per distinct diff,
/// so opening and closing a row is instant no matter how large the patch is.
private enum DiffDetailCache {
    @MainActor private static var cache = RecentCache<String, DiffFile>(limit: 60)

    @MainActor
    static func file(diff: String, path: String) -> DiffFile {
        let key = path + "\n" + diff
        if let hit = cache.value(for: key) { return hit }
        let parsed = DiffParser.parseHunks(diff, path: path)
        cache.insert(parsed, for: key)
        return parsed
    }
}

enum ToolPresentation {
    /// The row's text run: verb plus subject. Agents that title a call with the
    /// tool's own name ("Edit file", "Write file") would read "Edited Edit file" or
    /// "Editing Write file" — a leading word that only restates the kind goes, so it
    /// reads "Edited file".
    static func label(for call: ToolCall) -> String {
        let roots: [String] = switch call.kind {
        case .command: ["run", "bash", "shell", "exec", "execute", "command"]
        case .read: ["read", "view", "open", "cat"]
        case .edit: ["edit", "write", "multiedit", "create", "notebookedit", "apply_patch", "patch"]
        case .search: ["search", "grep", "glob", "find"]
        case .web: ["fetch", "webfetch", "websearch", "browse"]
        case .mcp: ["call"]
        case .agent: ["delegate", "agent", "task"]
        case .other: ["use"]
        }
        var subject = call.title
        let words = call.title.split(separator: " ", maxSplits: 1)
        if let first = words.first?.lowercased(), roots.contains(first) {
            subject = words.count > 1 ? String(words[1]) : ""
        }
        return [verb(for: call), subject].filter { !$0.isEmpty }.joined(separator: " ")
    }

    static func verb(for call: ToolCall) -> String {
        let running = call.status == .running
        return switch call.kind {
        case .command: running ? "Running" : "Ran"
        case .read: running ? "Reading" : "Read"
        case .edit: running ? "Editing" : "Edited"
        case .search: running ? "Searching" : "Searched"
        case .web: running ? "Browsing" : "Browsed"
        case .mcp: running ? "Calling" : "Called"
        case .agent: running ? "Delegating" : "Delegated"
        case .other: running ? "Using" : "Used"
        }
    }

    static func symbol(for kind: ToolCall.Kind) -> String {
        switch kind {
        case .command: "terminal"
        case .read: "doc.text"
        case .edit: "pencil"
        case .search: "magnifyingglass"
        case .web: "globe"
        case .mcp: "puzzlepiece.extension"
        case .agent: "person.2"
        case .other: "wrench.and.screwdriver"
        }
    }

    static func stats(for call: ToolCall) -> (additions: Int, deletions: Int)? {
        guard !call.edits.isEmpty else { return nil }
        let additions = call.edits.reduce(0) { $0 + $1.additions }
        let deletions = call.edits.reduce(0) { $0 + $1.deletions }
        return additions + deletions > 0 ? (additions, deletions) : nil
    }
}

struct PlanCard: View {
    @Environment(\.chatZoom) private var zoom
    let entry: TimelineEntry
    let runtime: ThreadRuntime

    @State private var isShowingPlan = false

    var body: some View {
        if case .plan(let plan) = entry.item.content {
            ChatBadge(
                title: Self.title(for: plan.state),
                caption: Self.caption(for: plan.state),
                showsChevron: !plan.markdown.isEmpty,
                needsAttention: Self.needsAttention(for: plan.state),
                isEnabled: !plan.markdown.isEmpty,
                isPresented: $isShowingPlan,
                glyph: {
                    if plan.state == .drafting {
                        WorkingSpinner(cellSize: 3).frame(width: 14)
                    } else {
                        Image(systemName: "list.bullet.clipboard").foregroundStyle(.tint)
                    }
                },
                detail: {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack {
                            Text("Plan").font(.chat(.headline, zoom: zoom))
                            Spacer()
                            CopyButton(text: plan.markdown)
                        }
                        ScrollView {
                            MarkdownView(text: plan.markdown, isStreaming: plan.state == .drafting).equatable()
                                .padding(.horizontal, 16)
                        }
                        .scrollBounceBehavior(.basedOnSize)
                        // An ideal height as well as the cap: a popover sizes to its content's
                        // ideal, and a scroll view offered nothing collapses (see HydraReportPopover).
                        .frame(idealHeight: 320, maxHeight: 420)
                        if plan.state == .proposed {
                            // The controls stay where they are while the thread is busy, greyed
                            // rather than gone: a plan whose buttons vanish mid-turn reads as a
                            // plan that has already been dealt with.
                            let isBusy = runtime.pendingPlanApproval != nil || runtime.isRunning
                            HStack(spacing: 8) {
                                Button("Implement plan") {
                                    isShowingPlan = false
                                    runtime.implementPlan(entry.id)
                                }
                                .buttonStyle(.glassProminent)
                                Button("Dismiss") {
                                    isShowingPlan = false
                                    runtime.dismissPlan(entry.id)
                                }
                                .buttonStyle(.glass)
                            }
                            .disabled(isBusy)
                            .help(isBusy ? "Wait for the current turn to finish" : "")
                        }
                    }
                    .padding(16)
                    .frame(width: 520)
                }
            )
        }
    }

    private static func title(for state: ProposedPlan.State) -> String {
        switch state {
        case .drafting: "Writing a plan"
        case .proposed: "Plan ready"
        case .accepted, .dismissed: "Plan"
        }
    }

    private static func caption(for state: ProposedPlan.State) -> String? {
        switch state {
        case .drafting: nil
        case .proposed: "Implement or dismiss"
        case .accepted: "Approved"
        case .dismissed: "Dismissed"
        }
    }

    private static func needsAttention(for state: ProposedPlan.State) -> Bool {
        state == .proposed
    }
}

struct TodoListRow: View {
    @Environment(\.chatZoom) private var zoom
    let entry: TimelineEntry

    var body: some View {
        if case .todos(let steps) = entry.item.content, !steps.isEmpty {
            VStack(alignment: .leading, spacing: 7) {
                HStack(spacing: 6) {
                    Image(systemName: "checklist")
                    Text("\(steps.count { $0.status == .done }) of \(steps.count) done")
                }
                .font(.chat(.caption, zoom: zoom))
                .foregroundStyle(.secondary)
                ForEach(Array(steps.enumerated()), id: \.offset) { _, step in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Image(systemName: Self.symbol(for: step.status))
                            .foregroundStyle(step.status == .pending ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.tint))
                        Text(step.text)
                            .foregroundStyle(step.status == .done ? .secondary : .primary)
                            .strikethrough(step.status == .done, color: .secondary)
                    }
                    .font(.chat(.callout, zoom: zoom))
                }
            }
            .padding(14)
            .background(.quaternary.opacity(0.3), in: .rect(cornerRadius: 14, style: .continuous))
            // The card hugs its longest step, like the badges: the row takes the
            // width (less the badges' trailing room) only to lead-align the card and
            // give long steps a line to wrap at.
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.trailing, 96)
            // One checklist per turn: updates rewrite these steps in place, so
            // checking an item off animates the same list instead of adding a row.
            .animation(.snappy(duration: 0.2), value: steps)
        }
    }

    private static func symbol(for status: TodoStep.Status) -> String {
        switch status {
        case .pending: "circle"
        case .active: "circle.dotted.circle"
        case .done: "checkmark.circle.fill"
        }
    }
}

struct NoticeRow: View {
    @Environment(\.chatZoom) private var zoom
    let entry: TimelineEntry

    var body: some View {
        if case .notice(let notice) = entry.item.content {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: Self.symbol(for: notice.level))
                    .foregroundStyle(Self.color(for: notice.level))
                Text(notice.message)
                    .textSelection(.enabled)
                    .foregroundStyle(notice.level == .info ? .secondary : .primary)
            }
            .font(.chat(.callout, zoom: zoom))
            .padding(.horizontal, notice.level == .info ? 0 : 12)
            .padding(.vertical, notice.level == .info ? 0 : 9)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Self.background(for: notice.level), in: .rect(cornerRadius: 12, style: .continuous))
        }
    }

    private static func symbol(for level: Notice.Level) -> String {
        switch level {
        case .info: "info.circle"
        case .warning: "exclamationmark.triangle"
        case .error: "exclamationmark.octagon"
        }
    }

    /// The theme's own status colours, like every other warning and failure in the app,
    /// rather than the system orange and red.
    private static func color(for level: Notice.Level) -> Color {
        switch level {
        case .info: .secondary
        case .warning: Chrome.warning
        case .error: Chrome.danger
        }
    }

    private static func background(for level: Notice.Level) -> Color {
        switch level {
        case .info: .clear
        case .warning: Chrome.warning.opacity(0.1)
        case .error: Chrome.danger.opacity(0.1)
        }
    }
}

struct TurnEndRow: View {
    @Environment(\.chatZoom) private var zoom
    let summary: TurnSummary
    /// Whether the turn produced a reply. Set by the timeline; when true the duration
    /// lives on the reply's hover line instead.
    let hasReply: Bool

    var body: some View {
        if !hasReply {
            Text(Self.label(for: summary))
                .font(.chat(.caption, zoom: zoom))
                .foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    static func label(for summary: TurnSummary) -> String {
        let duration = RelativeTime.duration(summary.duration)
        return switch summary.status {
        case .completed, .running: "Worked for \(duration)"
        case .interrupted: "Stopped after \(duration)"
        case .failed: "Failed after \(duration)"
        }
    }
}

/// A finished turn, collapsed to nothing more than its header, its final response
/// and its file summary. The chevron re-opens the turn's full steps.
/// The quiet round control on a finished turn's line (revert, changes): a flat overlay
/// disc with a secondary glyph that comes up to full under the pointer. Solid accent
/// discs here shouted from every turn of every chat.
private struct MutedRoundGlyph: View {
    let symbol: String
    @State private var isHovering = false

    var body: some View {
        Image(systemName: symbol)
            .font(Chrome.iconFont)
            .foregroundStyle(isHovering ? Chrome.primaryText : Chrome.secondaryText)
            .frame(width: 28, height: 28)
            .background(Chrome.overlay(isHovering ? 0.18 : 0.1), in: .circle)
            .contentShape(.circle)
            .onHover { hovering in
                withAnimation(Chrome.hover) { isHovering = hovering }
            }
    }
}

struct TurnFinishedBlock: View {
    @Environment(\.chatZoom) private var zoom
    let runtime: ThreadRuntime
    let turnID: UUID
    let summary: TurnSummary
    let userEntries: [TimelineEntry]
    /// Everything in the turn except the user message, the turn-end marker and
    /// reasoning. Pre-partitioned by the timeline, so rows never scan the thread.
    let content: [TimelineEntry]
    let workingDirectory: String?
    /// Whether the turn can be reverted right now. Decided by the timeline, which reads
    /// the provider and the turn list once for every row.
    let canUndo: Bool

    @State private var isExpanded = false
    @Environment(\.revealTimelineEnd) private var revealBox
    /// What the body last derived from the turn, keyed on its entries' identities and the
    /// fold: an evaluation with the same key reads it back instead of walking the turn
    /// again. Written after the body (see the `onChange` below), never in it.
    @State private var derived: (key: DerivedKey, value: Derived)?

    /// What `Derived` depends on: which entries the turn holds (each a class, so identity
    /// is enough) and whether the full steps are showing.
    private struct DerivedKey: Equatable {
        var ids: [ObjectIdentifier]
        var expanded: Bool
    }

    @State private var legacyChanges: FileChangeSummary?

    /// Everything the body derives from the turn's content, gathered in one pass per
    /// render instead of a filter per use.
    private struct Derived {
        /// Every reply with something to read, narration included, in order.
        var assistantEntries: [TimelineEntry] = []
        /// The turn's conclusive answer: the replies that follow its last tool call.
        /// Prose between tool calls narrates work the collapsed turn doesn't show, and
        /// stays with the steps behind the chevron. A turn that ended on a tool (stopped
        /// or failed mid-work) keeps the last words it said rather than showing nothing.
        var answerEntries: [TimelineEntry] = []
        var collapsedPlans: [TimelineEntry] = []
        /// Errors and warnings stay visible even when collapsed, so a failed turn
        /// never hides what went wrong. Plain info notices stay in the expanded view.
        var collapsedNotices: [TimelineEntry] = []
        var hasEdits = false
        var hasResponse = false
        /// The heads the turn sent out, always shown below the final response, folded or not.
        var headEntries: [TimelineEntry] = []
        /// The turn's full steps, built only while they are showing. The heads are not
        /// among them: they have their list of their own below the response.
        var detailGroups: [TimelineGroup] = []

        @MainActor
        init(content: [TimelineEntry], expanded: Bool) {
            for entry in content {
                switch entry.kind {
                case .assistant:
                    // A message still streaming when its turn failed can be blank. Asked
                    // character by character, which stops at the first word: trimming made
                    // a second copy of every reply in the turn on every render of the block.
                    guard case .assistant(let message) = entry.item.content,
                          message.text.contains(where: { !$0.isWhitespace }) else { continue }
                    assistantEntries.append(entry)
                    answerEntries.append(entry)
                    hasResponse = true
                case .plan:
                    collapsedPlans.append(entry)
                case .notice:
                    if case .notice(let notice) = entry.item.content, notice.level != .info {
                        collapsedNotices.append(entry)
                    }
                case .tool:
                    // Reaching for a tool ends the answer so far: whatever the model says
                    // next is about the work, until it stops calling tools and concludes.
                    // Plans, notices and checklists don't interrupt it.
                    answerEntries.removeAll(keepingCapacity: true)
                    guard case .tool(let call) = entry.item.content else { continue }
                    if call.kind == .agent { headEntries.append(entry) }
                    if !call.edits.isEmpty { hasEdits = true }
                default:
                    break
                }
            }
            if answerEntries.isEmpty, let last = assistantEntries.last { answerEntries = [last] }
            if !collapsedPlans.isEmpty || !collapsedNotices.isEmpty { hasResponse = true }
            if expanded {
                detailGroups = TimelineGroup.build(content, showReasoning: false).filter {
                    if case .heads = $0 { return false }
                    return true
                }
            }
        }
    }

    var body: some View {
        let key = DerivedKey(ids: content.map(ObjectIdentifier.init), expanded: isExpanded)
        let derived = (self.derived?.key == key ? self.derived?.value : nil) ?? Derived(content: content, expanded: isExpanded)
        let showsHeads = !derived.headEntries.isEmpty
        // The files as the turn's own diff saw them, or as an older turn's were worked out
        // once (see the task below).
        let changes = summary.changes ?? legacyChanges
        let hasFiles = changes?.files.isEmpty == false
        let showsBody = showsHeads || hasFiles || (isExpanded ? !derived.detailGroups.isEmpty : derived.hasResponse)
        VStack(alignment: .leading, spacing: 0) {
            ForEach(userEntries) { entry in
                UserMessageRow(entry: entry, runtime: runtime, canRevert: canUndo)
                    .padding(.bottom, TimelineMetrics.rowSpacing)
            }
            Button {
                withAnimation(.snappy(duration: 0.24)) {
                    isExpanded.toggle()
                    if isExpanded { revealBox?.action() }
                }
            } label: {
                HStack(spacing: 6) {
                    Text(TurnEndRow.label(for: summary))
                        .font(.chat(.callout, zoom: zoom))
                        .foregroundStyle(.secondary)
                    Image(systemName: "chevron.right")
                        .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .help(isExpanded ? "Hide this turn's steps" : "Show this turn's steps")
            .accessibilityLabel(Text(isExpanded ? "Hide this turn's steps" : "Show this turn's steps"))

            if showsBody {
                Divider()
                    .opacity(0.6)
                    // Half a row gap on each side, so the header and the body sit one
                    // row gap apart with the rule in between.
                    .padding(.vertical, TimelineMetrics.rowSpacing / 2)

                if isExpanded {
                VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
                    ForEach(derived.detailGroups) { group in
                        switch group {
                        case .single(let entry):
                            switch entry.kind {
                            case .assistant:
                                AssistantMessageRow(entry: entry, summary: nil, runtime: runtime)
                            case .tool:
                                WorkGroup(entries: [entry], runtime: runtime, workingDirectory: workingDirectory)
                            case .plan:
                                PlanCard(entry: entry, runtime: runtime)
                            case .todos:
                                TodoListRow(entry: entry)
                            case .notice:
                                NoticeRow(entry: entry)
                            case .user, .reasoning, .turnEnd:
                                EmptyView()
                            }
                        case .work(_, let entries, let startsCollapsed):
                            WorkGroup(entries: entries, runtime: runtime, workingDirectory: workingDirectory, startsCollapsed: startsCollapsed)
                        case .heads:
                            // Never among the steps: the heads list below shows them.
                            EmptyView()
                        }
                    }
                }
                .padding(.bottom, derived.hasResponse || showsHeads ? TimelineMetrics.rowSpacing : 0)
            } else if derived.hasResponse {
                VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
                    ForEach(derived.answerEntries) { entry in
                        if case .assistant(let message) = entry.item.content, !message.text.isEmpty {
                            // The answer of a folded turn: the same menu as a reply row.
                            FoldedAnswerView(text: message.text, runtime: runtime)
                        }
                    }
                    ForEach(derived.collapsedPlans) { entry in
                        PlanCard(entry: entry, runtime: runtime)
                    }
                    ForEach(derived.collapsedNotices) { entry in
                        NoticeRow(entry: entry)
                    }
                }
                .padding(.bottom, TimelineMetrics.rowSpacing)
            }

            // The heads the turn sent out stay in view under the response, folded or not.
            if showsHeads {
                HydraHeadsList(entries: derived.headEntries, runtime: runtime, workingDirectory: workingDirectory)
                    .padding(.bottom, hasFiles ? TimelineMetrics.rowSpacing : 0)
            }

            if let changes, !changes.files.isEmpty {
                // Weak runtime: the card (and its review closure) must not keep the thread
                // alive after its turn scrolls away.
                let reviewRuntime = runtime
                TurnFileCard(
                    changes: changes,
                    canUndo: canUndo,
                    onRevert: { Task { _ = try? await runtime.revert(to: turnID, restoreFiles: true) } },
                    // On the button itself, so the changes open where the reader
                    // clicked, whatever is above the chat box.
                    onReview: { [weak reviewRuntime] button in reviewRuntime?.showDiff(on: button, turn: turnID) }
                )
            }
            }
        }
        // The derivation the body just made, kept for the evaluations until the key moves.
        .onChange(of: key, initial: true) { _, _ in
            if self.derived?.key != key { self.derived = (key, derived) }
        }
        .task(id: turnID) {
            guard summary.changes == nil, summary.filesChanged > 0 || derived.hasEdits else { return }
            let files = await runtime.parsedDiff(selection: turnID)
            guard !Task.isCancelled else { return }
            let changes = FileChangeSummary(files: files)
            legacyChanges = changes
            runtime.storeLegacyChanges(changes, turnID: turnID)
        }
    }
}

private struct TurnFileCard: View {
    @Environment(\.chatZoom) private var zoom
    /// The turn's files, header totals included: one list, so the count in the title and
    /// the rows behind it can never disagree.
    let changes: FileChangeSummary
    let canUndo: Bool
    /// Runs the revert; the card confirms first.
    let onRevert: () -> Void
    /// Receives the Review button's own view, for the popover to open on.
    let onReview: (NSView) -> Void
    /// The Review button's own view, captured once the pointer has been on it: a folded
    /// turn is a block of the lazy stack, and the anchor is only ever for a click, which
    /// has to hover the button first.
    @State private var reviewAnchor = WeakView()
    @State private var needsReviewAnchor = false
    @State private var isShowingFiles = false
    @State private var isConfirmingRevert = false

    var body: some View {
        // The anchor box alone: the hover responder and the anchor view must not keep
        // this card alive after it scrolls away.
        let reviewBox = reviewAnchor
        let files = changes.files
        let title = files.count == 1 ? "Edited 1 file" : "Edited \(files.count) files"
        HStack(spacing: 8) {
            Button { isShowingFiles.toggle() } label: {
                HStack(spacing: 8) {
                    Image("badge-edit")
                        .renderingMode(.template)
                        .resizable()
                        .frame(width: 18, height: 18)
                        .foregroundStyle(Chrome.secondaryText)
                    Text(verbatim: title)
                        .font(.chat(.callout, weight: .medium, zoom: zoom))
                        .foregroundStyle(Chrome.primaryText.opacity(0.9))
                    DiffStatLabel(additions: changes.additions, deletions: changes.deletions)
                    if !files.isEmpty {
                        Image(systemName: "chevron.right")
                            .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                            .foregroundStyle(.tertiary)
                    }
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(files.isEmpty)
            .help("Show the files this turn changed")
            .popover(isPresented: $isShowingFiles, arrowEdge: .bottom) {
                filesPopover.presentedChrome()
            }
            // A fixed gap, not a spacer: the card hugs its title and stats, and the round
            // buttons follow it rather than sitting at the far side of the column.
            Color.clear.frame(width: 20, height: 1)
            if canUndo {
                Button { isConfirmingRevert = true } label: {
                    MutedRoundGlyph(symbol: "arrow.uturn.backward")
                }
                .buttonStyle(.plain)
                .focusable(false)
                .help("Revert this turn's files and conversation")
                .popover(isPresented: $isConfirmingRevert, arrowEdge: .bottom) {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("Revert up to this point?")
                            .font(.system(size: 14, weight: .semibold))
                        Text("Files go back to how they were before this turn, and the turn leaves the conversation.")
                            .font(.system(size: 12))
                            .foregroundStyle(Chrome.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                        HStack(spacing: 8) {
                            Spacer()
                            Button("Cancel") { isConfirmingRevert = false }
                                .buttonStyle(.glass)
                            Button("Revert") { isConfirmingRevert = false; onRevert() }
                                .buttonStyle(.glassProminent)
                                .keyboardShortcut(.defaultAction)
                        }
                        .controlSize(.small)
                    }
                    .padding(16)
                    .frame(width: 300)
                    .presentedChrome()
                }
            }
            Button { [weak reviewBox] in
                guard let view = reviewBox?.value else { return }
                onReview(view)
            } label: {
                MutedRoundGlyph(symbol: "plusminus")
            }
            .buttonStyle(.plain)
            .focusable(false)
            .onHover { [needed = $needsReviewAnchor] hovering in
                if hovering, !needed.wrappedValue { needed.wrappedValue = true }
            }
            .background {
                if needsReviewAnchor {
                    AttachmentAnchorCapture { [weak reviewBox] in reviewBox?.value = $0 }
                }
            }
            .help("Show this turn's changes")
        }
        .padding(.leading, 12)
        // The round buttons sit in the card with the same clearance on every side.
        .padding(.trailing, 6)
        .padding(.vertical, 6)
        .background(.quaternary.opacity(0.32), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .fixedSize(horizontal: true, vertical: false)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(title))
    }

    private var filesPopover: some View {
        let files = changes.files
        return VStack(alignment: .leading, spacing: 0) {
            ForEach(files.prefix(12), id: \.path) { file in
                HStack(spacing: 8) {
                    Text(verbatim: file.path)
                        .font(.chat(.callout, zoom: zoom))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: 8)
                    if file.isBinary {
                        Text("Binary")
                            .font(.chat(.caption, zoom: zoom))
                            .foregroundStyle(.secondary)
                    } else {
                        DiffStatLabel(additions: file.additions, deletions: file.deletions)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 7)
            }
            if files.count > 12 {
                Text(files.count == 13 ? "and 1 more file" : "and \(files.count - 12) more files")
                    .font(.chat(.callout, zoom: zoom))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
            }
        }
    }
}
