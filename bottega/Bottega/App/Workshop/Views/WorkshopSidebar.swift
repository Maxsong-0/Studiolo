// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

struct WorkshopSidebar: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        // Clicking the selected row again must not clear the page.
        let selection = Binding<Page?>(get: { model.page }, set: { if let page = $0 { model.page = page } })
        List(selection: selection) {
            Section {
                row(.dispatcher)
                waitingRow
                row(.board)
                row(.activity)
                row(.devices)
            }
            ForEach(WorkshopState.LineKind.allCases, id: \.self) { kind in
                let lines = model.state.threads(of: kind)
                if !lines.isEmpty {
                    Section(Wording.lineKind(kind)) {
                        ForEach(lines) { line in
                            LineRow(line: line, waits: model.waitingLines.contains(line.id)).tag(Page.line(line.id))
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .safeAreaBar(edge: .bottom, spacing: 0) { SidebarStatus() }
        .scrollEdgeEffectStyle(.hard, for: .bottom)
    }

    private func row(_ page: Page) -> some View {
        Label(page.fixedTitle, systemImage: page.symbol).tag(page)
    }

    private var waitingRow: some View {
        let count = model.waiting.count
        return HStack {
            Label(Page.waiting.fixedTitle, systemImage: count > 0 ? "hand.raised.fill" : "hand.raised")
            Spacer()
            WaitingCount(count: count)
        }
        .tag(Page.waiting)
    }
}

/// Red means one thing in this app: waiting for the user.
struct WaitingCount: View {
    let count: Int

    var body: some View {
        if count > 0 {
            Text(count, format: .number)
                .font(.caption.weight(.semibold).monospacedDigit())
                .foregroundStyle(.white)
                .contentTransition(.numericText(value: Double(count)))
                .padding(.horizontal, 7)
                .padding(.vertical, 2)
                .background(.red, in: .capsule)
                .animation(.smooth(duration: 0.2), value: count)
                .accessibilityLabel("\(count) waiting for you")
        }
    }
}

private struct LineRow: View {
    @Environment(WorkshopModel.self) private var model
    let line: BottegaKit.Line
    let waits: Bool

    var body: some View {
        let arrangement = model.state.arrangement(of: line.id)
        return VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 4) {
                Text(verbatim: model.state.title(of: line)).lineLimit(1)
                Spacer(minLength: 4)
                if arrangement.isArchived {
                    Image(systemName: "archivebox").foregroundStyle(.secondary).imageScale(.small).accessibilityLabel("Archived")
                } else if arrangement.isPinned {
                    Image(systemName: "pin.fill").foregroundStyle(.secondary).imageScale(.small).accessibilityLabel("Pinned")
                }
                if waits {
                    Image(systemName: "hand.raised.fill").foregroundStyle(.red).imageScale(.small)
                        .accessibilityLabel("Waiting for you")
                }
            }
            HStack(spacing: 3) {
                ForEach(line.members.prefix(5), id: \.self) { MemberGlyph(name: $0, size: 14) }
                Spacer(minLength: 4)
                Text(verbatim: Wording.time(line.lastAt)).font(.caption2).foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
    }
}

private struct SidebarStatus: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        let (text, symbol) = label
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: symbol)
            Text(verbatim: text).lineLimit(2)
            Spacer(minLength: 0)
            if case .catchingUp(let latest) = model.status, latest > 0 {
                Text(verbatim: "\(model.state.through)/\(latest)").monospacedDigit()
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    private var label: (text: String, symbol: String) {
        switch model.source {
        case .none: ("No LMAS folder chosen", "bolt.horizontal.circle")
        case .recording: ("Recording, nothing can be sent", "play.rectangle")
        case .lmas: Wording.status(model.status)
        }
    }
}
