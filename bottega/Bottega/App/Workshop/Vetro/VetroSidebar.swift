// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// The one sidebar: room for the window buttons, the section switcher, that section's list, and along the bottom
/// you, the LMAS status and the backdrop.
struct VetroSidebar: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // The window's own close, minimise and zoom buttons sit over this strip.
            Color.clear.frame(height: 18).padding(.bottom, 10)
            SectionSwitcher()
                .padding(.horizontal, 2)
                .padding(.bottom, 10)
            SectionList(section: model.section)
                .frame(maxHeight: .infinity, alignment: .top)
            SidebarFooter()
        }
        .padding(EdgeInsets(top: 12, leading: 8, bottom: 8, trailing: 8))
        .frame(width: 276)
        .frame(maxHeight: .infinity)
        .vetroPane(cornerRadius: 20, isSidebar: true)
    }
}

/// Five places in one row; the pill slides to the one picked.
private struct SectionSwitcher: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Namespace private var pill

    var body: some View {
        HStack(spacing: 0) {
            ForEach(SidebarSection.allCases) { section in
                let isOn = model.section == section
                Button {
                    withAnimation(reduceMotion ? nil : .smooth(duration: 0.25)) { model.show(section) }
                } label: {
                    Image(systemName: section.symbol)
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(isOn ? .primary : .secondary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .background {
                            if isOn {
                                RoundedRectangle(cornerRadius: 9, style: .continuous)
                                    .fill(look.selection)
                                    .shadow(color: .black.opacity(0.12), radius: 1.5, y: 1)
                                    .matchedGeometryEffect(id: "pill", in: pill)
                            }
                        }
                        .overlay(alignment: .top) {
                            if section == .waiting { WaitingBadge(count: model.waiting.count).offset(x: 10, y: -4) }
                        }
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help(section.title)
                .accessibilityLabel(section.title)
                .accessibilityAddTraits(isOn ? .isSelected : [])
            }
        }
        .frame(height: 36)
        .padding(3)
        .background(.black.opacity(0.12 * look.strength), in: .rect(cornerRadius: 12, style: .continuous))
    }
}

/// Red means one thing in this app: waiting for the user.
struct WaitingBadge: View {
    let count: Int

    var body: some View {
        if count > 0 {
            Text(count, format: .number)
                .font(.system(size: 10, weight: .bold).monospacedDigit())
                .foregroundStyle(.white)
                .contentTransition(.numericText(value: Double(count)))
                .padding(.horizontal, 4)
                .frame(minWidth: 16, minHeight: 16)
                .background(.red, in: .capsule)
                .animation(.smooth(duration: 0.2), value: count)
                .accessibilityLabel("\(count) waiting for you")
        }
    }
}

private struct SectionList: View {
    @Environment(WorkshopModel.self) private var model
    let section: SidebarSection

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(section.title)
                .font(.system(size: 15, weight: .semibold))
                .kerning(-0.15)
                .frame(height: 34)
                .padding(.horizontal, 10)
                .padding(.bottom, 6)
                .accessibilityAddTraits(.isHeader)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    switch section {
                    case .rooms: RoomsList()
                    case .bots: BotsList()
                    case .waiting: WaitingList()
                    case .cards: CardsList()
                    case .activity: ActivityList()
                    }
                }
            }
            .scrollIndicators(.never)
        }
    }
}

// MARK: Rows

/// A row the user can pick; the selected one sits on a lighter piece of glass.
struct VetroRow<Content: View>: View {
    @Environment(Look.self) private var look
    let isOn: Bool
    var height: CGFloat = 34
    var indent: CGFloat = 10
    let action: () -> Void
    @ViewBuilder let content: Content
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) { content }
                .padding(.leading, indent)
                .padding(.trailing, 10)
                .frame(maxWidth: .infinity, minHeight: height, alignment: .leading)
                .background {
                    let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)
                    if isOn {
                        shape.fill(look.selection)
                            .overlay { shape.strokeBorder(LinearGradient(colors: [look.rim, .clear], startPoint: .top, endPoint: .center)) }
                    } else if isHovered {
                        shape.fill(look.hover)
                    }
                }
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .accessibilityAddTraits(isOn ? .isSelected : [])
    }
}

private struct SidebarCaption: View {
    let text: String

    var body: some View {
        Text(verbatim: text)
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
            .padding(EdgeInsets(top: 14, leading: 10, bottom: 4, trailing: 10))
            .accessibilityAddTraits(.isHeader)
    }
}

/// What LMAS does not have yet; drawn so the place is known, with nothing made up in it.
struct NotInLMASYet: View {
    let text: String

    var body: some View {
        Label {
            Text(verbatim: text)
        } icon: {
            Image(systemName: "clock.badge.questionmark")
        }
        .font(.system(size: 12))
        .foregroundStyle(.secondary)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }
}

// MARK: Sections

private struct RoomsList: View {
    var body: some View {
        NotInLMASYet(text: "LMAS has no rooms yet.")
    }
}

private struct BotsList: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        VetroRow(isOn: model.page == .dispatcher, action: { model.page = .dispatcher }) {
            Image(systemName: "d.circle.fill").font(.system(size: 18)).foregroundStyle(.secondary)
            Text("Dispatcher").font(.system(size: 13)).frame(maxWidth: .infinity, alignment: .leading)
            Text("default").font(.system(size: 11)).foregroundStyle(.secondary)
        }
        ForEach(WorkshopState.LineKind.allCases, id: \.self) { kind in
            let lines = model.state.threads(of: kind)
            if !lines.isEmpty {
                SidebarCaption(text: Wording.lineKind(kind))
                ForEach(lines) { line in
                    LineRow(line: line, waits: model.waitingLines.contains(line.id))
                }
            }
        }
    }
}

private struct LineRow: View {
    @Environment(WorkshopModel.self) private var model
    let line: BottegaKit.Line
    let waits: Bool

    var body: some View {
        let arrangement = model.state.arrangement(of: line.id)
        VetroRow(isOn: model.page == .line(line.id), height: 30, indent: 12, action: { model.page = .line(line.id) }) {
            dot
            Text(verbatim: model.state.title(of: line))
                .font(.system(size: 12.5))
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            if arrangement.isArchived {
                Image(systemName: "archivebox").imageScale(.small).foregroundStyle(.secondary).accessibilityLabel("Archived")
            } else if arrangement.isPinned {
                Image(systemName: "pin.fill").imageScale(.small).foregroundStyle(.secondary).accessibilityLabel("Pinned")
            }
            Text(verbatim: Wording.time(line.lastAt))
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }

    /// Red: waiting for you. Filled: something you have not read.
    @ViewBuilder private var dot: some View {
        if waits {
            Circle().fill(.red).frame(width: 9, height: 9)
                .background { Circle().fill(.red.opacity(0.2)).frame(width: 15, height: 15) }
                .accessibilityLabel("Waiting for you")
        } else if model.state.hasUnread(line) {
            Circle().fill(.primary).frame(width: 7, height: 7).frame(width: 9).accessibilityLabel("Unread")
        } else {
            Circle().strokeBorder(.secondary, lineWidth: 1.4).frame(width: 9, height: 9).opacity(0.7)
        }
    }
}

private struct WaitingList: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        let items = model.waiting
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(items.count, format: .number)
                .font(.system(size: 44, weight: .semibold))
                .kerning(-1.3)
                .foregroundStyle(items.isEmpty ? Color.secondary : Color.red)
                .contentTransition(.numericText(value: Double(items.count)))
            Text(items.count == 1 ? "thing waiting" : "things waiting")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
        }
        .padding(EdgeInsets(top: 2, leading: 10, bottom: 12, trailing: 10))
        .accessibilityElement(children: .combine)
        ForEach(items, id: \.what) { item in
            VetroRow(isOn: false, height: 30, action: { model.page = .waiting }) {
                Circle().fill(.red).frame(width: 9, height: 9)
                Text(verbatim: Wording.waiting(item, in: model.state))
                    .font(.system(size: 12.5))
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                TimelineView(.everyMinute) { context in
                    Text(verbatim: Wording.waited(since: item.at, now: context.date))
                        .font(.system(size: 11).monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}

private struct CardsList: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        VetroRow(isOn: model.page == .board, action: { model.page = .board }) {
            Text("All cards").font(.system(size: 13)).frame(maxWidth: .infinity, alignment: .leading)
            Text(model.state.cards.count, format: .number).font(.system(size: 11).monospacedDigit()).foregroundStyle(.secondary)
        }
    }
}

private struct ActivityList: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        VetroRow(isOn: model.page == .activity, action: { model.page = .activity }) {
            Text("Everything").font(.system(size: 13)).frame(maxWidth: .infinity, alignment: .leading)
        }
        Text("Append-only, kept by LMAS")
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
            .padding(EdgeInsets(top: 14, leading: 10, bottom: 4, trailing: 10))
    }
}

// MARK: Footer

private struct SidebarFooter: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    @State private var showsBackdrop = false

    var body: some View {
        HStack(spacing: 8) {
            Menu {
                Button("Devices") { model.page = .devices }
                SettingsLink { Text("Settings…") }
            } label: {
                Image(systemName: "person.fill")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .frame(width: 30, height: 30)
                    .background(look.selection, in: .circle)
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("You: devices and settings")
            .accessibilityLabel("You")
            LMASStatusLine()
            Button { showsBackdrop.toggle() } label: {
                Image(systemName: "photo")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .frame(width: 30, height: 30)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .help("Backdrop")
            .accessibilityLabel("Backdrop")
            .popover(isPresented: $showsBackdrop, arrowEdge: .top) { BackdropPanel() }
        }
        .frame(height: 38)
        .padding(EdgeInsets(top: 6, leading: 4, bottom: 0, trailing: 2))
    }
}

private struct LMASStatusLine: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        HStack(spacing: 7) {
            Circle()
                .fill(isLive ? Color.green : Color.secondary)
                .frame(width: 7, height: 7)
                .background { if isLive { Circle().fill(.green.opacity(0.22)).frame(width: 13, height: 13) } }
            Text(verbatim: text).lineLimit(1)
            if case .catchingUp(let latest) = model.status, latest > 0 {
                Text(verbatim: "\(model.state.through)/\(latest)").monospacedDigit()
            }
        }
        .font(.system(size: 11.5))
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .help(Text(verbatim: text))
        .accessibilityElement(children: .combine)
    }

    private var isLive: Bool { model.source != .none && model.isLive }

    private var text: String {
        switch model.source {
        case .none: "No LMAS folder chosen"
        case .recording: "Recording, nothing can be sent"
        case .lmas: model.isLive ? "LMAS connected · in sync" : Wording.status(model.status).text
        }
    }
}

/// Which photo, how blurred and grained, how clear the glass, and light or dark.
struct BackdropPanel: View {
    @Environment(Look.self) private var look

    var body: some View {
        @Bindable var look = look
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Backdrop").font(.system(size: 13, weight: .semibold))
                Spacer()
                Text(verbatim: look.photo.title.uppercased()).font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary)
            }
            HStack(spacing: 6) {
                ForEach(Look.Photo.allCases) { photo in
                    Button { look.photo = photo } label: {
                        Image(photo.asset)
                            .resizable()
                            .scaledToFill()
                            .frame(width: 52, height: 34)
                            .clipShape(.rect(cornerRadius: 7, style: .continuous))
                            .overlay {
                                RoundedRectangle(cornerRadius: 7, style: .continuous)
                                    .strokeBorder(look.photo == photo ? Color.primary : .clear, lineWidth: 2)
                            }
                    }
                    .buttonStyle(.plain)
                    .help(photo.title)
                    .accessibilityLabel(photo.title)
                    .accessibilityAddTraits(look.photo == photo ? .isSelected : [])
                }
            }
            slider("Blur", value: $look.blur, in: Look.blurRange, text: "\(Int(look.blur.rounded())) px")
            slider("Grain", value: $look.grain, in: Look.percentRange, text: "\(Int(look.grain.rounded()))%")
            slider("Transparency", value: $look.transparency, in: Look.percentRange, text: "\(Int(look.transparency.rounded()))%")
            Picker("Appearance", selection: $look.appearance) {
                ForEach(Look.Appearance.allCases) { Text($0.title).tag($0) }
            }
            .font(.system(size: 12))
        }
        .padding(14)
        .frame(width: 320)
    }

    private func slider(_ title: String, value: Binding<Double>, in range: ClosedRange<Double>, text: String) -> some View {
        HStack(spacing: 10) {
            Text(title).font(.system(size: 12)).foregroundStyle(.secondary).frame(width: 88, alignment: .leading)
            Slider(value: value, in: range).controlSize(.small)
            Text(verbatim: text).font(.system(size: 11).monospacedDigit()).foregroundStyle(.secondary).frame(width: 42, alignment: .trailing)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(title)
    }
}
