import SwiftUI

struct CommandPalette: View {
    @Environment(AppModel.self) private var model

    @Environment(\.colorScheme) private var colorScheme

    @State private var query = ""
    @State private var selection = 0
    /// The rows for the query as the library stands: built when either changes, not on
    /// every arrow key or selection change.
    @State private var items: [Item] = []
    @FocusState private var isFocused: Bool

    private struct ResultsKey: Equatable {
        var query: String
        var threads: [ChatThread]
        var projects: [Project]
        var selectedThreadID: UUID?
        var finishActionTitle: String
    }

    private struct Item: Identifiable {
        let id: String
        let title: String
        var subtitle: String?
        let symbol: String
        var shortcut: String?
        let perform: () -> Void
    }

    var body: some View {
        ZStack(alignment: .top) {
            // A dark window needs more of a wash than a light one to push the palette
            // forward; a single fixed value read as nothing at all in dark themes.
            Color.black.opacity(colorScheme == .dark ? 0.24 : 0.08)
                .ignoresSafeArea()
                .contentShape(.rect)
                .onTapGesture { close() }
                .accessibilityHidden(true)
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(.secondary)
                    TextField("Search threads, projects and actions", text: $query)
                        .textFieldStyle(.plain)
                        .font(.title3)
                        .focused($isFocused)
                        .onSubmit { run(items) }
                        .onKeyPress(.upArrow) {
                            selection = max(0, selection - 1)
                            return .handled
                        }
                        .onKeyPress(.downArrow) {
                            selection = min(max(0, items.count - 1), selection + 1)
                            return .handled
                        }
                        .onKeyPress(.escape) {
                            close()
                            return .handled
                        }
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 15)
                if items.isEmpty {
                    // Never a blank box: a query that matches nothing says so rather
                    // than leaving the field alone above an empty panel.
                    Text("Nothing matches “\(query)”")
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 18)
                        .padding(.bottom, 16)
                } else {
                    VStack(spacing: 2) {
                        ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                            row(item, isSelected: index == selection)
                                // Hovering moves the selection, so the pointer and the
                                // arrow keys never disagree about what Return will run.
                                .onHover { hovering in
                                    if hovering { selection = index }
                                }
                                .onTapGesture {
                                    selection = index
                                    run(items)
                                }
                        }
                    }
                    .padding(.horizontal, 6)
                    .padding(.bottom, 6)
                }
            }
            .frame(width: 600)
            .glassEffect(.regular, in: .rect(cornerRadius: 24, style: .continuous))
            .padding(.top, 72)
        }
        .onAppear { isFocused = true }
        .onChange(of: query) { selection = 0 }
        .onChange(of: ResultsKey(
            query: query, threads: model.threads, projects: model.projects,
            selectedThreadID: model.selectedThreadID, finishActionTitle: model.finishActionTitle
        ), initial: true) { _, _ in
            items = results
        }
    }

    private func row(_ item: Item, isSelected: Bool) -> some View {
        HStack(spacing: 12) {
            Image(systemName: item.symbol)
                .frame(width: 18)
                .foregroundStyle(isSelected ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
            VStack(alignment: .leading, spacing: 1) {
                Text(item.title)
                    .lineLimit(1)
                if let subtitle = item.subtitle {
                    Text(subtitle)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            Spacer(minLength: 8)
            if let shortcut = item.shortcut {
                Text(shortcut)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(isSelected ? AnyShapeStyle(.tint.opacity(0.16)) : AnyShapeStyle(Color.clear), in: .rect(cornerRadius: 12, style: .continuous))
        .contentShape(.rect)
        // A tap gesture on its own is invisible to VoiceOver: the row reads as one
        // element that says it can be run, and says which one Return will run.
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }

    private var results: [Item] {
        let needle = query.trimmingCharacters(in: .whitespaces).lowercased()
        // Chords come from the shortcut store, so the column never shows a key the user
        // has remapped away, and shows nothing at all once a chord is cleared.
        var actions = [
            Item(id: "new-thread", title: "New thread", symbol: "square.and.pencil", shortcut: ShortcutStore.label(for: .newThread)) {
                model.newThread()
            },
            Item(id: "new-worktree", title: "New thread in worktree", symbol: "arrow.triangle.branch", shortcut: ShortcutStore.label(for: .newWorktreeThread)) {
                model.newThread(workspace: .worktree)
            },
            Item(id: "add-project", title: "Add project", symbol: "folder.badge.plus", shortcut: ShortcutStore.label(for: .addProject)) {
                model.chooseProjectFolder()
            },
        ]
        if let threadID = model.selectedThreadID {
            let runtime = model.runtime(for: threadID)
            actions.append(Item(id: "terminal", title: "Toggle terminal", symbol: "terminal", shortcut: ShortcutStore.label(for: .toggleTerminal)) {
                runtime.isTerminalVisible.toggle()
            })
            actions.append(Item(id: "changes", title: "Toggle changes", symbol: "plusminus", shortcut: ShortcutStore.label(for: .toggleChanges)) {
                runtime.toggleDiff()
            })
            actions.append(Item(id: "finish", title: model.finishActionTitle, symbol: model.finishActionSymbol, shortcut: ShortcutStore.label(for: .finishThread)) {
                model.finishSelectedThread()
            })
        }
        actions.append(Item(id: "settings", title: "Settings", symbol: "gearshape", shortcut: ShortcutStore.label(for: .openSettings)) {
            WindowManager.shared.showSettings()
        })

        let matchingActions = needle.isEmpty ? actions : actions.filter { $0.title.lowercased().contains(needle) }
        let threads = model.threads
            .filter { !$0.isArchived && !$0.isInPanel && (needle.isEmpty || $0.title.lowercased().contains(needle)) }
            .sorted { $0.updatedAt > $1.updatedAt }
            .prefix(needle.isEmpty ? 6 : 10)
            .map { thread in
                Item(id: thread.id.uuidString, title: thread.title, subtitle: model.project(thread.projectID)?.name, symbol: "bubble.left") {
                    model.selectedThreadID = thread.id
                }
            }
        let projects = needle.isEmpty ? [] : model.projects
            .filter { $0.name.lowercased().contains(needle) }
            .map { project in
                Item(id: project.id.uuidString, title: "New thread in \(project.name)", subtitle: project.path, symbol: "folder") {
                    model.newThread(in: project)
                }
            }
        let ordered = needle.isEmpty ? Array(threads) + matchingActions : matchingActions + Array(threads) + projects
        return Array(ordered.prefix(12))
    }

    private func run(_ items: [Item]) {
        guard items.indices.contains(selection) else { return }
        let item = items[selection]
        close()
        item.perform()
    }

    private func close() {
        model.isCommandPalettePresented = false
    }
}
