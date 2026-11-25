// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// Letters on a line between agents: one vertical line per agent, time going down, each letter an arrow across.
/// Solid for what asks for something, dashed for what answers; the dot is who sent it.
struct LanesView: View {
    @Environment(WorkshopModel.self) private var model
    let line: BottegaKit.Line

    private static let laneWidth: CGFloat = 92
    private static let rowHeight: CGFloat = 46
    private static let textWidth: CGFloat = 250

    var body: some View {
        let lanes = model.state.lanes(of: line)
        VStack(spacing: 0) {
            LanesHeader(line: line)
            Divider()
            ScrollView([.vertical, .horizontal]) {
                LazyVStack(alignment: .leading, spacing: 0, pinnedViews: .sectionHeaders) {
                    Section {
                        ForEach(Array(lanes.letters.enumerated()), id: \.element.id) { index, letter in
                            row(letter, index, lanes).transition(.newRow)
                        }
                    } header: {
                        names(lanes.members)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 20)
                .animation(.smooth(duration: 0.2), value: lanes.letters.last?.id)
            }
            .defaultScrollAnchor(.topLeading)
            .listMovement(ids: lanes.letters.map(\.id), selection: selection)
        }
    }

    private var selection: Binding<String?> {
        Binding {
            if case .message(let id) = model.detail { id } else { nil }
        } set: { id in
            if let id { model.detail = .message(id) }
        }
    }

    private func names(_ members: [String]) -> some View {
        HStack(spacing: 0) {
            ForEach(members, id: \.self) { name in
                VStack(spacing: 4) {
                    MemberGlyph(name: name, size: 22)
                    Text(verbatim: UntrustedText.line(name, limit: 30))
                        .font(.caption2)
                        .lineLimit(2)
                        .multilineTextAlignment(.center)
                }
                .frame(width: Self.laneWidth)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 8)
        .background(.background)
    }

    private func row(_ letter: WorkshopState.Letter, _ index: Int, _ lanes: WorkshopState.Lanes) -> some View {
        let isSelected = model.detail == .message(letter.id)
        return HStack(spacing: 12) {
            Canvas { context, size in
                draw(letter, index, lanes, in: &context, size)
            }
            .frame(width: CGFloat(lanes.members.count) * Self.laneWidth, height: Self.rowHeight)
            .accessibilityHidden(true)
            LetterSummary(id: letter.id)
                .frame(width: Self.textWidth, alignment: .leading)
            Spacer(minLength: 0)
        }
        .background(isSelected ? AnyShapeStyle(Color.accentColor.opacity(0.14)) : AnyShapeStyle(.clear), in: .rect(cornerRadius: 6))
        .contentShape(.rect)
        .onTapGesture { model.detail = .message(letter.id) }
        .onAppear { report(letter, true) }
        .onDisappear { report(letter, false) }
    }

    private func report(_ letter: WorkshopState.Letter, _ isOnScreen: Bool) {
        if model.state.messages[letter.id]?.isToUser == true { model.letter(letter.id, isOnScreen: isOnScreen) }
    }

    private func draw(_ letter: WorkshopState.Letter, _ index: Int, _ lanes: WorkshopState.Lanes, in context: inout GraphicsContext, _ size: CGSize) {
        func x(_ lane: Int) -> CGFloat { (CGFloat(lane) + 0.5) * Self.laneWidth }
        let middle = size.height / 2

        for lane in lanes.members.indices {
            var path = Path()
            path.move(to: CGPoint(x: x(lane), y: 0))
            path.addLine(to: CGPoint(x: x(lane), y: size.height))
            context.stroke(path, with: .style(.separator), lineWidth: 1)
        }
        // Busy from the letter that asked until the one that answered.
        for stretch in lanes.working where index >= stretch.start && index <= (stretch.end ?? .max) {
            let top = index == stretch.start ? middle : 0
            let bottom = index == stretch.end ? middle : size.height
            guard bottom > top else { continue }
            let bar = CGRect(x: x(stretch.lane) - 3.5, y: top, width: 7, height: bottom - top)
            context.fill(Path(bar), with: .style(.tint.opacity(0.35)))
        }

        let from = x(letter.from)
        let to = x(letter.to)
        let style = StrokeStyle(lineWidth: 1.5, lineCap: .round, dash: letter.isReply ? [4, 4] : [])
        let dot = Path(ellipseIn: CGRect(x: from - 3.5, y: middle - 3.5, width: 7, height: 7))
        if letter.from == letter.to {
            // To itself: a loop beside the line.
            var loop = Path()
            loop.move(to: CGPoint(x: from, y: middle - 8))
            loop.addQuadCurve(to: CGPoint(x: from + 4, y: middle + 8), control: CGPoint(x: from + 34, y: middle))
            context.stroke(loop, with: .style(.primary), style: style)
        } else {
            let direction: CGFloat = to > from ? 1 : -1
            var shaft = Path()
            shaft.move(to: CGPoint(x: from, y: middle))
            shaft.addLine(to: CGPoint(x: to - direction * 5, y: middle))
            context.stroke(shaft, with: .style(.primary), style: style)
            var head = Path()
            head.move(to: CGPoint(x: to - direction * 8, y: middle - 4.5))
            head.addLine(to: CGPoint(x: to - direction * 1, y: middle))
            head.addLine(to: CGPoint(x: to - direction * 8, y: middle + 4.5))
            context.stroke(head, with: .style(.primary), style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
        }
        context.fill(dot, with: .style(.primary))
    }
}

private struct LetterSummary: View {
    @Environment(WorkshopModel.self) private var model
    let id: String

    var body: some View {
        if let message = model.state.messages[id] {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(verbatim: Wording.messageKind(message.kind)).fontWeight(.medium)
                    Text(verbatim: UntrustedText.line(id, limit: 24)).font(.caption.monospaced()).foregroundStyle(.secondary)
                    if message.isToUser, message.isOpenQuestion {
                        Image(systemName: "hand.raised.fill").foregroundStyle(.red).imageScale(.small)
                            .accessibilityLabel("Waiting for you")
                    } else if case .failed = message.state {
                        Image(systemName: "xmark").foregroundStyle(.orange).imageScale(.small).accessibilityLabel("Not delivered")
                    }
                }
                Text(verbatim: UntrustedText.line(message.body, limit: 120)).font(.callout).foregroundStyle(.secondary).lineLimit(1)
            }
            .accessibilityElement(children: .combine)
        }
    }
}

/// The line's numbers: how many letters, how far passed on, who writes to each other most.
private struct LanesHeader: View {
    let line: BottegaKit.Line

    var body: some View {
        HStack(spacing: 22) {
            fact("Letters", "\(line.messages.count)")
            fact("Hops", "\(line.deepestHop)")
            if let busiest = line.busiestPair {
                fact(
                    "Busiest pair",
                    "\(UntrustedText.line(busiest.pair.one, limit: 24)) ↔ \(UntrustedText.line(busiest.pair.other, limit: 24)) · \(busiest.count)")
            }
            Spacer()
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 10)
    }

    private func fact(_ name: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(name).font(.caption2).foregroundStyle(.secondary)
            Text(verbatim: value).font(.callout).monospacedDigit().lineLimit(1)
        }
    }
}
