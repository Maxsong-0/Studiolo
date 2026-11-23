// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The floating panel a user pastes an MCP key into. Opens the entry's `keysURL`
/// in the browser as it appears, so the key page sits beside the panel.
@MainActor
final class MCPKeyPanel {
    static let shared = MCPKeyPanel()

    private var panel: NSPanel?
    private var escapeMonitor: Any?

    private init() {}

    func present(entry: MCPCatalogEntry, model: AppModel) {
        if let keysURL = entry.keysURL, let url = URL(string: keysURL) {
            NSWorkspace.shared.open(url)
        }
        let content = MCPKeyPanelView(entry: entry, model: model) { [weak self] in self?.close() }
            .presentedChrome()
            .environment(model)
        let height = NSHostingView(rootView: content.frame(width: 380)).fittingSize.height.rounded(.up)
        let panel = self.panel ?? makePanel()
        panel.contentViewController = NSHostingController(rootView: content)
        panel.setContentSize(NSSize(width: 380, height: max(height, 100)))
        if let visible = NSScreen.main?.visibleFrame {
            panel.setFrameOrigin(NSPoint(x: visible.maxX - 380 - 24, y: visible.maxY - panel.frame.height - 24))
        }
        startEscapeMonitor()
        panel.orderFrontRegardless()
    }

    func close() {
        stopEscapeMonitor()
        panel?.orderOut(nil)
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 380, height: 200),
            styleMask: [.nonactivatingPanel, .fullSizeContentView, .titled, .closable],
            backing: .buffered,
            defer: false
        )
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.level = .floating
        panel.hidesOnDeactivate = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        // No traffic lights: the panel closes from its own xmark and on Escape, and the
        // close button sat over the content in a corner that has no title bar to hold it.
        for button in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            panel.standardWindowButton(button)?.isHidden = true
        }
        self.panel = panel
        return panel
    }

    private func startEscapeMonitor() {
        guard escapeMonitor == nil else { return }
        escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, self.panel?.isVisible == true, event.keyCode == 53 else { return event }
            self.close()
            return nil
        }
    }

    private func stopEscapeMonitor() {
        if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) }
        escapeMonitor = nil
    }
}

/// The panel's content: numbered steps, one field per entry field, and Save.
struct MCPKeyPanelView: View {
    let entry: MCPCatalogEntry
    let model: AppModel
    let onDone: @MainActor () -> Void

    @Environment(AppModel.self) private var envModel
    @FocusState private var focusedField: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            steps
            ForEach(entry.fields) { field in
                fieldView(field)
            }
            HStack {
                Spacer()
                Button(entry.isOAuth ? "Sign in" : "Save and connect") {
                    save()
                }
                .buttonStyle(.glassProminent)
                .disabled(!model.mcp.canConnect(entry))
            }
        }
        .padding(18)
        .frame(width: 380)
        // The key page opens in the browser as the panel appears, and the panel has to be
        // movable off it: a press anywhere the fields and buttons do not claim drags the
        // panel, as it does in the setup guide's panel.
        .background {
            WindowDragArea()
        }
        .background {
            WindowBackdrop(opacity: 0.5)
        }
        .clipShape(RoundedRectangle(cornerRadius: Chrome.windowCornerRadius, style: .continuous))
        .onAppear { focusedField = entry.fields.first?.key }
        .onSubmit { if model.mcp.canConnect(entry) { save() } }
        .onChange(of: model.mcp.state(for: entry.id).isConnected) { _, isConnected in
            if isConnected { onDone() }
        }
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 10) {
            MCPIcon(entry: entry, size: 26)
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: entry.name)
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                Text("Connect")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
            }
            Spacer(minLength: 8)
            Button {
                onDone()
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Chrome.secondaryText)
            }
            .buttonStyle(.plain)
        }
    }

    @ViewBuilder
    private var steps: some View {
        if let keysURL = entry.keysURL, let url = URL(string: keysURL) {
            VStack(alignment: .leading, spacing: 6) {
                stepRow(1, text: Text("The page is open in your browser"), accessory: AnyView(
                    Link("Open it again", destination: url).font(.system(size: 11))
                ))
                stepRow(2, text: Text("Copy the \(entry.fields.first?.label.lowercased() ?? "key")"))
                stepRow(3, text: Text("Paste it below and save"))
            }
        } else {
            VStack(alignment: .leading, spacing: 6) {
                stepRow(1, text: Text("Enter the \(entry.fields.first?.label.lowercased() ?? "value")"))
                stepRow(2, text: Text(entry.isOAuth ? "Sign in in your browser" : "Save"))
            }
        }
    }

    private func stepRow(_ number: Int, text: Text, accessory: AnyView? = nil) -> some View {
        HStack(spacing: 8) {
            ZStack {
                Circle()
                    .fill(Chrome.accent.opacity(0.18))
                    .frame(width: 18, height: 18)
                Text(verbatim: "\(number)")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(Chrome.accent)
            }
            HStack(spacing: 4) {
                text
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.primaryText)
                if let accessory { accessory }
            }
        }
    }

    @ViewBuilder
    private func fieldView(_ field: MCPField) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(verbatim: field.label)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Chrome.secondaryText)
            HStack(spacing: 6) {
                Group {
                    switch field.kind {
                    case .secret:
                        SecureField(field.placeholder, text: draftBinding(field))
                    case .text, .path:
                        TextField(field.placeholder, text: draftBinding(field))
                    }
                }
                .textFieldStyle(.plain)
                .focusEffectDisabled()
                .font(.system(size: 12, design: .monospaced))
                .focused($focusedField, equals: field.key)
                if field.kind == .path {
                    Button {
                        pickPath(for: field)
                    } label: {
                        Image(systemName: "folder")
                            .font(.system(size: 12))
                            .foregroundStyle(Chrome.secondaryText)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 7)
            .background {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(Chrome.overlay(0.06))
            }
            if let help = field.help, !help.isEmpty {
                Text(verbatim: help)
                    .font(.system(size: 10.5))
                    .foregroundStyle(Chrome.secondaryText)
            }
        }
    }

    private func draftBinding(_ field: MCPField) -> Binding<String> {
        Binding(
            get: { model.mcp.value(for: field, of: entry) },
            set: { model.mcp.setDraft($0, for: field, of: entry) }
        )
    }

    private func pickPath(for field: MCPField) {
        let open = NSOpenPanel()
        if entry.id == "filesystem" {
            open.canChooseDirectories = true
            open.canChooseFiles = false
        } else {
            open.canChooseDirectories = false
            open.canChooseFiles = true
        }
        open.allowsMultipleSelection = false
        guard open.runModal() == .OK, let url = open.url else { return }
        model.mcp.setDraft(url.path, for: field, of: entry)
    }

    private func save() {
        model.mcp.connect(entry)
        onDone()
    }
}
