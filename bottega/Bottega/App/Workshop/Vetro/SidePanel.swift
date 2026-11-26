// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The panel on the right of the main pane: how full the context is, then four pages. Only the first has anything
/// from LMAS today; the others say so rather than show made-up numbers (ADR-047).
struct SidePanel: View {
    enum Tab: String, CaseIterable, Identifiable {
        case details, changelog, files, tasks

        var id: Self { self }

        var title: String {
            switch self {
            case .details: "Details"
            case .changelog: "Changelog"
            case .files: "Files"
            case .tasks: "Tasks"
            }
        }

        /// What the page will hold once LMAS tells it.
        var promise: String {
            switch self {
            case .details: ""
            case .changelog: "What each turn on this line changed, with a way to undo a turn."
            case .files: "The files this line changed and read."
            case .tasks: "Background work still running, each with its own Stop."
            }
        }
    }

    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var tab: Tab = .details
    @Namespace private var pill

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ContextRing()
                .padding(EdgeInsets(top: 12, leading: 14, bottom: 10, trailing: 14))
            tabs
                .padding(.horizontal, 10)
                .padding(.bottom, 6)
            switch tab {
            case .details:
                WorkshopInspector()
            case .changelog, .files, .tasks:
                VStack(alignment: .leading, spacing: 8) {
                    Text(verbatim: tab.promise).font(.system(size: 12)).foregroundStyle(.secondary)
                    NotInLMASYet(text: "LMAS does not send this yet.").padding(.horizontal, -10)
                }
                .padding(16)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
        }
        // Picking something elsewhere shows it.
        .onChange(of: model.detail) { if model.detail != nil { tab = .details } }
    }

    private var tabs: some View {
        HStack(spacing: 0) {
            ForEach(Tab.allCases) { item in
                let isOn = tab == item
                Button {
                    withAnimation(reduceMotion ? nil : .smooth(duration: 0.25)) { tab = item }
                } label: {
                    Text(item.title)
                        .font(.system(size: 12, weight: isOn ? .semibold : .regular))
                        .foregroundStyle(isOn ? .primary : .secondary)
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, minHeight: 26)
                        .background {
                            if isOn {
                                RoundedRectangle(cornerRadius: 8, style: .continuous)
                                    .fill(look.selection)
                                    .shadow(color: .black.opacity(0.12), radius: 1.5, y: 1)
                                    .matchedGeometryEffect(id: "tab", in: pill)
                            }
                        }
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(isOn ? .isSelected : [])
            }
        }
        .padding(3)
        .background(.black.opacity(0.1 * look.strength), in: .rect(cornerRadius: 10, style: .continuous))
    }
}

/// How much of the agent's context window is used. LMAS does not report it yet, so the ring stays empty.
private struct ContextRing: View {
    var body: some View {
        HStack(spacing: 12) {
            Circle()
                .strokeBorder(.tertiary, style: StrokeStyle(lineWidth: 3, dash: [2, 3]))
                .frame(width: 26, height: 26)
            VStack(alignment: .leading, spacing: 1) {
                Text("Context").font(.system(size: 13, weight: .semibold))
                Text("LMAS does not report it yet").font(.system(size: 11)).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
    }
}
