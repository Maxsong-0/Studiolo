// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The floating panel shown when a server refuses on its own side: the guide's
/// page opens in the browser as the panel appears, so the steps sit beside it.
@MainActor
final class MCPSetupPanel {
    static let shared = MCPSetupPanel()

    private var panel: NSPanel?
    private var escapeMonitor: Any?
    private var presentedEntryID: String?

    private init() {}

    /// True while the panel is up for this server: a re-check that fails again must not
    /// reopen the browser page or rebuild the steps under the user.
    func isPresenting(_ entry: MCPCatalogEntry) -> Bool {
        panel?.isVisible == true && presentedEntryID == entry.id
    }

    func present(entry: MCPCatalogEntry, guide: MCPSetupGuide, model: AppModel) {
        if isPresenting(entry) {
            panel?.orderFrontRegardless()
            return
        }
        presentedEntryID = entry.id
        if let url = URL(string: guide.pageURL) {
            NSWorkspace.shared.open(url)
        }
        let content = MCPSetupPanelView(entry: entry, guide: guide, model: model) { [weak self] in self?.close() }
            .presentedChrome()
            .environment(model)
        let height = NSHostingView(rootView: content.frame(width: 400)).fittingSize.height.rounded(.up)
        let panel = self.panel ?? makePanel()
        panel.contentViewController = NSHostingController(rootView: content)
        panel.setContentSize(NSSize(width: 400, height: max(height, 100)))
        if let visible = NSScreen.main?.visibleFrame {
            panel.setFrameOrigin(NSPoint(x: visible.maxX - 400 - 24, y: visible.maxY - panel.frame.height - 24))
        }
        startEscapeMonitor()
        panel.orderFrontRegardless()
    }

    func close() {
        stopEscapeMonitor()
        presentedEntryID = nil
        panel?.orderOut(nil)
    }

    /// Follows the content as it grows (the status line, the success view), keeping the
    /// top edge where it is.
    func fit(height: CGFloat) {
        guard let panel, panel.isVisible else { return }
        let wanted = max(height.rounded(.up), 100)
        guard abs(panel.frame.height - wanted) > 1 else { return }
        var frame = panel.frame
        frame.origin.y += frame.height - wanted
        frame.size.height = wanted
        panel.setFrame(frame, display: true)
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 200),
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

/// The panel's content: the reason, numbered steps, an open page button with
/// Check again, and the success view once the server connects.
struct MCPSetupPanelView: View {
    let entry: MCPCatalogEntry
    let guide: MCPSetupGuide
    let model: AppModel
    let onDone: @MainActor () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var checkedOnce = false
    @State private var successPopped = false

    private var state: MCPConnectionState { model.mcp.state(for: entry.id) }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            if state.isConnected {
                successView
            } else {
                Text(verbatim: guide.reason)
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.primaryText)
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(guide.steps.enumerated()), id: \.offset) { index, step in
                        stepRow(index + 1, text: Text(verbatim: step))
                    }
                }
                HStack {
                    Button(guide.pageLabel) {
                        if let url = URL(string: guide.pageURL) {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .buttonStyle(.glass)
                    Spacer()
                    if state.isConnecting {
                        HStack(spacing: 6) {
                            WorkingSpinner(cellSize: 3)
                            Text("Checking")
                                .font(.system(size: 11))
                                .foregroundStyle(Chrome.secondaryText)
                        }
                    } else {
                        Button("Check again") {
                            checkedOnce = true
                            model.mcp.refresh(entry)
                        }
                        .buttonStyle(.glassProminent)
                    }
                }
                if checkedOnce && !state.isConnecting {
                    if case .needsSetup = state {
                        Text(verbatim: "Still off on the server's side. " + (guide.note ?? ""))
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.warning)
                            .fixedSize(horizontal: false, vertical: true)
                    } else if case .failed(let message) = state {
                        Text(verbatim: message)
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.danger)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .padding(18)
        .frame(width: 400)
        // The steps sit beside the browser page the panel opened, and the page is behind
        // whichever half of the screen the panel lands on: a press anywhere its controls do
        // not claim drags it clear, the way the window's own chrome does. The panel's own
        // steps, reason and status line are not controls, so they drag with the background.
        .background {
            WindowDragArea()
        }
        .background {
            WindowBackdrop(opacity: 0.5)
        }
        .clipShape(RoundedRectangle(cornerRadius: Chrome.windowCornerRadius, style: .continuous))
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height in
            MCPSetupPanel.shared.fit(height: height)
        }
        .onChange(of: state.isConnected) { _, isConnected in
            if isConnected {
                Task { @MainActor in
                    try? await Task.sleep(for: .seconds(1.2))
                    onDone()
                }
            }
        }
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 10) {
            MCPIcon(entry: entry, size: 26)
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: entry.name)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                Text(verbatim: guide.title)
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
                    .fixedSize(horizontal: false, vertical: true)
                if let accessory { accessory }
            }
        }
    }

    private var successView: some View {
        VStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 34))
                .foregroundStyle(Chrome.success)
                .scaleEffect(successPopped ? 1 : 0.5)
            let count: Int = {
                if case .connected(let count) = state { return count }
                return 0
            }()
            Text(verbatim: "Connected · \(count) tools")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(Chrome.primaryText)
        }
        .frame(maxWidth: .infinity, minHeight: 120, alignment: .center)
        .onAppear {
            if reduceMotion {
                successPopped = true
            } else {
                withAnimation(.spring(duration: 0.5, bounce: 0.4)) { successPopped = true }
            }
        }
    }
}
