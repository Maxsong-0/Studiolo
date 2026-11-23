// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The right sidebar's column: the tabs across the top, the page under them, and the
/// thread on screen is whose deck, shell and changes it shows. Laid out like the list on
/// the left — flush with the window's edge, the backdrop showing through, its width
/// dragged from its inner edge — so the window reads as three columns rather than a chat
/// with things floating over it.
struct RightSidebarView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let bar = model.rightSidebar
        VStack(alignment: .leading, spacing: 0) {
            // The same clearance the left sidebar gives the window buttons, so both
            // columns' first rows sit on one line.
            Color.clear
                .frame(height: Chrome.trafficLightDiameter)
                .padding(.top, Chrome.trafficLightTop)
            RightSidebarTabs(bar: bar)
                .padding(.horizontal, Chrome.listInset)
                .padding(.top, 14)
                .padding(.bottom, 10)
            page(bar)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background { WindowDragArea() }
        // A hairline on the inner edge, the one the column is dragged by: it says where
        // the chat stops, as the sidebar's own edge does on the left.
        .overlay(alignment: .leading) {
            Rectangle()
                .fill(Chrome.overlay(colorScheme == .dark ? 0.12 : 0.08))
                .frame(width: 1)
        }
    }

    @ViewBuilder
    private func page(_ bar: RightSidebarLayout) -> some View {
        if let tab = bar.tab {
            if let threadID = model.selectedThreadID, let thread = model.thread(threadID) {
                let project = model.project(thread.projectID)
                let workingDirectory = thread.worktreePath ?? project?.path
                let runtime = model.runtime(for: threadID)
                switch tab {
                case .crew:
                    CrewDeckPane(
                        runtime: runtime,
                        // Every hand the lead sent out, not only those still in its panel:
                        // a finished hand cleared from the panel stays on this page.
                        hands: model.crewTeam(of: threadID),
                        workingDirectory: workingDirectory,
                        projectName: project?.name
                    )
                    // Two threads with a team each: the deck is replaced, never one
                    // thread's hands morphed into the other's.
                    .id(threadID)
                case .changes:
                    DiffInspector(runtime: runtime)
                        .id(threadID)
                case .task:
                    TaskPane(runtime: runtime)
                }
            } else {
                RightSidebarEmpty(text: "No thread open")
            }
        }
    }
}

/// The three tabs, as one capsule of switches, and the button that closes the column.
private struct RightSidebarTabs: View {
    @Environment(AppModel.self) private var model
    let bar: RightSidebarLayout

    var body: some View {
        HStack(spacing: 8) {
            ChromeCapsule {
                ForEach(Array(RightSidebarLayout.Tab.allCases.enumerated()), id: \.element.id) { index, tab in
                    if index > 0 { ChromeDivider() }
                    RightSidebarTab(tab: tab, isActive: bar.tab == tab) { bar.open(tab) }
                }
            }
            Spacer(minLength: 0)
            ChromeCircleButton(symbol: "sidebar.right", help: "Hide the right sidebar") {
                bar.close()
            }
        }
    }
}

private struct RightSidebarTab: View {
    let tab: RightSidebarLayout.Tab
    let isActive: Bool
    let select: () -> Void

    @State private var isHovering = false

    var body: some View {
        Button(action: select) {
            HStack(spacing: 6) {
                Group {
                    if let asset = tab.asset {
                        Image(asset)
                            .resizable()
                            .renderingMode(.template)
                            .aspectRatio(contentMode: .fit)
                            .frame(width: Chrome.symbolSize, height: Chrome.symbolSize)
                    } else {
                        Image(systemName: tab.symbol)
                            .font(Chrome.iconFont)
                    }
                }
                Text(verbatim: tab.title)
                    .zoomedFont(size: 12, weight: isActive ? .semibold : .regular)
            }
            .foregroundStyle(isActive ? Chrome.primaryText : Chrome.primaryText.opacity(isHovering ? 0.92 : 0.7))
            .padding(.horizontal, 10)
            .frame(height: Chrome.capsuleContentHeight)
            .background {
                if isActive {
                    Capsule(style: .continuous)
                        .fill(Chrome.overlay(0.12))
                }
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(tab.title + (tab.shortcut.map { ShortcutStore.hint(for: $0) } ?? ""))
        .accessibilityLabel(Text(tab.title))
        .accessibilityAddTraits(isActive ? [.isButton, .isSelected] : .isButton)
    }
}

/// What a page shows when there is nothing for it yet.
struct RightSidebarEmpty: View {
    let text: String
    var detail: String? = nil

    var body: some View {
        VStack(spacing: 6) {
            Text(verbatim: text)
                .zoomedFont(size: 13)
                .foregroundStyle(Chrome.secondaryText)
            if let detail {
                Text(verbatim: detail)
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText.opacity(0.75))
                    .multilineTextAlignment(.center)
            }
        }
        .padding(.horizontal, 20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
