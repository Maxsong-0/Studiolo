// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The MCP servers: a grid of cards where one tap connects, with the connected
/// ones gathered in a section on top.
struct MCPSettingsPage: View {
    let query: String
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// Servers flashing their success before they leave the grid for Connected.
    @State private var celebrating: Set<String> = []

    private static let columns = [
        GridItem(.flexible(), spacing: 10),
        GridItem(.flexible(), spacing: 10),
    ]

    private var connected: [MCPCatalogEntry] {
        MCPCatalog.entries
            .filter { !celebrating.contains($0.id) && model.mcp.state(for: $0.id).isConnected }
            .sorted { $0.name < $1.name }
    }

    private var visibleSections: [(category: MCPCategory, entries: [MCPCatalogEntry])] {
        MCPCatalog.byCategory.compactMap { group in
            let entries = group.entries.filter { entry in
                (!model.mcp.state(for: entry.id).isConnected || celebrating.contains(entry.id))
                    && Self.matches(entry, query: query)
            }
            return entries.isEmpty ? nil : (category: group.category, entries: entries)
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Chrome.sectionSpacing) {
            MCPLeadCard()
            if !connected.isEmpty {
                ChromeSection(title: "Connected") {
                    ChromeCard {
                        ForEach(Array(connected.enumerated()), id: \.element.id) { index, entry in
                            if index > 0 { ChromeRowDivider() }
                            MCPConnectedRow(entry: entry)
                        }
                    }
                }
            }
            MCPCustomSection(query: query)
            ForEach(visibleSections, id: \.category) { section in
                ChromeSection(title: section.category.title) {
                    LazyVGrid(columns: Self.columns, spacing: 10) {
                        ForEach(section.entries) { entry in
                            MCPServerCard(entry: entry, celebrating: celebrating.contains(entry.id))
                                .transition(.scale(scale: 0.9).combined(with: .opacity))
                        }
                    }
                }
                .animation(.spring(duration: 0.35), value: celebrating)
            }
            Text("Connected servers reach every provider when a thread starts: the CLIs at launch, the API models through Bottega itself.")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
        }
        .onChange(of: model.mcp.states) { previous, current in
            guard !reduceMotion else { return }
            for (id, state) in current where state.isConnected && !(previous[id]?.isConnected ?? false) {
                celebrating.insert(id)
                SettleChime.play()
                Task { @MainActor in
                    try? await Task.sleep(for: .milliseconds(900))
                    celebrating.remove(id)
                }
            }
        }
    }

    fileprivate static func matches(_ entry: MCPCatalogEntry, query: String) -> Bool {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return true }
        return entry.name.localizedCaseInsensitiveContains(trimmed)
            || entry.vendor.localizedCaseInsensitiveContains(trimmed)
            || entry.summary.localizedCaseInsensitiveContains(trimmed)
            || entry.id.localizedCaseInsensitiveContains(trimmed)
            || entry.sampleTools.contains { $0.localizedCaseInsensitiveContains(trimmed) }
    }
}

private struct MCPLeadCard: View {
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "checkmark.shield.fill")
                .font(.system(size: 22))
                .foregroundStyle(Chrome.accent)
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 4) {
                    Text("Connected servers lead")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Chrome.primaryText)
                    // The three facts live behind the `i`: stacked in the card they took
                    // more room than the point they made.
                    MCPHoverInfoButton(label: "About connected servers") {
                        VStack(alignment: .leading, spacing: 8) {
                            fact(symbol: "lock.fill", text: "Keys stay in your Keychain")
                            fact(symbol: "arrow.uturn.backward", text: "Disconnect to go back to your own setup")
                            fact(symbol: "sparkles", text: "Every provider, whatever the model")
                        }
                        .padding(14)
                    }
                }
                Text("Connect a server here and every thread can use it. Anything set up in a terminal or another app stays just as it is.")
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Chrome.primaryText.opacity(0.05))
        )
    }

    private func fact(symbol: String, text: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: symbol)
                .foregroundStyle(Chrome.accent)
                .frame(width: 14)
            Text(verbatim: text)
        }
        .font(.system(size: 12))
        .foregroundStyle(Chrome.primaryText.opacity(0.9))
    }
}

/// A connected server: its icon, name, tool count and version, a switch for
/// handing it to providers, and a menu for refreshing, peeking at tools and leaving.
private struct MCPConnectedRow: View {
    @Environment(AppModel.self) private var model
    let entry: MCPCatalogEntry

    @State private var showMenu = false
    @State private var showTools = false

    private var connection: MCPConnection? { model.mcp.connection(for: entry.id) }

    private var toolCount: Int {
        if case .connected(let count) = model.mcp.state(for: entry.id) { return count }
        return connection?.tools.count ?? 0
    }

    var body: some View {
        HStack(spacing: 12) {
            MCPIcon(entry: entry, size: 22)
                .frame(width: 26, alignment: .center)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: entry.name)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                Text(verbatim: "\(toolCount) tools · \(connection?.serverVersion ?? entry.vendor)")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
            }
            Spacer(minLength: 12)
            SettingsSwitch(isOn: Binding(
                get: { model.mcp.connection(for: entry.id)?.isEnabled ?? true },
                set: { model.mcp.setEnabled($0, for: entry) }
            ))
            Button {
                showMenu.toggle()
            } label: {
                Image(systemName: "ellipsis.circle")
                    .font(.system(size: 13))
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(width: 22, height: 22)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .help("Server options")
            .popover(isPresented: $showMenu, arrowEdge: .bottom) {
                PopoverMenu {
                    PopoverItem("Refresh", symbol: "arrow.clockwise") {
                        model.mcp.refresh(entry)
                    }
                    PopoverItem("Show tools", symbol: "wrench") {
                        showMenu = false
                        showTools = true
                    }
                    PopoverDivider()
                    PopoverItem("Disconnect", symbol: "xmark.circle", isDestructive: true) {
                        model.mcp.disconnect(entry)
                    }
                }
            }
            // On the button, not the row: hung from the row it opened in the row's middle.
            .popover(isPresented: $showTools, arrowEdge: .bottom) {
                toolsPopover
            }
        }
        .padding(.leading, 16)
        .padding(.trailing, Chrome.rowControlTrailingPadding)
        .padding(.vertical, 11)
    }

    private var toolsPopover: some View {
            PopoverMenu {
                PopoverSectionHeader("\(entry.name) tools")
                let tools = connection?.tools ?? []
                if tools.isEmpty {
                    PopoverNote("No tools listed yet.")
                }
                ForEach(tools) { tool in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(verbatim: tool.name)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(Chrome.primaryText)
                        if let summary = tool.summary, !summary.isEmpty {
                            Text(verbatim: summary)
                                .font(.system(size: 11))
                                .foregroundStyle(Chrome.secondaryText)
                                .lineLimit(2)
                        }
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
        }
    }
}

/// One server in the grid: its icon, name, summary and sample tools, with a
/// Connect button that either connects at once or opens its fields inline.
private struct MCPServerCard: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let entry: MCPCatalogEntry
    var celebrating = false

    @State private var successPopped = false

    private var state: MCPConnectionState { model.mcp.state(for: entry.id) }

    private var toolCount: Int {
        if case .connected(let count) = state { return count }
        return 0
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if celebrating {
                successView
            } else {
                headerRow
                // Every card keeps the same shape: two lines held for the summary whether
                // it needs them or not, one line of tools, so the grid reads as a grid.
                Text(verbatim: entry.summary)
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.secondaryText)
                    .lineLimit(2, reservesSpace: true)
                    .fixedSize(horizontal: false, vertical: true)
                Text(verbatim: entry.sampleTools.isEmpty ? " " : entry.sampleTools.joined(separator: " · "))
                    .font(.system(size: 10.5, design: .monospaced))
                    .foregroundStyle(Chrome.secondaryText.opacity(0.8))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    // Clear of the info button in the corner.
                    .padding(.trailing, 24)
                if case .failed(let message) = state {
                    Text(verbatim: message)
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.danger)
                        .lineLimit(3)
                }
                if case .needsSetup(let guide) = state {
                    Text(verbatim: guide.title)
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.warning)
                        .lineLimit(2)
                }
            }
        }
        .padding(14)
        .onChange(of: state) { _, now in
            // The moment the server asks for a step on its side, the panel with the steps
            // opens by itself: the user came back from the browser expecting a result.
            if case .needsSetup(let guide) = now {
                MCPSetupPanel.shared.present(entry: entry, guide: guide, model: model)
            }
        }
        // The cell fills its row: a card whose fields are open makes its neighbour as tall.
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Chrome.primaryText.opacity(0.05))
        )
        .overlay(alignment: .bottomTrailing) {
            if !celebrating {
                MCPInfoButton(entry: entry)
                    .padding(10)
            }
        }
    }

    private var headerRow: some View {
        HStack(alignment: .top, spacing: 10) {
            MCPIcon(entry: entry, size: 26)
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: entry.name)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                Text(verbatim: entry.vendor)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
            }
            Spacer(minLength: 8)
            action
        }
    }

    @ViewBuilder
    private var action: some View {
        switch state {
        case .notConnected:
            // Sign in for a server that signs in; Connect for the rest. A server that
            // needs a key gets the floating key panel, which opens the page the key is
            // made on and takes the paste (see `MCPKeyPanel`); the card stays clean.
            Button(entry.isOAuth ? "Sign in" : "Connect") { begin() }
                .buttonStyle(.glass)
                .controlSize(.small)
        case .connecting:
            HStack(spacing: 6) {
                WorkingSpinner(cellSize: 3)
                Text(entry.isOAuth ? "Waiting" : "Connecting")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .fixedSize()
            }
        case .connected:
            EmptyView()
        case .needsSetup(let guide):
            Button("Fix it") { MCPSetupPanel.shared.present(entry: entry, guide: guide, model: model) }
                .buttonStyle(.glassProminent)
                .controlSize(.small)
        case .failed:
            Button(entry.isOAuth ? "Sign in again" : "Try again") { begin() }
                .buttonStyle(.glass)
                .controlSize(.small)
        }
    }

    private func begin() {
        if !entry.needsInput {
            model.mcp.connect(entry)
        } else {
            MCPKeyPanel.shared.present(entry: entry, model: model)
        }
    }

    private var successView: some View {
        VStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 34))
                .foregroundStyle(Chrome.success)
                .scaleEffect(successPopped ? 1 : 0.5)
            Text(verbatim: "Connected · \(toolCount) tools")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(Chrome.primaryText)
        }
        .frame(maxWidth: .infinity, minHeight: 120, alignment: .center)
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.spring(duration: 0.5, bounce: 0.4)) { successPopped = true }
        }
    }
}

/// The user's own servers, between Connected and the catalog: one row each (state, switch
/// and menu like a connected row, a Connect button like a card's), and the row that adds
/// one. The section steps aside when a search matches none of them.
private struct MCPCustomSection: View {
    @Environment(AppModel.self) private var model
    let query: String

    @State private var isAdding = false

    private var visible: [MCPCustomServer] {
        model.mcp.customs.filter { MCPSettingsPage.matches($0.catalogEntry(), query: query) }
    }

    var body: some View {
        let isSearching = !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        if !visible.isEmpty || !isSearching {
            ChromeSection(title: "Custom") {
                ChromeCard {
                    if visible.isEmpty {
                        Text("Servers of your own: a command that runs on this Mac, or a remote address, with the variables they need.")
                            .font(.system(size: 12))
                            .foregroundStyle(Chrome.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, 16)
                            .padding(.vertical, 11)
                        ChromeRowDivider()
                    } else {
                        ForEach(Array(visible.enumerated()), id: \.element.id) { index, server in
                            if index > 0 { ChromeRowDivider() }
                            MCPCustomRow(server: server)
                        }
                        ChromeRowDivider()
                    }
                    addRow
                }
            }
        }
    }

    private var addRow: some View {
        Button {
            isAdding = true
        } label: {
            HStack(spacing: 12) {
                Image(systemName: "plus.circle")
                    .font(.system(size: 13))
                    .foregroundStyle(Chrome.accent)
                    .frame(width: 26, alignment: .center)
                Text("Add server")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(Chrome.primaryText)
                Spacer(minLength: 12)
            }
            .padding(.leading, 16)
            .padding(.trailing, Chrome.rowControlTrailingPadding)
            .padding(.vertical, 11)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .popover(isPresented: $isAdding, arrowEdge: .bottom) {
            MCPCustomEditor(server: nil)
                .presentedChrome()
        }
    }
}

/// One of the user's own servers: the connected row's shape (icon, name, state, a switch
/// once connected) plus the Connect button a catalog card has, and a menu for editing,
/// disconnecting and deleting.
private struct MCPCustomRow: View {
    @Environment(AppModel.self) private var model
    let server: MCPCustomServer

    @State private var showMenu = false
    @State private var isEditing = false

    private var entry: MCPCatalogEntry { server.catalogEntry() }
    private var state: MCPConnectionState { model.mcp.state(for: server.id) }
    private var connection: MCPConnection? { model.mcp.connection(for: server.id) }

    private var toolCount: Int {
        if case .connected(let count) = state { return count }
        return connection?.tools.count ?? 0
    }

    var body: some View {
        HStack(spacing: 12) {
            MCPIcon(entry: entry, size: 22)
                .frame(width: 26, alignment: .center)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: server.name)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                statusLine
            }
            Spacer(minLength: 12)
            action
            menuButton
        }
        .padding(.leading, 16)
        .padding(.trailing, Chrome.rowControlTrailingPadding)
        .padding(.vertical, 11)
    }

    @ViewBuilder
    private var statusLine: some View {
        switch state {
        case .connected:
            Text(verbatim: "\(toolCount) tools · \(connection?.serverVersion ?? entry.vendor)")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
        case .failed(let message):
            Text(verbatim: message)
                .font(.system(size: 11))
                .foregroundStyle(Chrome.danger)
                .lineLimit(3)
        default:
            Text(verbatim: server.summary)
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
        }
    }

    @ViewBuilder
    private var action: some View {
        switch state {
        case .notConnected:
            Button("Connect") { begin() }
                .buttonStyle(.glass)
                .controlSize(.small)
        case .connecting:
            HStack(spacing: 6) {
                WorkingSpinner(cellSize: 3)
                Text("Connecting")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .fixedSize()
            }
        case .connected:
            SettingsSwitch(isOn: Binding(
                get: { connection?.isEnabled ?? true },
                set: { model.mcp.setEnabled($0, for: entry) }
            ))
        case .needsSetup(let guide):
            Button("Fix it") { MCPSetupPanel.shared.present(entry: entry, guide: guide, model: model) }
                .buttonStyle(.glassProminent)
                .controlSize(.small)
        case .failed:
            Button("Try again") { begin() }
                .buttonStyle(.glass)
                .controlSize(.small)
        }
    }

    /// The card's connect, unchanged: no fields needed connects at once, otherwise the
    /// key panel takes the variable values.
    private func begin() {
        if !entry.needsInput {
            model.mcp.connect(entry)
        } else {
            MCPKeyPanel.shared.present(entry: entry, model: model)
        }
    }

    private var menuButton: some View {
        Button {
            showMenu.toggle()
        } label: {
            Image(systemName: "ellipsis.circle")
                .font(.system(size: 13))
                .foregroundStyle(Chrome.secondaryText)
                .frame(width: 22, height: 22)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .help("Server options")
        .popover(isPresented: $showMenu, arrowEdge: .bottom) {
            PopoverMenu {
                PopoverItem("Edit", symbol: "pencil") {
                    showMenu = false
                    isEditing = true
                }
                PopoverItem("Disconnect", symbol: "xmark.circle", isEnabled: state.isConnected, isDestructive: true) {
                    model.mcp.disconnect(entry)
                }
                PopoverDivider()
                PopoverItem("Delete", symbol: "trash", isDestructive: true) {
                    model.mcp.deleteCustom(server)
                }
            }
        }
        // On the button, not the row: hung from the row it opened in the row's middle.
        .popover(isPresented: $isEditing, arrowEdge: .bottom) {
            MCPCustomEditor(server: server)
                .presentedChrome()
        }
    }
}

/// The add/edit popover for a custom server: its name, kind, and how it runs, then the
/// *names* of the variables it needs — never their values. Values are typed on connect,
/// through the same key panel a catalog server's fields use (see `MCPKeyPanel`), or
/// edited here, where a locked one never leaves a secure field.
private struct MCPCustomEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    /// nil when adding; the server being edited otherwise.
    let server: MCPCustomServer?

    @State private var name: String
    @State private var kind: MCPCustomServer.Kind
    @State private var command: String
    @State private var argsText: String
    @State private var url: String
    @State private var variables: [VariableDraft]
    @State private var error: String?
    @State private var savedID: String?
    /// Focus lands on the name only when adding: in an edit it would read as a prompt
    /// to rename (the same call `CrewPairEditor` makes).
    @FocusState private var isNameFocused: Bool

    /// A variable row being typed. Its identity is a UUID, not the key: the key changes
    /// with every keystroke, which would throw the row out of the list mid-edit.
    private struct VariableDraft: Identifiable {
        let id = UUID()
        var key = ""
        var value = ""
        var isSecret = false

        init() {}
        init(_ variable: MCPCustomServer.Variable) {
            key = variable.key
            isSecret = variable.isSecret
        }
    }

    init(server: MCPCustomServer?) {
        self.server = server
        _name = State(initialValue: server?.name ?? "")
        _kind = State(initialValue: server?.kind ?? .stdio)
        _command = State(initialValue: server?.command ?? "")
        _argsText = State(initialValue: (server?.args ?? []).joined(separator: "\n"))
        _url = State(initialValue: server?.url ?? "")
        _variables = State(initialValue: (server?.variables ?? []).map(VariableDraft.init))
    }

    private static let width: CGFloat = 380
    private static let sectionSpacing: CGFloat = 12
    private static let outerPadding: CGFloat = 16
    private static let variableRowHeight: CGFloat = 34

    var body: some View {
        VStack(alignment: .leading, spacing: Self.sectionSpacing) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: Self.sectionSpacing) {
                    nameField
                    field("Kind") {
                        GlassPickerButton(
                            options: [(MCPCustomServer.Kind.stdio, "stdio — a command on this Mac"), (.http, "http — a remote server")],
                            selection: $kind,
                            symbol: { $0 == .stdio ? "terminal" : "network" }
                        )
                    }
                    if kind == .stdio {
                        field("Command") {
                            editorField("npx", text: $command)
                        }
                        argsEditor
                        variablesSection(title: "Environment", addLabel: "Add environment variable")
                    } else {
                        field("URL") {
                            editorField("https://…", text: $url)
                        }
                        variablesSection(title: "Headers", addLabel: "Add header")
                    }
                }
            }
            footer
        }
        .padding(Self.outerPadding)
        // The height follows the form: the arguments editor and each variable row claim
        // their space, header and save stay put, and the form scrolls inside rather than
        // stretching the popover off the screen (the lesson of the pair editor).
        .frame(width: Self.width, height: height)
        .onAppear {
            if server == nil {
                isNameFocused = true
            } else if let id = server?.id {
                // Editing starts from the stored values: the Keychain's for locked
                // variables, the connection's for the rest.
                for index in variables.indices {
                    let variable = variables[index]
                    if let stored = model.mcp.storedValue(server: id, field: variable.key, secret: variable.isSecret) {
                        variables[index].value = stored
                    }
                }
            }
        }
    }

    /// A bounded height responsive to the transport and the variable rows: roughly 450
    /// points for an empty stdio form, 320 for an empty http one, each row adding its
    /// field, and 560 the most the popover ever takes.
    private var height: CGFloat {
        let base: CGFloat = kind == .stdio ? 450 : 320
        return min(560, base + CGFloat(variables.count) * Self.variableRowHeight)
    }

    /// Error and save live below the scroll view: they never scroll out of reach, and
    /// the fields above them never sit in a tall empty panel.
    private var footer: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let error {
                Text(verbatim: error)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.danger)
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer(minLength: 0)
                Button(server == nil ? "Add server" : "Save") { save() }
                    .buttonStyle(.glassProminent)
            }
        }
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 10) {
            MCPIcon(entry: draft.catalogEntry(), size: 26)
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: server?.name ?? "New server")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                Text("Custom · \(kind == .stdio ? "stdio" : "http")")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
            }
            Spacer(minLength: 8)
            Button {
                dismiss()
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(width: 24, height: 24)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("Close"))
        }
    }

    private var nameField: some View {
        field("Name") {
            TextField("My server", text: $name)
                .textFieldStyle(.plain)
                .focused($isNameFocused)
                .focusEffectDisabled()
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(Chrome.primaryText)
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .mcpFieldSurface()
        }
    }

    /// A labeled editor row, the key panel's field shape: small caps-ish label over the control.
    private func field<Control: View>(_ label: String, @ViewBuilder control: () -> Control) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(verbatim: label)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Chrome.secondaryText)
            control()
        }
    }

    private func editorField(_ prompt: String, text: Binding<String>) -> some View {
        TextField(prompt, text: text)
            .textFieldStyle(.plain)
            .focusEffectDisabled()
            .font(.system(size: 12, design: .monospaced))
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .mcpFieldSurface()
    }

    /// The secret value's field, dressed like `editorField`.
    private func secureEditorField(_ prompt: String, text: Binding<String>) -> some View {
        SecureField(prompt, text: text)
            .textFieldStyle(.plain)
            .focusEffectDisabled()
            .font(.system(size: 12, design: .monospaced))
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .mcpFieldSurface()
    }

    private var argsEditor: some View {
        field("Arguments") {
            // The editor carries an inset of its own, so it needs less padding than the
            // single-line fields to put its text on the same edge.
            TextEditor(text: $argsText)
                .font(.system(size: 12, design: .monospaced))
                .scrollContentBackground(.hidden)
                .padding(.horizontal, 6)
                .padding(.vertical, 4)
                .frame(height: 66)
                .mcpFieldSurface()
            Text("One argument per line; empty lines are dropped.")
                .font(.system(size: 10.5))
                .foregroundStyle(Chrome.secondaryText)
        }
    }

    private func variablesSection(title: String, addLabel: String) -> some View {
        field(title) {
            ForEach($variables) { $variable in
                HStack(spacing: 6) {
                    editorField("KEY", text: $variable.key)
                    if variable.isSecret {
                        secureEditorField("value", text: $variable.value)
                    } else {
                        editorField("value", text: $variable.value)
                    }
                    Button {
                        variable.isSecret.toggle()
                    } label: {
                        Image(systemName: variable.isSecret ? "lock.fill" : "lock.open")
                            .font(.system(size: 12, weight: .medium))
                            .foregroundStyle(variable.isSecret ? Chrome.accent : Chrome.secondaryText)
                            .frame(width: 24, height: 24)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .help(variable.isSecret ? "Kept in the Keychain" : "Stored in mcp.json; click to keep it in the Keychain instead")
                    Button {
                        variables.removeAll { $0.id == variable.id }
                    } label: {
                        Image(systemName: "minus.circle")
                            .font(.system(size: 12, weight: .medium))
                            .foregroundStyle(Chrome.secondaryText)
                            .frame(width: 24, height: 24)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .help("Remove this variable")
                }
            }
            Button {
                variables.append(VariableDraft())
            } label: {
                Label(addLabel, systemImage: "plus")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Chrome.accent)
            }
            .buttonStyle(.plain)
            Text("Locked values are kept in the Keychain.")
                .font(.system(size: 10.5))
                .foregroundStyle(Chrome.secondaryText)
        }
    }

    /// The draft as a server, for the header's icon and for saving. Rows whose key was
    /// never filled are left out rather than failing validation on an empty key, and the
    /// variable list survives a kind switch: keys typed as env vars read fine as headers.
    private var draft: MCPCustomServer {
        let kept = variables
            .map { MCPCustomServer.Variable(key: $0.key.trimmingCharacters(in: .whitespaces), isSecret: $0.isSecret) }
            .filter { !$0.key.isEmpty }
        var draft = MCPCustomServer(id: server?.id ?? savedID ?? MCPCustomServer.makeID(name), name: name, kind: kind)
        draft.command = command.trimmingCharacters(in: .whitespacesAndNewlines)
        // One argument per line: an argument with spaces survives this way, and nothing
        // trims it, so a leading dash or an embedded quote reaches the server as typed.
        draft.args = argsText
            .components(separatedBy: "\n")
            .filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        draft.url = url.trimmingCharacters(in: .whitespacesAndNewlines)
        switch kind {
        case .stdio: draft.env = kept
        case .http: draft.headers = kept
        }
        return draft
    }

    private func save() {
        let draft = draft
        let wasConnected = server.map { model.mcp.state(for: $0.id).isConnected } ?? false
        if let failure = model.mcp.saveCustom(draft, replacing: server?.id ?? savedID) {
            error = failure.message
            return
        }
        savedID = draft.id
        // The values go where `connect` would put them, so connecting later is only the
        // probe: plain ones to mcp.json, locked ones to the Keychain. Keys trim and
        // empty-key rows drop exactly as `draft` keeps them, and a variable that changed
        // lock keeps its stored value (see `saveCustomValues`).
        let keptValues = variables
            .map { (key: $0.key.trimmingCharacters(in: .whitespaces), value: $0.value, isSecret: $0.isSecret) }
            .filter { !$0.key.isEmpty }
        guard model.mcp.saveCustomValues(keptValues, for: draft.id, replacingFieldsOf: server?.catalogEntry()) else {
            error = "The Keychain could not save a value. Keep this editor open and try again."
            return
        }
        // An edit that changed a connected server's command, address or variables takes
        // effect on the next probe; `saveCustom` dropped it from the providers meanwhile.
        if wasConnected, let entry = model.mcp.entry(for: draft.id) {
            model.mcp.refresh(entry)
        }
        dismiss()
    }
}

/// The inset surface every field in the custom-server editor sits on: the fill, radius
/// and edge padding of `CrewPairEditor`'s name field, so the two popovers read alike.
private extension View {
    func mcpFieldSurface() -> some View {
        background(Chrome.overlay(0.06), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
    }
}

/// The small `i` in a card's corner: hovering it opens the server's details, and so does a
/// click. The popover stays while the pointer is on it, so its docs link can be reached.
private struct MCPInfoButton: View {
    let entry: MCPCatalogEntry

    var body: some View {
        MCPHoverInfoButton(label: "About \(entry.name)") {
            MCPInfoPopover(entry: entry)
        }
    }
}

/// An `i` that opens `content` in a popover on hover or click, and keeps it open while
/// the pointer is on either the button or the popover.
private struct MCPHoverInfoButton<Content: View>: View {
    let label: String
    @ViewBuilder let content: Content

    @State private var isPresented = false
    @State private var isOverButton = false
    @State private var isOverPopover = false
    @State private var settle: Task<Void, Never>?

    var body: some View {
        Button {
            settle?.cancel()
            isPresented.toggle()
        } label: {
            Image(systemName: "info.circle")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(Chrome.secondaryText.opacity(isOverButton || isPresented ? 1 : 0.55))
                .frame(width: 22, height: 22)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .focusable(false)
        .accessibilityLabel(Text(verbatim: label))
        .onHover { over in
            isOverButton = over
            reconsider()
        }
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            content
                .onHover { over in
                    isOverPopover = over
                    reconsider()
                }
                .presentedChrome()
        }
    }

    /// Opens a beat after the pointer arrives, closes a beat after it has left both the
    /// button and the popover: the gap between the two is crossed without it closing.
    private func reconsider() {
        settle?.cancel()
        let wanted = isOverButton || isOverPopover
        guard wanted != isPresented else { return }
        settle = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(wanted ? 220 : 320))
            guard !Task.isCancelled, (isOverButton || isOverPopover) == wanted else { return }
            isPresented = wanted
        }
    }
}

/// A server's details: what it is, how it connects, what it needs, the tools it brings,
/// and where its docs are.
private struct MCPInfoPopover: View {
    let entry: MCPCatalogEntry
    @Environment(AppModel.self) private var model

    private static let width: CGFloat = 340

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .center, spacing: 10) {
                MCPIcon(entry: entry, size: 28)
                VStack(alignment: .leading, spacing: 1) {
                    Text(verbatim: entry.name)
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(Chrome.primaryText)
                    Text(verbatim: "\(entry.vendor) · \(entry.category.title)")
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                }
            }
            Text(verbatim: entry.summary)
                .font(.system(size: 12))
                .foregroundStyle(Chrome.primaryText.opacity(0.9))
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 8) {
                detail("Connects", connection)
                detail("Needs", needs)
            }
            if !entry.sampleTools.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Tools")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Chrome.secondaryText)
                    FlowChips(entry.sampleTools)
                }
            }
            if let optional = entry.fields.first(where: { !$0.isRequired }) {
                Button("Change the \(optional.label.lowercased())…") {
                    MCPKeyPanel.shared.present(entry: entry, model: model)
                }
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(Chrome.accent)
                .buttonStyle(.plain)
            }
            if let url = URL(string: entry.docsURL) {
                Link(destination: url) {
                    HStack(spacing: 5) {
                        Text("Read the docs")
                        Image(systemName: "arrow.up.right")
                            .font(.system(size: 9, weight: .bold))
                    }
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Chrome.accent)
                }
            }
        }
        .padding(16)
        .frame(width: Self.width, alignment: .leading)
    }

    private func detail(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(verbatim: label)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Chrome.secondaryText)
                .frame(width: 58, alignment: .leading)
            Text(verbatim: value)
                .font(.system(size: 11.5))
                .foregroundStyle(Chrome.primaryText.opacity(0.9))
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var connection: String {
        switch entry.transport {
        case .stdio(let command, let args):
            let package = args.first { !$0.hasPrefix("-") } ?? command
            return "Runs on this Mac through \(command) (\(package))."
        case .http(let url, _):
            return "Remote server at \(Self.host(of: url))."
        case .oauth(let url):
            return "Remote server at \(Self.host(of: entry.fill(url, values: [:]))). You sign in once in your browser."
        }
    }

    private var needs: String {
        let required = entry.fields.filter(\.isRequired).map(\.label)
        let optional = entry.fields.filter { !$0.isRequired }.map(\.label)
        if required.isEmpty && optional.isEmpty {
            return entry.isOAuth ? "Only your sign-in." : "Nothing — press Connect."
        }
        var parts = required
        if !optional.isEmpty { parts.append("optional: " + optional.joined(separator: ", ").lowercased()) }
        return parts.joined(separator: "; ") + "."
    }

    private static func host(of url: String) -> String {
        URL(string: url)?.host() ?? url
    }
}

/// Monospaced chips wrapping onto as many lines as they need.
private struct FlowChips: View {
    let items: [String]

    init(_ items: [String]) { self.items = items }

    var body: some View {
        FlowLayout(spacing: 6) {
            ForEach(items, id: \.self) { item in
                Text(verbatim: item)
                    .font(.system(size: 10.5, design: .monospaced))
                    .foregroundStyle(Chrome.primaryText.opacity(0.85))
                    .padding(.horizontal, 7)
                    .padding(.vertical, 3)
                    .background(Chrome.overlay(0.08), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            }
        }
    }
}

/// Rows of subviews, each as wide as it needs, wrapping at the proposed width.
private struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        var x: CGFloat = 0, y: CGFloat = 0, rowHeight: CGFloat = 0, maxX: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
            maxX = max(maxX, x - spacing)
        }
        return CGSize(width: proposal.width ?? maxX, height: y + rowHeight)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x: CGFloat = bounds.minX, y: CGFloat = bounds.minY, rowHeight: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > bounds.minX, x + size.width > bounds.maxX {
                x = bounds.minX
                y += rowHeight + spacing
                rowHeight = 0
            }
            subview.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
    }
}

struct MCPIcon: View {
    let entry: MCPCatalogEntry
    var size: CGFloat = 20

    var body: some View {
        image
            .resizable()
            .renderingMode(.template)
            .aspectRatio(contentMode: .fit)
            .foregroundStyle(entry.asset.isEmpty || entry.isMonochrome ? Chrome.primaryText : entry.color)
            .frame(width: size, height: size)
    }

    /// A custom server has no asset of its own (`asset == ""`): a symbol stands in for it.
    private var image: Image {
        entry.asset.isEmpty ? Image(systemName: "server.rack") : Image(entry.asset)
    }
}
