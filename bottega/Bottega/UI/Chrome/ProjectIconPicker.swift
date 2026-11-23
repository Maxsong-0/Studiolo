// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

// A project's mark: the emoji or the SF Symbol it wears in the activity list's icon style,
// and the picker that chooses it. Core owns the value (`ProjectIcon`); this file draws it,
// reads the generated catalogue of every symbol on offer, and presents the popover. Every symbol on offer
// is the filled mark, so a project's icon is a solid shape rather than an outline.

/// A project's mark as the rows draw it: the emoji or symbol the reader picked, or the
/// folder mark until one has been picked.
struct ProjectIconMark: View {
    let icon: ProjectIcon?
    var size: CGFloat = 13

    var body: some View {
        Group {
            if let emoji = icon?.emoji {
                Text(verbatim: emoji)
                    .zoomedFont(size: size)
            } else {
                Image(systemName: icon?.symbolName ?? "folder.fill")
                    .zoomedFont(size: size)
            }
        }
        .foregroundStyle(Chrome.primaryText.opacity(0.9))
    }
}

/// A project's mark as a button: clicking it opens the picker over it.
struct ProjectIconButton: View {
    @Binding var icon: ProjectIcon?
    var size: CGFloat = 13

    @State private var isPresented = false

    var body: some View {
        Button {
            isPresented.toggle()
        } label: {
            ProjectIconMark(icon: icon, size: size)
                .frame(width: size + 9, height: size + 9)
                .contentShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .buttonStyle(.plain)
        .help("Choose this project's icon")
        .accessibilityLabel(Text("Project icon"))
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            ProjectIconPicker(icon: $icon)
        }
    }
}

/// Chooses a project's mark: an emoji grid, a search over the SF Symbols the app offers,
/// and a Folder tile that clears the mark back to its default.
struct ProjectIconPicker: View {
    @Binding var icon: ProjectIcon?

    @State private var query = ""

    private static let emojis = [
        "🚀", "🛠️", "📦", "🎨", "🧪", "🌱", "🔥", "🧩",
        "📊", "🕹️", "🎯", "🧠", "⚙️", "💎", "🪄", "🍿",
        "🛰️", "🧭", "📚", "🔮", "🐙", "🦀", "🐝", "🦊",
        "🌊", "🌙", "⚡️", "🍀", "🧵", "🏗️", "🔧", "🧰",
        "🫧", "🧊", "🪵", "🪨", "🧱", "🏺", "🪑", "🛋️",
        "🧲", "🔋", "🔌", "💡", "🔦", "🕯️", "🚿", "🛁",
        "🧴", "🧹", "🧺", "🧼", "🪣", "🧽", "🪥", "🪒",
        "🗝️", "🔐", "🛡️", "⚔️", "🪓", "🗡️", "🏹", "🪃",
        "🧿", "📡", "🎛️", "🖲️", "🖨️", "⌨️", "🖥️", "💻",
        "🖱️", "📱", "⌚️", "🎧", "🔊", "🎤", "🎬", "📷",
        "🎥", "🖼️", "🖌️", "🖍️", "✏️", "📝", "📌", "📍",
        "📎", "🗂️", "🗃️", "🗄️", "📁", "📂", "🗓️", "📅",
        "⏰", "⏳", "🧮", "🔍", "🔎", "🔭", "🔬", "🧫",
        "🧬", "🦠", "🌡️", "🌋", "🏔️", "🏕️", "🧗", "🌈",
        "☄️", "🪐", "🛸", "🌍", "🗺️", "🧳", "🎒", "🪁",
        "🎲", "♟️", "🧸", "🪩", "🎺", "🎸", "🥁", "🏆",
        "🥇", "🎓", "🧑‍🏫",
    ]

    /// The marks the picker opens on, each checked against this Mac's own catalogue as the
    /// list is first read, so a name the system does not have is never offered. Every one is
    /// a filled mark: a project's icon reads as a solid shape, never an outline.
    private static let favourites: [String] = shortlist.filter {
        NSImage(systemSymbolName: $0, accessibilityDescription: nil) != nil
    }

    private static let shortlist = [
        "hammer.fill", "wrench.and.screwdriver.fill", "gearshape.fill", "terminal.fill",
        "curlybraces.square.fill", "chevron.left.square.fill", "shippingbox.fill", "cube.fill",
        "tray.full.fill", "doc.fill", "square.stack.3d.up.fill", "memorychip.fill",
        "cpu.fill", "externaldrive.fill", "paintbrush.fill", "paintpalette.fill",
        "book.closed.fill", "graduationcap.fill", "lightbulb.fill", "key.fill",
        "lock.fill", "leaf.fill", "flame.fill", "drop.fill",
        "sun.max.fill", "moon.stars.fill", "cloud.fill", "bolt.fill",
        "star.fill", "heart.fill", "diamond.fill", "crown.fill",
        "trophy.fill", "medal.fill", "checkmark.seal.fill", "gift.fill",
        "gamecontroller.fill", "dpad.fill", "popcorn.fill", "film.fill",
        "mic.fill", "speaker.wave.3.fill", "camera.fill", "photo.fill",
        "globe.fill", "point.3.filled.connected.trianglepath.dotted", "hand.point.up.left.fill", "person.fill",
        "person.2.fill", "pawprint.fill", "bird.fill", "fish.fill",
        "cup.and.saucer.fill", "waterbottle.fill", "takeoutbag.and.cup.and.straw.fill", "theatermasks.fill",
    ]

    /// The sections the picker scrolls: the shortlist it used to open on under the name
    /// Popular, then every symbol the generated catalogue holds, in the SF Symbols app's
    /// own category order.
    private static let groups: [ProjectSymbolCatalogue.Group] =
        [ProjectSymbolCatalogue.Group(id: "popular", title: "Popular", names: favourites)]
        + ProjectSymbolCatalogue.groups

    private static let columns = Array(repeating: GridItem(.fixed(26), spacing: 4), count: 9)

    /// The symbols the query names, with the one the reader typed in front when the
    /// catalogue knows that name in a filled form the shortlist does not hold.
    private var matchingSymbols: [String] {
        let needle = query.trimmingCharacters(in: .whitespaces)
        guard !needle.isEmpty else { return [] }
        var names = ProjectSymbolCatalogue.all.filter { $0.localizedCaseInsensitiveContains(needle) }
        if let typed = Self.filledForm(of: needle), !names.contains(typed) {
            names.insert(typed, at: 0)
        }
        return names
    }

    /// A typed name as the grid may offer it: the name itself when it is already a filled
    /// mark, its `.fill` mark when the catalogue has one, and nothing at all otherwise, so
    /// a symbol chosen from the search is as solid as the ones in the shortlist.
    private static func filledForm(of name: String) -> String? {
        if name.hasSuffix(".fill") || name.hasSuffix(".inverse") {
            return NSImage(systemSymbolName: name, accessibilityDescription: nil) != nil ? name : nil
        }
        let filled = name + ".fill"
        return NSImage(systemSymbolName: filled, accessibilityDescription: nil) != nil ? filled : nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(verbatim: "Project icon")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(Chrome.primaryText)
            searchField
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    if matchingSymbols.isEmpty {
                        emojiGrid
                        ForEach(Self.groups) { group in
                            symbolSection(group)
                        }
                    } else {
                        symbolGrid(matchingSymbols)
                    }
                }
                .padding(.vertical, 1)
            }
            .scrollIndicators(.never)
            Divider()
                .overlay(Chrome.overlay(0.12))
            HStack(spacing: 8) {
                IconTile(isSelected: icon == nil, action: { icon = nil }) {
                    Image(systemName: "folder.fill")
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                }
                Text(verbatim: "Folder")
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.secondaryText)
                Spacer(minLength: 0)
            }
        }
        .padding(12)
        .frame(width: 300, height: 420)
    }

    private func symbolSection(_ group: ProjectSymbolCatalogue.Group) -> some View {
        section(group.title) {
            LazyVGrid(columns: Self.columns, spacing: 4) {
                ForEach(group.names, id: \.self) { name in
                    IconTile(isSelected: icon?.symbolName == name, action: { icon = .symbol(name) }) {
                        Image(systemName: name)
                            .font(.system(size: 12))
                            .foregroundStyle(Chrome.primaryText.opacity(0.9))
                    }
                }
            }
        }
    }

    private var searchField: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
            TextField("Search symbols", text: $query)
                .textFieldStyle(.plain)
                .font(.system(size: 12))
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background {
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(Chrome.overlay(0.10))
        }
    }

    private var emojiGrid: some View {
        section("Emoji") {
            LazyVGrid(columns: Self.columns, spacing: 4) {
                ForEach(Self.emojis, id: \.self) { emoji in
                    IconTile(isSelected: icon?.emoji == emoji, action: { icon = .emoji(emoji) }) {
                        Text(verbatim: emoji)
                            .font(.system(size: 15))
                    }
                }
            }
        }
    }

    private func symbolGrid(_ names: [String]) -> some View {
        section("Symbols") {
            LazyVGrid(columns: Self.columns, spacing: 4) {
                ForEach(names, id: \.self) { name in
                    IconTile(isSelected: icon?.symbolName == name, action: { icon = .symbol(name) }) {
                        Image(systemName: name)
                            .font(.system(size: 12))
                            .foregroundStyle(Chrome.primaryText.opacity(0.9))
                    }
                }
            }
        }
    }

    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(verbatim: title)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(Chrome.secondaryText)
            content()
        }
    }
}

/// One mark in the picker's grid: a tile that fills as the pointer crosses it and wears the
/// accent ring when it is the project's mark.
private struct IconTile<Content: View>: View {
    let isSelected: Bool
    let action: () -> Void
    @ViewBuilder var content: () -> Content

    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            content()
                .frame(width: 26, height: 26)
                .background {
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(Chrome.overlay(isHovering || isSelected ? 0.14 : 0))
                }
                .overlay {
                    if isSelected {
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .strokeBorder(Chrome.accent, lineWidth: 1.5)
                    }
                }
                .contentShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
    }
}
