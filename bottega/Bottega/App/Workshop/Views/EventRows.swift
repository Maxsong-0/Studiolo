// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// A bordered row in a timeline about one thing LMAS is doing.
struct EventBox<Content: View>: View {
    @Environment(WorkshopModel.self) private var model
    let symbol: String
    let title: String
    let at: UInt64?
    let detail: Detail?
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: symbol).foregroundStyle(.secondary).frame(width: 18)
                Text(verbatim: title).fontWeight(.medium)
                Spacer(minLength: 8)
                if let at {
                    Text(verbatim: Wording.time(at)).font(.caption).foregroundStyle(.tertiary).monospacedDigit()
                }
            }
            content
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: .rect(cornerRadius: 10))
        .overlay {
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(isSelected ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.separator), lineWidth: isSelected ? 1.5 : 1)
        }
        .contentShape(.rect)
        .onTapGesture { if let detail { model.detail = detail } }
    }

    private var isSelected: Bool { detail != nil && detail == model.detail }
}

struct TestRow: View {
    let test: TestRun

    var body: some View {
        let stage = Wording.testStage(test.stage)
        EventBox(symbol: "testtube.2", title: "Tests \(UntrustedText.line(test.id, limit: 24))", at: test.submittedAt, detail: .test(test.id)) {
            if let request = test.request {
                Text(verbatim: UntrustedText.line(request.command, limit: 200)).font(.callout.monospaced()).lineLimit(2)
            }
            HStack(spacing: 10) {
                StatusChip(text: stage.text, symbol: stage.symbol)
                if case .finished(_, let report?, _) = test.stage, let duration = report.durationMs {
                    Text(verbatim: Wording.duration(duration)).font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
                if let card = test.card {
                    Text(verbatim: UntrustedText.line(card, limit: 24)).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                if let merge = test.merge {
                    Text(verbatim: "for \(UntrustedText.line(merge, limit: 24))").font(.caption.monospaced()).foregroundStyle(.secondary)
                }
            }
        }
    }
}

struct ReviewRow: View {
    let card: String
    let review: Card.Review

    var body: some View {
        let state = Wording.review(review)
        EventBox(symbol: "eye", title: "Review of \(UntrustedText.line(card, limit: 24))", at: review.startedAt, detail: .card(card)) {
            HStack(spacing: 10) {
                if let reviewer = review.reviewer { MemberName(name: reviewer).font(.callout) }
                StatusChip(text: state.text, symbol: state.symbol)
            }
        }
    }
}

struct MergeRow: View {
    let merge: Merge

    var body: some View {
        EventBox(symbol: "arrow.triangle.merge", title: title, at: merge.requestedAt, detail: .merge(merge.id)) {
            MergeFacts(merge: merge)
        }
    }

    private var title: String {
        let what = merge.card.map { UntrustedText.line($0, limit: 24) } ?? UntrustedText.line(merge.id, limit: 24)
        return merge.target.map { "Merge \(what) into \(UntrustedText.line($0, limit: 60))" } ?? "Merge \(what)"
    }
}

struct MergeFacts: View {
    let merge: Merge

    var body: some View {
        let stage = Wording.mergeStage(merge.stage)
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                StatusChip(text: stage.text, symbol: stage.symbol)
                if let size = merge.size {
                    Text(verbatim: "\(Wording.count(size.files, "file", "files")), +\(size.insertions) −\(size.deletions)")
                        .font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
                if merge.cleaned {
                    Text("Cleaned up").font(.caption).foregroundStyle(.secondary)
                }
            }
            ForEach(Array(merge.gaps.enumerated()), id: \.offset) { _, gap in
                Label {
                    Text(verbatim: UntrustedText.line(gap.text))
                } icon: {
                    Image(systemName: "exclamationmark.circle")
                }
                .font(.callout)
            }
            ForEach(Array(merge.restarts.enumerated()), id: \.offset) { _, why in
                Label {
                    Text(verbatim: UntrustedText.line(why.text))
                } icon: {
                    Image(systemName: "arrow.counterclockwise")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
    }
}

struct TripRow: View {
    let trip: Trip

    var body: some View {
        EventBox(symbol: "gauge.with.dots.needle.100percent", title: "Held back: \(Wording.limit(trip.limit))", at: trip.at, detail: nil) {
            TripFacts(trip: trip)
        }
    }
}

struct TripFacts: View {
    @Environment(WorkshopModel.self) private var model
    let trip: Trip

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 5) {
                MemberName(name: trip.from)
                Image(systemName: "arrow.right").imageScale(.small).foregroundStyle(.tertiary)
                MemberName(name: trip.to)
                Text(verbatim: Wording.messageKind(trip.kind)).foregroundStyle(.secondary)
            }
            .font(.callout)
            HStack {
                Text("This letter was not sent. Speaking on the line lets it go on: what you say does not count.")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                if let thread = trip.thread, model.page != .line(thread), model.state.threads[thread] != nil {
                    Button("Open Line") { model.page = .line(thread) }.controlSize(.small)
                }
                if model.isWaiting(trip.id) {
                    Button("Put Away") { model.putAway(trip.id) }.controlSize(.small)
                }
            }
        }
    }
}

/// An ask in a timeline: what it came to, or the way to its sheet.
struct AskRow: View {
    @Environment(WorkshopModel.self) private var model
    let ask: Ask

    var body: some View {
        EventBox(symbol: "hand.raised", title: "Asked by \(UntrustedText.line(ask.by, limit: 40))", at: ask.at, detail: ask.about.merge.map(Detail.merge)) {
            AskText(ask: ask)
            HStack {
                if let answer = ask.answer {
                    StatusChip(text: Wording.answer(answer.word, via: answer.via), symbol: answer.word == "approved" ? "checkmark.circle" : "xmark.circle")
                    Text(verbatim: Wording.time(answer.at)).font(.caption).foregroundStyle(.tertiary).monospacedDigit()
                } else {
                    StatusChip(text: "Waiting for you", symbol: "hand.raised.fill", waits: true)
                    Spacer()
                    Button("Review…") { model.askBeingRead = .init(id: ask.id) }
                }
            }
        }
    }
}
