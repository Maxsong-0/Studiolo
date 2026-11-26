// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// On the right: the letter, card, merge, test run or turn picked elsewhere.
struct WorkshopInspector: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        if case .turn(let id) = model.detail {
            TurnDetail(id: id)
        } else {
            page
        }
    }

    private var page: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                switch model.detail {
                case .message(let id):
                    if let message = model.state.messages[id] { MessageDetail(message: message) } else { nothing }
                case .card(let id):
                    if let card = model.state.cards[id] { CardDetail(card: card) } else { nothing }
                case .merge(let id):
                    if let merge = model.state.merges[id] { MergeDetail(merge: merge) } else { nothing }
                case .test(let id):
                    if let test = model.state.tests[id] { TestDetail(test: test) } else { nothing }
                case .turn, nil: nothing
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var nothing: some View {
        Text("Pick a letter, card, merge or test run to see all of it.")
            .font(.callout)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
            .padding(.top, 40)
    }
}

struct DetailHeader: View {
    let title: String
    let id: String

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(verbatim: UntrustedText.line(id, limit: 40)).font(.caption.monospaced()).foregroundStyle(.secondary)
            Text(verbatim: title).font(.headline).textSelection(.enabled)
        }
    }
}

struct Facts: View {
    let rows: [(name: String, value: String, isCode: Bool)]

    init(_ rows: (String, String?, Bool)...) {
        self.rows = rows.compactMap { name, value, isCode in value.map { (name, $0, isCode) } }
    }

    var body: some View {
        Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 6) {
            ForEach(rows, id: \.name) { row in
                GridRow {
                    Text(row.name).foregroundStyle(.secondary)
                    Text(verbatim: UntrustedText.line(row.value, limit: 400))
                        .font(row.isCode ? .callout.monospaced() : .callout)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .font(.callout)
    }
}

struct DetailSection<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.subheadline.weight(.semibold))
            content
        }
    }
}

private struct MessageDetail: View {
    @Environment(WorkshopModel.self) private var model
    let message: Message

    var body: some View {
        DetailHeader(title: Wording.messageKind(message.kind), id: message.id)
        HStack(spacing: 6) {
            MemberName(name: message.from)
            Image(systemName: "arrow.right").imageScale(.small).foregroundStyle(.tertiary)
            MemberName(name: message.to)
        }
        .font(.callout)
        Facts(
            ("Sent", Wording.exactTime(message.at), false),
            ("State", Wording.messageState(message.state).text, false),
            ("Hop", "\(message.hop)", false),
            ("Through", message.device, true),
            ("About", message.card, true))
        DetailSection(title: "Text") { PlainBlock(text: message.body, limit: 420, monospaced: false) }
        if !message.files.isEmpty {
            DetailSection(title: "Files") {
                ForEach(message.files, id: \.self) { FileChip(file: $0) }
            }
        }
        TurnList(turns: model.state.turns(with: message))
        if let original = message.replyTo { letterLink("Replies to", original) }
        if case .answered(let reply) = message.state { letterLink("Answered by", reply) }
        if case .line(message.thread) = model.page {} else {
            Button("Open Line") {
                let detail = model.detail
                model.page = .line(message.thread)
                model.detail = detail
            }
            .controlSize(.small)
        }
    }

    private func letterLink(_ title: String, _ id: String) -> some View {
        DetailSection(title: title) {
            Button {
                model.detail = .message(id)
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: UntrustedText.line(id, limit: 24)).font(.caption.monospaced())
                    if let body = model.state.messages[id]?.body {
                        Text(verbatim: UntrustedText.line(body, limit: 160)).font(.callout).foregroundStyle(.secondary).lineLimit(3)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
        }
    }
}

private struct CardDetail: View {
    @Environment(WorkshopModel.self) private var model
    let card: Card

    var body: some View {
        let column = model.state.column(of: card)
        DetailHeader(title: UntrustedText.line(card.title ?? "Untitled", limit: 200), id: card.id)
        StatusChip(text: Wording.column(column), symbol: Wording.columnSymbol(column), waits: column == .needsYou)
        Facts(
            ("Made by", card.author, false),
            ("For", card.vendor, false),
            ("Reviewer", card.reviewer, false),
            ("Reworks", card.continues, true),
            ("Worker", card.work?.worker, false),
            ("Branch", card.work?.branch, true),
            ("From", card.work.map { String($0.base.prefix(12)) }, true),
            ("Worktree", card.work?.worktree, true),
            ("Made", card.createdAt.map(Wording.exactTime), false),
            ("Started", card.work.map { Wording.exactTime($0.at) }, false))
        if let fresh = card.work?.freshStart {
            DetailSection(title: "Started afresh") {
                Text(verbatim: "From \(UntrustedText.line(fresh.target, limit: 80)), which conflicted in \(Wording.count(fresh.conflicts, "file", "files")).")
                    .font(.callout)
                ForEach(fresh.named ?? [], id: \.self) {
                    Text(verbatim: UntrustedText.line($0, limit: 200)).font(.callout.monospaced())
                }
            }
        }
        if !card.requeues.isEmpty {
            DetailSection(title: "Went back to wait") {
                ForEach(Array(card.requeues.enumerated()), id: \.offset) { _, requeue in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: "\(Wording.exactTime(requeue.at)): \(Wording.requeue(requeue.action))").font(.caption).foregroundStyle(.secondary)
                        Text(verbatim: UntrustedText.clean(requeue.why.text)).font(.callout).textSelection(.enabled)
                    }
                }
            }
        }
        if case .failed(let why, _) = card.end {
            DetailSection(title: "Failed") { Text(verbatim: UntrustedText.clean(why.text)).font(.callout).textSelection(.enabled) }
        }
        if case .done(let result, _) = card.end, let letter = model.state.messages[result] {
            DetailSection(title: "Result") { PlainBlock(text: letter.body, limit: 260, monospaced: false) }
        }
        TurnList(turns: turns)
        block("Instructions", card.instructions)
        block("Acceptance", card.acceptance)
        block("Conventions", card.conventions)
        if !card.tests.isEmpty {
            DetailSection(title: "Tests") {
                ForEach(card.tests.compactMap { model.state.tests[$0] }) { TestRow(test: $0) }
            }
        }
        if !card.reviews.isEmpty {
            DetailSection(title: "Reviews") {
                ForEach(Array(card.reviews.enumerated()), id: \.offset) { _, review in
                    ReviewDetail(review: review)
                }
            }
        }
        if !card.merges.isEmpty {
            DetailSection(title: "Merges") {
                ForEach(card.merges.compactMap { model.state.merges[$0] }) { MergeRow(merge: $0) }
            }
        }
        if let thread = card.thread, model.state.threads[thread] != nil, model.page != .line(thread) {
            Button("Open Line") { model.page = .line(thread) }.controlSize(.small)
        }
    }

    /// Of whoever worked on it or reviewed it, oldest first.
    private var turns: [Turn] {
        let sessions = [card.work?.worker].compactMap(\.self) + card.reviews.compactMap(\.reviewer)
        return Set(sessions).flatMap(model.state.turns(of:)).sorted { ($0.startedAt ?? 0, $0.id) < ($1.startedAt ?? 0, $1.id) }
    }

    @ViewBuilder private func block(_ title: String, _ text: String?) -> some View {
        if let text, !text.isEmpty {
            DetailSection(title: title) { PlainBlock(text: text, limit: 260, monospaced: false) }
        }
    }
}

private struct ReviewDetail: View {
    @Environment(WorkshopModel.self) private var model
    let review: Card.Review

    var body: some View {
        let state = Wording.review(review)
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                if let reviewer = review.reviewer { MemberName(name: reviewer).font(.callout) }
                StatusChip(text: state.text, symbol: state.symbol)
            }
            if case .finished(_, let result, _) = review.outcome, let letter = model.state.messages[result] {
                PlainBlock(text: letter.body, limit: 220, monospaced: false)
            }
        }
    }
}

private struct MergeDetail: View {
    @Environment(WorkshopModel.self) private var model
    let merge: Merge

    var body: some View {
        DetailHeader(title: merge.target.map { "Merge into \(UntrustedText.line($0, limit: 80))" } ?? "Merge", id: merge.id)
        MergeFacts(merge: merge)
        Facts(
            ("Card", merge.card, true),
            ("Asked by", merge.by, false),
            ("Asks you", merge.asksUser ? "Yes" : "No", false),
            ("Requested", merge.requestedAt.map(Wording.exactTime), false),
            ("Test", merge.test?.command, true),
            ("Onto", merge.build.map { String($0.onto.prefix(12)) }, true),
            ("Commit", merge.build.map { String($0.commit.prefix(12)) }, true),
            ("Branch", merge.build?.branch, true))
        let asks = model.state.asks.values.filter { $0.about.merge == merge.id }.sorted { $0.at < $1.at }
        ForEach(asks) { ask in
            DetailSection(title: "Asked you") { AskPanel(ask: ask).font(.callout) }
        }
        if let card = merge.card, model.state.cards[card] != nil {
            Button("Show Card") { model.detail = .card(card) }.controlSize(.small)
        }
    }
}

private struct TestDetail: View {
    @Environment(WorkshopModel.self) private var model
    let test: TestRun

    var body: some View {
        let stage = Wording.testStage(test.stage)
        DetailHeader(title: "Tests", id: test.id)
        StatusChip(text: stage.text, symbol: stage.symbol)
        Facts(
            ("Command", test.request?.command, true),
            ("Tier", test.request?.tier, false),
            ("Needs", test.request.map { "\($0.memoryGib) GiB, \($0.cpu) CPU" }, false),
            ("Card", test.card, true),
            ("Merge", test.merge, true),
            ("By", test.by, false),
            ("Job", test.job, true),
            ("Submitted", test.submittedAt.map(Wording.exactTime), false),
            ("Exit code", report?.exitCode.map(String.init), true),
            ("Took", report?.durationMs.map(Wording.duration), false),
            ("Log", report?.log, true))
        if let report {
            DetailSection(title: report.truncated ? "End of the log (cut)" : "End of the log") {
                PlainBlock(text: report.tail, limit: 420)
            }
        }
        if let card = test.card, model.state.cards[card] != nil {
            Button("Show Card") { model.detail = .card(card) }.controlSize(.small)
        }
    }

    private var report: Event.TestReport? {
        if case .finished(_, let report, _) = test.stage { report } else { nil }
    }
}
