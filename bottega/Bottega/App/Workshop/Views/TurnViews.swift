// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// What went on inside a turn, and what goes on now.
struct TurnDetail: View {
    @Environment(WorkshopModel.self) private var model
    let id: String

    var body: some View {
        let turn = model.state.turns[id]
        let inside = model.turns[id]
        let items = inside?.details.items ?? []
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                TurnHeader(id: id, turn: turn, head: inside?.details.head)
                ForEach(items) { item in
                    TurnItemView(item: item)
                    Divider()
                }
                TurnFooter(turn: turn, inside: inside)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .defaultScrollAnchor(turn?.isRunning == true ? .bottom : .top, for: .initialOffset)
        .defaultScrollAnchor(turn?.isRunning == true ? .bottom : nil, for: .sizeChanges)
        .id(id)
        .task(id: Need(isOver: turn?.isRunning != true, isLive: model.isLive, isBehind: inside?.details.completeness != .growing)) {
            model.fetch(turn: id)
        }
    }

    /// When any of these changes there may be more to fetch.
    private struct Need: Equatable { var isOver, isLive, isBehind: Bool }
}

private struct TurnHeader: View {
    let id: String
    let turn: Turn?
    let head: TurnHead?

    var body: some View {
        DetailHeader(title: "Turn", id: id)
        if let session = turn?.session ?? head?.session { MemberName(name: session).font(.callout) }
        if let turn { TurnState(turn: turn, isHeading: true) }
        Facts(
            ("Agent", turn?.vendor ?? head?.vendor, false),
            ("Model", turn?.model ?? head?.model, true),
            ("Effort", turn?.effort ?? head?.effort, true),
            ("CLI", turn?.cliVersion, true),
            ("Started", (turn?.startedAt ?? head?.startedAtMs).map(Wording.exactTime), false),
            ("Took", turn.flatMap(Wording.took), false),
            ("Model calls", turn?.end.map { "\($0.modelTurns)" }, false),
            ("Tools", turn?.end.flatMap { Wording.tools($0.tools) }, false),
            ("Denied", turn?.end.flatMap { $0.denied > 0 ? "\($0.denied)" : nil }, false))
        Divider()
    }
}

/// Running, or how it ended. At the top of a turn's details a running turn is its status line; in a list, the small dots.
struct TurnState: View {
    let turn: Turn
    var isHeading = false

    var body: some View {
        if let end = turn.end {
            let outcome = Wording.turnOutcome(end.outcome)
            StatusChip(text: outcome.text, symbol: outcome.symbol)
        } else if isHeading {
            DoingNow(turn: turn.id, isStatusLine: true)
        } else {
            WorkingLabel()
        }
    }
}

/// What can be said about what is not there.
private struct TurnFooter: View {
    @Environment(WorkshopModel.self) private var model
    let turn: Turn?
    let inside: TurnModel?

    var body: some View {
        if inside?.isFetching == true {
            ProgressView().controlSize(.small)
        } else if let problem = inside?.problem {
            note(Wording.turnProblem(problem), "questionmark.circle")
        } else if inside?.details.completeness == .partial {
            note("LMAS did not keep all of this turn.", "ellipsis.circle")
        } else if case .recording = model.source, inside == nil {
            note("What went on inside is not in this recording.", "questionmark.circle")
        } else if let turn, turn.end != nil, inside?.details.completeness == .behind, model.hasLink {
            Button("Get the Rest") { model.fetch(turn: turn.id) }.controlSize(.small)
        } else if turn?.isRunning == true, inside?.details.completeness != .growing, model.hasLink {
            Button("Get What There Is Now") { if let turn { model.fetch(turn: turn.id) } }.controlSize(.small)
        }
    }

    private func note(_ text: String, _ symbol: String) -> some View {
        Label(text, systemImage: symbol).font(.callout).foregroundStyle(.secondary)
    }
}

/// One thing in a turn. All the words are the agent's or its programs': plain text throughout.
private struct TurnItemView: View {
    let item: TurnDetails.Item

    var body: some View {
        let what = item.item
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                let title = Wording.turnItem(what, hasText: item.text.end > 0)
                Image(systemName: title.symbol).frame(width: 18).foregroundStyle(title.isTrouble ? AnyShapeStyle(.orange) : AnyShapeStyle(.secondary))
                Text(verbatim: title.text).fontWeight(.medium).lineLimit(2)
                Spacer(minLength: 6)
                status
            }
            .font(.callout)
            if let title = what.title { caption(title) }
            content
        }
    }

    @ViewBuilder private var status: some View {
        if item.item.isOver {
            if let status = Wording.turnItemStatus(item.item.status) { StatusChip(text: status.text, symbol: status.symbol) }
        } else if item.item.kind == "command" {
            // A program running, not the AI thinking: the grey spinner stays.
            HStack(spacing: 6) { ProgressView().controlSize(.mini); Text("Now") }.font(.caption).foregroundStyle(.secondary)
        } else {
            HStack(spacing: 6) { RunningStepDot(); Text("Now") }.font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var content: some View {
        let what = item.item
        switch what.kind {
        case "text", "reasoning", "notice":
            if item.text.end > 0 {
                Words(text: item.text, isRunning: !what.isOver)
                    .foregroundStyle(what.kind == "text" ? .primary : .secondary)
            }
        case "command":
            if let input = what.input { PlainBlock(text: input.text, limit: 120, wraps: true).overlay(alignment: .topTrailing) { cutMark(input) } }
            if let cwd = what.cwd { caption(cwd) }
            Output(text: item.text, isRunning: !what.isOver)
            if let code = what.exitCode { caption("Exit code \(code)") }
        case "edit":
            ForEach(Array(what.files.enumerated()), id: \.offset) { _, file in FileChangeView(file: file) }
            Output(text: item.text, isRunning: !what.isOver)
        case "plan":
            if item.text.end > 0 { Words(text: item.text, isRunning: !what.isOver).foregroundStyle(.secondary) }
            ForEach(Array(what.steps.enumerated()), id: \.offset) { _, step in
                let status = Wording.step(step.status)
                Label {
                    Text(verbatim: UntrustedText.line(step.text))
                } icon: {
                    Image(systemName: status.symbol).accessibilityLabel(status.text)
                }
                .font(.callout)
            }
        default:
            ForEach(what.paths, id: \.self) { caption($0) }
            if let input = what.input, what.paths.isEmpty { PlainBlock(text: input.text, limit: 120, wraps: true) }
            Output(text: item.text, isRunning: !what.isOver)
        }
    }

    private func caption(_ text: String) -> some View {
        Text(verbatim: UntrustedText.line(text, limit: 300))
            .font(.caption.monospaced())
            .foregroundStyle(.secondary)
            .textSelection(.enabled)
    }

    @ViewBuilder private func cutMark(_ text: CutText) -> some View {
        if text.cut != nil { Image(systemName: "ellipsis.circle").padding(6).help("Too long: its middle is left out") }
    }
}

/// What it said or thought, as it comes.
private struct Words: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let text: GrownText
    let isRunning: Bool

    private static let longest = 64 * 1024

    var body: some View {
        let shown = UntrustedText.clean(text.tail(Self.longest))
        if !text.isWhole || text.end > UInt64(Self.longest) { LeftOut(bytes: nil) }
        Text(verbatim: shown)
            .font(.callout)
            .textSelection(.enabled)
            .lineSpacing(2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentTransition(.opacity)
            .animation(isRunning && !reduceMotion ? .linear(duration: 0.12) : nil, value: text.end)
    }
}

/// What a command printed or a tool gave back: its end, and all of it on request.
private struct Output: View {
    let text: GrownText
    let isRunning: Bool
    @State private var showsAll = false

    private static let end = 8 * 1024

    var body: some View {
        if text.end > 0 {
            if showsAll, !isRunning {
                ForEach(Array(text.pieces.enumerated()), id: \.offset) { index, piece in
                    if text.missing(before: index) > 0 { LeftOut(bytes: text.missing(before: index)) }
                    PlainBlock(text: piece.text)
                }
            } else {
                let isCut = text.end > UInt64(Self.end) || !text.isWhole
                if isCut {
                    HStack {
                        LeftOut(bytes: nil)
                        Spacer()
                        if !isRunning { Button("Show All") { showsAll = true }.controlSize(.mini) }
                    }
                }
                PlainBlock(text: text.tail(Self.end), limit: 200)
                    .defaultScrollAnchor(isRunning || isCut ? .bottomLeading : .topLeading)
            }
        }
    }
}

/// In Bottega's own hand, outside the text: an agent can write such a line too.
private struct LeftOut: View {
    let bytes: UInt64?

    var body: some View {
        Label(bytes.map { "\(Wording.bytes($0)) left out" } ?? "Only the end", systemImage: "ellipsis")
            .font(.caption)
            .foregroundStyle(.secondary)
    }
}

private struct FileChangeView: View {
    let file: TurnItem.FileChange
    @State private var isOpen = false

    var body: some View {
        let change = Wording.fileChange(file.change)
        if let diff = file.diff {
            Button {
                isOpen.toggle()
            } label: {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    label(change)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .imageScale(.small)
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(isOpen ? 90 : 0))
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityValue(isOpen ? "Expanded" : "Collapsed")
            .help(isOpen ? "Hide what changed" : "Show what changed")
            if isOpen {
                PlainBlock(text: diff.text, limit: 320)
                if let cut = diff.cut { LeftOut(bytes: cut.bytes) }
            }
        } else {
            label(change).textSelection(.enabled)
        }
    }

    private func label(_ change: (text: String, symbol: String)) -> some View {
        Label {
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: UntrustedText.line(file.path, limit: 300)).lineLimit(2).truncationMode(.middle)
                if let to = file.to { Text(verbatim: "→ \(UntrustedText.line(to, limit: 300))").lineLimit(2).truncationMode(.middle) }
                if file.added != nil || file.removed != nil {
                    Text(verbatim: "+\(file.added ?? 0) −\(file.removed ?? 0)").foregroundStyle(.secondary).monospacedDigit()
                }
            }
        } icon: {
            Image(systemName: change.symbol).accessibilityLabel(change.text)
        }
        .font(.caption.monospaced())
        .help(change.text)
    }
}

/// Turns, newest first, each opening what went on in it.
struct TurnList: View {
    @Environment(WorkshopModel.self) private var model
    let turns: [Turn]

    private static let most = 8

    var body: some View {
        if !turns.isEmpty {
            DetailSection(title: turns.count > Self.most ? "Last \(Self.most) of \(turns.count) turns" : "Turns") {
                ForEach(turns.suffix(Self.most).reversed()) { turn in
                    Button {
                        model.detail = .turn(turn.id)
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text(verbatim: UntrustedText.line(turn.id, limit: 24)).font(.caption.monospaced()).foregroundStyle(.secondary)
                            TurnState(turn: turn)
                            Spacer(minLength: 6)
                            if let took = Wording.took(turn) { Text(verbatim: took).font(.caption).foregroundStyle(.secondary).monospacedDigit() }
                            Image(systemName: "chevron.right").imageScale(.small).foregroundStyle(.tertiary)
                        }
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }
}

/// What an agent is doing right now, in a few words. Changes every two seconds at most, however fast things go.
struct DoingNow: View {
    @Environment(WorkshopModel.self) private var model
    let turn: String
    /// The dots and the swept words, as at the top of a running turn; plain words elsewhere.
    var isStatusLine = false
    @State private var words: String?

    var body: some View {
        if isStatusLine {
            WorkingStatus(text: words ?? "Working")
                .task(id: model.turns[turn].map(ObjectIdentifier.init)) {
                    if let inside = model.turns[turn] { await follow(inside) }
                }
        } else if let inside = model.turns[turn] {
            // Nothing is seen of a turn whose inside is not being told: then not even the line is there.
            // Between two things the line stays, so that what is below it does not jump.
            Text(verbatim: words ?? " ")
                .lineLimit(1)
                .truncationMode(.tail)
                .contentTransition(.opacity)
                .font(.caption)
                .foregroundStyle(.secondary)
                .task(id: ObjectIdentifier(inside)) { await follow(inside) }
        }
    }

    private func follow(_ inside: TurnModel) async {
        while !Task.isCancelled {
            // What stopped being told is not what goes on now.
            let current = inside.details.completeness == .growing ? inside.details.current : nil
            withAnimation(.easeInOut(duration: 0.15)) { words = current.map { Wording.doing($0.item) } }
            try? await Task.sleep(for: .seconds(2))
        }
    }
}
